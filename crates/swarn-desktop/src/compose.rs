//! Everything that sends something to the daemon from the window: the prompt composer, the
//! new-workspace form and adding a repository. Every call runs off the UI thread.

use gpui::{
    AnyElement, Context, Entity, FontWeight, PathPromptOptions, SharedString, Window, div,
    prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use swarn_core::local_api;
use swarn_core::schema::{Status, Workspace};

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
            .auto_grow(1, 8)
            .placeholder("Tell the agent what to do. Enter sends, Shift-Enter adds a line.")
            .submit_on_enter(true)
    });
    cx.subscribe_in(&state, window, |this, _, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter { shift: false, .. } = event {
            this.submit(window, cx);
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
            Some(session_id) => (
                "session.prompt",
                json!({ "session_id": session_id, "text": text }),
            ),
            None => {
                let mut params =
                    json!({ "workspace_id": workspace_id, "prompt": text, "agent": self.agent });
                if let Some(model) = self.chosen_model(self.agent) {
                    params["model"] = json!(model);
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
                .spawn(
                    async move { local_api::call::<T>(method, params).map_err(|e| e.to_string()) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.last_error = result.as_ref().err().cloned();
                done(this, result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The bar under the transcript: a growing text area, the agent choice when it starts a new
    /// session, Stop while the agent works, and Send.
    pub(crate) fn composer(
        &self,
        theme: &Theme,
        workspace_id: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = self.board.latest_session(workspace_id);
        let live = self.live_session(workspace_id).is_some();
        let working = session.is_some_and(|s| {
            matches!(
                s.status,
                Status::Working | Status::NeedsInput | Status::Starting
            )
        });

        let mut actions = div().flex().items_center().gap(px(8.));
        if live {
            let agent = session.map(|s| s.agent.as_str()).unwrap_or_default();
            let name = AGENTS
                .iter()
                .find(|(id, _)| *id == agent)
                .map_or(agent, |(_, n)| n);
            actions = actions.child(label(theme, name));
        } else {
            actions = actions.child(self.agent_toggle(theme, cx));
            actions = actions.child(self.model_chip(theme, cx));
        }
        actions = actions.child(div().flex_1());
        if working {
            actions = actions.child(
                button(theme, "stop", "Stop", ButtonKind::Secondary)
                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
            );
        }
        actions = actions.child(
            button(
                theme,
                "send",
                if live { "Send" } else { "Start" },
                ButtonKind::Primary,
            )
            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
        );

        div()
            .flex_none()
            .px(px(GUTTER))
            .py(px(12.))
            .border_t(px(HAIRLINE))
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(10.))
                    .border(px(1.))
                    .border_color(theme.border_strong)
                    .bg(theme.input_bg)
                    .child(
                        Textarea::new(&self.composer)
                            .appearance(false)
                            .text_size(px(13.5)),
                    )
                    .child(actions),
            )
            .into_any_element()
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

    /// Plexo's cyclable chip: click to step through the agent's models.
    fn model_chip(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let agent = self.agent;
        let models = self.models.get(agent).cloned().unwrap_or_default();
        let pick = self
            .model_pick
            .get(agent)
            .copied()
            .unwrap_or(0)
            .min(models.len().saturating_sub(1));
        let name = models
            .get(pick)
            .and_then(|m| m["label"].as_str())
            .unwrap_or("Default")
            .to_owned();
        div()
            .id("model-chip")
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(26.))
            .px(px(8.))
            .rounded(px(6.))
            .border(px(HAIRLINE))
            .border_color(theme.border)
            .font_family(MONO)
            .text_size(px(11.))
            .text_color(theme.text_secondary)
            .hover(|s| s.bg(theme.border_subtle))
            .when(models.len() > 1, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    let next = (this.model_pick.get(agent).copied().unwrap_or(0) + 1)
                        % models.len().max(1);
                    this.model_pick.insert(agent, next);
                    cx.notify();
                }))
            })
            .child(div().text_color(theme.text_tertiary).child("MODEL"))
            .child(name)
    }

    /// Plexo's segmented toggle (its MB/s | Mbps switch), here choosing the agent.
    fn agent_toggle(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .h(px(26.))
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(6.))
            .border(px(HAIRLINE))
            .border_color(theme.border)
            .children(AGENTS.iter().map(|(id, name)| {
                let on = self.agent == *id;
                div()
                    .id(SharedString::from(format!("agent-{id}")))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .font_family(MONO)
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { theme.text } else { theme.text_tertiary })
                    .when(on, |d| d.bg(theme.track_bg))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.agent = id;
                        cx.notify();
                    }))
                    .child(*name)
            }))
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
