# Line Profile Widget Integration Guide

## Overview
The `LineProfileWidget` provides intensity profile analysis along user-defined lines in images.

## Integration with ImageViewerPanel

### 1. Add widget to ImageViewerPanel struct

```rust
use crate::widgets::LineProfileWidget;

pub struct ImageViewerPanel {
    // ... existing fields ...
    line_profile: LineProfileWidget,
}
```

### 2. Initialize in constructor

```rust
impl ImageViewerPanel {
    pub fn new(...) -> Self {
        Self {
            // ... existing init ...
            line_profile: LineProfileWidget::new(),
        }
    }
}
```

### 3. Update profiles when frame arrives

In the `handle_frame_updates` or similar method:

```rust
if let Some(data) = &self.last_frame_data {
    self.line_profile.update_profiles(
        data,
        self.width,
        self.height,
        self.bit_depth,
    );
}
```

### 4. Handle input on the image

In the image rendering section, after displaying the image texture:

```rust
let pointer_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();
let primary_down = ui.input(|i| i.pointer.primary_down());
let primary_released = ui.input(|i| i.pointer.primary_released());

self.line_profile.handle_input(
    ui,
    image_rect,
    pointer_pos,
    primary_down,
    primary_released,
);

// Draw overlays
self.line_profile.draw_overlays(ui, image_rect, Some(pointer_pos));
```

### 5. Add toolbar controls

In the toolbar UI:

```rust
ui.separator();
ui.toggle_value(&mut self.line_profile.show_plot, "📈 Profile");
ui.toggle_value(&mut self.line_profile.show_stats, "📊 Stats");
if ui.button("Clear Lines").clicked() {
    self.line_profile.clear();
}
```

### 6. Show side panels (optional)

For side-by-side display:

```rust
// In a side panel or separate window:
if self.line_profile.show_plot {
    ui.group(|ui| {
        ui.heading("Intensity Profile");
        self.line_profile.show_plot_panel(ui);
    });
}

if self.line_profile.show_stats {
    ui.group(|ui| {
        self.line_profile.show_stats_panel(ui);
    });
}
```

## Features

### Drawing Lines
- Click and drag on the image to draw a new line
- Lines are automatically colored from a palette
- Multiple lines can be active simultaneously

### Editing Lines
- Click near an existing line to toggle edit mode
- Lines in edit mode are not included in profile extraction

### Statistics
Each profile provides:
- **Min**: Minimum intensity along the line
- **Max**: Maximum intensity along the line  
- **Mean**: Average intensity along the line
- **FWHM**: Full Width at Half Maximum (useful for beam profiling)

### CSV Export
Click "Export to CSV" in the stats panel to copy profile data to clipboard.
Format: `Distance (px), Intensity`

## Use Cases

1. **Beam Profiling**: Measure laser spot diameter and intensity distribution
2. **Edge Analysis**: Evaluate MTF (Modulation Transfer Function) for system characterization
3. **Spectral Lines**: Analyze line width and shape in spectroscopy images
4. **Quality Control**: Measure feature dimensions and uniformity

## API Reference

### LineProfileWidget Methods

- `new()` - Create new widget
- `update_profiles(data, width, height, bit_depth)` - Update from current frame
- `handle_input(ui, rect, pos, down, released)` - Process mouse interaction
- `draw_overlays(ui, rect, mouse_pos)` - Draw lines on image
- `show_plot_panel(ui)` - Render intensity plot
- `show_stats_panel(ui)` - Render statistics panel
- `clear()` - Remove all lines
- `num_lines()` - Get number of active lines

### Data Structures

- `LineSelection` - A line drawn on the image
- `IntensityProfile` - Extracted intensity data along a line
- `ProfileStats` - Statistical measures (min, max, mean, FWHM)
