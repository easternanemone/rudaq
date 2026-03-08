//! rust-daq Slint UI — unified native + WASM entry point.
//!
//! Native: launches DockManager with live camera feed and mode chooser.
//! WASM:   launches SplitterMode directly; gRPC-web health check updates log.

pub mod panels;

#[cfg(not(target_arch = "wasm32"))]
use panels::camera_panel::{render_frame, FpsTracker, FrameData};
#[cfg(not(target_arch = "wasm32"))]
use panels::plot_panel::{render_plot, PlotState};
#[cfg(not(target_arch = "wasm32"))]
use rand::Rng;
#[cfg(not(target_arch = "wasm32"))]
use slint::{Rgb8Pixel, SharedPixelBuffer, Timer, TimerMode};
#[cfg(not(target_arch = "wasm32"))]
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

slint::include_modules!();

#[cfg(not(target_arch = "wasm32"))]
const DEMO_CAM_W: u32 = 256;
#[cfg(not(target_arch = "wasm32"))]
const DEMO_CAM_H: u32 = 256;

/// Default daemon URL for gRPC-web connections.
#[cfg(target_arch = "wasm32")]
const DEFAULT_DAEMON_URL: &str = "http://localhost:50051";

pub fn make_log_model() -> Rc<slint::VecModel<slint::StandardListViewItem>> {
    let entries: slint::VecModel<slint::StandardListViewItem> = slint::VecModel::default();
    for msg in [
        "[INFO]  Daemon connected on :50051",
        "[INFO]  Camera initialized (mock)",
        "[INFO]  Stage homed: x=0 y=0 z=0",
        "[WARN]  Ring buffer at 80% capacity",
        "[INFO]  Experiment plan loaded: grid_scan_5x5",
        "[INFO]  Exposure set to 100 ms",
        "[DEBUG] Frame pool: 64 frames pre-allocated",
        "[INFO]  Acquisition started",
    ] {
        entries.push(slint::StandardListViewItem::from(
            slint::SharedString::from(msg),
        ));
    }
    Rc::new(entries)
}

#[cfg(not(target_arch = "wasm32"))]
fn generate_demo_frame(n: u64) -> FrameData {
    let mut rng = rand::thread_rng();
    let t = n as f64 * 0.03;
    let mut data = Vec::with_capacity((DEMO_CAM_W * DEMO_CAM_H) as usize);
    for y in 0..DEMO_CAM_H {
        for x in 0..DEMO_CAM_W {
            let fx = x as f64 / DEMO_CAM_W as f64;
            let fy = y as f64 / DEMO_CAM_H as f64;
            let v = ((fx * 4.0 * std::f64::consts::PI + t).sin()
                * (fy * 3.0 * std::f64::consts::PI + t * 0.7).cos()
                + 1.0)
                * 0.5;
            let noise = rng.gen::<f64>() * 0.03;
            data.push(((v + noise) * 65535.0).clamp(0.0, 65535.0) as u16);
        }
    }
    FrameData {
        width: DEMO_CAM_W,
        height: DEMO_CAM_H,
        data,
        frame_number: n,
    }
}

/// Start the live camera + plot rendering timer for a SplitterMode window.
///
/// Returns the [`slint::Timer`] — the caller must keep it alive for the
/// duration of the window event loop (`win.run()`).  Dropping it cancels
/// the timer immediately.
#[cfg(not(target_arch = "wasm32"))]
pub fn start_live_panels(win: &SplitterMode) -> slint::Timer {
    let camera_buf = Rc::new(RefCell::new(SharedPixelBuffer::<Rgb8Pixel>::new(1, 1)));
    let plot_buf = Rc::new(RefCell::new(SharedPixelBuffer::<Rgb8Pixel>::new(1, 1)));
    let plot_state = Rc::new(RefCell::new(PlotState::new(1000, 10.0)));
    let fps_tracker = Rc::new(RefCell::new(FpsTracker::new(2.0)));
    let frame_counter = Rc::new(RefCell::new(0u64));
    let win_weak = win.as_weak();

    let timer = Timer::default();
    timer.start(
        TimerMode::Repeated,
        std::time::Duration::from_millis(33),
        move || {
            let Some(w) = win_weak.upgrade() else { return };

            let mut n = frame_counter.borrow_mut();
            *n += 1;
            let frame_num = *n;

            let frame = generate_demo_frame(frame_num);
            let img = render_frame(&frame, &mut camera_buf.borrow_mut());
            w.set_camera_image(img);

            fps_tracker.borrow_mut().tick();
            w.set_camera_fps(format!("{:.1} fps", fps_tracker.borrow().fps()).into());

            let t = frame_num as f64 * 0.033;
            {
                let mut ps = plot_state.borrow_mut();
                let val = (t * 2.0).sin() * 50.0 + (t * 5.3).sin() * 20.0 + 100.0;
                ps.push(t, val);
            }
            // Plot rendered but not yet wired to a UI slot — computed to validate pipeline.
            let _plot_img = render_plot(&plot_state.borrow(), 400, 300, &mut plot_buf.borrow_mut());
        },
    );

    timer
}

#[cfg(not(target_arch = "wasm32"))]
pub fn launch_splitter(log: &Rc<slint::VecModel<slint::StandardListViewItem>>) {
    let win = SplitterMode::new().expect("create splitter window");
    win.set_log_model(slint::ModelRc::from(log.clone()));
    // Keep the timer alive on the stack until win.run() returns.
    let _timer = start_live_panels(&win);
    win.run().expect("splitter mode");
}

