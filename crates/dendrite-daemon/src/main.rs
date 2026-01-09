//! Dendrite Daemon - Main entry point
//!
//! This is the main daemon that runs discovery and serves the web UI.

mod api;
mod config;
mod server;
mod state;
mod ws;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(name = "dendrite")]
#[command(about = "CogniPilot hardware discovery and visualization daemon")]
#[command(version)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, default_value = "dendrite.toml")]
    config: PathBuf,

    /// Bind address for web server
    #[arg(short, long)]
    bind: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Run a single scan and exit
    #[arg(long)]
    scan_once: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let level = match args.log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(true)
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    info!("Dendrite v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let mut config = config::load_config(&args.config)?;

    // Override bind address if specified
    if let Some(bind) = args.bind {
        config.daemon.bind = bind;
    }

    info!(
        subnet = %config.discovery.subnet,
        prefix = config.discovery.prefix_len,
        "Configuration loaded"
    );

    // Create application state
    let state = state::AppState::new(config.clone()).await?;

    if args.scan_once {
        // Single scan mode
        info!("Running single discovery scan");
        let devices = state.scanner.scan_once().await?;
        println!("Discovered {} devices:", devices.len());
        for device in devices {
            println!(
                "  - {} ({}) at {}:{}",
                device.name,
                device.id,
                device.discovery.ip,
                device.discovery.port
            );
            if let Some(board) = &device.info.board {
                println!("    Board: {}", board);
            }
            if let Some(version) = &device.firmware.version {
                println!("    Firmware: {}", version);
            }
        }
    } else {
        // Daemon mode - run web server and discovery
        server::run(state, &config.daemon.bind, config.daemon.tls.as_ref()).await?;
    }

    Ok(())
}
