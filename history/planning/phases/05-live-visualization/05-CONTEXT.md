# Phase 5: Live Visualization - Context

**Gathered:** 2026-01-22
**Status:** Ready for planning

<domain>
## Phase Boundary

Scientists see real-time plots and images updating during acquisition. This phase adds live camera frame display in an image viewer, auto-scaling plots with manual override, and simultaneous multi-detector visualization. Data analysis and post-processing are separate phases.

</domain>

<decisions>
## Implementation Decisions

### Camera Frame Display
- **Fit mode:** User choice between fit-to-window and 1:1 pixel mapping (toggle button)
- **Histogram stretch:** Both auto-contrast button AND manual min/max sliders for fine-tuning
- **Pixel info:** Status bar shows (x, y) coordinates and pixel value under cursor

### Auto-Scale Behavior
- **Trigger:** Only when data exceeds current range (grow to fit, never shrink automatically)
- **Axis independence:** X-axis and Y-axis have separate auto-scale controls (can lock X while Y auto-scales)
- **Image auto-scale:** Intensity and spatial zoom are independent (auto-contrast for intensity, fit-to-window for spatial)

### Multi-Detector Layout
- **Layout style:** Automatic grid layout (1x1, 1x2, 2x2, etc.) based on detector count
- **Plot mixing:** Default mixed grid, but user can rearrange cameras and line plots
- **Panel creation:** All detectors in graph get visualization panels automatically when experiment starts

### Update Rate Tradeoffs
- **Target FPS:** Match camera acquisition rate up to 30 FPS cap
- **Frame skip indication:** Show actual vs display FPS in status (e.g., "60 FPS acquired, 30 FPS displayed")

### Claude's Discretion
- Colormap selection (grayscale default, offer viridis/inferno/turbo options)
- Manual axis range override UX pattern (lock button, double-click reset, etc.)
- Panel collapse/minimize behavior (whether to include, how it works)
- High-res image downsampling strategy (auto-downsample vs always full-res)
- Background window update behavior (pause when minimized or always update)

</decisions>

<specifics>
## Specific Ideas

No specific references or examples provided — open to standard scientific visualization patterns.

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope.

</deferred>

---

*Phase: 05-live-visualization*
*Context gathered: 2026-01-22*
