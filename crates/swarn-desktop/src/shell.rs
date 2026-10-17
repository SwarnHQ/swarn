//! The window, drawn after the Codex desktop app on Conductor's three columns: a sidebar of
//! projects and workspaces, the selected session's transcript, and a panel for its changes. Everything shown comes from the daemon; the desktop holds no agent state (§1.1).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, Context, Entity, FocusHandle, FollowMode, FontWeight, ListAlignment, ListState,
    MouseButton, SharedString, Window, div, list, prelude::*, px,
};
use serde_json::{Value, json};
use swarn_core::local_api::{self, DaemonInfo};
use swarn_core::schema::{Decision, Event, EventKind, Repo, Session, Status, Workspace};

use gpui_component::input::TextareaState;

use crate::compose::{AGENTS, NewWorkspace, composer_state};
use crate::icons::{self, icon};
use crate::model::Board;
use crate::review::{DiffState, Tab};
use crate::service::{self, Service};
use crate::setup::Setup;
use crate::theme::{HAIRLINE, MONO, SANS, TITLE_BAR_HEIGHT, Theme, label};

/// Plexo's 20px side gutter, used by every row and strip.
pub const GUTTER: f32 = 20.;
/// The width of the transcript and composer column, centred in wider windows.
pub const COLUMN: f32 = 760.;
const SIDEBAR_WIDTH: f32 = 264.;

