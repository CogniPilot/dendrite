# HCDF Fragment Enhancement Plan

## Overview

Enhance the Dendrite fragment system to support:
1. **Composite visuals** - Multiple GLB models per device at different poses
2. **Reference frames** - Named coordinate frames with descriptions and hover interaction
3. **HCDF-based fragments** - Convert fragment definitions from TOML to HCDF XML
4. **Remote fetching** - Devices report HCDF URL + SHA via MCUmgr, daemon downloads and caches
5. **GitHub Pages model hosting** - Public models site at models.cognipilot.org

## Current State

### Fragment System (dendrite-core/src/fragment.rs)
- TOML-based fragment index at `fragments/index.toml`
- Matching: board + app → fragment with priority scoring
- Each fragment has single model path, mass, description

### HCDF Parser (dendrite-core/src/hcdf.rs)
- Already supports `<visual>`, `<frame>`, `<model>`, `<pose>` elements
- Pose format: `"x y z roll pitch yaw"` (meters, radians)
- Visual has name, pose, model href

### MCUmgr (dendrite-mcumgr/src/query.rs)
- Uses NmpGroup enum with custom groups available at 64+
- Current queries: os_info, bootloader_info, image_state

### Frontend (dendrite-web/src/models.rs)
- ModelCache tracks loading/loaded models
- sync_device_entities spawns single SceneRoot per device

---

## Proposed Design

### 1. HCDF Fragment Format

Fragments become HCDF XML files with composite visuals and frames:

```xml
<?xml version="1.0"?>
<hcdf version="1.2">
  <comp name="optical-flow-assembly" role="sensor">
    <description>PMW3901 optical flow sensor on MR-MCXN-T1</description>

    <!-- Multiple visuals with individual poses -->
    <visual name="board">
      <pose>0 0 0 0 0 0</pose>
      <model href="models/mcnt1hub.glb"/>
    </visual>
    <visual name="sensor">
      <pose>0 0 -0.005 3.14159 0 0</pose>
      <model href="models/pmw3901.glb"/>
    </visual>

    <!-- Reference frames (hidden by default, show on checkbox) -->
    <frame name="flow">
      <description>PMW3901 optical flow sensor frame</description>
      <pose>0 0 -0.005 3.14159 0 0</pose>
    </frame>
    <frame name="board_origin">
      <description>Board origin at center</description>
      <pose>0 0 0 0 0 0</pose>
    </frame>
  </comp>
</hcdf>
```

### 2. Simplified Fragment Index (TOML)

```toml
version = "1.0"

# Exact match: board + app
[[fragment]]
board = "mr_mcxn_t1"
app = "optical-flow"
hcdf = "optical_flow.hcdf"

# Wildcard match: any app on this board
[[fragment]]
board = "spinali"
app = "*"
hcdf = "spinali.hcdf"
```

**Matching rules** (simplified from priority system):
1. Exact match (board + app) takes precedence
2. Wildcard match (board + "*") as fallback
3. No match = use fallback cube

### 3. MCUmgr HCDF Group

Add custom MCUmgr group (`GROUP_HCDF`) for querying device's HCDF location and content hash:

```rust
// dendrite-mcumgr/src/query.rs
pub const GROUP_HCDF: u16 = 100;  // Custom group for HCDF queries
pub const ID_HCDF_INFO: u8 = 0;   // Command ID for HCDF info query

/// Response from HCDF info query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HcdfInfoResponse {
    /// URL to fetch the HCDF file (e.g., "https://models.cognipilot.org/spinali/v1.2.hcdf")
    pub url: String,
    /// SHA256 hash of the HCDF content (hex string)
    pub sha: String,
}

pub async fn query_hcdf_info(transport: &UdpTransport, addr: SocketAddr) -> Result<Option<HcdfInfoResponse>>;
```

**Flow with SHA check:**
1. Query device for HCDF info (URL + SHA)
2. Check local cache for matching SHA
3. If SHA matches cached file → use cached (no network fetch)
4. If SHA differs or not cached → fetch from URL, verify SHA, cache

Device firmware implements handler returning CBOR-encoded `{url: "...", sha: "..."}`.

### 4. Remote Fetching with SHA-Based Caching

