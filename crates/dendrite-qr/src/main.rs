//! dendrite-qr - Display QR code for connecting to Dendrite daemon
//!
//! This tool checks if a Dendrite daemon is running and displays a QR code
//! that can be scanned to open the web UI with the correct daemon address.

use clap::Parser;
use network_interface::{NetworkInterface, NetworkInterfaceConfig};
use qrcode::QrCode;
use std::net::Ipv4Addr;

#[derive(Parser, Debug)]
#[command(name = "dendrite-qr")]
#[command(about = "Display QR code for connecting to Dendrite daemon")]
#[command(version)]
struct Args {
    /// Daemon port (default: 8080)
    #[arg(short, long, default_value = "8080")]
    port: u16,

    /// Use HTTPS instead of HTTP
    #[arg(long)]
    https: bool,

    /// Frontend URL (defaults to dendrite.cognipilot.org)
    #[arg(long, default_value = "https://dendrite.cognipilot.org")]
    frontend_url: String,

    /// Skip daemon availability check
    #[arg(long)]
    no_check: bool,

    /// Show URL only (no QR code)
    #[arg(long)]
    url_only: bool,

    /// Use direct daemon URL instead of remote frontend
    #[arg(long)]
    local: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Get local IP addresses
    let ips = get_local_ips();

    if ips.is_empty() {
        eprintln!("Error: No network interfaces found");
        std::process::exit(1);
    }

    // Pick the best IP (prefer common local network ranges)
    let best_ip = ips
        .iter()
        // Prefer 192.168.x.x (most common home/office networks)
        .find(|ip| ip.octets()[0] == 192 && ip.octets()[1] == 168)
        // Then 10.x.x.x (common enterprise networks)
        .or_else(|| ips.iter().find(|ip| ip.octets()[0] == 10))
        // Then 172.16-31.x.x (less common private range)
        .or_else(|| ips.iter().find(|ip| {
            let octets = ip.octets();
            octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31
        }))
        // Fall back to any non-loopback, non-link-local
        .or_else(|| ips.iter().find(|ip| !ip.is_loopback() && !ip.is_link_local()))
        .or_else(|| ips.first())
        .expect("No IP address found");

    let protocol = if args.https { "https" } else { "http" };
    let daemon_addr = format!("{}:{}", best_ip, args.port);
    let daemon_url = format!("{}://{}", protocol, daemon_addr);

    // Check if daemon is running
    if !args.no_check {
        print!("Checking daemon at {}... ", daemon_url);
        match check_daemon(&daemon_url).await {
            Ok(true) => println!("OK"),
            Ok(false) => {
                println!("NOT RESPONDING");
                eprintln!("\nDendrite daemon is not running at {}", daemon_url);
                eprintln!("Start it with: dendrite");
                std::process::exit(1);
            }
            Err(e) => {
                println!("ERROR");
                eprintln!("\nFailed to check daemon: {}", e);
                eprintln!("Use --no-check to skip this check");
                std::process::exit(1);
            }
        }
    }

    // Build the connection URL
    let connect_url = if args.local {
        // Direct connection to daemon
        daemon_url.clone()
    } else {
        // Remote frontend with daemon parameter
        format!("{}?daemon={}", args.frontend_url.trim_end_matches('/'), daemon_addr)
    };

    println!();
    println!("=== Dendrite Connection ===");
    println!();
    println!("Daemon: {}", daemon_url);
    println!("Connect URL: {}", connect_url);
    println!();

    if args.url_only {
        return;
    }

    // Generate and display QR code
    match QrCode::new(&connect_url) {
        Ok(code) => {
            let qr_string = render_qr_terminal(&code);
            println!("{}", qr_string);
            println!();
            println!("Scan the QR code above to connect from your mobile device.");
        }
        Err(e) => {
            eprintln!("Failed to generate QR code: {}", e);
            std::process::exit(1);
        }
    }

    // Show all available IPs
    if ips.len() > 1 {
        println!();
        println!("Other available addresses:");
        for ip in &ips {
            if ip != best_ip {
                println!("  {}://{}:{}", protocol, ip, args.port);
            }
        }
    }
}

/// Get all local IPv4 addresses
fn get_local_ips() -> Vec<Ipv4Addr> {
    let mut ips = Vec::new();

    if let Ok(interfaces) = NetworkInterface::show() {
        for iface in interfaces {
            // Skip loopback
            if iface.name == "lo" {
                continue;
            }

            for addr in iface.addr {
                if let network_interface::Addr::V4(v4) = addr {
                    let ip = v4.ip;
                    // Skip loopback and link-local
                    if !ip.is_loopback() {
                        ips.push(ip);
                    }
                }
            }
        }
    }

    ips
}

/// Check if daemon is responding
async fn check_daemon(url: &str) -> Result<bool, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .danger_accept_invalid_certs(true) // Allow self-signed certs
        .build()
        .map_err(|e| e.to_string())?;

    let check_url = format!("{}/api/devices", url);

    match client.get(&check_url).send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(e) if e.is_timeout() => Ok(false),
        Err(e) if e.is_connect() => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// Render QR code as terminal-friendly string using Unicode block characters
fn render_qr_terminal(code: &QrCode) -> String {
    let colors = code.to_colors();
    let width = code.width();

    let mut result = String::new();

    // Top border (white)
    result.push_str(&"  ".repeat(width + 4));
    result.push('\n');
    result.push_str(&"  ".repeat(width + 4));
    result.push('\n');

    // Process two rows at a time using half-block characters
    for y in (0..width).step_by(2) {
        // Left border
        result.push_str("    ");

        for x in 0..width {
            let top = colors[y * width + x];
            let bottom = if y + 1 < width {
                colors[(y + 1) * width + x]
            } else {
                qrcode::Color::Light // Padding
            };

            // Use Unicode half-block characters
            // Upper half block: \u{2580} (top black, bottom white)
            // Lower half block: \u{2584} (top white, bottom black)
            // Full block: \u{2588} (both black)
            // Space: both white
            let ch = match (top, bottom) {
                (qrcode::Color::Dark, qrcode::Color::Dark) => "\u{2588}\u{2588}", // Full block (██)
                (qrcode::Color::Dark, qrcode::Color::Light) => "\u{2580}\u{2580}", // Upper half (▀▀)
                (qrcode::Color::Light, qrcode::Color::Dark) => "\u{2584}\u{2584}", // Lower half (▄▄)
                (qrcode::Color::Light, qrcode::Color::Light) => "  ",              // Space
            };
            result.push_str(ch);
        }

        // Right border
        result.push_str("    ");
        result.push('\n');
    }

    // Bottom border (white)
    result.push_str(&"  ".repeat(width + 4));
    result.push('\n');
    result.push_str(&"  ".repeat(width + 4));

    result
}
