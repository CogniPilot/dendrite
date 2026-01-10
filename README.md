# Dendrite

CogniPilot hardware discovery and 3D visualization system for T1 ethernet-connected devices.

## Overview

Dendrite is a Rust-based daemon that:
- Discovers devices on T1 ethernet networks via ARP scanning and MCUmgr probing
- Queries devices using the MCUmgr protocol to get chip IDs and firmware info
- Maintains a hardware registry with device fragments (board definitions)
- Provides a WebGPU-powered 3D visualization of device topology
- Supports remote access via GitHub Pages frontend at [dendrite.cognipilot.org](https://dendrite.cognipilot.org)

## Quick Start

```bash
# Build and run the daemon
cargo build --release -p dendrite-daemon
./target/release/dendrite

# Open the web UI
# Local: http://localhost:8080
# Remote: https://dendrite.cognipilot.org?daemon=YOUR_IP:8080

# Generate a QR code for mobile access
cargo build --release -p dendrite-qr
./target/release/dendrite-qr
```

## Architecture

```
+------------------------------------------------------------------+
|                          dendrite-daemon                          |
+------------------------------------------------------------------+
|  +----------------+  +----------------+  +---------------------+  |
|  |   Discovery    |  |    MCUmgr      |  |   Device Registry   |  |
|  |  (ARP scan)    |->|    Client      |->|   (fragments)       |  |
|  +----------------+  +----------------+  +---------------------+  |
|                              |                                    |
|  +--------------------------------------------------------+      |
|  |              Web Server (axum + tower)                  |      |
|  |   REST API: /api/devices, /api/scan, /api/interfaces   |      |
|  |   WebSocket: /ws (real-time device updates)            |      |
|  |   Static: WASM frontend served at /                    |      |
|  +--------------------------------------------------------+      |
+------------------------------------------------------------------+
                               |
                               v
+------------------------------------------------------------------+
|                   Browser (WASM Frontend)                         |
+------------------------------------------------------------------+
|  +----------------+  +----------------+  +---------------------+  |
|  |   Bevy +       |  |   Device       |  |   UI Panels         |  |
|  |   WebGPU       |->|   3D Models    |->|   (bevy_egui)       |  |
|  |   (3D view)    |  |   (glTF)       |  |                     |  |
|  +----------------+  +----------------+  +---------------------+  |
+------------------------------------------------------------------+
```

## Crates

| Crate | Description |
|-------|-------------|
| `dendrite-daemon` | Main daemon binary with web server and discovery |
| `dendrite-web` | Bevy-based WebGPU visualization (compiles to WASM) |
| `dendrite-qr` | CLI tool to generate QR codes for mobile connection |
| `dendrite-core` | Core types, HCDF parsing, fragment database, SHA-based caching |
| `dendrite-mcumgr` | Async MCUmgr protocol client (wraps mcumgr-client) |
| `dendrite-discovery` | Network discovery (ARP scanning, MCUmgr probing) |

## Building

### Prerequisites

- Rust 1.75+ with `wasm32-unknown-unknown` target
- For WASM builds: `wasm-bindgen-cli`

```bash
# Install WASM target
rustup target add wasm32-unknown-unknown

# Install wasm-bindgen CLI
cargo install wasm-bindgen-cli
```

### Build All

```bash
# Build daemon (runs on your device)
cargo build --release -p dendrite-daemon

# Build QR code generator
cargo build --release -p dendrite-qr

# Build WASM frontend
cargo build --release -p dendrite-web --target wasm32-unknown-unknown
wasm-bindgen --out-dir web --target web --no-typescript \
    target/wasm32-unknown-unknown/release/dendrite_web.wasm
```

## Running

### Daemon

```bash
# Run with default configuration
./target/release/dendrite

# The web UI is available at http://localhost:8080
```

The daemon will:
1. Start the web server on port 8080
2. Serve the WASM frontend at `/`
3. Provide REST API at `/api/*`
4. Provide WebSocket at `/ws`
5. Optionally check device connectivity via ARP (disabled by default)

### QR Code Generator

For easy mobile access, use `dendrite-qr` to display a QR code:

```bash
./target/release/dendrite-qr

# Options:
#   -p, --port <PORT>       Daemon port (default: 8080)
#   --https                 Use HTTPS instead of HTTP
#   --frontend-url <URL>    Frontend URL (default: https://dendrite.cognipilot.org)
#   --no-check              Skip daemon availability check
#   --url-only              Show URL only, no QR code
#   --local                 Use direct daemon URL instead of remote frontend
```

This will:
1. Auto-detect your local IP address
2. Check if the daemon is running
3. Display a QR code that opens the web UI with your daemon address

### Remote Access

The frontend is hosted at [dendrite.cognipilot.org](https://dendrite.cognipilot.org). Connect to your local daemon by adding a URL parameter:

```
https://dendrite.cognipilot.org?daemon=192.168.1.100:8080
```

Or use the "Connect" button in the UI to enter the daemon address manually.

## Configuration

Create a `dendrite.toml` file in the working directory:

```toml
[daemon]
bind = "0.0.0.0:8080"
heartbeat_interval_secs = 2    # ARP connectivity check interval
heartbeat_enabled = false      # Disable connectivity checking by default

[discovery]
subnet = "192.168.1.0"         # Network to scan
prefix_len = 24                # Subnet mask (/24 = 255.255.255.0)
mcumgr_port = 1337             # MCUmgr UDP port
use_lldp = true
use_arp = true

[cache]
path = "./fragments/cache"       # Downloaded HCDFs and models

[hcdf]
path = "./dendrite.hcdf"         # Output HCDF file
```

## REST API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/devices` | GET | List all discovered devices |
| `/api/devices/:id` | DELETE | Remove a device |
| `/api/interfaces` | GET | List network interfaces |
| `/api/subnet` | POST | Update scan subnet |
| `/api/scan` | POST | Trigger network scan |
| `/api/heartbeat` | GET | Get connectivity check status |
| `/api/heartbeat` | POST | Enable/disable connectivity checking |

## WebSocket

Connect to `/ws` for real-time device updates:

```javascript
const ws = new WebSocket('ws://192.168.1.100:8080/ws');
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  // msg.type: "device_discovered", "device_offline", "device_updated"
  // msg.data: device object
};
```

## Device Fragments

Device definitions (HCDF files) map board/application combinations to 3D models and reference frames. All fragments are fetched from [hcdf.cognipilot.org](https://hcdf.cognipilot.org) and cached locally for offline use.

### URL Pattern

The daemon constructs HCDF URLs from the device's board and app names:

```
https://hcdf.cognipilot.org/{board}/{app}/{app}.hcdf
```

For example, a device with board `mr_mcxn_t1` and app `optical-flow` fetches:
```
https://hcdf.cognipilot.org/mr_mcxn_t1/optical-flow/optical-flow.hcdf
```

### Caching

Downloaded HCDFs and models are cached in `fragments/cache/` with SHA-prefixed names:

```
fragments/cache/
├── manifest.json                              # Cache index
├── mr_mcxn_t1/
│   └── optical-flow/
│       ├── a1b2c3d4-optical-flow.hcdf        # SHA-prefixed version
│       └── optical-flow.hcdf                  # Symlink to latest
└── models/
    ├── fbf4836d-mcxnt1hub.glb
    └── 72eef172-optical_flow.glb
```

The symlink allows offline fallback - if the server is unreachable, the daemon uses the most recently cached version.

### HCDF Format

HCDF (Hardware Configuration Descriptive Format) files define the visuals and reference frames for a device:

```xml
<?xml version="1.0"?>
<hcdf version="1.2">
  <comp name="optical-flow-assembly" role="sensor">
    <description>PMW3901 optical flow sensor</description>

    <!-- Multiple visuals with individual poses -->
    <visual name="board">
      <pose>0 0 0 0 0 0</pose>
      <model href="models/fbf4836d-mcxnt1hub.glb" sha="fbf4836d..."/>
    </visual>
    <visual name="sensor">
      <pose>0 0 -0.005 3.14159 0 0</pose>
      <model href="models/72eef172-optical_flow.glb" sha="72eef172..."/>
    </visual>

    <!-- Reference frames (shown via "Show Reference Frames" checkbox) -->
    <frame name="flow">
      <description>Optical flow sensor frame</description>
      <pose>0 0 -0.005 3.14159 0 0</pose>
    </frame>
  </comp>
</hcdf>
```

Models are cached locally with SHA-prefixed names (`{short_sha}-{name}.glb`) for deduplication.

## 3D Models

All 3D models are hosted at [hcdf.cognipilot.org](https://hcdf.cognipilot.org) and fetched automatically when referenced by HCDF files. See the [hcdf_models repository](https://github.com/CogniPilot/hcdf_models) for available models.

### Web UI Features

- **Camera**: Orbit (drag), pan (right-drag), zoom (scroll/pinch)
- **Selection**: Click devices to view details and edit position/rotation
- **Composite visuals**: Devices can have multiple 3D models at different poses
- **Reference frames**: Toggle "Show Reference Frames" to visualize coordinate frames
- **Frame tooltips**: Hover over frame gizmos to see name and description
- **Connection status**: Green=online, red=offline, white=unknown (when heartbeat disabled)
- **Connectivity checking**: Toggle via "Check connection" checkbox

## GitHub Pages Deployment

The WASM frontend is automatically deployed to GitHub Pages on push to `main`. The workflow:

1. Builds `dendrite-web` for `wasm32-unknown-unknown`
2. Runs `wasm-bindgen` to generate JS bindings
3. Deploys the `web/` directory to GitHub Pages

The frontend at `dendrite.cognipilot.org` can connect to any daemon via the `?daemon=` parameter.

## Development

```bash
# Run daemon with debug logging
RUST_LOG=debug cargo run -p dendrite-daemon

# Build and run WASM locally (requires local daemon)
./build-web.sh
# Then open http://localhost:8080

# Run clippy
cargo clippy --all-targets

# Format code
cargo fmt
```

## License

Apache-2.0 - See [LICENSE](LICENSE)
