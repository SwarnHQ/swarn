//! One transcript block per list row: the user's prompts, the agent's markdown, tool calls,
//! approval cards and notices.

use gpui::{
    AnyElement, Context, FontWeight, HighlightStyle, Hsla, StyledText, UnderlineStyle, Window, div,
    prelude::*, px,
};
use swarn_core::schema::{Decision, Status};

use crate::compose::AGENTS;
use crate::markdown::{Inline, Md, parse};
use crate::model::{Approval, Block, ToolState};
use crate::shell::{GUTTER, Shell, chip};
use crate::theme::{ButtonKind, HAIRLINE, MONO, Theme, button, label};

impl Shell {
    pub(crate) fn render_block(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *Theme::get(cx);
        let Some(shown) = self.shown.clone() else {
            return div().into_any_element();
        };
        let Some(block) = self
            .board
            .transcripts
            .get(&shown)
            .and_then(|t| t.blocks.get(ix))
            .cloned()
        else {
            return div().into_any_element();
        };
        let session = self.board.sessions.iter().find(|s| s.id == shown);
        let live = session.is_some_and(|s| !matches!(s.status, Status::Exited | Status::Failed));
        let agent = session
            .and_then(|s| AGENTS.iter().find(|(id, _)| *id == s.agent))
            .map_or("Agent", |(_, name)| *name);

        let body = match block {
            Block::User(text) => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(8.))
                .border(px(HAIRLINE))
                .border_color(theme.border)
                .bg(theme.bg_secondary)
                .child(label(&theme, "You"))
                .child(div().text_size(px(13.5)).line_height(px(21.)).child(text))
                .into_any_element(),
            Block::Agent { text, .. } => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(label(&theme, agent))
                .children(parse(&text).into_iter().map(|md| markdown(&theme, md)))
                .into_any_element(),
            Block::Tool {
                name,
                target,
                state,
                ..
            } => {
                let (dot, word) = match state {
                    ToolState::Pending => (theme.icon_muted, "pending"),
                    ToolState::Completed => (theme.success, "done"),
                    ToolState::Failed => (theme.danger, "failed"),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .child(div().size(px(6.)).flex_none().rounded_full().bg(dot))
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.text_secondary)
                            .child(name),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.text_tertiary)
                            .child(target),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.text_tertiary)
                            .child(word),
                    )
                    .into_any_element()
            }
            Block::Approval(approval) => self.approval_card(&theme, ix, approval, live, cx),
            Block::Note { text, error } => div()
                .font_family(MONO)
                .text_size(px(11.5))
                .text_color(if error {
                    theme.danger
                } else {
                    theme.text_tertiary
                })
                .child(text)
                .into_any_element(),
        };
        div()
            .px(px(GUTTER))
            .pt(px(12.))
            .child(body)
            .into_any_element()
    }

    fn approval_card(
        &self,
        theme: &Theme,
        ix: usize,
        a: Approval,
        live: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = a.decision.is_none();
        let (bg, border, accent) = if open {
            (theme.amber.bg, theme.amber.border, theme.amber.text)
        } else {
            (theme.bg_secondary, theme.border, theme.text_tertiary)
        };

        let footer = if open && live {
            let action = |decision: Decision| {
                let request_id = a.request_id.clone();
                cx.listener(move |this, _, _, cx| this.resolve(request_id.clone(), decision, cx))
            };
            div()
                .flex()
                .gap(px(8.))
                .justify_end()
                .child(
                    button(theme, ("deny", ix), "Deny", ButtonKind::Destructive)
                        .on_click(action(Decision::Deny)),
                )
                .child(
                    button(
                        theme,
                        ("allow-session", ix),
                        "Allow for session",
                        ButtonKind::Secondary,
                    )
                    .on_click(action(Decision::AllowSession)),
                )
                .child(
                    button(theme, ("allow-once", ix), "Allow once", ButtonKind::Primary)
                        .on_click(action(Decision::AllowOnce)),
                )
                .into_any_element()
        } else {
            let mut outcome = match (a.decision.as_deref(), a.reason.as_deref()) {
                (None, _) => "The session ended before an answer".to_owned(),
                (Some("allow_once"), _) => "Allowed once".to_owned(),
                (Some("allow_session"), _) => "Allowed for this session".to_owned(),
                (Some(_), Some(reason)) => format!("Denied ({reason})"),
                (Some(_), None) => "Denied".to_owned(),
            };
            // Answers from a paired phone say which one.
            if let Some(by) = a.by.as_deref().filter(|by| *by != "local") {
                let name = self
                    .devices
                    .iter()
                    .find(|d| d["id"] == by)
                    .and_then(|d| d["name"].as_str())
                    .unwrap_or("a phone");
                outcome.push_str(&format!(" from {name}"));
            }
            div()
                .font_family(MONO)
                .text_size(px(11.5))
                .text_color(theme.text_secondary)
                .child(outcome)
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(14.))
            .rounded(px(8.))
            .border(px(1.))
            .border_color(border)
            .bg(bg)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        label(theme, if open { "Needs approval" } else { "Approval" })
                            .text_color(accent),
                    )
                    .child(chip(theme, a.risk.to_uppercase())),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(a.summary),
            )
            .child(
                div()
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .bg(theme.input_bg)
                    .border(px(HAIRLINE))
                    .border_color(theme.border)
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .text_color(theme.text_secondary)
                    .children(a.detail.lines().map(|l| {
                        div()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(l.to_owned())
                    })),
            )
            .child(footer)
            .into_any_element()
    }
}

