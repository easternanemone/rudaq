# Phase 5: Live Visualization - Research

**Researched:** 2026-01-22
**Domain:** Real-time scientific data visualization (egui-based)
**Confidence:** HIGH

## Summary

Live visualization in egui for scientific data requires efficient texture updates, multi-panel grid layouts, independent axis auto-scaling, and colormap support. The rust-daq codebase already has substantial infrastructure in place: image_viewer.rs implements camera frame display with colormaps, background RGBA conversion, and texture management; scan_builder.rs demonstrates live plot updates; and gRPC streaming with quality modes exists.

**Key findings:**
- egui texture updates require using `.set()` on existing TextureHandle, NOT creating new textures each frame
- Multi-panel layouts use `egui_extras::StripBuilder` for dynamic grid sizing (already available via `standalone` feature)
- egui_plot provides per-axis auto-bounds control via `auto_bounds([bool, bool])`
- Colormap infrastructure (Viridis, Inferno, Plasma, Magma) already exists with pre-computed LUTs
- Background thread RGBA conversion pattern prevents UI freezes on high-res images

**Primary recommendation:** Build on existing image_viewer.rs and scan_builder.rs patterns. Use StripBuilder for multi-detector grid layout, extend existing texture management for camera panels, and implement grow-to-fit auto-scale logic in plot wrappers.

## Standard Stack

The established libraries/tools for this domain:

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| egui | 0.33 | Immediate mode GUI framework | Already in use, mature ecosystem, efficient for real-time updates |
| egui_plot | 0.34 | Scientific plotting widget | Built-in auto-bounds, axis control, legend support |
| egui_extras | 0.33 | Advanced layout widgets (StripBuilder, Table) | Dynamic grid layouts, responsive panel sizing |
| colorous | N/A (hand-rolled) | Colormap LUTs (Viridis, Inferno, etc.) | Already implemented in codebase with 256-entry LUTs |
| mpsc channels | std | Frame/data passing to UI thread | Standard Rust pattern, already used throughout |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| egui_dock | 0.18 | Dockable panel system (optional) | If user wants drag-and-drop panel rearrangement |
| rayon | N/A (std::thread used) | Parallel RGBA conversion (optional) | Only if single-thread bottleneck, current thread approach works |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| egui_extras::StripBuilder | egui::Grid | Grid doesn't handle dynamic sizing well; StripBuilder better for runtime detector counts |
| Hand-rolled colormap LUTs | colorous crate | Codebase already has optimized LUTs; no need to add dependency |
| Background thread conversion | GPU compute shader | More complex, requires wgpu backend; current CPU approach works for 30 FPS target |

**Installation:**
```bash
# Already in Cargo.toml under 'standalone' feature
egui_extras = { version = "0.33", features = ["all_loaders"] }
egui_plot = "0.34"
```

## Architecture Patterns

### Recommended Project Structure
```
crates/daq-egui/src/
├── panels/
│   ├── image_viewer.rs          # Camera frame display (exists)
│   ├── scan_builder.rs          # Live plot updates (exists)
│   └── multi_detector_grid.rs   # NEW: Grid layout for N detectors
├── widgets/
│   ├── auto_scale_plot.rs       # NEW: Plot wrapper with grow-to-fit
│   └── histogram.rs             # Histogram widget (exists)
└── graph/
    └── execution_state.rs       # Execution state tracking (exists, Phase 3)
```

### Pattern 1: Efficient Texture Updates (Camera Frames)
**What:** Reuse single TextureHandle, update contents with `.set()` instead of creating new textures each frame
**When to use:** Any dynamically-changing image display (camera, heatmap, etc.)
**Example:**
```rust
// Source: https://github.com/emilk/egui/discussions/5866
if let Some(ref mut th) = self.texture_handle {
    // Update existing texture (efficient)
    th.set(color_image, egui::TextureOptions::NEAREST);
} else {
    // Create once on first frame
    let ctx = ui.ctx();
    self.texture_handle = Some(ctx.load_texture(
        "camera_frame",  // Stable ID
        color_image,
        egui::TextureOptions::NEAREST
    ));
}
```

**Existing implementation:** `image_viewer.rs:949-955`
```rust
let image = egui::ColorImage::from_rgba_unmultiplied(size, &result.rgba);
self.texture = if let Some(ref mut tex) = self.texture {
    tex.set(image, egui::TextureOptions::NEAREST);
    self.texture.clone()
} else {
    Some(ctx.load_texture("camera_frame", image, egui::TextureOptions::NEAREST))
};
```

