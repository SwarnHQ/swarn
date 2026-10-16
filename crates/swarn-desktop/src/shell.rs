//! The window's one view: Plexo's title bar, a body, and its footer strip. Everything shown comes
//! from the daemon over the local socket; the desktop holds no state of its own.

use std::time::Duration;

use gpui::{
    AnyElement, Context, FontWeight, MouseButton, SharedString, Window, div, prelude::*, px,
};
use serde_json::json;
use swarn_core::local_api::{self, DaemonInfo};
use swarn_core::schema::Repo;

use crate::theme::{HAIRLINE, MONO, SANS, TITLE_BAR_HEIGHT, Theme};

/// Plexo's 20px side gutter, used by every row and strip.
const GUTTER: f32 = 20.;

enum Board {
    Loading,
    Offline,
    Ready { info: DaemonInfo, repos: Vec<Repo> },
}

pub struct Shell {
    board: Board,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Theme::sync(window, cx);
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync(window, cx);
            cx.notify();
        })
        .detach();

        // ponytail: polls every 2 s until session.subscribe streams changes (week 2).
        cx.spawn(async move |this, cx| {
            loop {
                let board = cx.background_executor().spawn(async { fetch() }).await;
                if this
                    .update(cx, |this, cx| {
                        this.board = board;
                        cx.notify();
                    })
                    .is_err()
                {
                    break; // window closed
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        })
        .detach();

        Self {
            board: Board::Loading,
        }
    }
}

fn fetch() -> Board {
    let result = local_api::call::<DaemonInfo>("daemon.info", json!({})).and_then(|info| {
        local_api::call::<Vec<Repo>>("repo.list", json!({}))
            .map(|repos| Board::Ready { info, repos })
    });
    result.unwrap_or(Board::Offline)
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx);
        let body = match &self.board {
            Board::Loading => div().flex_1().into_any_element(),
            Board::Offline => empty_state(
                theme,
                "Daemon not running",
                "Start it, and this window connects on its own:",
                "cargo run -p swarn-daemon",
            ),
            Board::Ready { repos, .. } if repos.is_empty() => empty_state(
                theme,
                "No repositories yet",
                "Add one from a terminal:",
                r#"swarnd call repo.add '{"path":"/path/to/repo"}'"#,
            ),
            Board::Ready { repos, .. } => repo_list(theme, repos),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.text)
            .font_family(SANS)
            .child(title_bar(theme))
            .child(div().flex_1().min_h_0().flex().flex_col().child(body))
            .child(footer(theme, &self.board))
    }
}

/// Plexo's title bar: the app name centred on a strip drawn under the native traffic lights.
fn title_bar(theme: &Theme) -> impl IntoElement {
    div()
        .id("title-bar")
        .h(px(TITLE_BAR_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .px(px(94.))
        .bg(theme.bg_secondary)
        .border_b(px(HAIRLINE))
        .border_color(theme.border)
        // Plexo drags from the whole strip (`-webkit-app-region: drag`), and double-click zooms.
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
        .child(
            div()
                .text_size(px(13.))
                .line_height(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_secondary)
                .child("Swarn"),
        )
}

/// Plexo's small-caps section label: mono, uppercase, tertiary. GPUI has no letter-spacing, so the
/// tracking Plexo adds is missing here.
fn label(theme: &Theme, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .font_family(MONO)
        .text_size(px(10.5))
        .line_height(px(10.5))
        .text_color(theme.text_tertiary)
        .child(text.into().to_uppercase())
}

fn repo_list(theme: &Theme, repos: &[Repo]) -> AnyElement {
    let count = if repos.len() == 1 {
        "1 repository".to_owned()
    } else {
        format!("{} repositories", repos.len())
    };
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(GUTTER))
                .pt(px(18.))
                .pb(px(10.))
                .border_b(px(HAIRLINE))
                .border_color(theme.border)
                .child(label(theme, "Repositories"))
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(px(11.))
                        .text_color(theme.text_tertiary)
                        .child(count),
                ),
        )
        .child(
            div()
                .id("repos")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(repos.iter().map(|repo| repo_row(theme, repo))),
        )
        .into_any_element()
}

fn repo_row(theme: &Theme, repo: &Repo) -> impl IntoElement {
    let name = repo
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&repo.path)
        .to_owned();
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(GUTTER))
        .py(px(11.))
        .border_b(px(HAIRLINE))
        .border_color(theme.border_subtle)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(name),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(px(11.5))
                        .text_color(theme.text_secondary)
                        // The end of a path is the part that tells repos apart.
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis_start()
                        .child(repo.path.clone()),
                ),
        )
        .child(
            div()
                .flex_none()
                .px(px(6.))
                .py(px(3.))
                .rounded(px(4.))
                .border(px(HAIRLINE))
                .border_color(theme.border_strong)
                .bg(theme.bg_secondary)
                .font_family(MONO)
                .text_size(px(10.5))
                .line_height(px(10.5))
                .text_color(theme.text_secondary)
                .child(repo.default_branch.clone()),
        )
}

fn empty_state(
    theme: &Theme,
    title: &'static str,
    hint: &'static str,
    command: &'static str,
) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme.text_secondary)
                .child(hint),
        )
        .child(
            div()
                .mt(px(4.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(6.))
                .border(px(HAIRLINE))
                .border_color(theme.border)
                .bg(theme.input_bg)
                .font_family(MONO)
                .text_size(px(11.5))
                .text_color(theme.text_secondary)
                .child(command),
        )
        .into_any_element()
}

/// Plexo's ScreenFooter: one mono status line on the secondary surface.
fn footer(theme: &Theme, board: &Board) -> impl IntoElement {
    let (dot, text): (_, SharedString) = match board {
        Board::Loading => (theme.icon_muted, "Connecting…".into()),
        Board::Offline => (theme.danger, "Daemon offline · retrying every 2 s".into()),
        Board::Ready { info, .. } => (
            theme.success,
            format!(
                "{} · swarnd {} · schema {}",
                info.machine_name, info.version, info.schema_max
            )
            .into(),
        ),
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(GUTTER))
        .py(px(8.))
        .bg(theme.bg_secondary)
        .border_t(px(HAIRLINE))
        .border_color(theme.footer_border)
        .child(div().size(px(6.)).rounded_full().bg(dot))
        .child(
            div()
                .font_family(MONO)
                .text_size(px(11.))
                .text_color(theme.text_tertiary)
                .child(text),
        )
}
