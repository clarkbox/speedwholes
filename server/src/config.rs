use clap::Parser;
use lru::LruCache;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::resize_stats::ResizeStats;

/// Log output format
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum LogFormat {
    /// JSON format (structured, one object per line) - best for production/log aggregation
    #[default]
    Json,
    /// Human-readable text format - best for development/debugging
    Text,
}

/// CLI subcommands
#[derive(clap::Subcommand, Debug, Clone)]
pub enum Command {
    /// Run interactive configuration setup
    Init,
    /// Restart the speedwholes systemd service
    Restart,
    /// Stop the speedwholes systemd service
    Stop,
    /// Show status of the speedwholes systemd service
    Status,
}

/// CLI arguments for speedwholes
#[derive(Parser, Debug, Clone)]
#[command(name = "speedwholes")]
#[command(about = "A fast image resizing and caching HTTP service")]
pub struct CliArgs {
    /// Subcommand to run (if any)
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Override port from config file
    #[arg(long, short = 'p')]
    pub port: Option<u16>,

    /// Path to configuration directory (contains config files and PID lock)
    #[arg(long, default_value = "/etc/speedwholes")]
    pub config_dir: PathBuf,

    /// Override cache path from config file
    #[arg(long)]
    pub cache_path: Option<PathBuf>,

    /// Log output format: json (structured) or text (human-readable)
    #[arg(long, default_value = "json", value_enum)]
    pub log_format: LogFormat,
}

impl CliArgs {
    pub fn parse_args() -> Self {
        CliArgs::parse()
    }
}

// ============================================================================
// Default value functions for serde
// ============================================================================

fn default_unknown_host_policy() -> String {
    "reject".to_string()
}

fn default_port() -> u16 {
    3000
}
fn default_cache_path() -> String {
    "./cache".to_string()
}
fn default_max_dimension() -> u32 {
    4096
}
fn default_timeout() -> u64 {
    30
}
fn default_max_source_pixels() -> u64 {
    100_000_000
}
fn default_max_source_bytes() -> u64 {
    52_428_800
}
fn default_max_concurrent_resizes() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 8)
}
fn default_pool_max_idle() -> usize {
    8
}
fn default_disk_min_free_mb() -> u64 {
    500
}
fn default_disk_check_interval_secs() -> u64 {
    60
}

// ============================================================================
// Main configuration struct - mirrors the JSON config file
// ============================================================================

