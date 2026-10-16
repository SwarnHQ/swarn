//! Pairing a phone and managing paired devices (§2.5, §5.1): a QR sheet with a countdown, and a
//! list with Revoke. Lives in the setup screen; the QR stays dark-on-white in both themes because
//! that is what cameras read best.

use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Context, FontWeight, SharedString, canvas, div, fill, point, prelude::*, px, rgb,
    size,
};
use serde_json::{Value, json};

use crate::shell::Shell;
use crate::theme::{ButtonKind, MONO, Theme, button, label};

pub struct Pairing {
    pub url: String,
    pub expires: Instant,
    /// Where the phone will look for this Mac, for the reachability line under the code.
    pub candidates: Vec<Value>,
    /// Devices known when the window opened; a new one means pairing worked.
    pub before: Vec<String>,
}

impl Shell {
    pub(crate) fn start_pairing(&mut self, cx: &mut Context<Self>) {
        let before: Vec<String> = self
            .devices
            .iter()
            .filter_map(|d| d["id"].as_str().map(str::to_owned))
            .collect();
        self.call::<Value>(
            "device.pair_window",
            json!({}),
            move |this, result, cx| {
                if let Ok(window) = result {
                    this.pairing = Some(Pairing {
                        url: window["url"].as_str().unwrap_or_default().to_owned(),
                        expires: Instant::now()
                            + Duration::from_secs(
                                window["expires_in_secs"].as_u64().unwrap_or(120),
                            ),
                        candidates: window["candidates"].as_array().cloned().unwrap_or_default(),
                        before,
                    });
                    this.tick_pairing(cx);
                }
            },
            cx,
        );
    }

    /// Redraws the countdown every second and refreshes the device list until the window closes
    /// or a new device shows up.
    fn tick_pairing(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let open = this.update(cx, |this, cx| {
                    this.load_devices(cx);
                    let Some(pairing) = &this.pairing else {
                        return false;
                    };
                    let joined = this.devices.iter().find(|d| {
                        d["id"]
                            .as_str()
                            .is_some_and(|id| !pairing.before.iter().any(|b| b == id))
                    });
                    if let Some(device) = joined {
                        this.notice = Some(format!(
                            "Paired {}",
                            device["name"].as_str().unwrap_or("a phone")
                        ));
                        this.pairing = None;
                    } else if pairing.expires <= Instant::now() {
                        this.pairing = None;
                    }
                    cx.notify();
                    this.pairing.is_some()
                });
                if !matches!(open, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn load_devices(&mut self, cx: &mut Context<Self>) {
        self.call::<Vec<Value>>(
            "device.list",
            json!({}),
            |this, result, cx| {
                if let Ok(devices) = result {
                    this.devices = devices;
                    cx.notify();
                }
            },
            cx,
        );
    }

    fn revoke(&mut self, device_id: String, cx: &mut Context<Self>) {
        self.call::<Value>(
            "device.revoke",
            json!({ "device_id": device_id }),
            |this, _, cx| this.load_devices(cx),
            cx,
        );
    }

    /// The "Phone" part of the setup screen.
    pub(crate) fn phone_section(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut section = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(label(theme, "Phone").mt(px(8.)));
        if let Some(pairing) = &self.pairing {
            return section
                .child(pairing_sheet(theme, pairing, cx))
                .into_any_element();
        }
        let active: Vec<&Value> = self
            .devices
            .iter()
            .filter(|d| d["revoked_at"].is_null())
            .collect();
        if active.is_empty() {
            section = section.child(
                div()
                    .text_size(px(13.))
                    .text_color(theme.text_secondary)
                    .child("Pair your iPhone to get approvals and replies on it, end-to-end encrypted."),
            );
        }
        for device in active {
            let id = device["id"].as_str().unwrap_or_default().to_owned();
            let connected = device["connected"] == true;
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(10.))
                    .border(px(1.))
                    .border_color(theme.border)
                    .bg(theme.bg_secondary)
                    .child(div().size(px(7.)).rounded_full().bg(if connected {
                        theme.success
                    } else {
                        theme.icon_muted
                    }))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(device["name"].as_str().unwrap_or("Phone").to_owned()),
                            )
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(11.))
                                    .text_color(theme.text_tertiary)
                                    .child(if connected {
                                        "connected".to_owned()
                                    } else {
                                        format!(
                                            "paired {}",
                                            short_date(device["paired_at"].as_str())
                                        )
                                    }),
                            ),
                    )
                    .child(
                        button(
                            theme,
                            SharedString::from(format!("revoke-{id}")),
                            "Revoke",
                            ButtonKind::Destructive,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.revoke(id.clone(), cx))),
                    ),
            );
        }
        section
            .child(
                div().flex().child(
                    button(theme, "pair-phone", "Pair a phone", ButtonKind::Secondary)
                        .on_click(cx.listener(|this, _, _, cx| this.start_pairing(cx))),
                ),
            )
            .into_any_element()
    }
}

