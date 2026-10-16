//! The background daemon as a per-user launch agent, registered through `SMAppService` (PRD §5.5
//! step 1): one Login Items approval, no terminal. Only works from the app bundle that
//! `scripts/bundle.sh` builds; a `cargo run` build reports `NotBundled`.

use objc2_foundation::NSString;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// Ships at `Swarn.app/Contents/Library/LaunchAgents/`, pointing at `Contents/MacOS/swarnd`.
const PLIST: &str = "dev.swarn.daemon.plist";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    NotRegistered,
    Enabled,
    /// The user turned it off in System Settings › Login Items, or has yet to allow it.
    RequiresApproval,
    /// Not running from Swarn.app, so there is no plist to register.
    NotBundled,
}

fn agent() -> objc2::rc::Retained<SMAppService> {
    // SAFETY: plain ServiceManagement calls with a valid NSString, on the main thread.
    unsafe { SMAppService::agentServiceWithPlistName(&NSString::from_str(PLIST)) }
}

pub fn status() -> Service {
    // SAFETY: see `agent`.
    match unsafe { agent().status() } {
        SMAppServiceStatus::Enabled => Service::Enabled,
        SMAppServiceStatus::RequiresApproval => Service::RequiresApproval,
        SMAppServiceStatus::NotRegistered => Service::NotRegistered,
        _ => Service::NotBundled,
    }
}

pub fn register() -> Result<Service, String> {
    // SAFETY: see `agent`.
    unsafe { agent().registerAndReturnError() }
        .map_err(|e| e.localizedDescription().to_string())?;
    Ok(status())
}

pub fn open_login_items() {
    // SAFETY: see `agent`.
    unsafe { SMAppService::openSystemSettingsLoginItems() }
}
