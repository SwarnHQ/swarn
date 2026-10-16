//! A stand-in for `claude -p` stream-json, for benchmarks: answers every prompt by streaming
//! words at a fixed rate, then ends the turn. FAKE_RATE is deltas per second (default 50),
//! FAKE_SECONDS the length of each answer (default 30).

use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use serde_json::json;

fn main() {
    let rate: f64 = std::env::var("FAKE_RATE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50.0);
    let seconds: f64 = std::env::var("FAKE_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.0);
    let out = std::io::stdout();
    let send = |msg: serde_json::Value| {
        let mut out = out.lock();
        let _ = writeln!(out, "{msg}");
        let _ = out.flush();
    };
    send(
        json!({ "type": "system", "subtype": "init", "session_id": "bench", "model": "fake",
        "claude_code_version": "0.0.0-bench", "permissionMode": "manual" }),
    );

    let words = [
        "The ",
        "agent ",
        "is ",
        "writing ",
        "`code` ",
        "and ",
        "**notes** ",
        "about ",
        "it.\n\n",
    ];
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if !line.contains("\"type\":\"user\"") {
            continue;
        }
        let start = Instant::now();
        let gap = Duration::from_secs_f64(1.0 / rate);
        let mut i = 0usize;
        while start.elapsed().as_secs_f64() < seconds {
            send(
                json!({ "type": "stream_event", "event": { "type": "content_block_delta", "index": 0,
                "delta": { "type": "text_delta", "text": words[i % words.len()] } } }),
            );
            i += 1;
            std::thread::sleep(gap);
        }
        send(
            json!({ "type": "result", "subtype": "success", "is_error": false, "stop_reason": "end_turn",
            "total_cost_usd": 0.0, "usage": { "input_tokens": 0, "output_tokens": i } }),
        );
    }
}
