//! Everything that sends something to the daemon from the window: the prompt composer, the
//! new-workspace form and adding a repository. Every call runs off the UI thread.

use gpui::{
    AnyElement, Context, Entity, FontWeight, PathPromptOptions, SharedString, Window, div,
    prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use swarn_core::local_api::{self, ApprovalMode};
use swarn_core::schema::{Status, Workspace};

use crate::icons::{self, icon};
use crate::shell::{GUTTER, Shell};
use crate::theme::{ButtonKind, HAIRLINE, MONO, Theme, button, label};

pub const AGENTS: [(&str, &str); 2] = [("claude-code", "Claude"), ("codex", "Codex")];

pub struct NewWorkspace {
    pub repo_id: Option<String>,
    pub name: Entity<InputState>,
    pub error: Option<String>,
}

pub fn composer_state(window: &mut Window, cx: &mut Context<Shell>) -> Entity<TextareaState> {
    let state = cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(1, 12)
            .placeholder("Do anything")
            .submit_on_enter(true)
    });
    cx.subscribe_in(&state, window, |this, _, event: &InputEvent, window, cx| {
        match event {
            InputEvent::PressEnter { shift: false, .. } => this.submit(window, cx),
            // The send button dims while the input is empty.
            InputEvent::Change => cx.notify(),
            _ => {}
        }
    })
    .detach();
    state
}

