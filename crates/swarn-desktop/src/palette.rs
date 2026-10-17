//! Keyboard actions and the command palette (§5.4). Every action is a GPUI action with a default
//! binding, and the palette lists the same actions, so a new one shows up in both places.

use gpui::{Action, App, Context, KeyBinding, WeakEntity, Window, actions, prelude::*};
use gpui_component::WindowExt as _;
use gpui_component::command::{Command, CommandItem, CommandState};

use crate::review::Tab;
use crate::shell::Shell;

actions!(
    swarn,
    [
        CommandPalette,
        NewWorkspace,
        AddRepository,
        ShowTranscript,
        ShowDiff,
        CreatePullRequest,
        StopAgent,
        OpenSetup,
        PairPhone,
        ArchiveWorkspace,
        NextWorkspace,
        PreviousWorkspace,
        FocusComposer,
    ]
);

/// ponytail: defaults only; the PRD's `~/.config/swarn/keymap.json` overrides come when someone
/// asks to rebind something.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-k", CommandPalette, None),
        KeyBinding::new("cmd-shift-p", CommandPalette, None),
        KeyBinding::new("cmd-n", NewWorkspace, None),
        KeyBinding::new("cmd-o", AddRepository, None),
        KeyBinding::new("cmd-1", ShowTranscript, None),
        KeyBinding::new("cmd-2", ShowDiff, None),
        KeyBinding::new("cmd-shift-r", CreatePullRequest, None),
        KeyBinding::new("cmd-.", StopAgent, None),
        KeyBinding::new("cmd-,", OpenSetup, None),
        KeyBinding::new("alt-cmd-down", NextWorkspace, None),
        KeyBinding::new("alt-cmd-up", PreviousWorkspace, None),
        KeyBinding::new("cmd-l", FocusComposer, None),
    ]);
}

/// What the palette offers, in order. Labels are what people search for.
fn commands() -> Vec<(&'static str, &'static [&'static str], Box<dyn Action>)> {
    vec![
        (
            "New workspace",
            &["create", "branch", "worktree"],
            Box::new(NewWorkspace),
        ),
        (
            "Add repository",
            &["open", "folder", "repo"],
            Box::new(AddRepository),
        ),
        (
            "Show transcript",
            &["chat", "session"],
            Box::new(ShowTranscript),
        ),
        ("Show diff", &["changes", "review"], Box::new(ShowDiff)),
        (
            "Create pull request",
            &["pr", "github", "push"],
            Box::new(CreatePullRequest),
        ),
        (
            "Stop the agent",
            &["cancel", "interrupt"],
            Box::new(StopAgent),
        ),
        (
            "Focus the prompt",
            &["composer", "type"],
            Box::new(FocusComposer),
        ),
        (
            "Next workspace",
            &["down", "switch"],
            Box::new(NextWorkspace),
        ),
        (
            "Previous workspace",
            &["up", "switch"],
            Box::new(PreviousWorkspace),
        ),
        (
            "Archive workspace",
            &["remove", "delete", "close", "worktree"],
            Box::new(ArchiveWorkspace),
        ),
        (
            "Pair a phone",
            &["iphone", "qr", "device", "mobile"],
            Box::new(PairPhone),
        ),
        (
            "Setup & checks",
            &["settings", "sign in", "policy", "approvals"],
            Box::new(OpenSetup),
        ),
    ]
}

/// Handles every action at the app level, where all dispatch paths end: key bindings from
/// anywhere in the window, and the palette, whose overlay sits outside the shell's element tree.
pub fn register_handlers(cx: &mut App, shell: WeakEntity<Shell>) {
    fn on<A: Action>(
        cx: &mut App,
        shell: &WeakEntity<Shell>,
        run: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
    ) {
        let shell = shell.clone();
        let run = std::rc::Rc::new(run);
        cx.on_action(move |_: &A, cx| {
            // Dispatch happens while the window is borrowed, so the work waits until it's free.
            let (shell, run) = (shell.clone(), run.clone());
            cx.defer(move |cx| {
                let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied())
                else {
                    return;
                };
                let _ = window.update(cx, |_, window, cx| {
                    let _ = shell.update(cx, |this, cx| run(this, window, cx));
                });
            });
        });
    }
    on::<CommandPalette>(cx, &shell, |this, window, cx| this.open_palette(window, cx));
    on::<NewWorkspace>(cx, &shell, |this, window, cx| {
        this.open_new_workspace(window, cx)
    });
    on::<AddRepository>(cx, &shell, |this, _, cx| this.add_repository(cx));
    on::<ShowTranscript>(cx, &shell, |this, _, cx| {
        this.tab = Tab::Transcript;
        cx.notify();
    });
    on::<ShowDiff>(cx, &shell, |this, _, cx| {
        this.tab = Tab::Diff;
        this.fetch_diff(cx);
        cx.notify();
    });
    on::<CreatePullRequest>(cx, &shell, |this, _, cx| this.create_pr(cx));
    on::<StopAgent>(cx, &shell, |this, _, cx| this.stop(cx));
    on::<OpenSetup>(cx, &shell, |this, _, cx| this.open_setup(cx));
    on::<ArchiveWorkspace>(cx, &shell, |this, window, cx| {
        this.archive_workspace(false, window, cx)
    });
    on::<PairPhone>(cx, &shell, |this, _, cx| {
        this.open_setup(cx);
        this.start_pairing(cx);
    });
    on::<NextWorkspace>(cx, &shell, |this, _, cx| this.step_workspace(1, cx));
    on::<PreviousWorkspace>(cx, &shell, |this, _, cx| this.step_workspace(-1, cx));
    on::<FocusComposer>(cx, &shell, |this, window, cx| {
        this.composer
            .update(cx, |state, cx| state.focus(window, cx));
    });
}

impl Shell {
    pub(crate) fn step_workspace(&mut self, by: isize, cx: &mut Context<Self>) {
        let ids: Vec<&String> = self.board.workspaces.iter().map(|w| &w.id).collect();
        if ids.is_empty() {
            return;
        }
        let at = self
            .selected
            .as_ref()
            .and_then(|s| ids.iter().position(|id| *id == s))
            .unwrap_or(0) as isize;
        let next = (at + by).rem_euclid(ids.len() as isize) as usize;
        self.selected = Some(ids[next].clone());
        cx.notify();
    }

    /// Picking an item dispatches its action, which `register_handlers` catches; the palette only
    /// has to close itself.
    pub(crate) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = cx.new(|cx| CommandState::new(window, cx));
        // The search field only knows it's searchable after its first render, so focus it then;
        // otherwise typing goes nowhere.
        let field = state.clone();
        window
            .on_next_frame(move |window, cx| field.update(cx, |state, cx| state.focus(window, cx)));
        window.open_dialog(cx, move |dialog, _, _| {
            let items = commands().into_iter().map(|(label, keywords, action)| {
                CommandItem::new()
                    .label(label)
                    .keywords(keywords.iter().copied())
                    .action(action)
            });
            dialog.w(gpui::px(520.)).child(
                Command::new(&state)
                    .items(items)
                    .placeholder("Type a command…")
                    .bordered(false)
                    .on_confirm(|_, window, cx| window.close_dialog(cx)),
            )
        });
    }
}
