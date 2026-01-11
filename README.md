# Dendrite

CogniPilot hardware discovery and 3D visualization system for T1 ethernet-connected devices.

## Overview

Dendrite is a Rust-based daemon that:
- Discovers devices on T1 ethernet networks via ARP scanning and MCUmgr probing
- Queries devices using the MCUmgr protocol to get chip IDs and firmware info
- Fetches HCDF (Hardware Configuration Descriptive Format) files from [hcdf.cognipilot.org](https://hcdf.cognipilot.org)
- Provides a WebGPU-powered 3D visualization of device topology with sensors, ports, and reference frames
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
|  |  (ARP scan)    |->|    Client      |->|   + HCDF fetch      |  |
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
|  |   Bevy 0.17 +  |  |   Device       |  |   UI Panels         |  |
|  |   WebGPU       |->|   3D Models    |->|   (bevy_egui)       |  |
|  |   (3D view)    |  |   (glTF)       |  |                     |  |
|  +----------------+  +----------------+  +---------------------+  |
+------------------------------------------------------------------+
```

## Crates

| Crate | Description |
|-------|-------------|
| `dendrite-daemon` | Main daemon binary with web server, discovery, and HCDF fetching |
| `dendrite-web` | Bevy 0.17 WebGPU visualization (compiles to WASM) |
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

# Build WASM frontend (use the build script)
./build-web.sh
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
4. Provide WebSocket at `/ws` for real-time updates
5. Auto-fetch HCDF files and models from hcdf.cognipilot.org
6. Optionally check device connectivity via ARP (toggle in UI)

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

[fragments]
path = "./fragments/index.toml"

[models]
path = "./assets/models"

[cache]
path = "./fragments/cache"     # Downloaded HCDFs and models

[hcdf]
path = "./dendrite.hcdf"       # Output HCDF file
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

## HCDF Format

HCDF (Hardware Configuration Descriptive Format) version 2.0 files define the complete hardware configuration:

```xml
<?xml version="1.0"?>
<hcdf version="2.0">
  <comp name="optical-flow-assembly" role="sensor">
    <description>Optical flow sensor assembly</description>

    <!-- Ports with mesh highlighting -->
    <port name="ETH0" type="ethernet" visual="hub_board" mesh="ETH0">
      <pose>0.0225 -0.0155 -0.0085 0 0 0</pose>
      <geometry><box><size>0.006 0.004 0.003</size></box></geometry>
    </port>

    <!-- Sensors with axis alignment and FOV visualization -->
    <sensor name="imu_hub">
      <inertial type="accel_gyro">
        <pose>0.016 -0.001 -0.008 0 0 0</pose>
        <driver name="icm45686">
          <axis-align x="X" y="Y" z="Z"/>
        </driver>
      </inertial>
    </sensor>

    <sensor name="optical_flow">
      <optical type="optical_flow">
        <pose>0 0 0.002 0 0 0</pose>
        <driver name="paa3905"/>
        <fov name="imager" color="#88ff88">
          <geometry>
            <pyramidal_frustum>
              <near>0.08</near><far>50.0</far>
              <hfov>0.733</hfov><vfov>0.733</vfov>
            </pyramidal_frustum>
          </geometry>
        </fov>
      </optical>
    </sensor>

    <!-- Multiple visuals with toggle groups -->
    <visual name="hub_board">
      <pose>0 0 -0.009 1.5708 0 1.5708</pose>
      <model href="models/mcxnt1hub.glb" sha="fbf4836d..."/>
    </visual>
    <visual name="case_bottom" toggle="case">
      <pose>0 0 -0.014 0 3.14159 -1.5708</pose>
      <model href="models/mcxnt1hub_base.glb" sha="86d9c8c8..."/>
    </visual>

    <!-- Reference frames -->
    <frame name="board_origin">
      <description>Main board origin</description>
      <pose>0 0 0 0 0 0</pose>
    </frame>
  </comp>
</hcdf>
```

### HCDF Features

- **Ports**: Define physical connectors with type-based highlighting (Ethernet=green, CAN=yellow, etc.)
  - `mesh` attribute links to named meshes in glTF models for precise highlighting
- **Sensors**: IMUs, magnetometers, barometers, optical flow, ToF sensors
  - `axis-align` defines sensor-to-body frame transformation
  - `fov` elements visualize sensor field of view (conical, pyramidal frustum)
- **Visuals**: Multiple glTF models with individual poses
  - `toggle` groups allow showing/hiding visual sets (e.g., case on/off)
- **Frames**: Named coordinate frames for sensor mounting visualization

### Remote HCDF Fetching

The daemon automatically fetches HCDF files based on device board/app info:

```
https://hcdf.cognipilot.org/{board}/{app}/{app}.hcdf
```

For example, board `mr_mcxn_t1` with app `optical-flow`:
```
https://hcdf.cognipilot.org/mr_mcxn_t1/optical-flow/optical-flow.hcdf
```

### Caching

Downloaded HCDFs and models are cached with SHA-prefixed names for deduplication:

```
fragments/cache/
├── mr_mcxn_t1/
│   └── optical-flow/
│       ├── b05fb19d-optical-flow.hcdf    # SHA-prefixed version
│       └── optical-flow.hcdf              # Symlink to latest
└── models/
    ├── fbf4836d-mcxnt1hub.glb
    └── 72eef172-optical_flow.glb
```

## Web UI Features

### 3D Visualization
- **Camera**: Orbit (left-drag), pan (right-drag), zoom (scroll/pinch)
- **Selection**: Click devices to view details and edit position/rotation
- **Device highlight**: Wireframe box shows selected device (green=online, red=offline, white=unknown)

### Sensors
- **Sensor axes**: Toggle per-sensor coordinate frame visualization
- **Axis alignment**: Shows raw vs aligned axes based on HCDF `axis-align`
- **FOV cones**: Visualize sensor field of view for cameras and ToF sensors
- **Hover highlighting**: Sensors dim when hovering others for clarity

### Ports
- **Port highlighting**: Hover ports in the UI to highlight corresponding mesh on the 3D model
- **Type-based colors**: Ethernet (green), CAN (yellow), SPI (magenta), I2C (cyan), UART (orange), USB (blue)
- **Mesh linking**: Ports reference named meshes in glTF models via `mesh` attribute

### UI Panels
- **Device list**: All discovered devices with status indicators
- **Device details**: Position, rotation, sensors, ports for selected device
- **Visual toggles**: Show/hide visual groups (e.g., case, PCB)
- **Reference frames**: Toggle coordinate frame gizmos per device
- **Connection status**: Real-time online/offline status with heartbeat checking

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

# Build WASM frontend locally
./build-web.sh
# Then open http://localhost:8080

# Run clippy
cargo clippy --all-targets

# Format code
cargo fmt
```

## License

Apache-2.0 - See [LICENSE](LICENSE)
