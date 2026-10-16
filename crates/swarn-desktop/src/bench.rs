//! Bench mode, driven by `swarn-bench` (§9.3). With SWARN_BENCH_OUT set, the window records how
//! long it took to show a connected board, switches workspace every second and times each switch,
//! then after SWARN_BENCH_SECONDS writes frame statistics from GPUI's profiler and quits. Frame
//! histograms need the `bench` feature, which turns that profiler on.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui::{Context, Window};
use serde_json::json;

use crate::shell::Shell;

static LAUNCHED: OnceLock<Instant> = OnceLock::new();
static STATS: Mutex<Stats> = Mutex::new(Stats {
    cold_start_ms: None,
    switches_ms: Vec::new(),
});

struct Stats {
    cold_start_ms: Option<f64>,
    switches_ms: Vec<f64>,
}

/// With SWARN_TRACE_STARTUP set, prints how long after launch each startup step finished.
pub fn mark(label: &str) {
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("SWARN_TRACE_STARTUP").is_some()) {
        return;
    }
    if let Some(start) = LAUNCHED.get() {
        eprintln!(
            "TRACE {:>8.1} ms  {label}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}

pub fn enabled() -> bool {
    std::env::var_os("SWARN_BENCH_OUT").is_some()
}

/// Call first thing in `main`.
pub fn launched() {
    LAUNCHED.get_or_init(Instant::now);
}

/// The first render with the daemon's lists in hand.
pub fn connected() {
    let mut stats = STATS.lock().unwrap();
    if stats.cold_start_ms.is_none()
        && let Some(start) = LAUNCHED.get()
    {
        stats.cold_start_ms = Some(ms(start.elapsed()));
    }
}

pub fn start(window: &mut Window, cx: &mut Context<Shell>) {
    let seconds: f64 = std::env::var("SWARN_BENCH_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(35.0);
    cx.spawn_in(window, async move |this, cx| {
        let end = Instant::now() + Duration::from_secs_f64(seconds);
        while Instant::now() < end {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.step_workspace(1, cx);
                let switched = Instant::now();
                window.on_next_frame(move |_, _| {
                    STATS
                        .lock()
                        .unwrap()
                        .switches_ms
                        .push(ms(switched.elapsed()))
                });
            });
        }
        let _ = this.update_in(cx, |_, window, cx| {
            write_report(window);
            cx.quit();
        });
    })
    .detach();
}

fn write_report(window: &Window) {
    let stats = STATS.lock().unwrap();
    let report = json!({
        "cold_start_ms": stats.cold_start_ms,
        "workspace_switch_ms": percentiles(&stats.switches_ms),
    });
    #[cfg(feature = "bench")]
    let report = {
        let mut report = report;
        let frames = window.frame_duration_snapshot();
        let hist = |h: &hdrhistogram::Histogram<u64>| {
            json!({
                "frames": h.len(),
                "p50": h.value_at_quantile(0.5) as f64 / 1e6,
                "p99": h.value_at_quantile(0.99) as f64 / 1e6,
                "max": h.max() as f64 / 1e6,
            })
        };
        report["draw_ms"] = hist(&frames.draw_duration_histogram);
        report["dirty_to_present_ms"] = hist(&frames.dirty_to_present_histogram);
        report
    };
    #[cfg(not(feature = "bench"))]
    let _ = window;
    if let Some(path) = std::env::var_os("SWARN_BENCH_OUT") {
        let _ = std::fs::write(
            path,
            serde_json::to_string_pretty(&report).unwrap_or_default(),
        );
    }
}

fn percentiles(values: &[f64]) -> serde_json::Value {
    if values.is_empty() {
        return json!(null);
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    json!({ "count": sorted.len(), "p50": at(0.5), "p99": at(0.99), "max": at(1.0) })
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1e5).round() / 100.0
}