pub struct Shell {
    pub(crate) board: Board,
    /// `None` until the first poll answers.
    online: Option<bool>,
    pub(crate) selected: Option<String>,
    /// The transcript list and the session it currently shows.
    pub(crate) list: ListState,
    pub(crate) shown: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) composer: Entity<TextareaState>,
    /// The agent a new session starts with.
    pub(crate) agent: &'static str,
    pub(crate) form: Option<NewWorkspace>,
    pub(crate) tab: Tab,
    pub(crate) diff: Option<DiffState>,
    /// Files marked viewed in the diff, as `<workspace id>:<path>`.
    pub(crate) viewed: HashSet<String>,
    pub(crate) pr_busy: bool,
    /// A one-off message for the footer, such as the URL of a new pull request.
    pub(crate) notice: Option<String>,
    /// Notification tags to the workspace each one is about.
    pub(crate) notified: HashMap<String, String>,
    /// The root's focus, so keyboard actions reach this view when nothing inside has focus.
    pub(crate) focus: FocusHandle,
    pub(crate) setup: Option<Setup>,
    pub(crate) devices: Vec<serde_json::Value>,
    pub(crate) pairing: Option<crate::pairing::Pairing>,
    /// What each agent can run, from the daemon, and which one the composer has picked.
    pub(crate) models: HashMap<&'static str, Vec<serde_json::Value>>,
    pub(crate) model_pick: HashMap<&'static str, usize>,
    /// Reasoning effort per agent; None is the agent's own default.
    pub(crate) effort: HashMap<&'static str, String>,
    /// The composer's model and effort popover, and whether it shows the model list.
    pub(crate) effort_open: bool,
    pub(crate) model_list_open: bool,
    pub(crate) policy: Option<swarn_core::local_api::Policy>,
    /// The first-run checklist opens by itself once, on a daemon with no repositories yet.
    pub(crate) setup_seen: bool,
    /// Whether the background helper is registered; checked while the daemon is unreachable.
    pub(crate) service: Service,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Theme::sync(window, cx);
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync(window, cx);
            cx.notify();
        })
        .detach();

        // ponytail: lists (repos, workspaces, sessions) poll every 2 s; there are no events for
        // them yet. Transcripts and statuses come from the event stream below.
        cx.spawn(async move |this, cx| {
            // The first answer was requested at the top of `main`, while GPUI was still starting.
            let mut first = PREFETCH.lock().unwrap().take();
            if first.is_none() {
                // Already applied in the constructor; wait a poll interval before the next read.
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
            loop {
                let lists = match first.take() {
                    Some(prefetched) => {
                        cx.background_executor()
                            .spawn(async move { prefetched.join().ok().flatten() })
                            .await
                    }
                    None => cx.background_executor().spawn(async { fetch() }).await,
                };
                if this
                    .update(cx, |this, cx| this.set_lists(lists, cx))
                    .is_err()
                {
                    break; // window closed
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        })
        .detach();

        // One thread holds the subscription and reconnects from the last seq it saw. The UI
        // drains it in 16 ms batches, so a chatty agent costs at most one relayout per frame (§5.2).
        let (tx, mut rx) = mpsc::unbounded::<(Event, bool)>();
        std::thread::spawn(move || {
            let mut last = 0;
            while !tx.is_closed() {
                let _ = local_api::subscribe(last, |event, live| {
                    last = event.seq;
                    tx.unbounded_send((event, live)).is_ok()
                });
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let mut batch = vec![first];
                while let Ok(event) = rx.try_recv() {
                    batch.push(event);
                }
                if this.update(cx, |this, cx| this.apply(&batch, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let shell = cx.weak_entity();
        cx.on_system_notification_response(move |response, cx| {
            let _ = shell.update(cx, |this, cx| this.on_notification(response, cx));
        });

        let list = ListState::new(0, ListAlignment::Top, px(400.));
        list.set_follow_mode(FollowMode::Tail);
        let composer = composer_state(window, cx);
        if crate::bench::enabled() {
            crate::bench::start(window, cx);
        }
        let mut shell = Self {
            board: Board::default(),
            online: None,
            selected: None,
            list,
            shown: None,
            last_error: None,
            composer,
            agent: AGENTS[0].0,
            form: None,
            tab: Tab::Transcript,
            diff: None,
            viewed: HashSet::new(),
            pr_busy: false,
            notice: None,
            notified: HashMap::new(),
            focus: cx.focus_handle(),
            setup: None,
            devices: Vec::new(),
            pairing: None,
            models: HashMap::new(),
            model_pick: HashMap::new(),
            effort: HashMap::new(),
            effort_open: false,
            model_list_open: false,
            policy: None,
            setup_seen: false,
            service: Service::NotBundled,
        };
        // Usually the prefetch has finished by now: show real data in the very first frame instead of
        // waiting for the poll task's first turn.
        let finished = PREFETCH
            .lock()
            .unwrap()
            .take_if(|handle| handle.is_finished());
        if let Some(lists) = finished.and_then(|handle| handle.join().ok()) {
            shell.set_lists(lists, cx);
        }
        shell
    }

    /// Re-reads the lists now instead of at the next poll, after something this window changed.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let lists = cx.background_executor().spawn(async { fetch() }).await;
            let _ = this.update(cx, |this, cx| this.set_lists(lists, cx));
        })
        .detach();
    }

    fn set_lists(&mut self, lists: Option<Lists>, cx: &mut Context<Self>) {
        self.online = Some(lists.is_some());
        if lists.is_none() {
            self.service = service::status();
        }
        if let Some(lists) = &lists
            && lists.repos.is_empty()
            && !self.setup_seen
            && self.setup.is_none()
        {
            self.setup_seen = true;
            self.open_setup(cx);
        }
        if lists.is_some() && self.models.is_empty() {
            self.load_models(cx);
            self.load_policy(cx);
        }
        if lists.is_some() {
            crate::bench::connected();
            crate::bench::mark("lists arrived");
        }
        if let Some(lists) = lists {
            self.board.info = Some(lists.info);
            self.board.repos = lists.repos;
            self.board.workspaces = lists.workspaces;
            self.board.sessions = lists.sessions;
            self.devices = lists.devices;
        }
        cx.notify();
    }

    fn apply(&mut self, batch: &[(Event, bool)], cx: &mut Context<Self>) {
        let mut dirty: Option<usize> = None;
        for (event, live) in batch {
            if *live {
                self.notify_live(event, cx);
            }
            if let Some((session, ix)) = self.board.apply(event)
                && self.shown.as_deref() == Some(session.as_str())
            {
                dirty = Some(dirty.map_or(ix, |d| d.min(ix)));
            }
        }
        // The diff only changes when the agent finishes an edit or a turn.
        let edited = batch.iter().any(|(e, _)| {
            self.shown.as_deref() == Some(e.session_id.as_str())
                && matches!(
                    e.kind,
                    EventKind::ToolCallUpdate | EventKind::AgentMessageEnd | EventKind::DiffStat
                )
        });
        if edited && self.tab == Tab::Diff {
            self.fetch_diff(cx);
        }
        if let (Some(from), Some(shown)) = (dirty, &self.shown) {
            let old = self.list.item_count();
            let new = self
                .board
                .transcripts
                .get(shown)
                .map_or(0, |t| t.blocks.len());
            let from = from.min(old);
            self.list.splice(from..old, new - from);
        }
        cx.notify();
    }

    pub(crate) fn resolve(
        &mut self,
        request_id: String,
        decision: Decision,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let params = json!({ "request_id": request_id, "decision": decision });
            let result = cx
                .background_executor()
                .spawn(async move { local_api::call::<Value>("approval.resolve", params) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.last_error = result.err().map(|e| e.to_string());
                cx.notify();
            });
        })
        .detach();
    }

    /// Keeps a valid selection, preferring a workspace that is waiting on the user.
    fn selected_workspace(&mut self) -> Option<&Workspace> {
        let still_there = self
            .selected
            .as_ref()
            .is_some_and(|id| self.board.workspaces.iter().any(|w| &w.id == id));
        if !still_there {
            let waiting = self
                .board
                .workspaces
                .iter()
                .find(|w| self.board.workspace_status(&w.id) == Some(Status::NeedsInput));
            self.selected = waiting
                .or(self.board.workspaces.first())
                .map(|w| w.id.clone());
        }
        let id = self.selected.clone()?;
        self.board.workspaces.iter().find(|w| w.id == id)
    }
}

struct Lists {
    info: DaemonInfo,
    repos: Vec<Repo>,
    workspaces: Vec<Workspace>,
    sessions: Vec<Session>,
    devices: Vec<serde_json::Value>,
}

static PREFETCH: std::sync::Mutex<Option<std::thread::JoinHandle<Option<Lists>>>> =
    std::sync::Mutex::new(None);

/// Starts reading the daemon's lists on a thread, before GPUI is up, so they're ready by the time
/// the window is (cold start, §8.1).
pub fn prefetch() {
    *PREFETCH.lock().unwrap() = Some(std::thread::spawn(fetch));
}

fn fetch() -> Option<Lists> {
    Some(Lists {
        info: local_api::call("daemon.info", json!({})).ok()?,
        repos: local_api::call("repo.list", json!({})).ok()?,
        workspaces: local_api::call("workspace.list", json!({})).ok()?,
        sessions: local_api::call("session.list", json!({})).ok()?,
        // An older daemon without devices still works.
        devices: local_api::call("device.list", json!({})).unwrap_or_default(),
    })
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let workspace = self.selected_workspace().cloned();
        let session = workspace
            .as_ref()
            .and_then(|w| self.board.latest_session(&w.id))
            .cloned();

        let shown = session.as_ref().map(|s| s.id.clone());
        if shown != self.shown {
            let count = shown
                .as_ref()
                .and_then(|id| self.board.transcripts.get(id))
                .map_or(0, |t| t.blocks.len());
            self.list.reset(count);
            self.list.scroll_to_end();
            self.shown = shown;
        }

        let main = match (self.online, &workspace, &session) {
            (None, _, _) => div().flex_1().into_any_element(),
            (Some(false), _, _) => self.offline_view(&theme, cx),
            (Some(true), _, _) if self.setup.is_some() => self.setup_view(&theme, cx),
            _ if self.form.is_some() => self.new_workspace_form(&theme, cx),
            (Some(true), None, _) if self.board.repos.is_empty() => {
                hero(&theme, "Add a repository to get started".into())
            }
            (Some(true), None, _) => hero(&theme, "Pick a workspace, or start a new one".into()),
            (Some(true), Some(ws), session) => {
                let repo = self
                    .board
                    .repos
                    .iter()
                    .find(|r| r.id == ws.repo_id)
                    .map(|r| r.path.rsplit('/').next().unwrap_or(&r.path).to_owned())
                    .unwrap_or_default();
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(header(
                        &theme,
                        ws,
                        session.as_ref().map(|s| s.status),
                        self.header_actions(&theme, cx),
                    ))
                    .child(match session {
                        Some(_) => list(
                            self.list.clone(),
                            cx.processor(|this, ix, window, cx| this.render_block(ix, window, cx)),
                        )
                        .flex_1()
                        .pb(px(12.))
                        .into_any_element(),
                        None => hero(&theme, format!("What should we build in {repo}?")),
                    })
                    .child(self.composer(&theme, &ws.id, cx))
                    .into_any_element()
            }
        };
        let panel = match (&workspace, self.tab) {
            (Some(ws), Tab::Diff) if self.online == Some(true) && self.setup.is_none() => {
                Some(self.right_panel(&theme, &ws.id, cx))
            }
            _ => None,
        };

        div()
            .track_focus(&self.focus)
            .key_context("Swarn")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.chrome)
            .text_color(theme.text)
            .font_family(SANS)
            // Any click outside the composer's popover closes it; the popover stops its own clicks.
            .when(self.effort_open, |d| {
                d.on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.effort_open = false;
                        cx.notify();
                    }),
                )
            })
            .child(title_bar())
            // Codex's frame: the content sits on a rounded panel inset from the window edges.
            // GPUI clips to rectangles, so each column rounds its own outer corners.
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .px(px(6.))
                    .child(self.sidebar(&theme, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .bg(theme.bg)
                            .when(panel.is_none(), |d| d.rounded_r(px(10.)))
                            .child(main),
                    )
                    .when_some(panel, |d, panel| d.child(panel)),
            )
            .child(footer(
                &theme,
                self.online,
                self.board.info.as_ref(),
                self.last_error.as_deref(),
                self.notice.as_deref(),
            ))
    }
}