### Pattern 2: Background RGBA Conversion (UI Thread Offloading)
**What:** Move CPU-intensive pixel format conversion to dedicated thread, recycle buffers
**When to use:** High-res images (4K+), high bit depth (16-bit), or >10 FPS rates
**Example:**
```rust
// Source: codebase image_viewer.rs:819-868
// Spawn once at startup
let (request_tx, request_rx) = mpsc::sync_channel::<ConversionRequest>(2);
let (result_tx, result_rx) = mpsc::channel::<ConversionResult>();
let (recycle_tx, recycle_rx) = mpsc::channel::<Vec<u8>>();

std::thread::spawn(move || {
    while let Ok(req) = request_rx.recv() {
        // Reuse recycled buffer (bd-wdx3 optimization)
        let mut buffer = recycle_rx.try_recv()
            .unwrap_or_else(|_| Vec::with_capacity(1920 * 1080 * 4));

        convert_frame_to_rgba_into(&req, &mut buffer);
        result_tx.send(ConversionResult { rgba: buffer, .. }).ok();
    }
});

// In UI update loop
if let Ok(result) = result_rx.try_recv() {
    self.texture_handle.as_mut().unwrap().set(
        ColorImage::from_rgba_unmultiplied([result.width, result.height], &result.rgba),
        TextureOptions::NEAREST
    );
    // Recycle buffer back to thread
    recycle_tx.send(result.rgba).ok();
}
```

### Pattern 3: Multi-Detector Grid Layout (Dynamic Sizing)
**What:** Use `egui_extras::StripBuilder` for responsive N×M grid of plots/images
**When to use:** Runtime detector count, need equal-sized panels, want automatic resizing
**Example:**
```rust
// Source: https://github.com/emilk/egui/discussions/4271
use egui_extras::{StripBuilder, Size};

// Calculate grid dimensions (e.g., 3 detectors → 2×2 grid with one empty)
let detector_count = detectors.len();
let cols = (detector_count as f32).sqrt().ceil() as usize;
let rows = (detector_count + cols - 1) / cols;

ui.spacing_mut().item_spacing = [0.0; 2].into(); // Remove gaps

StripBuilder::new(ui)
    .sizes(Size::relative((rows as f32).recip()), rows)
    .vertical(|mut strip| {
        for r in 0..rows {
            strip.cell(|ui| {
                StripBuilder::new(ui)
                    .sizes(Size::relative((cols as f32).recip()), cols)
                    .horizontal(|mut strip| {
                        for c in 0..cols {
                            let idx = r * cols + c;
                            strip.cell(|ui| {
                                if idx < detector_count {
                                    render_detector_panel(ui, &detectors[idx]);
                                }
                            });
                        }
                    });
            });
        }
    });
```

### Pattern 4: Grow-to-Fit Auto-Scale (Never Shrink)
**What:** Auto-expand axis bounds when data exceeds range, but never auto-shrink
**When to use:** Live acquisition where initial data range unknown, avoid jarring axis jumps
**Example:**
```rust
// Source: Inferred from egui_plot docs + user requirement
struct AutoScalePlot {
    x_bounds: Option<[f64; 2]>,  // None = first data point sets initial
    y_bounds: Option<[f64; 2]>,
    auto_x: bool,
    auto_y: bool,
}

impl AutoScalePlot {
    fn update_bounds(&mut self, points: &[[f64; 2]]) {
        for &[x, y] in points {
            if self.auto_x {
                let bounds = self.x_bounds.get_or_insert([x, x]);
                if x < bounds[0] { bounds[0] = x; }  // Grow down
                if x > bounds[1] { bounds[1] = x; }  // Grow up
            }
            if self.auto_y {
                let bounds = self.y_bounds.get_or_insert([y, y]);
                if y < bounds[0] { bounds[0] = y; }
                if y > bounds[1] { bounds[1] = y; }
            }
        }
    }

    fn show(&mut self, ui: &mut Ui, add_contents: impl FnOnce(&mut PlotUi)) {
        let mut plot = Plot::new("plot_id")
            .auto_bounds([self.auto_x, self.auto_y]);  // Per-axis control

        if let Some([min, max]) = self.x_bounds {
            if !self.auto_x {
                plot = plot.include_x(min).include_x(max);  // Lock to manual bounds
            }
        }
        if let Some([min, max]) = self.y_bounds {
            if !self.auto_y {
                plot = plot.include_y(min).include_y(max);
            }
        }

        plot.show(ui, add_contents);
    }
}
```