```
fragments/
├── index.toml           # Local fragment index
├── spinali.hcdf         # Local HCDF files
├── cache/
│   ├── manifest.json    # Cache manifest: SHA → {url, local_path, fetched_at}
│   ├── abc123def456.hcdf      # Cached remote HCDF (named by SHA256)
│   └── abc123def456/          # Associated models for that HCDF
│       └── sensor.glb
```

**Flow:**
1. Device discovered → query HCDF info via MCUmgr (URL + SHA)
2. Check cache by SHA: if we have file with matching SHA → use cached (skip network)
3. If SHA not in cache → fetch HCDF from URL, verify SHA matches, store
4. Parse HCDF, check model SHAs (if embedded), fetch missing models
5. Cache models in SHA-named directory

### 5. Frontend Frame Visualization

```rust
// dendrite-web/src/app.rs
#[derive(Debug, Clone, Resource, Default)]
pub struct FrameVisibility {
    pub show_frames: bool,
    pub hovered_frame: Option<String>,  // device_id:frame_name
}

// dendrite-web/src/scene.rs
#[derive(Component)]
pub struct FrameGizmo {
    pub device_id: String,
    pub frame_name: String,
    pub description: String,
}
```

**Frame rendering:**
- RGB axis arrows (X=red, Y=green, Z=blue)
- 50% alpha by default, 100% on hover
- Tooltip shows frame name and description on hover
- Toggle via "Show Frames" checkbox in device details panel

### 6. GitHub Pages Model Hosting (hcdf.cognipilot.org)

A new GitHub repository hosts public HCDF files and GLB models:

**Repository structure:**
```
cognipilot/hcdf_models/
├── CNAME                    # Contains: hcdf.cognipilot.org
├── index.html               # Simple landing page / directory listing
├── spinali/
│   ├── v1.0.hcdf
│   ├── v1.1.hcdf
│   └── spinali.glb
├── mr_mcxn_t1/
│   ├── optical-flow.hcdf
│   ├── mcnt1hub.glb
│   └── pmw3901.glb
└── navq95/
    ├── v1.0.hcdf
    └── navq95.glb
```

**GitHub Actions workflow (.github/workflows/pages.yml):**
```yaml
name: Deploy to GitHub Pages

on:
  push:
    branches: [main]

permissions:
  contents: read
  pages: write
  id-token: write

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Setup Pages
        uses: actions/configure-pages@v4
      - name: Upload artifact
        uses: actions/upload-pages-artifact@v3
        with:
          path: '.'
      - name: Deploy to GitHub Pages
        uses: actions/deploy-pages@v4
```

**DNS configuration:**
- Add CNAME record: `hcdf.cognipilot.org` → `cognipilot.github.io`
- GitHub Pages custom domain settings point to `hcdf_models` repo

**URL pattern:**
```
https://hcdf.cognipilot.org/{board}/{version}.hcdf
https://hcdf.cognipilot.org/{board}/{model}.glb
```

### 7. HCDF Model SHA for Smart Caching

Each visual in the HCDF can optionally include a SHA256 hash of the GLB file:

```xml
<visual name="board">
  <pose>0 0 0 0 0 0</pose>
  <model href="models/mcnt1hub.glb" sha="a1b2c3d4..."/>
</visual>
```

**Cache structure with model SHAs:**
```
fragments/cache/
├── manifest.json         # SHA → {url, path, models: {name: sha}}
├── abc123.hcdf           # Cached HCDF (by HCDF SHA)
├── abc123/               # Models for this HCDF version
│   └── mcnt1hub.glb
├── def456.hcdf           # Newer HCDF version
└── def456/
    └── mcnt1hub.glb      # Symlink to abc123/mcnt1hub.glb if SHA matches
```

**Smart caching flow:**
1. Device reports HCDF SHA → check if HCDF cached
2. If HCDF cached → use cached, done
3. If HCDF not cached → fetch, parse, check model SHAs
4. For each model in HCDF:
   - If model SHA matches any existing cached model → symlink/copy
   - If model SHA not found → fetch from URL
5. This avoids re-downloading unchanged GLB files when HCDF is updated

---

## Implementation Steps

### Phase 1: HCDF Fragment Parser
1. Update `Fragment` struct in `dendrite-core/src/fragment.rs`:
   - Add `visuals: Vec<Visual>` and `frames: Vec<Frame>`
   - Remove single `model_path` field
