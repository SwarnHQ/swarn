//! The first-run checklist (PRD §5.5): what the daemon found, one card per item, with a way to fix
//! each one. Also reachable later from the sidebar.

use std::time::Duration;

use gpui::{AnyElement, Context, FontWeight, SharedString, div, prelude::*, px};
use serde_json::{Value, json};
use swarn_core::local_api::{ApprovalMode, Policy};

use crate::service::{self, Service};
use crate::shell::{GUTTER, Shell};
use crate::theme::{ButtonKind, Hue, MONO, Theme, button, label};

pub struct Setup {
    pub check: Option<Value>,
    pub policy: Option<Policy>,
}

impl Shell {
    pub(crate) fn open_setup(&mut self, cx: &mut Context<Self>) {
        if self.setup.is_some() {
            return; // already open, with its recheck loop running
        }
        self.setup = Some(Setup {
            check: None,
            policy: None,
        });
        self.recheck(cx);
        // ponytail: rechecks every 5 s while the screen is open, so a sign-in finished in Terminal
        // shows up without a button press. Each check runs a handful of CLIs, about a second.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let open = this.update(cx, |this, cx| {
                    if this.setup.is_some() {
                        this.recheck(cx);
                    }
                    this.setup.is_some()
                });
                if !matches!(open, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn recheck(&mut self, cx: &mut Context<Self>) {
        self.load_devices(cx);
        self.call::<Value>(
            "setup.check",
            json!({}),
            |this, result, cx| {
                if let (Some(setup), Ok(check)) = (&mut this.setup, result) {
                    setup.check = Some(check);
                    cx.notify();
                }
            },
            cx,
        );
        self.call::<Policy>(
            "policy.get",
            json!({}),
            |this, result, _| {
                if let (Some(setup), Ok(policy)) = (&mut this.setup, result) {
                    setup.policy = Some(policy);
                }
            },
            cx,
        );
    }

    fn set_policy(&mut self, change: impl FnOnce(&mut Policy), cx: &mut Context<Self>) {
        let Some(mut policy) = self.setup.as_ref().and_then(|s| s.policy.clone()) else {
            return;
        };
        change(&mut policy);
        if let Some(setup) = &mut self.setup {
            setup.policy = Some(policy.clone());
        }
        self.call::<Policy>("policy.set", json!(policy), |_, _, _| {}, cx);
        cx.notify();
    }

    pub(crate) fn setup_view(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(setup) = &self.setup else {
            return div().into_any_element();
        };
        let mut page = div()
            .id("setup")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(12.))
            .px(px(GUTTER))
            .py(px(20.))
            .child(label(theme, "Set up Swarn"))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme.text_secondary)
                    .child("Swarn runs your own Claude Code and Codex, signed in as you. Here is what it found on this Mac."),
            );

        let Some(check) = &setup.check else {
            return page
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(px(11.5))
                        .text_color(theme.text_tertiary)
                        .child("Checking…"),
                )
                .into_any_element();
        };

        page = page.child(card(
            theme,
            check["git"]["ok"] == true,
            "Git",
            check["git"]["detail"].as_str().unwrap_or_default(),
            None,
        ));
        for agent in check["agents"].as_array().into_iter().flatten() {
            let name = agent["name"].as_str().unwrap_or("Agent");
            let installed = agent["installed"] == true;
            let ok = installed && agent["signed_in"] == true;
            let detail = match agent["version"].as_str() {
                Some(v) if installed => {
                    format!("{v} · {}", agent["detail"].as_str().unwrap_or_default())
                }
                _ => agent["detail"].as_str().unwrap_or_default().to_owned(),
            };
            let action = (installed && !ok).then(|| {
                sign_in(
                    theme,
                    name,
                    agent["sign_in"].as_str().unwrap_or_default(),
                    cx,
                )
            });
            page = page.child(card(theme, ok, name, &detail, action));
        }
        let gh = &check["gh"];
        let gh_action = (gh["installed"] == true && gh["signed_in"] != true)
            .then(|| sign_in(theme, "GitHub", "gh auth login", cx));
        page = page.child(card(
            theme,
            gh["signed_in"] == true,
            "GitHub CLI (for pull requests)",
            gh["detail"].as_str().unwrap_or_default(),
            gh_action,
        ));
        for warning in check["warnings"].as_array().into_iter().flatten() {
            page = page.child(card(
                theme,
                false,
                "Heads up",
                warning.as_str().unwrap_or_default(),
                None,
            ));
        }

        if let Some(policy) = &setup.policy {
            page = page.child(label(theme, "Approvals").mt(px(8.))).child(self.mode_toggle(theme, policy, cx)).child(
                div()
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .text_color(theme.text_tertiary)
                    .child(format!(
                        "Requests you don't answer expire after {} minutes and count as denied.",
                        policy.approvals.default_expiry_secs / 60
                    )),
            );
            let awake = policy.power.keep_awake;
            page = page.child(label(theme, "Power").mt(px(8.))).child(
                div()
                    .id("keep-awake")
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_policy(|p| p.power.keep_awake = !awake, cx)))
                    .child(switch(theme, awake))
                    .child(div().text_size(px(13.)).child("Keep the Mac awake while agents work")),
            ).child(
                div()
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .text_color(theme.text_tertiary)
                    .child("A closed lid on battery still sleeps, and agents pause until it wakes."),
            );
        }

        page.child(self.phone_section(theme, cx))
            .child(div().flex().justify_end().mt(px(8.)).child(
                button(theme, "setup-done", "Continue", ButtonKind::Primary).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.setup = None;
                        this.pairing = None;
                        this.setup_seen = true;
                        cx.notify();
                    },
                )),
            ))
            .into_any_element()
    }

    /// Plexo's segmented toggle, two approval modes.
    fn mode_toggle(
        &self,
        theme: &Theme,
        policy: &Policy,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let options = [
            (ApprovalMode::Ask, "Ask before writes and commands"),
            (ApprovalMode::AllowEdits, "Allow edits, ask before commands"),
        ];
        div()
            .flex()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(8.))
            .border(px(1.))
            .border_color(theme.border)
            .children(options.into_iter().map(|(mode, text)| {
                let on = policy.approvals.mode == mode;
                div()
                    .id(SharedString::from(format!("mode-{text}")))
                    .flex_1()
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(6.))
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { theme.text } else { theme.text_tertiary })
                    .when(on, |d| d.bg(theme.track_bg))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_policy(|p| p.approvals.mode = mode, cx)
                    }))
                    .child(text)
            }))
    }
}