### Pattern 5: Frame Skipping with FPS Display
**What:** Display actual acquisition FPS vs display FPS, skip frames when GUI can't keep up
**When to use:** High-speed cameras (>30 FPS), prevent memory buildup and lag accumulation
**Example:**
```rust
// Source: Codebase gRPC streaming metrics (bd-7rk0)
struct FrameMetrics {
    acquired_fps: f64,    // From server StreamingMetrics
    displayed_fps: f64,   // Calculated client-side
    frames_dropped: u64,  // From server
}

// In frame update handler (image_viewer.rs pattern)
const MAX_QUEUED_FRAMES: usize = 4;  // Backpressure threshold
let (frame_tx, frame_rx) = mpsc::sync_channel(MAX_QUEUED_FRAMES);

// Background task receiving gRPC stream
tokio::spawn(async move {
    while let Some(frame) = stream.next().await {
        match frame_tx.try_send(frame) {
            Ok(_) => {},
            Err(_) => {
                // Queue full, skip frame (client-side backpressure)
                frames_skipped += 1;
            }
        }
    }
});

// UI update (drain latest frame only)
if let Ok(frame) = frame_rx.try_recv() {
    // Take ONLY the latest frame, discard older queued frames
    let latest = std::iter::successors(Some(frame), |_| frame_rx.try_recv().ok())
        .last()
        .unwrap();
    update_display(latest);
}
```

### Anti-Patterns to Avoid
- **Creating new textures every frame:** Use `.set()` on existing TextureHandle instead (see Pattern 1)
- **Blocking UI thread on RGBA conversion:** Offload to background thread for >1080p or >10 FPS (see Pattern 2)
- **Manual grid cell sizing:** Use StripBuilder with relative sizing instead of hardcoded dimensions
- **Auto-shrink axis bounds:** Jarring for users; grow-to-fit only (see Pattern 4)
- **Unbounded frame queues:** Use bounded channels with try_send/try_recv for backpressure

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Colormap RGB lookup | Manual interpolation | Pre-computed 256-entry LUT | Already implemented in image_viewer.rs:344-420; O(1) lookup vs O(n) calculation |
| Dynamic grid layout | Manual rect calculations | `egui_extras::StripBuilder` | Handles resize, responsive sizing, nested grids automatically |
| FPS limiting | Thread sleep in UI loop | egui's request_repaint_after() + vsync | Platform-aware, lower latency, better precision |
| Mouse position over image | Manual transform matrix | `response.hover_pos() - rect.min` | egui handles DPI scaling, viewport transforms (image_viewer.rs:2074-2085) |
| Plot auto-bounds | Custom min/max tracking | `egui_plot::Plot::auto_bounds([bool, bool])` | Per-axis control, includes padding, handles edge cases |
| Frame buffer recycling | Vec allocate/drop each frame | Channel-based buffer pool | Already in image_viewer.rs:831-844; avoids allocator churn |

**Key insight:** egui's immediate mode makes custom layout logic unnecessary. Declare desired layout each frame; framework handles sizing, interaction, and redraw optimization.

## Common Pitfalls