#[cfg(not(target_arch = "wasm32"))]
pub fn launch_mdi(log: &Rc<slint::VecModel<slint::StandardListViewItem>>) {
    let win = MdiMode::new().expect("create MDI window");
    win.set_log_model(slint::ModelRc::from(log.clone()));
    win.run().expect("mdi mode");
}

#[cfg(not(target_arch = "wasm32"))]
pub fn launch_floating(log: &Rc<slint::VecModel<slint::StandardListViewItem>>) {
    let win = FloatingMode::new().expect("create floating window");
    win.set_log_model(slint::ModelRc::from(log.clone()));

    let mw = win.as_weak();
    let fc: Rc<RefCell<Option<FloatingCamera>>> = Rc::new(RefCell::new(None));
    let fc2 = fc.clone();
    win.on_detach_camera(move || {
        let mut slot = fc2.borrow_mut();
        if slot.is_none() {
            let w = FloatingCamera::new().expect("floating camera");
            w.show().expect("show");
            *slot = Some(w);
            if let Some(m) = mw.upgrade() {
                m.set_camera_visible(false);
            }
        }
    });

    let mw = win.as_weak();
    let fp: Rc<RefCell<Option<FloatingParams>>> = Rc::new(RefCell::new(None));
    let fp2 = fp.clone();
    win.on_detach_params(move || {
        let mut slot = fp2.borrow_mut();
        if slot.is_none() {
            let w = FloatingParams::new().expect("floating params");
            w.show().expect("show");
            *slot = Some(w);
            if let Some(m) = mw.upgrade() {
                m.set_params_visible(false);
            }
        }
    });

    let mw = win.as_weak();
    let log2 = log.clone();
    let fl: Rc<RefCell<Option<FloatingLog>>> = Rc::new(RefCell::new(None));
    let fl2 = fl.clone();
    win.on_detach_log(move || {
        let mut slot = fl2.borrow_mut();
        if slot.is_none() {
            let w = FloatingLog::new().expect("floating log");
            w.set_log_model(slint::ModelRc::from(log2.clone()));
            w.show().expect("show");
            *slot = Some(w);
            if let Some(m) = mw.upgrade() {
                m.set_log_visible(false);
            }
        }
    });

    let mw = win.as_weak();
    let fs: Rc<RefCell<Option<FloatingStage>>> = Rc::new(RefCell::new(None));
    let fs2 = fs.clone();
    win.on_detach_stage(move || {
        let mut slot = fs2.borrow_mut();
        if slot.is_none() {
            let w = FloatingStage::new().expect("floating stage");
            w.show().expect("show");
            *slot = Some(w);
            if let Some(m) = mw.upgrade() {
                m.set_stage_visible(false);
            }
        }
    });

    win.run().expect("floating mode");
}

/// gRPC-web health check — WASM only. Appends status to the log model.
#[cfg(target_arch = "wasm32")]
async fn check_health(log: Rc<slint::VecModel<slint::StandardListViewItem>>) {
    use protocol::health::health_client::HealthClient;
    use protocol::health::HealthCheckRequest;
    use tonic_web_wasm_client::Client;

    let client = Client::new(DEFAULT_DAEMON_URL.to_string());
    let result = HealthClient::new(client)
        .check(HealthCheckRequest {
            service: String::new(),
        })
        .await;

    let status = match result {
        Ok(resp) => {
            if resp.into_inner().status == 1 {
                "[gRPC] Daemon: Connected (SERVING)".to_string()
            } else {
                "[gRPC] Daemon: Not serving".to_string()
            }
        }
        Err(e) => format!("[gRPC] Daemon: Disconnected — {e}"),
    };
    log.push(slint::StandardListViewItem::from(
        slint::SharedString::from(status),
    ));
}

/// Unified entry point for native binary and WASM (`wasm_bindgen(start)`).
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn main() {
    let log = make_log_model();

    #[cfg(target_arch = "wasm32")]
    {
        // WASM: launch splitter directly; gRPC health check runs async.
        let win = SplitterMode::new().expect("create splitter window");
        win.set_log_model(slint::ModelRc::from(log.clone()));
        wasm_bindgen_futures::spawn_local(check_health(log));
        win.run().expect("run");
        return;
    }

    // Native: mode selection via CLI arg or launcher window.
    #[cfg(not(target_arch = "wasm32"))]
    {
        match std::env::args().nth(1).as_deref() {
            Some("splitter" | "1") => return launch_splitter(&log),
            Some("mdi" | "2") => return launch_mdi(&log),
            Some("floating" | "3") => return launch_floating(&log),
            _ => {}
        }

        let launcher = Launcher::new().expect("create launcher");
        let (l1, l2, l3) = (log.clone(), log.clone(), log.clone());

        let lw = launcher.as_weak();
        launcher.on_launch_splitter(move || {
            if let Some(l) = lw.upgrade() {
                l.hide().expect("hide");
            }
            launch_splitter(&l1);
        });

        let lw = launcher.as_weak();
        launcher.on_launch_mdi(move || {
            if let Some(l) = lw.upgrade() {
                l.hide().expect("hide");
            }
            launch_mdi(&l2);
        });

        let lw = launcher.as_weak();
        launcher.on_launch_floating(move || {
            if let Some(l) = lw.upgrade() {
                l.hide().expect("hide");
            }
            launch_floating(&l3);
        });

        launcher.run().expect("launcher");
    }
}