impl Shell {
    /// Codex's sidebar: the app name with search, a few actions with icons, then projects with
    /// their workspaces indented under them.
    fn sidebar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut column = div()
            .id("sidebar")
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(1.))
            .rounded_l(px(10.))
            .bg(theme.bg_secondary)
            .border_r(px(HAIRLINE))
            .border_color(theme.border)
            .px(px(6.))
            .pb(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(44.))
                    .px(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Swarn"),
                    )
                    .child(
                        div()
                            .id("sidebar-search")
                            .p(px(4.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.selected))
                            .child(icon(icons::SEARCH, 15., theme.text_tertiary))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_palette(window, cx)),
                            ),
                    ),
            )
            .child(
                nav_row(
                    theme,
                    "new-workspace",
                    icons::SQUARE_PEN,
                    "New workspace",
                    false,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_new_workspace(window, cx))),
            )
            .child(
                nav_row(
                    theme,
                    "pair-phone",
                    icons::SMARTPHONE,
                    "Pair a phone",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_setup(cx);
                    this.start_pairing(cx);
                })),
            )
            .child(
                nav_row(
                    theme,
                    "open-setup",
                    icons::SETTINGS,
                    "Setup & checks",
                    self.setup.is_some(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.open_setup(cx))),
            )
            .child(label(theme, "Projects").px(px(8.)).pt(px(18.)).pb(px(6.)));

        for repo in &self.board.repos {
            let name = repo
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&repo.path)
                .to_owned();
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(28.))
                    .px(px(8.))
                    .text_size(px(13.5))
                    .text_color(theme.text)
                    .child(icon(icons::FOLDER, 15., theme.text_secondary))
                    .child(div().flex_1().min_w_0().truncate().child(name)),
            );
            for ws in self
                .board
                .workspaces
                .iter()
                .filter(|w| w.repo_id == repo.id)
            {
                let status = self.board.workspace_status(&ws.id);
                let selected = self.selected.as_deref() == Some(ws.id.as_str());
                let id = ws.id.clone();
                // Only states that want attention get a mark; quiet rows stay quiet, as in Codex.
                let mark = match status {
                    Some(Status::Working | Status::Starting) => Some(theme.teal.solid),
                    Some(Status::NeedsInput) => Some(theme.amber.solid),
                    Some(Status::Failed) => Some(theme.danger),
                    _ => None,
                };
                column = column.child(
                    div()
                        .id(SharedString::from(format!("ws-{}", ws.id)))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(28.))
                        .pl(px(31.))
                        .pr(px(8.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .text_size(px(13.5))
                        .text_color(if selected {
                            theme.text
                        } else {
                            theme.text_secondary
                        })
                        .when(selected, |d| d.bg(theme.selected))
                        .when(!selected, |d| d.hover(|s| s.bg(theme.border_subtle)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = Some(id.clone());
                            this.setup = None;
                            this.form = None;
                            cx.notify();
                        }))
                        .child(div().flex_1().min_w_0().truncate().child(ws.name.clone()))
                        .when_some(mark, |d, color| {
                            d.child(div().size(px(6.)).flex_none().rounded_full().bg(color))
                        }),
                );
            }
        }
        column.child(
            nav_row(theme, "add-repo", icons::PLUS, "Add repository", false)
                .text_color(theme.text_tertiary)
                .mt(px(4.))
                .on_click(cx.listener(|this, _, _, cx| this.add_repository(cx))),
        )
    }
}