fn pairing_sheet(theme: &Theme, pairing: &Pairing, cx: &mut Context<Shell>) -> AnyElement {
    let left = pairing
        .expires
        .saturating_duration_since(Instant::now())
        .as_secs();
    let tailscale = pairing
        .candidates
        .iter()
        .find(|c| c["kind"] == "tailscale")
        .and_then(|c| c["host"].as_str());
    let reach = match tailscale {
        Some(host) => format!("Reachable anywhere over Tailscale as {host}."),
        None => "No Tailscale found: the phone has to be on the same Wi-Fi as this Mac.".to_owned(),
    };
    div()
        .flex()
        .gap(px(18.))
        .p(px(16.))
        .rounded(px(10.))
        .border(px(1.))
        .border_color(theme.amber.border)
        .bg(theme.amber.bg)
        .child(qr(&pairing.url))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Scan with Swarn on your iPhone"))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme.text_secondary)
                        .child("The code is good for one phone and two minutes. Your phone will ask for camera, local network and notification access."),
                )
                .child(div().font_family(MONO).text_size(px(11.5)).text_color(theme.amber.text).child(reach))
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(px(11.5))
                        .text_color(theme.text_tertiary)
                        .child(format!("Expires in {}:{:02}", left / 60, left % 60)),
                )
                .child(div().flex_1())
                .child(
                    div().flex().child(
                        button(theme, "pair-cancel", "Cancel", ButtonKind::Secondary).on_click(cx.listener(|this, _, _, cx| {
                            this.pairing = None;
                            cx.notify();
                        })),
                    ),
                ),
        )
        .into_any_element()
}

/// The pairing URL as a QR code: dark modules painted on white with the four-module quiet zone
/// scanners expect.
fn qr(url: &str) -> AnyElement {
    let Ok(code) = qrcode::QrCode::new(url.as_bytes()) else {
        return div().child("QR code too large").into_any_element();
    };
    let width = code.width();
    let dark: Vec<bool> = code
        .to_colors()
        .into_iter()
        .map(|c| c == qrcode::Color::Dark)
        .collect();
    let side = px(188.);
    div()
        .flex_none()
        .size(side)
        .rounded(px(6.))
        .bg(rgb(0xffffff))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let cell = bounds.size.width / (width + 8) as f32;
                    for (i, on) in dark.iter().enumerate() {
                        if *on {
                            let (x, y) = ((i % width + 4) as f32, (i / width + 4) as f32);
                            let origin =
                                point(bounds.origin.x + cell * x, bounds.origin.y + cell * y);
                            // A hair over one cell wide, so neighbouring modules leave no seams.
                            window.paint_quad(fill(
                                gpui::Bounds::new(origin, size(cell * 1.02, cell * 1.02)),
                                rgb(0x111111),
                            ));
                        }
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

fn short_date(ts: Option<&str>) -> String {
    ts.and_then(|t| t.get(..10)).unwrap_or("").to_owned()
}
