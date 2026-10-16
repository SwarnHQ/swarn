//! How a phone finds this Mac (§2.7): an ordered list of addresses, Tailscale first because it is
//! the only one that works away from home, then the LAN, then mDNS; and the mDNS registration.

use swarn_core::schema::Candidate;
use tokio::process::Command;

use crate::env::AgentEnv;

pub async fn candidates(env: &AgentEnv, port: u16, mdns_instance: Option<&str>) -> Vec<Candidate> {
    let mut out = Vec::new();
    let candidate = |kind: &str, host: String| Candidate {
        kind: kind.into(),
        host,
        port,
    };
    if let Some(status) = tailscale_status(env).await {
        if let Some(name) = status["Self"]["DNSName"].as_str().filter(|n| !n.is_empty()) {
            out.push(candidate(
                "tailscale",
                name.trim_end_matches('.').to_owned(),
            ));
        }
        for ip in status["Self"]["TailscaleIPs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
        {
            if !ip.contains(':') {
                out.push(candidate("tailscale_ip", ip.to_owned()));
            }
        }
    }
    for ip in lan_addresses().await {
        if !out.iter().any(|c| c.host == ip) {
            out.push(candidate("lan", ip));
        }
    }
    if let Some(instance) = mdns_instance {
        out.push(candidate("mdns", format!("{instance}._swarn._tcp.local")));
    }
    out
}

async fn tailscale_status(env: &AgentEnv) -> Option<serde_json::Value> {
    let bin = env.which("tailscale").or_else(|| {
        Some("/Applications/Tailscale.app/Contents/MacOS/Tailscale".into())
            .filter(|p: &std::path::PathBuf| p.exists())
    })?;
    let out = Command::new(bin)
        .args(["status", "--json"])
        .output()
        .await
        .ok()?;
    out.status
        .success()
        .then(|| serde_json::from_slice(&out.stdout).ok())
        .flatten()
}

/// IPv4 addresses on up interfaces, minus loopback and Tailscale's CGNAT range (listed above).
async fn lan_addresses() -> Vec<String> {
    let Ok(out) = Command::new("/sbin/ifconfig").output().await else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().strip_prefix("inet "))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|ip| !ip.starts_with("127.") && !is_cgnat(ip))
        .map(str::to_owned)
        .collect()
}

fn is_cgnat(ip: &str) -> bool {
    let mut parts = ip.split('.').filter_map(|p| p.parse::<u8>().ok());
    matches!((parts.next(), parts.next()), (Some(100), Some(b)) if (64..128).contains(&b))
}

/// Registers `_swarn._tcp` so a phone on the same network finds the daemon by name. The returned
/// daemon keeps the registration alive; drop it to withdraw. None when mDNS is unavailable or
/// switched off with SWARN_NO_MDNS (tests).
pub fn register_mdns(machine_name: &str, port: u16) -> Option<(mdns_sd::ServiceDaemon, String)> {
    if std::env::var_os("SWARN_NO_MDNS").is_some() {
        return None;
    }
    let daemon = mdns_sd::ServiceDaemon::new().ok()?;
    let instance: String = format!("swarn-{machine_name}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let host = format!("{instance}.local.");
    let info = mdns_sd::ServiceInfo::new(
        "_swarn._tcp.local.",
        &instance,
        &host,
        "",
        port,
        None::<std::collections::HashMap<String, String>>,
    )
    .ok()?
    .enable_addr_auto();
    daemon.register(info).ok()?;
    Some((daemon, instance))
}

#[cfg(test)]
mod tests {
    #[test]
    fn cgnat_range_is_tailscales() {
        assert!(super::is_cgnat("100.101.2.3"));
        assert!(!super::is_cgnat("100.20.2.3"));
        assert!(!super::is_cgnat("192.168.1.42"));
    }
}