/// One sidebar action: an icon and a label on a 28px row, highlighted while its view is open.
fn nav_row(
    theme: &Theme,
    id: &'static str,
    svg: &'static [u8],
    text: &'static str,
    on: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .px(px(8.))
        .rounded(px(8.))
        .cursor_pointer()
        .text_size(px(13.5))
        .text_color(theme.text)
        .when(on, |d| d.bg(theme.selected))
        .when(!on, |d| d.hover(|s| s.bg(theme.border_subtle)))
        .child(icon(svg, 15., theme.text_secondary))
        .child(text)
}

pub fn status_word(status: Option<Status>) -> &'static str {
    match status {
        None => "idle",
        Some(Status::Starting) => "starting",
        Some(Status::Working) => "working",
        Some(Status::NeedsInput) => "needs you",
        Some(Status::Done) => "done",
        Some(Status::Failed) => "failed",
        Some(Status::Exited) => "ended",
    }
}

/// Above the transcript: the workspace name, its branch quieter beside it, a word of status, and
/// the actions on the right.
fn header(
    theme: &Theme,
    ws: &Workspace,
    status: Option<Status>,
    actions: AnyElement,
) -> impl IntoElement {
    let hue = theme.status_hue(status);
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(46.))
        .px(px(16.))
        // In a narrow window the branch gives way first, then a long name; the buttons never do.
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .items_center()
                .gap(px(8.))
                .whitespace_nowrap()
                .child(
                    div()
                        .flex_none()
                        .max_w(px(260.))
                        .truncate()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(ws.name.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(px(12.5))
                        .text_color(theme.text_tertiary)
                        .child(icon(icons::GIT_BRANCH, 13., theme.text_tertiary))
                        .child(div().truncate().child(ws.branch.clone())),
                ),
        )
        .when(status.is_some(), |d| {
            d.child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.5))
                    .text_color(theme.text_tertiary)
                    .child(div().size(px(6.)).rounded_full().bg(hue.solid))
                    .child(status_word(status)),
            )
        })
        .child(div().flex_none().child(actions))
}