fn markdown(theme: &Theme, md: Md) -> AnyElement {
    match md {
        Md::Para(inline) => prose(theme, inline).into_any_element(),
        Md::Heading(inline) => prose(theme, inline)
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(14.5))
            .into_any_element(),
        Md::Item(marker, inline) => div()
            .flex()
            .gap(px(8.))
            .child(
                div()
                    .flex_none()
                    .w(px(18.))
                    .font_family(MONO)
                    .text_size(px(12.))
                    .line_height(px(21.))
                    .text_color(theme.text_tertiary)
                    .child(marker.map_or("•".to_owned(), |n| format!("{n}."))),
            )
            .child(prose(theme, inline).flex_1())
            .into_any_element(),
        Md::Code(code) => div()
            .px(px(10.))
            .py(px(8.))
            .rounded(px(6.))
            .bg(theme.input_bg)
            .border(px(HAIRLINE))
            .border_color(theme.border)
            .font_family(MONO)
            .text_size(px(12.))
            .line_height(px(18.))
            .children(code.lines().map(|l| {
                div()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(l.to_owned())
            }))
            .into_any_element(),
    }
}

/// A paragraph with inline code, bold and links. ponytail: GPUI highlights cannot change the font
/// family, so inline code is marked by its background instead of Roboto Mono.
fn prose(theme: &Theme, inline: Inline) -> gpui::Div {
    let mut highlights: Vec<_> = inline
        .code
        .iter()
        .map(|r| {
            (
                r.clone(),
                HighlightStyle {
                    background_color: Some(hsla(theme.track_bg)),
                    ..Default::default()
                },
            )
        })
        .chain(inline.bold.iter().map(|r| {
            (
                r.clone(),
                HighlightStyle {
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                },
            )
        }))
        .chain(inline.links.iter().map(|r| {
            let style = HighlightStyle {
                color: Some(hsla(theme.accent)),
                underline: Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(hsla(theme.accent)),
                    wavy: false,
                }),
                ..Default::default()
            };
            (r.clone(), style)
        }))
        .collect();
    highlights.sort_by_key(|(r, _)| r.start);
    div()
        .text_size(px(13.5))
        .line_height(px(21.))
        .child(StyledText::new(inline.text).with_highlights(highlights))
}

fn hsla(color: gpui::Rgba) -> Hsla {
    color.into()
}
