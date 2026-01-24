//! Auto-scale plot widget with grow-to-fit logic.
//!
//! Provides a plot wrapper that expands axis bounds to fit data but never shrinks automatically,
//! preventing jarring visual jumps during live acquisition. Supports per-axis lock controls.

use egui::{Response, Ui, Vec2b};
use egui_plot::Plot;

/// Per-axis lock state for plot scaling.
#[derive(Debug, Clone, Default)]
pub struct AxisLockState {
    /// Whether X-axis is locked (true) or auto-scaling (false)
    pub x_locked: bool,
    /// Whether Y-axis is locked (true) or auto-scaling (false)
    pub y_locked: bool,
    /// Current X-axis bounds [min, max]. None = uninitialized (first data sets initial bounds)
    pub x_bounds: Option<[f64; 2]>,
    /// Current Y-axis bounds [min, max]. None = uninitialized (first data sets initial bounds)
    pub y_bounds: Option<[f64; 2]>,
}

/// Auto-scale plot wrapper with grow-to-fit logic.
///
/// ## Behavior
///
/// - **Grow-only**: Axis bounds expand when new data exceeds current range, but never shrink
/// - **Per-axis lock**: X and Y axes can be locked independently
/// - **Reset**: Clears bounds and unlocks both axes
///
/// ## Example
///
/// ```ignore
/// use daq_egui::widgets::{AutoScalePlot, AxisLockState};
/// use egui_plot::{Line, PlotPoints};
///
/// let mut plot = AutoScalePlot::new(AxisLockState::default());
/// let points = vec![[0.0, 1.0], [1.0, 2.0], [2.0, 1.5]];
///
/// // Update bounds to fit new data
/// plot.update_bounds(&points);
///
/// // Render with controls (inside an egui frame)
/// plot.show_with_controls(ui, "my_plot", |plot_ui| {
///     plot_ui.line(Line::new(PlotPoints::from_iter(points.iter().copied())));
/// });
/// ```
pub struct AutoScalePlot {
    state: AxisLockState,
}

impl AutoScalePlot {
    /// Create a new auto-scale plot with the given initial state.
    pub fn new(state: AxisLockState) -> Self {
        Self { state }
    }

    /// Get a reference to the current axis lock state.
    #[allow(dead_code)]
    pub fn state(&self) -> &AxisLockState {
        &self.state
    }

    /// Get a mutable reference to the axis lock state.
    #[allow(dead_code)]
    pub fn state_mut(&mut self) -> &mut AxisLockState {
        &mut self.state
    }

    /// Update bounds to fit new data points (grow-only).
    ///
    /// For unlocked axes, expands bounds to include all points. For locked axes, does nothing.
    /// On first call (bounds = None), initializes bounds from data.
    pub fn update_bounds(&mut self, points: &[[f64; 2]]) {
        if points.is_empty() {
            return;
        }

        // Calculate data extents
        let mut x_min = points[0][0];
        let mut x_max = points[0][0];
        let mut y_min = points[0][1];
        let mut y_max = points[0][1];

        for &[x, y] in points.iter().skip(1) {
            x_min = x_min.min(x);
            x_max = x_max.max(x);
            y_min = y_min.min(y);
            y_max = y_max.max(y);
        }

        // Update X bounds (only if unlocked)
        if !self.state.x_locked {
            match &mut self.state.x_bounds {
                Some(bounds) => {
                    // Grow-only: expand if data exceeds current bounds
                    bounds[0] = bounds[0].min(x_min);
                    bounds[1] = bounds[1].max(x_max);
                }
                None => {
                    // Initialize bounds from first data
                    self.state.x_bounds = Some([x_min, x_max]);
                }
            }
        }

        // Update Y bounds (only if unlocked)
        if !self.state.y_locked {
            match &mut self.state.y_bounds {
                Some(bounds) => {
                    // Grow-only: expand if data exceeds current bounds
                    bounds[0] = bounds[0].min(y_min);
                    bounds[1] = bounds[1].max(y_max);
                }
                None => {
                    // Initialize bounds from first data
                    self.state.y_bounds = Some([y_min, y_max]);
                }
            }
        }
    }

    /// Reset bounds to uninitialized state and unlock both axes.
    ///
    /// Next call to `update_bounds` will initialize from data.
    pub fn reset_bounds(&mut self) {
        self.state.x_bounds = None;
        self.state.y_bounds = None;
        self.state.x_locked = false;
        self.state.y_locked = false;
    }

    /// Show the plot with custom content.
    ///
    /// # Arguments
    ///
    /// - `ui`: The egui UI context
    /// - `id_salt`: Unique identifier for the plot (for egui state persistence)
    /// - `add_contents`: Closure that adds plot contents (lines, points, etc.)
    pub fn show<R>(
        &self,
        ui: &mut Ui,
        id_salt: impl std::hash::Hash,
        add_contents: impl FnOnce(&mut egui_plot::PlotUi) -> R,
    ) -> Response {
        let mut plot = Plot::new(id_salt);

        // Configure auto-bounds for unlocked axes
        let auto_bounds_vec = Vec2b::new(!self.state.x_locked, !self.state.y_locked);
        plot = plot.auto_bounds(auto_bounds_vec);

        // Enforce manual bounds for locked axes
        if self.state.x_locked || self.state.y_locked {
            if let (Some(x_bounds), Some(y_bounds)) = (&self.state.x_bounds, &self.state.y_bounds) {
                // For locked axes, enforce the stored bounds using include_x/include_y
                if self.state.x_locked {
                    plot = plot.include_x(x_bounds[0]).include_x(x_bounds[1]);
                }
                if self.state.y_locked {
                    plot = plot.include_y(y_bounds[0]).include_y(y_bounds[1]);
                }
            }
        }

        plot.show(ui, add_contents).response
    }