/// Server configuration loaded from speedwholes.config.json
/// Uses #[serde(default)] for all fields so missing values get sensible defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Port to listen on
    #[serde(default = "default_port")]
    pub port: u16,

    /// Path to disk cache directory
    #[serde(default = "default_cache_path")]
    pub cache_path: String,

    /// Maximum resize width
    #[serde(default = "default_max_dimension")]
    pub max_width: u32,

    /// Maximum resize height
    #[serde(default = "default_max_dimension")]
    pub max_height: u32,

    /// Request timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout: u64,

    /// Maximum source image pixels
    #[serde(default = "default_max_source_pixels")]
    pub max_source_pixels: u64,

    /// Maximum upstream response body, in bytes. Default 50 MiB.
    #[serde(default = "default_max_source_bytes")]
    pub max_source_bytes: u64,

    /// How many resizes may decode and encode at once. Default is `min(cpus, 8)`.
    #[serde(default = "default_max_concurrent_resizes")]
    pub max_concurrent_resizes: usize,

    /// Allowed source URL patterns
    #[serde(default)]
    pub allowed_sources: Vec<String>,

    /// Allow any public host for tenants that do not set their own allowlist.
    /// Private and reserved addresses are still blocked. Default false.
    #[serde(default)]
    pub allow_all_sources: bool,

    /// Allow private, loopback, and reserved addresses. For development,
    /// benchmarks, and tests. Default false, including when allow_all_sources is set.
    #[serde(default)]
    pub allow_private_networks: bool,

    /// Hosts with no matching tenant file. `"reject"` (default) returns 404.
    /// `"global"` keeps the old behaviour: global limits and allowlist, unsigned.
    #[serde(default = "default_unknown_host_policy")]
    pub unknown_host_policy: String,

    /// When true, `Forwarded` and `X-Forwarded-Host` select the tenant.
    /// Default false: only the `Host` header (or the URI authority) is used.
    #[serde(default)]
    pub trust_forwarded_host: bool,

    /// Enable compression for source image downloads (gzip/brotli)
    #[serde(default)]
    pub source_compression: bool,

    /// Max idle connections per host in the HTTP client pool
    #[serde(default = "default_pool_max_idle")]
    pub pool_max_idle_per_host: usize,

    /// Admin API token (None = endpoints unprotected)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_token: Option<String>,

    /// TLS certificate file path (PEM format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_cert: Option<String>,

    /// TLS private key file path (PEM format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_key: Option<String>,

    /// Minimum free disk space in MB before cache eviction starts (0 = disabled, default: 500)
    #[serde(default = "default_disk_min_free_mb")]
    pub disk_min_free_mb: u64,

    /// How often to check disk space in seconds (default: 60)
    #[serde(default = "default_disk_check_interval_secs")]
    pub disk_check_interval_secs: u64,

    /// Subdomain configurations (loaded from subdomainConfig/ folder, not serialized)
    #[serde(skip)]
    pub subdomains: Vec<SubdomainConfig>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            cache_path: default_cache_path(),
            max_width: default_max_dimension(),
            max_height: default_max_dimension(),
            timeout: default_timeout(),
            max_source_pixels: default_max_source_pixels(),
            max_source_bytes: default_max_source_bytes(),
            max_concurrent_resizes: default_max_concurrent_resizes(),
            allowed_sources: Vec::new(),
            allow_all_sources: false,
            allow_private_networks: false,
            unknown_host_policy: default_unknown_host_policy(),
            trust_forwarded_host: false,
            source_compression: false,
            pool_max_idle_per_host: default_pool_max_idle(),
            admin_token: None,
            tls_cert: None,
            tls_key: None,
            disk_min_free_mb: default_disk_min_free_mb(),
            disk_check_interval_secs: default_disk_check_interval_secs(),
            subdomains: Vec::new(),
        }
    }
}

impl ServerConfig {
    /// Load config from file, returning defaults if file doesn't exist
    pub fn load(config_dir: &std::path::Path) -> Self {
        let config_file = config_dir.join("speedwholes.config.json");

        let mut config = match std::fs::read_to_string(&config_file) {
            Ok(contents) => match serde_json::from_str::<ServerConfig>(&contents) {
                Ok(config) => {
                    tracing::info!("Loaded config from {:?}", config_file);
                    config
                }
                Err(e) => {
                    tracing::warn!("Failed to parse config file: {}, using defaults", e);
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!("No config file found at {:?}, using defaults", config_file);
                Self::default()
            }
            Err(e) => {
                tracing::warn!("Failed to read config file: {}, using defaults", e);
                Self::default()
            }
        };

        // Load subdomain configs from subdomain folder
        config.subdomains = Self::load_all_subdomains(config_dir);
        config
    }

    /// Load all subdomain configs from the subdomain folder
    fn load_all_subdomains(config_dir: &std::path::Path) -> Vec<SubdomainConfig> {
        let dir_path = config_dir.join("subdomainConfig");

        if !dir_path.exists() {
            tracing::debug!(
                "Subdomain config directory doesn't exist yet: {:?}",
                dir_path
            );
            return Vec::new();
        }

        let entries = match std::fs::read_dir(&dir_path) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to read subdomain config directory: {}", e);
                return Vec::new();
            }
        };