impl Shell {
    /// Sends the composer's text: a follow-up when the workspace has a live session, otherwise a
    /// new session with the chosen agent.
    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_owned();
        let Some(workspace_id) = self.selected.clone() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        let (method, params) = match self.live_session(&workspace_id) {
            Some(session_id) => {
                let agent = self.composer_agent(&workspace_id);
                (
                    "session.prompt",
                    json!({ "session_id": session_id, "text": text, "effort": self.effort.get(agent) }),
                )
            }
            None => {
                let mut params =
                    json!({ "workspace_id": workspace_id, "prompt": text, "agent": self.agent });
                if let Some(model) = self.chosen_model(self.agent) {
                    params["model"] = json!(model);
                }
                if let Some(effort) = self.effort.get(self.agent) {
                    params["effort"] = json!(effort);
                }
                ("session.start", params)
            }
        };
        self.call::<Value>(method, params, |_, _, _| {}, cx);
    }

    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(session_id) = self.selected.clone().and_then(|w| self.live_session(&w)) {
            self.call::<Value>(
                "session.cancel",
                json!({ "session_id": session_id }),
                |_, _, _| {},
                cx,
            );
        }
    }

    /// The selected workspace's latest session, if its agent process is still running.
    pub(crate) fn live_session(&self, workspace_id: &str) -> Option<String> {
        self.board
            .latest_session(workspace_id)
            .filter(|s| !matches!(s.status, Status::Exited | Status::Failed))
            .map(|s| s.id.clone())
    }

    pub(crate) fn add_repository(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add repository".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.call::<Value>(
                    "repo.add",
                    json!({ "path": path }),
                    |this, _, cx| this.refresh(cx),
                    cx,
                );
            });
        })
        .detach();
    }

    pub(crate) fn open_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("fix-login-bug"));
        cx.subscribe_in(&name, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.create_workspace(cx);
            }
        })
        .detach();
        name.update(cx, |state, cx| state.focus(window, cx));
        let repo_id = self.board.repos.first().map(|r| r.id.clone());
        self.form = Some(NewWorkspace {
            repo_id,
            name,
            error: None,
        });
        cx.notify();
    }

    fn create_workspace(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.form else { return };
        let name = form.name.read(cx).value().trim().to_owned();
        let Some(repo_id) = form.repo_id.clone() else {
            return;
        };
        let params = json!({ "repo_id": repo_id, "name": name });
        self.call::<Workspace>(
            "workspace.create",
            params,
            |this, result, cx| match result {
                Ok(ws) => {
                    this.form = None;
                    this.selected = Some(ws.id);
                    this.refresh(cx);
                }
                Err(e) => {
                    if let Some(form) = &mut this.form {
                        form.error = Some(e);
                    }
                }
            },
            cx,
        );
    }

    /// One daemon call in the background. `done` runs back on the UI thread with the result;
    /// failures also land in the footer.
    pub(crate) fn call<T: DeserializeOwned + Send + 'static>(
        &mut self,
        method: &'static str,
        params: Value,
        done: impl FnOnce(&mut Self, Result<T, String>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    // The daemon's message is written for people; its JSON-RPC code is not.
                    local_api::call::<T>(method, params).map_err(|e| match e {
                        local_api::CallError::Rpc(rpc) => rpc.message,
                        other => other.to_string(),
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.last_error = result.as_ref().err().cloned();
                done(this, result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The composer, after the Codex desktop app's (PRD §5.8): a context strip joined to the top
    /// of the input, then the input, then a toolbar with Approve for me, the model and effort
    /// control, and one round button that sends or stops.
    pub(crate) fn composer(
        &self,
        theme: &Theme,
        workspace_id: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let working = self.board.latest_session(workspace_id).is_some_and(|s| {
            matches!(
                s.status,
                Status::Working | Status::NeedsInput | Status::Starting
            )
        });
        let empty = self.composer.read(cx).value().trim().is_empty();
        let repo = self
            .board
            .workspaces
            .iter()
            .find(|w| w.id == workspace_id)
            .and_then(|w| self.board.repos.iter().find(|r| r.id == w.repo_id))
            .map(|r| r.path.rsplit('/').next().unwrap_or(&r.path).to_owned())
            .unwrap_or_default();

        let chip = |svg: &'static [u8], text: String| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.5))
                .text_color(theme.text_secondary)
                .child(icon(svg, 14., theme.text_secondary))
                .child(text)
        };
        let strip = div()
            .mx(px(14.))
            .flex()
            .items_center()
            .gap(px(18.))
            .px(px(12.))
            .h(px(32.))
            .rounded_t(px(10.))
            .border(px(HAIRLINE))
            .border_b(px(0.))
            .border_color(theme.border)
            .bg(theme.input_bg)
            .child(chip(icons::FOLDER, repo))
            .child(chip(icons::LAPTOP, "This Mac".into()))
            .child(div().flex_1())
            .child(
                div()
                    .id("composer-settings")
                    .cursor_pointer()
                    .child(icon(icons::SETTINGS, 14., theme.text_tertiary))
                    .on_click(cx.listener(|this, _, _, cx| this.open_setup(cx))),
            );

        let send = div()
            .id("composer-send")
            .size(px(30.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme.text)
            .when(empty && !working, |d| d.opacity(0.35))
            .cursor_pointer()
            .child(if working && empty {
                icon(icons::SQUARE, 12., theme.bg)
            } else {
                icon(icons::ARROW_UP, 16., theme.bg)
            })
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    // The second click of a double click must not stop the turn the first one started.
                    if event.click_count() > 1 {
                        return;
                    }
                    if working && this.composer.read(cx).value().trim().is_empty() {
                        this.stop(cx);
                    } else {
                        this.submit(window, cx);
                    }
                }),
            );

        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(self.approve_for_me(theme, cx))
            .child(div().flex_1())
            .child(self.model_button(theme, workspace_id, cx))
            .child(send);

        div()
            .flex_none()
            .relative()
            .px(px(GUTTER))
            .pt(px(10.))
            .pb(px(14.))
            .child(strip)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .px(px(14.))
                    .pt(px(12.))
                    .pb(px(10.))
                    .rounded(px(14.))
                    .border(px(HAIRLINE))
                    .border_color(theme.border_strong)
                    .bg(theme.track_bg)
                    .child(
                        Textarea::new(&self.composer)
                            .appearance(false)
                            .text_size(px(14.)),
                    )
                    .child(toolbar),
            )
            .when(self.effort_open, |d| {
                d.child(gpui::deferred(self.effort_popover(theme, workspace_id, cx)))
            })
            .into_any_element()
    }

    /// The approval policy as one toggle: off asks before writes and commands, on lets edits
    /// through and still asks before commands. Takes effect from the next session.
    fn approve_for_me(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let on = self
            .policy
            .as_ref()
            .is_some_and(|p| p.approvals.mode == ApprovalMode::AllowEdits);
        let color = if on {
            theme.amber.text
        } else {
            theme.text_tertiary
        };
        div()
            .id("approve-for-me")
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(color)
            .hover(|s| s.text_color(theme.text_secondary))
            .child(icon(icons::SHIELD, 15., color))
            .child("Approve for me")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_policy(
                    |p| {
                        p.approvals.mode = if on {
                            ApprovalMode::Ask
                        } else {
                            ApprovalMode::AllowEdits
                        }
                    },
                    cx,
                );
                this.notice = Some(if on {
                    "New sessions ask before writes and commands".into()
                } else {
                    "New sessions make edits without asking, and still ask before commands".into()
                });
            }))
    }

    /// The agent driving this composer: the live session's, else the one picked for a new session.
    fn composer_agent(&self, workspace_id: &str) -> &'static str {
        match self.live_session(workspace_id) {
            Some(_) => {
                let agent = self
                    .board
                    .latest_session(workspace_id)
                    .map(|s| s.agent.as_str());
                AGENTS
                    .iter()
                    .map(|(id, _)| *id)
                    .find(|id| Some(*id) == agent)
                    .unwrap_or(self.agent)
            }
            None => self.agent,
        }
    }

    fn picked_model(&self, agent: &str) -> Option<&Value> {
        let models = self.models.get(agent)?;
        models.get(self.model_pick.get(agent).copied().unwrap_or(0))
    }

    /// "Opus High ⌄": the model in full text, the effort quieter, one control for both.
    fn model_button(
        &self,
        theme: &Theme,
        workspace_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let agent = self.composer_agent(workspace_id);
        let agent_name = AGENTS
            .iter()
            .find(|(id, _)| *id == agent)
            .map_or("", |(_, n)| n);
        // The agent's own default reads as the agent's name; a picked model names itself.
        let model = self
            .picked_model(agent)
            .filter(|m| !m["id"].is_null())
            .and_then(|m| m["label"].as_str())
            .unwrap_or(agent_name)
            .to_owned();
        let effort = self.effort.get(agent).map(|e| effort_label(e));
        div()
            .id("model-button")
            .flex()
            .items_center()
            .gap(px(5.))
            .h(px(28.))
            .px(px(10.))
            .rounded_full()
            .cursor_pointer()
            .text_size(px(13.))
            .hover(|s| s.bg(theme.border_subtle))
            .when(self.effort_open, |d| d.bg(theme.border_subtle))
            .when(self.effort_open, |d| {
                d.child(
                    div()
                        .text_color(theme.text_secondary)
                        .child("Select effort"),
                )
            })
            .when(!self.effort_open, |d| {
                d.child(div().text_color(theme.text).child(model))
                    .when_some(effort, |d, e| {
                        d.child(div().text_color(theme.text_secondary).child(e))
                    })
            })
            .child(icon(icons::CHEVRON_DOWN, 14., theme.text_tertiary))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.effort_open = !this.effort_open;
                    this.model_list_open = false;
                    cx.notify();
                }),
            )
    }

    /// Above the model button: the effort as a title, the model with a way into the model list,
    /// a slider with one stop per effort, and reset to the agent's default.
    fn effort_popover(
        &self,
        theme: &Theme,
        workspace_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let agent = self.composer_agent(workspace_id);
        let live = self.live_session(workspace_id).is_some();
        let model = self.picked_model(agent);
        let model_name = model.and_then(|m| m["label"].as_str()).unwrap_or("Default");
        let efforts: Vec<String> = model
            .and_then(|m| m["efforts"].as_array())
            .map(|e| {
                e.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let current = self
            .effort
            .get(agent)
            .and_then(|e| efforts.iter().position(|x| x == e));
        // Claude reads its effort only at start; Codex takes it on every turn (PRD §5.8).
        let fixed = live && agent == "claude-code";

        let mut panel = div()
            .id("effort-popover")
            .occlude()
            .absolute()
            .right(px(GUTTER + 44.))
            .bottom(px(112.))
            .w(px(272.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .rounded(px(14.))
            .border(px(HAIRLINE))
            .border_color(theme.border_strong)
            .bg(theme.track_bg)
            .shadow_lg()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());

        if self.model_list_open {
            for (agent_id, agent_name) in AGENTS {
                if live && agent_id != agent {
                    continue;
                }
                panel = panel.child(label(theme, agent_name));
                for (ix, m) in self.models.get(agent_id).into_iter().flatten().enumerate() {
                    let on = agent_id == agent
                        && self.model_pick.get(agent_id).copied().unwrap_or(0) == ix;
                    panel = panel.child(
                        div()
                            .id(SharedString::from(format!("model-{agent_id}-{ix}")))
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(8.))
                            .h(px(28.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(if on { theme.text } else { theme.text_secondary })
                            .hover(|s| s.bg(theme.border_subtle))
                            .child(m["label"].as_str().unwrap_or("Default").to_owned())
                            .when(on, |d| d.child(icon(icons::CHECK, 14., theme.accent)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.agent = agent_id;
                                this.model_pick.insert(agent_id, ix);
                                this.effort.remove(agent_id);
                                this.model_list_open = false;
                                cx.notify();
                            })),
                    );
                }
            }
            return panel;
        }

        let title = current.map_or("Default effort".to_owned(), |i| effort_label(&efforts[i]));
        panel = panel.child(
            div()
                .flex()
                .items_start()
                .child(div().size(px(16.)))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.accent)
                                .child(title),
                        )
                        .child(
                            div()
                                .id("open-model-list")
                                .flex()
                                .items_center()
                                .gap(px(2.))
                                .text_size(px(12.5))
                                .text_color(theme.text_secondary)
                                .when(!live, |d| {
                                    d.cursor_pointer()
                                        .hover(|s| s.text_color(theme.text))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.model_list_open = true;
                                            cx.notify();
                                        }))
                                })
                                .child(model_name.to_owned())
                                .when(!live, |d| {
                                    d.child(icon(icons::CHEVRON_RIGHT, 12., theme.text_tertiary))
                                }),
                        ),
                )
                .child(
                    div()
                        .id("effort-reset")
                        .cursor_pointer()
                        .child(icon(icons::ROTATE, 15., theme.text_tertiary))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.effort.remove(agent);
                            cx.notify();
                        })),
                ),
        );

        if efforts.is_empty() {
            return panel.child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text_tertiary)
                    .child("This model doesn't offer effort levels."),
            );
        }
        let n = efforts.len() as f32;
        let mut track = div()
            .relative()
            .flex()
            .h(px(30.))
            .rounded_full()
            .bg(theme.bg)
            .when(fixed, |d| d.opacity(0.5));
        if let Some(i) = current {
            track = track.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .h_full()
                    .w(gpui::relative((i as f32 + 0.5) / n))
                    .min_w(px(30.))
                    .rounded_full()
                    .bg(theme.accent),
            );
        }
        for (ix, effort) in efforts.iter().enumerate() {
            let filled = current.is_some_and(|c| ix < c);
            let effort = effort.clone();
            track = track.child(
                div()
                    .id(SharedString::from(format!("effort-{ix}")))
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!fixed, |d| d.cursor_pointer())
                    .child(if current == Some(ix) {
                        div()
                            .size(px(24.))
                            .rounded_full()
                            .bg(gpui::white())
                            .shadow_sm()
                    } else {
                        div().size(px(4.)).rounded_full().bg(if filled {
                            theme.accent_on_solid
                        } else {
                            theme.text_tertiary
                        })
                    })
                    .when(!fixed, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.effort.insert(agent, effort.clone());
                            cx.notify();
                        }))
                    }),
            );
        }
        panel.child(track).when(fixed, |d| {
            d.child(
                div()
                    .text_size(px(11.5))
                    .text_color(theme.text_tertiary)
                    .child(
                        "Claude sets effort when a session starts. This applies to the next one.",
                    ),
            )
        })
    }

    /// Asks the daemon once what each agent can run.
    pub(crate) fn load_models(&mut self, cx: &mut Context<Self>) {
        for (agent, _) in AGENTS {
            self.call::<Vec<Value>>(
                "agent.models",
                json!({ "agent": agent }),
                move |this, result, cx| {
                    if let Ok(models) = result {
                        this.models.insert(agent, models);
                        cx.notify();
                    }
                },
                cx,
            );
        }
    }

    /// The model id to pass for `agent`, if one is picked (None means the agent's own default).
    fn chosen_model(&self, agent: &str) -> Option<String> {
        let models = self.models.get(agent)?;
        let pick = self.model_pick.get(agent).copied().unwrap_or(0);
        models.get(pick)?["id"].as_str().map(str::to_owned)
    }

    /// Plexo's form rows (its LINK / TO boxes), for creating a workspace.
    pub(crate) fn new_workspace_form(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = &self.form else {
            return div().into_any_element();
        };
        let row = |label_text: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .py(px(10.))
                .rounded(px(10.))
                .border(px(1.))
                .border_color(theme.border)
                .bg(theme.input_bg)
                .child(label(theme, label_text).flex_none().w(px(44.)))
        };
        let repos = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(self.board.repos.iter().map(|repo| {
                let on = form.repo_id.as_deref() == Some(repo.id.as_str());
                let id = repo.id.clone();
                let name = repo
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&repo.path)
                    .to_owned();
                div()
                    .id(SharedString::from(format!("repo-{}", repo.id)))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .font_family(MONO)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .bg(if on { theme.accent } else { theme.bg_tertiary })
                    .text_color(if on {
                        theme.accent_on_solid
                    } else {
                        theme.text_secondary
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(form) = &mut this.form {
                            form.repo_id = Some(id.clone());
                        }
                        cx.notify();
                    }))
                    .child(name)
            }));

        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(12.))
            .px(px(GUTTER))
            .py(px(20.))
            .child(label(theme, "New workspace"))
            .child(row("Repo").child(repos))
            .child(row("Name").child(div().flex_1().child(Input::new(&form.name).appearance(false))))
            .child(
                div()
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .text_color(theme.text_tertiary)
                    .child("Creates a worktree on branch swarn/<name>, outside the repo, with your .env files copied in."),
            )
            .when_some(form.error.clone(), |d, e| {
                d.child(div().font_family(MONO).text_size(px(11.5)).text_color(theme.danger).child(e))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(button(theme, "form-cancel", "Cancel", ButtonKind::Secondary).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.form = None;
                            cx.notify();
                        },
                    )))
                    .child(
                        button(theme, "form-create", "Create", ButtonKind::Primary)
                            .on_click(cx.listener(|this, _, _, cx| this.create_workspace(cx))),
                    ),
            )
            .into_any_element()
    }
}

/// "xhigh" reads as "Extra High" in the control, as in the reference.
fn effort_label(effort: &str) -> String {
    match effort {
        "xhigh" => "Extra High".into(),
        e => {
            let mut c = e.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
    }
}