    /// Show the plot with lock/unlock controls and reset button.
    ///
    /// Renders a toolbar with:
    /// - Lock X checkbox
    /// - Lock Y checkbox
    /// - Reset button
    ///
    /// Then renders the plot below using remaining space.
    pub fn show_with_controls<R>(
        &mut self,
        ui: &mut Ui,
        id_salt: impl std::hash::Hash + Clone,
        add_contents: impl FnOnce(&mut egui_plot::PlotUi) -> R,
    ) -> Response {
        ui.vertical(|ui| {
            // Toolbar
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.state.x_locked, "Lock X");
                ui.checkbox(&mut self.state.y_locked, "Lock Y");
                if ui.button("Reset").clicked() {
                    self.reset_bounds();
                }
            });

            // Plot
            self.show(ui, id_salt, add_contents)
        })
        .inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounds_grow_only() {
        let mut plot = AutoScalePlot::new(AxisLockState::default());

        // First data: initialize bounds
        let points1 = vec![[0.0, 0.0], [1.0, 1.0]];
        plot.update_bounds(&points1);
        assert_eq!(plot.state.x_bounds, Some([0.0, 1.0]));
        assert_eq!(plot.state.y_bounds, Some([0.0, 1.0]));

        // Second data: exceeds upper bound (should grow)
        let points2 = vec![[0.5, 0.5], [1.5, 2.0]];
        plot.update_bounds(&points2);
        assert_eq!(plot.state.x_bounds, Some([0.0, 1.5]));
        assert_eq!(plot.state.y_bounds, Some([0.0, 2.0]));

        // Third data: within bounds (should NOT shrink)
        let points3 = vec![[0.2, 0.3], [0.8, 0.9]];
        plot.update_bounds(&points3);
        assert_eq!(plot.state.x_bounds, Some([0.0, 1.5])); // No change
        assert_eq!(plot.state.y_bounds, Some([0.0, 2.0])); // No change

        // Fourth data: exceeds lower bound (should grow)
        let points4 = vec![[-1.0, -0.5], [0.5, 0.5]];
        plot.update_bounds(&points4);
        assert_eq!(plot.state.x_bounds, Some([-1.0, 1.5]));
        assert_eq!(plot.state.y_bounds, Some([-0.5, 2.0]));
    }

    #[test]
    fn test_axis_lock_prevents_update() {
        let mut plot = AutoScalePlot::new(AxisLockState::default());

        // Initialize bounds
        let points1 = vec![[0.0, 0.0], [1.0, 1.0]];
        plot.update_bounds(&points1);
        assert_eq!(plot.state.x_bounds, Some([0.0, 1.0]));
        assert_eq!(plot.state.y_bounds, Some([0.0, 1.0]));

        // Lock X axis
        plot.state_mut().x_locked = true;

        // New data exceeds bounds
        let points2 = vec![[-1.0, -1.0], [2.0, 2.0]];
        plot.update_bounds(&points2);

        // X locked (no change), Y unlocked (grows)
        assert_eq!(plot.state.x_bounds, Some([0.0, 1.0]));
        assert_eq!(plot.state.y_bounds, Some([-1.0, 2.0]));

        // Lock Y, unlock X
        plot.state_mut().x_locked = false;
        plot.state_mut().y_locked = true;

        // New data exceeds bounds again
        let points3 = vec![[-2.0, -2.0], [3.0, 3.0]];
        plot.update_bounds(&points3);

        // X unlocked (grows), Y locked (no change)
        assert_eq!(plot.state.x_bounds, Some([-2.0, 3.0]));
        assert_eq!(plot.state.y_bounds, Some([-1.0, 2.0]));
    }

    #[test]
    fn test_reset_clears_bounds() {
        let mut plot = AutoScalePlot::new(AxisLockState::default());

        // Initialize bounds and lock axes
        let points = vec![[0.0, 0.0], [1.0, 1.0]];
        plot.update_bounds(&points);
        plot.state_mut().x_locked = true;
        plot.state_mut().y_locked = true;

        assert!(plot.state.x_bounds.is_some());
        assert!(plot.state.y_bounds.is_some());
        assert!(plot.state.x_locked);
        assert!(plot.state.y_locked);

        // Reset
        plot.reset_bounds();

        // Bounds cleared, axes unlocked
        assert_eq!(plot.state.x_bounds, None);
        assert_eq!(plot.state.y_bounds, None);
        assert!(!plot.state.x_locked);
        assert!(!plot.state.y_locked);

        // Next update re-initializes from data
        let new_points = vec![[5.0, 10.0], [6.0, 11.0]];
        plot.update_bounds(&new_points);
        assert_eq!(plot.state.x_bounds, Some([5.0, 6.0]));
        assert_eq!(plot.state.y_bounds, Some([10.0, 11.0]));
    }
}