        let mut subdomains = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                match std::fs::read_to_string(&path) {
                    Ok(contents) => match serde_json::from_str::<SubdomainConfig>(&contents) {
                        Ok(config) => {
                            tracing::debug!("Loaded subdomain config: {}", config.name);
                            subdomains.push(config);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse subdomain config {:?}: {}", path, e);
                        }
                    },
                    Err(e) => {
                        tracing::warn!("Failed to read subdomain config {:?}: {}", path, e);
                    }
                }
            }
        }

        tracing::info!(
            "Loaded {} subdomain configs from {:?}",
            subdomains.len(),
            dir_path
        );
        subdomains
    }

    /// Save a single subdomain config to its own file
    pub fn save_subdomain(
        subdomain: &SubdomainConfig,
        config_dir: &std::path::Path,
    ) -> Result<(), std::io::Error> {
        let subdomain_config_dir = config_dir.join("subdomainConfig");
        std::fs::create_dir_all(&subdomain_config_dir)?;

        let safe_name = sanitize_filename(&subdomain.name);
        let file_path = subdomain_config_dir.join(format!("{}.json", safe_name));
        let contents = serde_json::to_string_pretty(subdomain)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&file_path, contents)?;
        tracing::info!(
            "Saved subdomain config '{}' to {:?}",
            subdomain.name,
            file_path
        );
        Ok(())
    }

    /// Delete a subdomain config file
    pub fn delete_subdomain(
        subdomain_name: &str,
        config_dir: &std::path::Path,
    ) -> Result<(), std::io::Error> {
        let subdomain_config_dir = config_dir.join("subdomainConfig");
        let safe_name = sanitize_filename(subdomain_name);
        let file_path = subdomain_config_dir.join(format!("{}.json", safe_name));

        match std::fs::remove_file(&file_path) {
            Ok(_) => {
                tracing::info!("Deleted subdomain config file: {:?}", file_path);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(
                    "Subdomain config file not found (already deleted?): {:?}",
                    file_path
                );
                Ok(())
            }
            Err(e) => {
                tracing::error!(
                    "Failed to delete subdomain config file {:?}: {}",
                    file_path,
                    e
                );
                Err(e)
            }
        }
    }
}

/// Sanitize a name for use as a filename by replacing invalid characters with underscores.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ============================================================================
// Config response for API endpoint
// ============================================================================

/// Config response for GET /config007/json
#[derive(Debug, Clone, Serialize)]
pub struct ConfigResponse {
    pub port: u16,
    pub max_width: u32,
    pub max_height: u32,
    pub timeout: u64,
    pub max_source_pixels: u64,
    pub max_source_bytes: u64,
    pub max_concurrent_resizes: usize,
    pub allowed_sources: Vec<String>,
    pub allow_all_sources: bool,
    pub allow_private_networks: bool,
    pub unknown_host_policy: String,
    pub trust_forwarded_host: bool,
    pub source_compression: bool,
    pub pool_max_idle_per_host: usize,
}

impl From<&ServerConfig> for ConfigResponse {
    fn from(config: &ServerConfig) -> Self {
        Self {
            port: config.port,
            max_width: config.max_width,
            max_height: config.max_height,
            timeout: config.timeout,
            max_source_pixels: config.max_source_pixels,
            max_source_bytes: config.max_source_bytes,
            max_concurrent_resizes: config.max_concurrent_resizes,
            allowed_sources: config.allowed_sources.clone(),
            allow_all_sources: config.allow_all_sources,
            allow_private_networks: config.allow_private_networks,
            unknown_host_policy: config.unknown_host_policy.clone(),
            trust_forwarded_host: config.trust_forwarded_host,
            source_compression: config.source_compression,
            pool_max_idle_per_host: config.pool_max_idle_per_host,
        }
    }
}

// ============================================================================
// Stats tracking
// ============================================================================

/// In-memory stats since app start
pub struct Stats {
    pub image_requests: AtomicU64,
    /// Resize cache hits (final resized image found in cache)
    pub cache_hits: AtomicU64,
    /// Resize cache misses (need to resize)
    pub cache_misses: AtomicU64,
    /// Source cache hits (original image found, avoided network fetch)
    pub source_cache_hits: AtomicU64,
    /// Source cache misses (need to fetch from network)
    pub source_cache_misses: AtomicU64,
    pub signature_cache_hits: AtomicU64,
    pub signature_cache_misses: AtomicU64,
    pub last_subdomain: Mutex<Option<String>>,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            image_requests: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            source_cache_hits: AtomicU64::new(0),
            source_cache_misses: AtomicU64::new(0),
            signature_cache_hits: AtomicU64::new(0),
            signature_cache_misses: AtomicU64::new(0),
            last_subdomain: Mutex::new(None),
        }
    }
}