2. Add HCDF fragment loading in `FragmentDatabase`:
   - Parse HCDF files referenced from index.toml
   - Extract visuals and frames from `<comp>` elements
3. Simplify matching: remove priority, use exact vs wildcard

### Phase 2: MCUmgr HCDF Group
4. Add `GROUP_HCDF` and `ID_HCDF_INFO` constants
5. Implement `query_hcdf_info()` function returning URL + SHA
6. Update `probe_device()` to optionally query HCDF info

### Phase 3: Remote Fetching & SHA-Based Caching
7. Add cache directory structure and `CacheManifest` type
8. Implement `fetch_and_cache_hcdf()`:
   - Check cache by HCDF SHA first (skip network if match)
   - HTTP GET with timeout if not cached
   - Verify SHA256 matches device-reported value
   - Store in cache with SHA as filename
9. Implement smart model caching:
   - Parse `sha` attribute from `<model>` elements
   - Build global model SHA index across all cached HCDFs
   - Reuse existing models by SHA (symlink or copy)
   - Only fetch models with unknown SHA
10. Integrate into fragment loading flow

### Phase 4: Frontend Composite Visuals
11. Update `DeviceData` to include `visuals: Vec<VisualData>`
12. Modify `sync_device_entities()` in `models.rs`:
    - Spawn parent entity with `DeviceEntity` component
    - Spawn child entities for each visual with relative transform
13. Update model loading to handle multiple models per device

### Phase 5: Frame Gizmos
14. Add `FrameVisibility` resource and "Show Frames" checkbox
15. Create `FrameGizmo` component and spawn systems
16. Implement hover detection with bevy_picking
17. Add transparency animation and tooltip rendering

### Phase 6: GitHub Pages Model Repository
18. Create `cognipilot/hcdf_models` repository
19. Add CNAME file with `hcdf.cognipilot.org`
20. Create GitHub Actions workflow for Pages deployment
21. Add initial models (spinali, navq95, etc.)
22. Configure DNS CNAME record pointing to GitHub Pages

---

## Files to Modify

### Backend (dendrite-daemon)
| File | Changes |
|------|---------|
| `crates/dendrite-core/src/fragment.rs` | Fragment struct, HCDF loading |
| `crates/dendrite-core/src/hcdf.rs` | Ensure visual/frame parsing complete |
| `crates/dendrite-mcumgr/src/query.rs` | Add HCDF URL query |
| `crates/dendrite-daemon/src/api.rs` | Return visuals/frames in device data |
| `fragments/index.toml` | Update format |

### Frontend (dendrite-web)
| File | Changes |
|------|---------|
| `crates/dendrite-web/src/app.rs` | Add FrameVisibility resource |
| `crates/dendrite-web/src/models.rs` | Composite visual spawning |
| `crates/dendrite-web/src/scene.rs` | Frame gizmo rendering |
| `crates/dendrite-web/src/ui.rs` | "Show Frames" checkbox |
| `crates/dendrite-web/src/network.rs` | Parse visual/frame data |

### New Repository (cognipilot/hcdf_models)
| File | Purpose |
|------|---------|
| `CNAME` | Custom domain: `hcdf.cognipilot.org` |
| `index.html` | Landing page with model directory |
| `.github/workflows/pages.yml` | GitHub Pages deployment |
| `{board}/*.hcdf` | HCDF fragment files per board |
| `{board}/*.glb` | GLB model files per board |

---

## Verification

1. **Unit tests**: Fragment parsing with multiple visuals/frames and model SHAs
2. **Integration test**: Mock MCUmgr HCDF group response (URL + SHA)
3. **Manual testing**:
   - Create test HCDF fragment with 2 visuals + 2 frames
   - Load in browser, verify both models render at correct poses
   - Toggle "Show Frames", verify gizmos appear
   - Hover frame, verify highlight and tooltip
4. **Cache testing**:
   - Restart daemon, verify cached HCDF/models used
   - Update HCDF but keep same model → verify model not re-downloaded
   - Update model SHA → verify only that model re-downloaded
5. **GitHub Pages testing**:
   - Verify `https://hcdf.cognipilot.org/{board}/...` URLs are accessible
   - Test CORS headers for cross-origin model loading