/// Plexo's network card: tinted teal when the item is fine, amber when it needs the user.
fn card(
    theme: &Theme,
    ok: bool,
    title: &str,
    detail: &str,
    action: Option<AnyElement>,
) -> AnyElement {
    let hue: Hue = if ok { theme.teal } else { theme.amber };
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(14.))
        .py(px(12.))
        .rounded(px(10.))
        .border(px(1.))
        .border_color(hue.border)
        .bg(hue.bg)
        .child(div().size(px(8.)).flex_none().rounded_full().bg(hue.solid))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(px(11.5))
                        .text_color(hue.text)
                        .child(detail.to_owned()),
                ),
        )
        .children(action)
        .into_any_element()
}

/// Opens Terminal on the agent's own sign-in command. Swarn never shows a login of its own or
/// touches the token (Anthropic's policy); the recheck loop notices when it's done.
fn sign_in(theme: &Theme, name: &str, command: &str, cx: &mut Context<Shell>) -> AnyElement {
    let command = command.to_owned();
    button(
        theme,
        SharedString::from(format!("sign-in-{name}")),
        "Sign in",
        ButtonKind::Primary,
    )
    .on_click(cx.listener(move |this, _, _, cx| {
        if let Err(e) = open_terminal(&command) {
            this.last_error = Some(format!("Couldn't open Terminal: {e}"));
            cx.notify();
        }
    }))
    .into_any_element()
}

/// A `.command` file opens in Terminal without asking for automation permission. It runs in a
/// login shell so the user's PATH is there.
fn open_terminal(command: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let file = std::env::temp_dir().join(format!("swarn-sign-in-{}.command", std::process::id()));
    std::fs::write(&file, format!("#!/bin/zsh -l\n{command}\n"))?;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700))?;
    std::process::Command::new("/usr/bin/open")
        .arg(&file)
        .status()
        .map(|_| ())
}

fn switch(theme: &Theme, on: bool) -> impl IntoElement {
    div()
        .w(px(30.))
        .h(px(18.))
        .p(px(2.))
        .rounded_full()
        .flex()
        .when(on, |d| d.justify_end())
        .bg(if on { theme.accent } else { theme.track_bg })
        .child(div().size(px(14.)).rounded_full().bg(if on {
            theme.accent_on_solid
        } else {
            theme.text_tertiary
        }))
}

impl Shell {
    /// No daemon answering. From Swarn.app, offer to run it in the background (§5.5 step 1); from a
    /// development build, say how to start it by hand.
    pub(crate) fn offline_view(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (ok, title, detail, action): (bool, &str, &str, Option<AnyElement>) = match self.service
        {
            Service::NotBundled => (
                false,
                "Daemon not running",
                "This is a development build. Start the daemon with `cargo run -p swarn-daemon`, and this window connects on its own.",
                None,
            ),
            Service::NotRegistered => (
                false,
                "Run Swarn in the background",
                "A small helper keeps agents working when this window is closed. macOS lists it under Login Items, where you can turn it off.",
                Some(
                    button(theme, "service-on", "Turn on", ButtonKind::Primary)
                        .on_click(cx.listener(|this, _, _, cx| {
                            match service::register() {
                                Ok(status) => this.service = status,
                                Err(e) => {
                                    this.last_error =
                                        Some(format!("Couldn't register the helper: {e}"))
                                }
                            }
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
            ),
            Service::RequiresApproval => (
                false,
                "Allow Swarn in Login Items",
                "The background helper is turned off in System Settings. Turn Swarn on there, then come back.",
                Some(
                    button(
                        theme,
                        "service-settings",
                        "Open Login Items",
                        ButtonKind::Primary,
                    )
                    .on_click(cx.listener(|_, _, _, _| service::open_login_items()))
                    .into_any_element(),
                ),
            ),
            Service::Enabled => (
                true,
                "Starting the background helper…",
                "It's registered and launchd is starting it. This takes a second or two.",
                None,
            ),
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .justify_center()
            .px(px(GUTTER * 2.))
            .gap(px(12.))
            .child(label(theme, "Set up Swarn"))
            .child(card(theme, ok, title, detail, action))
            .into_any_element()
    }
}