impl Stats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_request(&self) {
        self.image_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_source_cache_hit(&self) {
        self.source_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_source_cache_miss(&self) {
        self.source_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_signature_cache_hit(&self) {
        self.signature_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_signature_cache_miss(&self) {
        self.signature_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_subdomain(&self, subdomain: Option<String>) {
        *self.last_subdomain.lock() = subdomain;
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            image_requests: self.image_requests.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            source_cache_hits: self.source_cache_hits.load(Ordering::Relaxed),
            source_cache_misses: self.source_cache_misses.load(Ordering::Relaxed),
            signature_cache_hits: self.signature_cache_hits.load(Ordering::Relaxed),
            signature_cache_misses: self.signature_cache_misses.load(Ordering::Relaxed),
            last_subdomain: self.last_subdomain.lock().clone(),
        }
    }
}

/// Serializable stats snapshot
#[derive(Debug, Clone, Serialize)]
pub struct StatsSnapshot {
    pub image_requests: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub source_cache_hits: u64,
    pub source_cache_misses: u64,
    pub signature_cache_hits: u64,
    pub signature_cache_misses: u64,
    pub last_subdomain: Option<String>,
}

// ============================================================================
// Signature cache
// ============================================================================

const DEFAULT_SIGNATURE_CACHE_SIZE: usize = 10_000;

/// LRU cache for verified signatures
pub struct SignatureCache {
    cache: Mutex<LruCache<String, bool>>,
}

impl SignatureCache {
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity)
            .unwrap_or(NonZeroUsize::new(DEFAULT_SIGNATURE_CACHE_SIZE).unwrap());
        Self {
            cache: Mutex::new(LruCache::new(cap)),
        }
    }

    fn make_key(subdomain: &str, signing_path: &str, signature: &str) -> String {
        format!(
            "{}:{}:{}:{}",
            subdomain.len(),
            subdomain,
            signing_path,
            signature
        )
    }

    pub fn get(&self, subdomain: &str, signing_path: &str, signature: &str) -> Option<bool> {
        let key = Self::make_key(subdomain, signing_path, signature);
        self.cache.lock().get(&key).copied()
    }

    pub fn put(&self, subdomain: &str, signing_path: &str, signature: &str, is_valid: bool) {
        if !is_valid {
            return;
        }
        let key = Self::make_key(subdomain, signing_path, signature);
        self.cache.lock().put(key, is_valid);
    }

    pub fn invalidate_subdomain(&self, subdomain: &str) {
        let prefix = format!("{}:{}:", subdomain.len(), subdomain);
        let mut cache = self.cache.lock();
        let keys_to_remove: Vec<String> = cache
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, _)| k.clone())
            .collect();
        let invalidated_count = keys_to_remove.len();
        for key in keys_to_remove {
            cache.pop(&key);
        }
        if invalidated_count > 0 {
            tracing::info!(
                subdomain = %subdomain,
                invalidated = invalidated_count,
                "Invalidated signature cache entries"
            );
        }
    }
}

// ============================================================================
// Subdomain and template configuration
// ============================================================================

/// Resize template - preset parameters that can be referenced by name via /t/name/
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResizeTemplate {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_template: Option<String>,
}

/// Configuration for a specific subdomain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubdomainConfig {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_key: Option<String>,
    #[serde(default)]
    pub signing_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_sources: Option<Vec<String>>,
    /// `None` and `Some(true)` allow `/b/1`. `Some(false)` rejects bypass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bypass: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub templates: Option<Vec<ResizeTemplate>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_template: Option<String>,
}

// ============================================================================
// TLS configuration
// ============================================================================

/// TLS configuration for HTTPS
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

// ============================================================================
// Session store for admin auth
// ============================================================================

/// In-memory session store for admin authentication.
pub struct SessionStore {
    sessions: Mutex<HashMap<String, std::time::Instant>>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn create_session(&self) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        self.sessions
            .lock()
            .insert(token.clone(), std::time::Instant::now());
        tracing::info!("Created new admin session");
        token
    }

    pub fn validate_session(&self, token: &str) -> bool {
        self.sessions.lock().contains_key(token)
    }

    pub fn remove_session(&self, token: &str) -> bool {
        let removed = self.sessions.lock().remove(token).is_some();
        if removed {
            tracing::info!("Admin session removed");
        }
        removed
    }
}

// ============================================================================
// Application state
// ============================================================================

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub port: u16,
    pub config_dir: PathBuf,
    pub cache_path: PathBuf,
    pub config: Arc<ServerConfig>,
    pub subdomain_configs: Arc<RwLock<Vec<SubdomainConfig>>>,
    pub http_client: reqwest::Client,
    pub stats: Arc<Stats>,
    pub signature_cache: Arc<SignatureCache>,
    pub resize_stats: Arc<ResizeStats>,
    pub admin_token: Option<String>,
    pub sessions: Arc<SessionStore>,
    pub tls_config: TlsConfig,
    pub resize_permits: Arc<tokio::sync::Semaphore>,
    /// Identical in-flight resize requests share one result.
    pub inflight: Arc<crate::inflight::Inflight>,
}

