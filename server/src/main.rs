mod app;
mod auth;
mod cache;
mod config;
mod disk_monitor;
mod error;
mod handlers;
mod lock;
mod net;
mod resize;
mod resize_stats;
mod signing;
mod source_filter;
mod util;
mod webp_lossy;

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod test_support;

use axum_server::{tls_rustls::RustlsConfig, Handle};
use config::{AppState, CliArgs, Command, LogFormat};
use std::net::SocketAddr;
use tokio::signal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    // Install rustls crypto provider (must be done before any TLS operations)
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Parse CLI arguments early so we can configure logging format
    let cli = CliArgs::parse_args();

    // Handle subcommands before any other initialization
    if let Some(command) = &cli.command {
        match command {
            Command::Init => {
                run_init_script(&cli.config_dir);
            }
            Command::Restart => {
                run_systemctl_command("restart");
            }
            Command::Stop => {
                run_systemctl_command("stop");
            }
            Command::Status => {
                run_systemctl_command("status");
            }
        }
    }
    let use_json_logs = matches!(cli.log_format, LogFormat::Json);

    // Set up panic hook for better crash diagnostics
    std::panic::set_hook(Box::new(move |panic_info| {
        let location = panic_info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };

        // Print to stderr for systemd to capture
        if use_json_logs {
            let panic_log = format!(
                r#"{{"timestamp":"{}","level":"ERROR","fields":{{"message":"PANIC: {}","location":"{}","panic_type":"application_panic"}},"target":"speedwholes::panic"}}"#,
                chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ"),
                message.replace('"', r#"\""#),
                location
            );
            eprintln!("{}", panic_log);
        } else {
            eprintln!("\x1b[31m[ERROR]\x1b[0m PANIC at {}: {}", location, message);
        }
    }));

    // Initialize logging based on format choice
    // All logs go to stdout/stderr - systemd journal captures them
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "speedwholes=info,tower_http=info".into());

    match cli.log_format {
        LogFormat::Json => {
            // JSON format: structured logs for production (systemd captures stdout)
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().json())
                .init();
        }
        LogFormat::Text => {
            // Text format: human-readable, colored output for development
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().pretty())
                .init();
        }
    }

    tracing::info!("Starting speedwholes image resizer");
    tracing::info!("  Config dir: {:?}", cli.config_dir);

    // Acquire instance lock to prevent multiple instances with same config dir
    let _instance_lock = match lock::InstanceLock::acquire(&cli.config_dir) {
        Ok(lock) => lock,
        Err(e) => {
            tracing::error!("{}", e);
            std::process::exit(1);
        }
    };

    // Create application state from CLI args and config file
    let state = AppState::new(cli);

    // Extract values we'll need after state is moved
    let port = state.port;
    let cache_path = state.cache_path.clone();
    let tls_config = state.tls_config.clone();

    // Initialize cache directories (resize and source caches)
    // Use None for subdomain to create the base _default directories at startup
    let resize_cache_path = cache_path.join("resize");
    let source_cache_path = cache_path.join("source");

    if cache::DiskCache::new_initialized(resize_cache_path.clone(), "resize", None)
        .await
        .is_err()
    {
        std::process::exit(1);
    }

    if cache::DiskCache::new_initialized(source_cache_path.clone(), "source", None)
        .await
        .is_err()
    {
        std::process::exit(1);
    }

    tracing::info!("Cache directories initialized:");
    tracing::info!("  Resize cache: {:?}", resize_cache_path);
    tracing::info!("  Source cache: {:?}", source_cache_path);

    // Start background disk space monitor
    disk_monitor::start_disk_monitor(
        cache_path.clone(),
        state.config.disk_min_free_mb,
        state.config.disk_check_interval_secs,
    );

    // Check if cache directory may be shared with other instances
    if let Ok(mut entries) = tokio::fs::read_dir(&cache_path).await {
        let mut count = 0;
        while entries.next_entry().await.transpose().is_some() {
            count += 1;
        }
        if count > 0 {
            tracing::info!(
                cache_entries = count,
                "Cache directory contains existing entries"
            );
        }
    }

    let app = app::build_router(state);

    // Bind to address
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // Start HTTPS server (SSL is required)
    let rustls_config =
        match RustlsConfig::from_pem_file(&tls_config.cert_path, &tls_config.key_path).await {
            Ok(config) => config,
            Err(e) => {
                tracing::error!("Failed to load TLS certificates: {}", e);
                tracing::error!("  Certificate: {:?}", tls_config.cert_path);
                tracing::error!("  Private key: {:?}", tls_config.key_path);
                tracing::error!("");
                tracing::error!("The certificate files may be invalid or corrupted.");
                tracing::error!("Ensure they are valid PEM format files.");
                tracing::error!("");
                tracing::error!("To regenerate certificates, run:");
                tracing::error!("  ./setup-ssl.sh");
                std::process::exit(1);
            }
        };

    tracing::info!("🔒 Listening on https://{}", addr);

    // Set up graceful shutdown for HTTPS server
    let handle = Handle::new();
    let shutdown_handle = handle.clone();

    tokio::spawn(async move {
        shutdown_signal().await;
        shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(30)));
    });

    axum_server::bind_rustls(addr, rustls_config)
        .handle(handle)
        .serve(app.into_make_service())
        .await
        .expect("Server error");

    tracing::info!("Server shutdown complete");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received Ctrl+C, shutting down...");
        }
        _ = terminate => {
            tracing::info!("Received terminate signal, shutting down...");
        }
    }
}

/// Run a systemctl command (restart, stop, status) on the speedwholes service
fn run_systemctl_command(action: &str) -> ! {
    use std::process::Command;

    let status = Command::new("systemctl")
        .arg(action)
        .arg("speedwholes")
        .status();

    match status {
        Ok(exit_status) => {
            std::process::exit(exit_status.code().unwrap_or(0));
        }
        Err(e) => {
            eprintln!("Failed to run systemctl {} speedwholes: {}", action, e);
            eprintln!();
            eprintln!("Make sure:");
            eprintln!("  - systemd is available on this system");
            eprintln!("  - the speedwholes service is installed");
            eprintln!("  - you have permission to manage the service (try with sudo)");
            std::process::exit(1);
        }
    }
}

/// Run the interactive configuration setup script
fn run_init_script(config_dir: &std::path::Path) -> ! {
    use std::process::Command;

    let script_path = config_dir.join("scripts/setup-config.sh");

    if script_path.exists() {
        let status = Command::new("bash")
            .arg(&script_path)
            .env("CONFIG_DIR", config_dir)
            .status();

        match status {
            Ok(exit_status) => {
                std::process::exit(exit_status.code().unwrap_or(0));
            }
            Err(e) => {
                eprintln!("Failed to run setup script: {}", e);
                std::process::exit(1);
            }
        }
    }

    eprintln!("Setup script not found: {:?}", script_path);
    eprintln!();
    eprintln!("This command requires Speedwholes to be installed.");
    eprintln!("Run the installer first: sudo ./install.sh");
    std::process::exit(1);
}