pub fn chip(theme: &Theme, text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(3.))
        .rounded(px(6.))
        .bg(theme.track_bg)
        .font_family(MONO)
        .text_size(px(10.5))
        .line_height(px(10.5))
        .text_color(theme.text_secondary)
        .child(text.into())
}

/// The frame's top strip, under the native traffic lights. Drags the window; double-click zooms.
fn title_bar() -> impl IntoElement {
    div()
        .id("title-bar")
        .h(px(TITLE_BAR_HEIGHT))
        .flex_none()
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
}

/// Codex's empty canvas: one large, light line in the middle.
fn hero(theme: &Theme, text: String) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .px(px(32.))
        .child(
            div()
                .text_size(px(26.))
                .font_weight(FontWeight::LIGHT)
                .text_color(theme.text)
                .text_center()
                .child(text),
        )
        .into_any_element()
}

/// One quiet status line on the frame, under the content panel.
fn footer(
    theme: &Theme,
    online: Option<bool>,
    info: Option<&DaemonInfo>,
    error: Option<&str>,
    notice: Option<&str>,
) -> impl IntoElement {
    let (dot, text): (_, SharedString) = match (online, info, error) {
        (_, _, Some(error)) => (theme.danger, error.to_owned().into()),
        (Some(true), _, None) if notice.is_some() => {
            (theme.success, notice.unwrap_or_default().to_owned().into())
        }
        (None, _, _) => (theme.icon_muted, "Connecting…".into()),
        (Some(false), _, _) => (theme.danger, "Daemon offline · retrying every 2 s".into()),
        (Some(true), Some(info), _) => (
            theme.success,
            format!("{} · swarnd {}", info.machine_name, info.version).into(),
        ),
        (Some(true), None, _) => (theme.success, "Connected".into()),
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(7.))
        .h(px(26.))
        .px(px(14.))
        .child(div().size(px(6.)).rounded_full().bg(dot))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(theme.text_tertiary)
                .truncate()
                .child(text),
        )
}