impl AppState {
    pub fn new(cli: CliArgs) -> Self {
        // Load config from file
        let mut config = ServerConfig::load(&cli.config_dir);

        // CLI overrides (only port and cache_path)
        if let Some(port) = cli.port {
            config.port = port;
        }
        if let Some(ref cache_path) = cli.cache_path {
            config.cache_path = cache_path.to_string_lossy().to_string();
        }

        let port = config.port;
        let cache_path = PathBuf::from(&config.cache_path);

        // Log config
        tracing::info!(port = port, cache = ?cache_path, "Configuration loaded");

        let tls_config = Self::validate_tls(&config);
        let config_dir = cli.config_dir;
        Self::from_config(config, config_dir, tls_config)
    }

    /// Check that both TLS paths are set and the files exist. Exits with the
    /// setup hint when they are not. Does not build the rest of the process state.
    fn validate_tls(config: &ServerConfig) -> TlsConfig {
        match (&config.tls_cert, &config.tls_key) {
            (Some(cert), Some(key)) => {
                let cert_path = PathBuf::from(cert);
                let key_path = PathBuf::from(key);

                if !cert_path.exists() {
                    tracing::error!("TLS certificate file not found: {:?}", cert_path);
                    Self::exit_with_ssl_help();
                }
                if !key_path.exists() {
                    tracing::error!("TLS private key file not found: {:?}", key_path);
                    Self::exit_with_ssl_help();
                }

                tracing::info!(cert = ?cert_path, key = ?key_path, "TLS configured");
                TlsConfig {
                    cert_path,
                    key_path,
                }
            }
            _ => {
                tracing::error!("TLS not configured (both tls_cert and tls_key required)");
                Self::exit_with_ssl_help();
            }
        }
    }

