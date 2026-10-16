//! macOS notifications for live events while Swarn is in the background: an approval with
//! Allow once / Deny right on the banner, a finished turn, a failure. Replayed history never
//! notifies, and an approval answered anywhere else takes its banner away.

use gpui::{
    Context, SharedString, SystemNotification, SystemNotificationAction, SystemNotificationResponse,
};
use serde_json::Value;
use swarn_core::schema::{Decision, Event, EventKind};

use crate::shell::Shell;

impl Shell {
    pub(crate) fn notify_live(&mut self, event: &Event, cx: &mut Context<Self>) {
        let p = &event.payload;
        let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        let Some(workspace) = self
            .board
            .sessions
            .iter()
            .find(|s| s.id == event.session_id)
            .and_then(|s| {
                self.board
                    .workspaces
                    .iter()
                    .find(|w| w.id == s.workspace_id)
            })
            .cloned()
        else {
            return;
        };
        if event.kind == EventKind::PermissionResolved {
            cx.dismiss_system_notification(&text(&p["request_id"]));
            return;
        }
        // In front, the window itself shows all of this.
        if cx.active_window().is_some() {
            return;
        }
        let (tag, title, body, actions) = match event.kind {
            EventKind::PermissionRequest => (
                text(&p["request_id"]),
                format!("{} needs approval", workspace.name),
                text(&p["summary"]),
                vec![action("allow_once", "Allow once"), action("deny", "Deny")],
            ),
            EventKind::AgentMessageEnd if p["stop_reason"] != "cancelled" => (
                format!("done-{}", event.session_id),
                format!("{} is done", workspace.name),
                "The agent finished its turn.".into(),
                vec![],
            ),
            EventKind::Error => (
                format!("error-{}", event.session_id),
                format!("{} hit an error", workspace.name),
                text(&p["message"]),
                vec![],
            ),
            _ => return,
        };
        self.notified.insert(tag.clone(), workspace.id);
        cx.show_system_notification(SystemNotification {
            tag: tag.into(),
            title: title.into(),
            body: body.into(),
            actions,
        });
    }

    /// A button answers the approval; a click on the banner brings the workspace up.
    pub(crate) fn on_notification(
        &mut self,
        response: SystemNotificationResponse,
        cx: &mut Context<Self>,
    ) {
        let tag = response.tag.to_string();
        match response.action_id.as_deref() {
            Some("allow_once") => self.resolve(tag, Decision::AllowOnce, cx),
            Some("deny") => self.resolve(tag, Decision::Deny, cx),
            _ => {
                if let Some(workspace_id) = self.notified.get(&tag) {
                    self.selected = Some(workspace_id.clone());
                }
                cx.activate(true);
                cx.notify();
            }
        }
    }
}

fn action(id: &'static str, label: &'static str) -> SystemNotificationAction {
    SystemNotificationAction {
        id: SharedString::from(id),
        label: SharedString::from(label),
    }
}
