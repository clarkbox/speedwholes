//! Resize timing statistics tracking.
//!
//! Tracks average resize times per subdomain per template to monitor preset health.
//! Stats are stored in memory and cleaned up lazily via garbage collection.

use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Maximum age for entries before GC removes them (1 hour)
const ENTRY_MAX_AGE_SECS: u64 = 3600;

/// GC runs if this many seconds have passed since last cleanup
const GC_INTERVAL_SECS: u64 = 300; // 5 minutes

/// Maximum number of subdomains to track (LRU eviction beyond this)
const MAX_SUBDOMAINS: usize = 1000;

/// Maximum templates per subdomain
const MAX_TEMPLATES_PER_SUBDOMAIN: usize = 100;

/// Per-template timing entry
struct TemplateTimingEntry {
    count: u64,
    total_ms: f64,
    last_updated: Instant,
}

impl TemplateTimingEntry {
    fn new(duration_ms: f64) -> Self {
        Self {
            count: 1,
            total_ms: duration_ms,
            last_updated: Instant::now(),
        }
    }

    fn record(&mut self, duration_ms: f64) {
        self.count += 1;
        self.total_ms += duration_ms;
        self.last_updated = Instant::now();
    }

    fn avg_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.total_ms / self.count as f64
        }
    }

    fn is_stale(&self, max_age: Duration) -> bool {
        self.last_updated.elapsed() > max_age
    }
}

/// Serializable template stats for API response
#[derive(Debug, Clone, Serialize)]
pub struct TemplateStats {
    pub avg_ms: f64,
    pub count: u64,
}

/// Serializable subdomain stats for API response
#[derive(Debug, Clone, Serialize)]
pub struct SubdomainStats {
    pub templates: HashMap<String, TemplateStats>,
}

/// Full stats snapshot for API response
#[derive(Debug, Clone, Serialize)]
pub struct ResizeStatsSnapshot {
    pub subdomains: HashMap<String, SubdomainStats>,
    pub gc_interval_secs: u64,
    pub entry_max_age_secs: u64,
}

/// Thread-safe resize timing statistics tracker
pub struct ResizeStats {
    /// Subdomain -> Template -> Timing data
    data: Mutex<HashMap<String, HashMap<String, TemplateTimingEntry>>>,
    /// Last GC run time
    last_gc: Mutex<Instant>,
}

impl Default for ResizeStats {
    fn default() -> Self {
        Self::new()
    }
}

impl ResizeStats {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
            last_gc: Mutex::new(Instant::now()),
        }
    }

    /// Record a resize timing measurement.
    ///
    /// - `subdomain`: The subdomain name (or "_default" for no subdomain)
    /// - `template`: The template name (or "_direct" for non-template requests)
    /// - `duration_ms`: The resize duration in milliseconds
    pub fn record_timing(&self, subdomain: &str, template: &str, duration_ms: f64) {
        let mut data = self.data.lock();

        // Get or create subdomain entry
        let subdomain_entry = data.entry(subdomain.to_string()).or_default();

        // Enforce max templates per subdomain
        if subdomain_entry.len() >= MAX_TEMPLATES_PER_SUBDOMAIN
            && !subdomain_entry.contains_key(template)
        {
            // Remove oldest template
            if let Some(oldest_key) = subdomain_entry
                .iter()
                .min_by_key(|(_, v)| v.last_updated)
                .map(|(k, _)| k.clone())
            {
                subdomain_entry.remove(&oldest_key);
            }
        }

        // Record timing
        subdomain_entry
            .entry(template.to_string())
            .and_modify(|e| e.record(duration_ms))
            .or_insert_with(|| TemplateTimingEntry::new(duration_ms));

        // Enforce max subdomains (after insertion to ensure current one is kept)
        if data.len() > MAX_SUBDOMAINS {
            // Find oldest subdomain (by oldest template update)
            if let Some(oldest_subdomain) = data
                .iter()
                .filter(|(k, _)| *k != subdomain) // Don't evict the one we just updated
                .min_by_key(|(_, templates)| {
                    templates
                        .values()
                        .map(|t| t.last_updated)
                        .min()
                        .unwrap_or(Instant::now())
                })
                .map(|(k, _)| k.clone())
            {
                data.remove(&oldest_subdomain);
                tracing::debug!(subdomain = %oldest_subdomain, "Evicted stale subdomain stats (LRU)");
            }
        }
    }

    /// Check if GC should run and run it if needed.
    /// Called lazily after each request.
    pub fn maybe_gc(&self) {
        let should_gc = {
            let last_gc = self.last_gc.lock();
            last_gc.elapsed() > Duration::from_secs(GC_INTERVAL_SECS)
        };

        if should_gc {
            self.gc_stale_entries();
            *self.last_gc.lock() = Instant::now();
        }
    }

    /// Remove entries older than max age
    fn gc_stale_entries(&self) {
        let max_age = Duration::from_secs(ENTRY_MAX_AGE_SECS);
        let mut data = self.data.lock();
        let mut removed_templates = 0;

        // Remove stale templates from each subdomain
        for templates in data.values_mut() {
            let before = templates.len();
            templates.retain(|_, entry| !entry.is_stale(max_age));
            removed_templates += before - templates.len();
        }

        // Remove empty subdomains
        let before = data.len();
        data.retain(|_, templates| !templates.is_empty());
        let removed_subdomains = before - data.len();

        if removed_templates > 0 || removed_subdomains > 0 {
            tracing::info!(
                removed_templates = removed_templates,
                removed_subdomains = removed_subdomains,
                "Resize stats GC completed"
            );
        }
    }

    /// Get a snapshot of all stats for API response
    pub fn get_snapshot(&self) -> ResizeStatsSnapshot {
        let data = self.data.lock();

        let subdomains = data
            .iter()
            .map(|(subdomain, templates)| {
                let template_stats = templates
                    .iter()
                    .map(|(name, entry)| {
                        (
                            name.clone(),
                            TemplateStats {
                                avg_ms: entry.avg_ms(),
                                count: entry.count,
                            },
                        )
                    })
                    .collect();

                (
                    subdomain.clone(),
                    SubdomainStats {
                        templates: template_stats,
                    },
                )
            })
            .collect();

        ResizeStatsSnapshot {
            subdomains,
            gc_interval_secs: GC_INTERVAL_SECS,
            entry_max_age_secs: ENTRY_MAX_AGE_SECS,
        }
    }

    /// Reset all stats
    pub fn reset_all(&self) {
        let mut data = self.data.lock();
        data.clear();
        tracing::info!("Reset all resize stats");
    }

    /// Reset stats for a specific subdomain
    pub fn reset_subdomain(&self, subdomain: &str) -> bool {
        let mut data = self.data.lock();
        let removed = data.remove(subdomain).is_some();
        if removed {
            tracing::info!(subdomain = %subdomain, "Reset resize stats for subdomain");
        }
        removed
    }
}