    /// Build process state from an already-loaded config and validated TLS paths.
    /// Callers that have already checked the certificate files (or that are tests
    /// and never open them) use this directly.
    pub(crate) fn from_config(
        config: ServerConfig,
        config_dir: PathBuf,
        tls_config: TlsConfig,
    ) -> Self {
        let port = config.port;
        let cache_path = PathBuf::from(&config.cache_path);
        let admin_token = config.admin_token.clone();

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout))
            .user_agent("speedwholes/0.1.0")
            .pool_max_idle_per_host(config.pool_max_idle_per_host)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_nodelay(true)
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(crate::net::FilteringResolver::shared(
                config.allow_private_networks,
            ))
            .build()
            .expect("Failed to create HTTP client");

        if admin_token.is_none() {
            tracing::warn!(
                "SECURITY WARNING: No admin_token configured! \
                /config007/* endpoints are UNPROTECTED."
            );
        } else {
            tracing::info!("Admin token configured - /config007/* endpoints are protected");
        }

        for sub in &config.subdomains {
            if sub.signing_required
                && sub
                    .signing_key
                    .as_deref()
                    .map(|k| k.trim().is_empty())
                    .unwrap_or(true)
            {
                tracing::warn!(
                    tenant = %sub.name,
                    "signing_required is set but signing_key is missing; signed requests will fail"
                );
            }
        }

        if config.allow_all_sources {
            tracing::warn!(
                "allow_all_sources is true: open proxy for tenants without allowed_sources"
            );
        } else if config.allowed_sources.is_empty() {
            let tenants_without_list = config
                .subdomains
                .iter()
                .filter(|s| {
                    s.allowed_sources
                        .as_ref()
                        .map(|list| list.is_empty())
                        .unwrap_or(true)
                })
                .count();
            tracing::info!(
                tenants_without_list,
                "allow_all_sources is false and the global allowlist is empty; those tenants will receive 403"
            );
        }

        let subdomains = config.subdomains.clone();
        let permits = config.max_concurrent_resizes.max(1);

        Self {
            port,
            config_dir,
            cache_path,
            config: Arc::new(config),
            subdomain_configs: Arc::new(RwLock::new(subdomains)),
            http_client,
            stats: Arc::new(Stats::new()),
            signature_cache: Arc::new(SignatureCache::new(DEFAULT_SIGNATURE_CACHE_SIZE)),
            resize_stats: Arc::new(ResizeStats::new()),
            admin_token,
            sessions: Arc::new(SessionStore::new()),
            tls_config,
            resize_permits: Arc::new(tokio::sync::Semaphore::new(permits)),
            inflight: Arc::new(crate::inflight::Inflight::default()),
        }
    }

    fn exit_with_ssl_help() -> ! {
        tracing::error!("");
        tracing::error!("SSL is required to run speedwholes.");
        tracing::error!("Configure tls_cert and tls_key in speedwholes.config.json");
        tracing::error!("");
        tracing::error!("Example:");
        tracing::error!(r#"  {{"#);
        tracing::error!(r#"    "tls_cert": "/path/to/cert.pem","#);
        tracing::error!(r#"    "tls_key": "/path/to/key.pem""#);
        tracing::error!(r#"  }}"#);
        tracing::error!("");
        tracing::error!("To generate certificates, run: ./setup-ssl.sh");
        std::process::exit(1);
    }

    pub async fn persist_subdomain(&self, subdomain_name: &str) -> Result<(), std::io::Error> {
        let subdomains = self.subdomain_configs.read().await;
        let subdomain = subdomains
            .iter()
            .find(|s| s.name == subdomain_name)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Subdomain '{}' not found", subdomain_name),
                )
            })?;

        ServerConfig::save_subdomain(subdomain, &self.config_dir)
    }

    pub async fn persist_all_subdomains(&self) -> Result<(), std::io::Error> {
        let subdomains = self.subdomain_configs.read().await;
        for subdomain in subdomains.iter() {
            ServerConfig::save_subdomain(subdomain, &self.config_dir)?;
        }
        Ok(())
    }

    pub async fn delete_subdomain_file(&self, subdomain_name: &str) -> Result<(), std::io::Error> {
        ServerConfig::delete_subdomain(subdomain_name, &self.config_dir)
    }

    pub fn get_cache_path(&self) -> PathBuf {
        self.cache_path.clone()
    }

    pub fn max_width(&self) -> u32 {
        self.config.max_width
    }

    pub fn max_height(&self) -> u32 {
        self.config.max_height
    }

    pub fn max_source_pixels(&self) -> u64 {
        self.config.max_source_pixels
    }

    #[allow(dead_code)]
    pub async fn get_subdomain_config(&self, name: &str) -> Option<SubdomainConfig> {
        self.subdomain_configs
            .read()
            .await
            .iter()
            .find(|c| c.name == name)
            .cloned()
    }

    #[allow(dead_code)]
    pub async fn allowed_sources_for_subdomain(&self, subdomain: Option<&str>) -> Vec<String> {
        if let Some(name) = subdomain {
            if let Some(config) = self.get_subdomain_config(name).await {
                if let Some(sources) = config.allowed_sources {
                    // An empty list means "not set" and falls back to the global list.
                    if !sources.is_empty() {
                        return sources;
                    }
                }
            }
        }
        self.config.allowed_sources.clone()
    }

    pub fn allow_all_sources(&self) -> bool {
        self.config.allow_all_sources
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_denies_all_sources() {
        let cfg: ServerConfig = serde_json::from_str("{}").unwrap();
        assert!(!cfg.allow_all_sources);
        assert!(cfg.allowed_sources.is_empty());
        assert_eq!(cfg.unknown_host_policy, "reject");
        assert!(!cfg.trust_forwarded_host);
    }

    #[test]
    fn invalid_signature_is_not_cached() {
        let cache = SignatureCache::new(8);
        let key = "test-secret-key-at-least-32-chars!";
        let path = "/h/10/w/10/http://example.com/a.jpg";
        let bad = "not-a-real-signature";
        let valid = crate::signing::verify_signature(key, path, bad);
        assert!(!valid);
        if valid {
            cache.put("signed", path, bad, true);
        }
        cache.put("signed", path, bad, false);
        assert!(cache.get("signed", path, bad).is_none());

        let good = crate::signing::generate_signature(key, path);
        cache.put("signed", path, &good, true);
        assert_eq!(cache.get("signed", path, &good), Some(true));
    }
}
