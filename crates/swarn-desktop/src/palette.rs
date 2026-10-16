//! Keyboard actions and the command palette (§5.4). Every action is a GPUI action with a default
//! binding, and the palette lists the same actions, so a new one shows up in both places.

use gpui::{Action, App, Context, Div, KeyBinding, Window, actions, prelude::*};
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
            "Setup & checks",
            &["settings", "sign in", "policy", "approvals"],
            Box::new(OpenSetup),
        ),
    ]
}

impl Shell {
    /// Hangs every action off the window's root element.
    pub(crate) fn on_actions(&self, root: Div, cx: &mut Context<Self>) -> Div {
        root.on_action(
            cx.listener(|this, _: &CommandPalette, window, cx| this.open_palette(window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &NewWorkspace, window, cx| this.open_new_workspace(window, cx)),
        )
        .on_action(cx.listener(|this, _: &AddRepository, _, cx| this.add_repository(cx)))
        .on_action(cx.listener(|this, _: &ShowTranscript, _, cx| {
            this.tab = Tab::Transcript;
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ShowDiff, _, cx| {
            this.tab = Tab::Diff;
            this.fetch_diff(cx);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &CreatePullRequest, _, cx| this.create_pr(cx)))
        .on_action(cx.listener(|this, _: &StopAgent, _, cx| this.stop(cx)))
        .on_action(cx.listener(|this, _: &OpenSetup, _, cx| this.open_setup(cx)))
        .on_action(cx.listener(|this, _: &NextWorkspace, _, cx| this.step_workspace(1, cx)))
        .on_action(cx.listener(|this, _: &PreviousWorkspace, _, cx| this.step_workspace(-1, cx)))
        .on_action(cx.listener(|this, _: &FocusComposer, window, cx| {
            this.composer
                .update(cx, |state, cx| state.focus(window, cx));
        }))
    }

    fn step_workspace(&mut self, by: isize, cx: &mut Context<Self>) {
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

    /// The palette lives in a dialog outside this view's element tree, so on confirm it closes
    /// itself and dispatches the chosen action from this view's focus handle.
    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = cx.new(|cx| CommandState::new(window, cx));
        let focus = self.focus.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let focus = focus.clone();
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
                    .on_confirm(move |path, window, cx| {
                        window.close_dialog(cx);
                        if let Some((_, _, action)) = commands().into_iter().nth(path.row) {
                            focus.dispatch_action(&*action, window, cx);
                        }
                    }),
            )
        });
    }
}
