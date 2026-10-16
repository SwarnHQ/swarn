# Performance numbers

First run of `swarn-bench` (PRD §9.3), 16 Oct 2026, release build, on the dev MacBook Pro (Apple
silicon, macOS 27.0) driving an external 34-inch display. Ten fake agents each streamed 50 text
deltas a second for 30 seconds while the window switched workspace every second.

| Measure | Result | Release budget (§8.1) |
| --- | --- | --- |
| RSS, desktop and daemon together (peak) | 114.8 MB (desktop 107.5, daemon 8.2) | under 250 MB, met |
| Draw time per frame, p99 | 5.4 ms (p50 3.4, max 6.2, 1,410 frames) | under 8.3 ms, met |
| Change to pixels on screen, p99 | 14.9 ms (p50 9.8) | none |
| Workspace switch to next frame, p99 | 10.0 ms (p50 5.0) | none |
| Cold start to a connected board | 608 ms | under 400 ms, **missed** |

"Draw time" is GPUI's `Window::draw` duration. "Change to pixels" also includes waiting for the
display's next refresh, so it sits near one refresh interval by construction. The PRD's 8.3 ms
budget is read here as draw time; if it was meant end to end, the budget needs a second look on a
120 Hz display.

## The cold start miss

608 ms is measured from the top of `main` to the first render that has the daemon's lists. Not yet
profiled. Likely costs, to check first: `gpui_component::init` (it sets up themes, locales and a
dozen widget subsystems), loading seven font files, and the four sequential socket calls of the
first poll. Fixing it is week 7 work.

## Running it

```bash
cargo build --release -p swarn-daemon -p swarn-bench -p swarn-desktop --features swarn-desktop/bench
```

```bash
target/release/swarn-bench --agents 10 --rate 50 --seconds 30
```

The window has to be on screen: a locked Mac draws no frames, and the frame figures come back
empty.