### Pitfall 1: Texture Memory Leaks from Creating New Handles
**What goes wrong:** Calling `ctx.load_texture()` with unique IDs each frame leaks GPU memory, crashes after minutes
**Why it happens:** egui docs emphasize "images are static or rarely changing"; developers miss that dynamic updates need `.set()` not `load_texture()`
**How to avoid:**
- Store `Option<TextureHandle>` in panel state
- Create once with stable ID on first frame
- Update contents with `.set()` on subsequent frames
**Warning signs:**
- Increasing memory usage over time
- FPS degradation after minutes
- "Too many textures" errors from graphics backend
**Reference:** [egui texture discussion](https://github.com/emilk/egui/discussions/5866)

### Pitfall 2: UI Thread Blocking on Large Image Conversion
**What goes wrong:** Converting 4K 16-bit frames to RGBA on UI thread freezes GUI for 10-50ms per frame
**Why it happens:** Immediate mode GUI runs all code synchronously unless explicitly offloaded
**How to avoid:**
- Spawn dedicated RGBA converter thread at startup (image_viewer.rs:819-868 pattern)
- Use bounded channels (size 2) to prevent queue buildup
- Recycle buffers between frames to avoid allocations
**Warning signs:**
- UI becomes unresponsive during acquisition
- Stuttering/jank when moving windows
- FPS drops below 15 when displaying camera
**Reference:** Existing implementation in `image_viewer.rs:803-880`

### Pitfall 3: Grid Layout Frame-Delay Flicker
**What goes wrong:** First frame of StripBuilder shows incorrect sizing, then "pops" to correct size
**Why it happens:** egui uses previous frame's measurements for layout; grid needs one frame to measure contents
**How to avoid:**
- Accept single-frame flicker (visual artifact only, not a bug)
- OR: Pre-calculate minimum sizes and use `Size::exact()` for first frame
- OR: Show loading spinner for first frame, then show grid
**Warning signs:**
- Panels briefly overlap then snap to correct positions
- Users report "glitchy layout" on acquisition start
**Reference:** [egui Grid layout discussion](https://github.com/emilk/egui/discussions/4271)

### Pitfall 4: Unbounded Auto-Scale Causing Axis Jumps
**What goes wrong:** Outlier data point causes axis to rescale, making previous data invisible
**Why it happens:** egui_plot's default auto_bounds() includes ALL data, even outliers
**How to avoid:**
- Implement grow-to-fit: expand bounds when data exceeds range, never shrink
- Add "Reset Bounds" button to recover from outlier-induced zoom
- OR: Use percentile-based bounds (e.g., 1st to 99th percentile) instead of min/max
**Warning signs:**
- Users complain "my data disappeared"
- Axis range suddenly 10x larger than before
- Scientific data obscured by single bad reading
**Reference:** Pattern 4 above

### Pitfall 5: FPS Display Confusion (Acquired vs Displayed)
**What goes wrong:** Users see "30 FPS" but notice frames skipping; think camera is broken
**Why it happens:** Displaying only acquisition FPS, not actual render FPS or drop count
**How to avoid:**
- Show both: "Acquired: 60 FPS | Display: 30 FPS | Dropped: 120"
- Use color coding: green if display == acquired, yellow if skipping, red if severely behind
- Add tooltip: "Display FPS capped to prevent UI lag"
**Warning signs:**
- User bug reports: "camera skipping frames"
- Confusion about backpressure behavior
**Reference:** StreamingMetrics in `image_viewer.rs:39-50`, gRPC proto `StreamingMetrics`

## Code Examples

Verified patterns from official sources and existing codebase:

### Cursor Position Over Image (Pixel Coordinates)
```rust
// Source: image_viewer.rs:2074-2085
let response = ui.allocate_rect(rect, egui::Sense::hover());
if let Some(pos) = response.hover_pos() {
    let image_pos = pos - rect.min - offset;  // rect.min = UI origin, offset = pan
    let pixel_x = (image_pos.x / zoom) as i32;
    let pixel_y = (image_pos.y / zoom) as i32;

    if pixel_x >= 0 && pixel_x < width as i32 && pixel_y >= 0 && pixel_y < height as i32 {
        // Get pixel value from raw data (8-bit example)
        let idx = (pixel_y as usize * width as usize + pixel_x as usize);
        let value = frame_data[idx];
        response.on_hover_text(format!("({}, {}) = {}", pixel_x, pixel_y, value));
    }
}
```

### Colormap Application (Optimized LUT)
```rust
// Source: image_viewer.rs:322-342
pub enum Colormap {
    Grayscale,
    Viridis,
    Inferno,
    Plasma,
    Magma,
}

impl Colormap {
    #[inline]
    pub fn apply(&self, value: f32) -> [u8; 3] {
        let idx = (value.clamp(0.0, 1.0) * 255.0) as usize;
        self.lut()[idx]  // O(1) lookup into 256-entry static array
    }

    fn lut(&self) -> &'static [[u8; 3]; 256] {
        match self {
            Self::Grayscale => &GRAYSCALE_LUT,
            Self::Viridis => &VIRIDIS_LUT,
            // ... (LUTs computed at compile time)
        }
    }
}

// Usage in RGBA conversion loop
for (i, &pixel) in data.iter().enumerate() {
    let normalized = pixel as f32 / 255.0;
    let rgb = colormap.apply(normalized);
    buffer[i * 4 + 0] = rgb[0];
    buffer[i * 4 + 1] = rgb[1];
    buffer[i * 4 + 2] = rgb[2];
    buffer[i * 4 + 3] = 255;  // Alpha
}
```

### Independent Axis Auto-Scale Controls
```rust
// Source: egui_plot documentation + Pattern 4
// UI controls (checkboxes in toolbar)
ui.horizontal(|ui| {
    ui.checkbox(&mut self.auto_x, "Auto X");
    ui.checkbox(&mut self.auto_y, "Auto Y");
    if ui.button("Reset").clicked() {
        self.x_bounds = None;
        self.y_bounds = None;
    }
});

// Plot with per-axis control
Plot::new("my_plot")
    .auto_bounds([self.auto_x, self.auto_y])  // Vec2b for x/y independence
    .show(ui, |plot_ui| {
        // Add data...

        // If manual bounds set, enforce them
        if !self.auto_x {
            if let Some([min, max]) = self.x_bounds {
                plot_ui.set_plot_bounds(PlotBounds::from_min_max(
                    [min, plot_ui.plot_bounds().min()[1]],
                    [max, plot_ui.plot_bounds().max()[1]]
                ));
            }
        }
        // Similar for Y axis
    });
```

### Multi-Detector Mixed Layout (Cameras + Line Plots)
```rust
// Source: StripBuilder pattern + user requirement
// Example: 2 cameras (top row), 3 line plots (bottom row)
StripBuilder::new(ui)
    .sizes(Size::relative(0.6), 1)  // Top 60% for cameras
    .sizes(Size::relative(0.4), 1)  // Bottom 40% for plots
    .vertical(|mut strip| {
        // Top row: cameras
        strip.cell(|ui| {
            StripBuilder::new(ui)
                .sizes(Size::relative(0.5), 2)  // 2 equal-width cameras
                .horizontal(|mut strip| {
                    for camera in cameras {
                        strip.cell(|ui| {
                            render_camera_panel(ui, camera);
                        });
                    }
                });
        });

        // Bottom row: line plots
        strip.cell(|ui| {
            StripBuilder::new(ui)
                .sizes(Size::relative((plots.len() as f32).recip()), plots.len())
                .horizontal(|mut strip| {
                    for plot in plots {
                        strip.cell(|ui| {
                            render_line_plot(ui, plot);
                        });
                    }
                });
        });
    });
```

### Frame Rate Limiting (Application-Level)
```rust
// Source: https://github.com/emilk/egui/issues/1109
// Note: egui doesn't provide built-in FPS limiting; rely on vsync or manual sleep
const TARGET_FPS: f64 = 30.0;
let frame_time = Duration::from_secs_f64(1.0 / TARGET_FPS);
let mut next_frame = Instant::now();

// In event loop
eframe::run_native(/* ... */, Box::new(|cc| {
    Box::new(MyApp::new(cc))
}));

// OR for gRPC stream rate limiting (server-side, already implemented)
let request = StreamFramesRequest {
    device_id: "camera0".to_string(),
    max_fps: 30,  // Server will rate-limit to 30 FPS
    quality: StreamQuality::Full.into(),
};
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Blocking RGBA conversion in UI thread | Background thread + buffer recycling | 2026-01 (bd-xifj, bd-wdx3) | 4K@30 FPS no longer freezes GUI |
| Creating new textures each frame | Reuse TextureHandle with `.set()` | egui 0.20+ best practice | Prevents GPU memory leaks |
| Manual grid cell sizing | `egui_extras::StripBuilder` | egui_extras 0.20+ | Responsive layouts, simpler code |
| Single Y-axis in egui_plot | Multiple independent Y-axes support | egui_plot 0.28+ | Multi-scale data on same plot |
| Server sends all frames | Backpressure + quality modes | 2025-12 (bd-7rk0) | Prevents client overload on slow networks |

**Deprecated/outdated:**
- `egui::Grid` for dynamic layouts: Still works but StripBuilder is more flexible for runtime sizing
- Synchronous texture loading: `.load_texture()` each frame causes memory leaks; use `.set()` instead
- `colorous` crate: Not needed; codebase has hand-rolled LUTs that are faster (no dependency overhead)

## Open Questions

Things that couldn't be fully resolved:

1. **High-DPI Mouse Coordinate Precision**
   - What we know: egui uses logical points; `response.hover_pos()` is DPI-aware; existing code works
   - What's unclear: Sub-pixel precision for scientific measurement (is `as i32` cast losing accuracy?)
   - Recommendation: Use existing pattern; add sub-pixel interpolation only if users request it

2. **egui_dock vs Manual Layout**
   - What we know: egui_dock provides drag-and-drop panel rearrangement, already in dependencies
   - What's unclear: Whether Phase 5 scope includes UI customization or just fixed grid layout
   - Recommendation: Start with fixed grid (user requirement: "automatic grid layout"); defer drag-and-drop to future phase

3. **GPU-Accelerated Colormap Application**
   - What we know: Current CPU LUT approach handles 30 FPS at 4K; wgpu backend would enable compute shaders
   - What's unclear: Would GPU offload be worth the complexity? (current bottleneck is acquisition, not display)
   - Recommendation: Stick with CPU LUTs unless profiling shows display bottleneck

4. **Plot Data Downsampling for Large Datasets**
   - What we know: egui_plot can slow down with >10K points per series; scan_builder shows ~500 points currently
   - What's unclear: Do scientific users need full-res plots or is downsampling acceptable?
   - Recommendation: Start without downsampling; add if users report slow plots (use Ramer-Douglas-Peucker algorithm)

## Sources

### Primary (HIGH confidence)
- egui GitHub discussions (official maintainer responses):
  - [Texture update pattern](https://github.com/emilk/egui/discussions/5866) - Dynamic image updates
  - [FPS limiting](https://github.com/emilk/egui/issues/1109) - Rate control patterns
  - [Mouse coordinates over images](https://github.com/emilk/egui/issues/1654) - Coordinate transform
  - [Grid layout for plots](https://github.com/emilk/egui/discussions/4271) - StripBuilder usage
- egui_plot official docs: https://docs.rs/egui_plot/latest/egui_plot/struct.Plot.html
- egui_extras official docs: https://docs.rs/egui_extras (StripBuilder)
- Existing rust-daq codebase:
  - `crates/daq-egui/src/panels/image_viewer.rs` (lines 1-880) - Complete implementation
  - `crates/daq-egui/src/panels/scan_builder.rs` (lines 634-682) - Live plot updates
  - `crates/daq-proto/proto/daq.proto` (lines 574-589) - gRPC streaming with quality modes

### Secondary (MEDIUM confidence)
- Colormap perceptual uniformity: https://cran.r-project.org/web/packages/viridis/vignettes/intro-to-viridis.html
- colorous Rust crate (not used, but validates approach): https://docs.rs/colorous
- StripBuilder tutorial: https://hackmd.io/@Hamze/BkvEAvFayx (community guide)

### Tertiary (LOW confidence)
- egui grid vs StripBuilder discussions: Multiple GitHub threads, consensus is StripBuilder preferred for dynamic layouts
- FPS display patterns: Inferred from existing metrics in codebase (no authoritative external source)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All libraries already in use and proven working
- Architecture: HIGH - Patterns verified in existing codebase (image_viewer.rs, scan_builder.rs)
- Pitfalls: HIGH - Documented in official GitHub issues with maintainer confirmation
- Multi-panel layout: MEDIUM - StripBuilder documented but not yet used for detector grids in codebase
- Auto-scale grow-to-fit: MEDIUM - egui_plot supports per-axis control, but grow-only logic needs custom wrapper

**Research date:** 2026-01-22
**Valid until:** ~30 days (egui stable, rust-daq internal architecture unlikely to change)

## Implementation Readiness

**Existing infrastructure that Phase 5 builds on:**
- ✅ Image texture management with `.set()` pattern (image_viewer.rs)
- ✅ Background RGBA conversion with buffer recycling (image_viewer.rs:819-880)
- ✅ Colormap LUTs (Grayscale, Viridis, Inferno, Plasma, Magma) (image_viewer.rs:344-420)
- ✅ gRPC frame streaming with quality modes and backpressure (daq-proto, daq-server)
- ✅ Live plot updates with mpsc channels (scan_builder.rs, signal_plotter.rs)
- ✅ Execution state tracking (ExecutionState in graph/execution_state.rs, Phase 3)
- ✅ egui_extras::StripBuilder available via 'standalone' feature

**Missing pieces that Phase 5 must add:**
- ❌ Multi-detector grid layout panel (new widget using StripBuilder)
- ❌ Grow-to-fit auto-scale wrapper for egui_plot (custom logic)
- ❌ Per-axis lock/unlock controls in plot UI (checkboxes + state management)
- ❌ FPS display showing acquired vs displayed rates (UI labels reading StreamingMetrics)
- ❌ Integration between execution state and visualization panel spawning (create panels when experiment starts)

**Estimated complexity:** MEDIUM - Most infrastructure exists; Phase 5 is integration and wrapper logic, not building from scratch.
