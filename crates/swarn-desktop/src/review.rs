//! The workspace's diff and its pull request: the Transcript | Diff switch, the diff view, and
//! Create PR (§5.3).

use gpui::{AnyElement, Context, FontWeight, SharedString, div, prelude::*, px, uniform_list};
use serde_json::{Value, json};

use crate::compose::AGENTS;
use crate::diff::{self, Row};
use crate::shell::{GUTTER, Shell};
use crate::theme::{ButtonKind, HAIRLINE, MONO, Theme, button};

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Transcript,
    Diff,
}

pub struct DiffState {
    pub workspace_id: String,
    pub rows: Vec<Row>,
}

const ROW_HEIGHT: f32 = 20.;

impl Shell {
    pub(crate) fn fetch_diff(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.selected.clone() else {
            return;
        };
        let id = workspace_id.clone();
        self.call::<Value>(
            "diff.get",
            json!({ "workspace_id": workspace_id }),
            move |this, result, cx| {
                if let Ok(value) = result {
                    let rows = diff::parse(value["diff"].as_str().unwrap_or_default());
                    this.diff = Some(DiffState {
                        workspace_id: id,
                        rows,
                    });
                    cx.notify();
                }
            },
            cx,
        );
    }

    pub(crate) fn create_pr(&mut self, cx: &mut Context<Self>) {
        let Some(ws) = self
            .selected
            .clone()
            .and_then(|id| self.board.workspaces.iter().find(|w| w.id == id).cloned())
        else {
            return;
        };
        // The first prompt says what the work was for; fall back to the workspace name.
        let first_prompt = self
            .board
            .latest_session(&ws.id)
            .and_then(|s| self.board.transcripts.get(&s.id))
            .and_then(|t| {
                t.blocks.iter().find_map(|b| match b {
                    crate::model::Block::User(text) => {
                        Some(text.lines().next().unwrap_or_default().to_owned())
                    }
                    _ => None,
                })
            });
        let mut title = first_prompt.unwrap_or_else(|| ws.name.replace('-', " "));
        if title.chars().count() > 72 {
            title = title.chars().take(71).collect::<String>() + "…";
        }
        let agent = self
            .board
            .latest_session(&ws.id)
            .and_then(|s| AGENTS.iter().find(|(id, _)| *id == s.agent))
            .map_or("an agent", |(_, name)| *name);
        let body = format!("Work from the `{}` workspace, done with {agent}.", ws.name);
        self.pr_busy = true;
        self.call::<Value>(
            "pr.create",
            json!({ "workspace_id": ws.id, "title": title, "body": body }),
            |this, result, cx| {
                this.pr_busy = false;
                if let Ok(value) = result
                    && let Some(url) = value["url"].as_str()
                {
                    this.notice = Some(format!("Opened {url}"));
                    cx.open_url(url);
                }
            },
            cx,
        );
    }

    /// Right side of the workspace header: the view switch and Create PR.
    pub(crate) fn header_actions(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let tab = |id: &'static str, text: &'static str, which: Tab| {
            let on = self.tab == which;
            div()
                .id(id)
                .px(px(8.))
                .h_full()
                .flex()
                .items_center()
                .rounded(px(4.))
                .font_family(MONO)
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if on { theme.text } else { theme.text_tertiary })
                .when(on, |d| d.bg(theme.track_bg))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = which;
                    if which == Tab::Diff {
                        this.fetch_diff(cx);
                    }
                    cx.notify();
                }))
                .child(text)
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .h(px(26.))
                    .p(px(2.))
                    .gap(px(2.))
                    .rounded(px(6.))
                    .border(px(HAIRLINE))
                    .border_color(theme.border)
                    .child(tab("tab-transcript", "Transcript", Tab::Transcript))
                    .child(tab("tab-diff", "Diff", Tab::Diff)),
            )
            .child(
                button(
                    theme,
                    "create-pr",
                    if self.pr_busy {
                        "Opening…"
                    } else {
                        "Create PR"
                    },
                    ButtonKind::Secondary,
                )
                .h(px(26.))
                .text_size(px(12.5))
                .when(!self.pr_busy, |b| {
                    b.on_click(cx.listener(|this, _, _, cx| this.create_pr(cx)))
                }),
            )
            .into_any_element()
    }

    /// The diff as plain monospace rows; clicking a file header marks it viewed and folds it.
    pub(crate) fn diff_view(
        &self,
        theme: &Theme,
        workspace_id: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = match &self.diff {
            Some(d) if d.workspace_id == workspace_id => &d.rows,
            _ => return div().flex_1().into_any_element(),
        };
        if rows.is_empty() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .font_family(MONO)
                .text_size(px(12.))
                .text_color(theme.text_tertiary)
                .child("No changes yet")
                .into_any_element();
        }
        let count = diff::visible(rows, &|path| {
            self.viewed.contains(&format!("{workspace_id}:{path}"))
        })
        .len();
        let ws = workspace_id.to_owned();
        uniform_list(
            "diff",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let theme = *Theme::get(cx);
                let Some(state) = &this.diff else {
                    return Vec::new();
                };
                let viewed = |path: &str| this.viewed.contains(&format!("{ws}:{path}"));
                let shown = diff::visible(&state.rows, &viewed);
                shown[range.start.min(shown.len())..range.end.min(shown.len())]
                    .iter()
                    .map(|row| diff_row(&theme, row, &ws, viewed, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .py(px(8.))
        .into_any_element()
    }
}

fn diff_row(
    theme: &Theme,
    row: &Row,
    ws: &str,
    viewed: impl Fn(&str) -> bool,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let base = div()
        .h(px(ROW_HEIGHT))
        .px(px(GUTTER))
        .flex()
        .items_center()
        .font_family(MONO)
        .text_size(px(12.));
    let line = |sign: &'static str, text: &str| {
        div()
            .flex()
            .gap(px(10.))
            .whitespace_nowrap()
            .overflow_hidden()
            .child(div().w(px(8.)).child(sign))
            .child(text.to_owned())
    };
    match row {
        Row::File {
            path,
            additions,
            deletions,
        } => {
            let key = format!("{ws}:{path}");
            let done = viewed(path);
            base.id(SharedString::from(format!("file-{path}")))
                .mt(px(0.))
                .gap(px(10.))
                .bg(theme.bg_secondary)
                .border_t(px(HAIRLINE))
                .border_color(theme.border)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.viewed.remove(&key) {
                        this.viewed.insert(key.clone());
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .text_color(if done {
                            theme.success
                        } else {
                            theme.icon_muted
                        })
                        .child(if done { "✓" } else { "○" }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(path.clone()),
                )
                .child(
                    div()
                        .text_color(theme.success)
                        .child(format!("+{additions}")),
                )
                .child(
                    div()
                        .text_color(theme.danger)
                        .child(format!("−{deletions}")),
                )
                .into_any_element()
        }
        Row::Hunk(text) => base
            .text_color(theme.text_tertiary)
            .child(text.clone())
            .into_any_element(),
        Row::Added(text) => base
            .bg(theme.teal.bg)
            .text_color(theme.teal.text)
            .child(line("+", text))
            .into_any_element(),
        Row::Removed(text) => base
            .bg(theme.danger_bg)
            .text_color(theme.danger)
            .child(line("−", text))
            .into_any_element(),
        Row::Context(text) => base
            .text_color(theme.text_secondary)
            .child(line(" ", text))
            .into_any_element(),
    }
}
