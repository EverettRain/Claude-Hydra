//! Data of the dashboard: every container account's plan, identity and token usage, and the
//! combined usage.
//!
//! Building it scans the transcripts, which takes a while on a cold start. The last result is
//! kept on disk so the page can show it at once (`cached`) while a fresh one is built (`build`).
use crate::{code_usage, container, quota_history, settings};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    /// Share of the window used, 0-100.
    used: f64,
    /// When the window resets (ms).
    resets_at: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// When the status line last recorded them (ms).
    recorded_at: u64,
    five_hour: Option<Limit>,
    seven_day: Option<Limit>,
}

/// The latest readings of the account's limits, from its history (fed by the status line log).
fn limits(id: &str, dir: &Path) -> Option<Limits> {
    let h = match quota_history::sync(id, dir) {
        Ok(h) => h,
        Err(e) => { crate::paths::append_log("hydra-error.log", &format!("limits {id}: {e}")); return None; }
    };
    let window = |key: &str| h.latest(key).map(|r| (r.to, Limit { used: r.utilization, resets_at: r.resets_at }));
    let (five_hour, seven_day) = (window("five_hour"), window("seven_day"));
    let recorded_at = five_hour.iter().chain(seven_day.iter()).map(|(at, _)| *at).max()?;
    Some(Limits { recorded_at, five_hour: five_hour.map(|(_, l)| l), seven_day: seven_day.map(|(_, l)| l) })
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerUsage {
    id: String,
    name: String,
    account: code_usage::Account,
    usage: code_usage::Usage,
    blocked: Option<String>,
    blocked_soft: bool,
    /// The plan's limits as the account's latest Claude Code session saw them.
    limits: Option<Limits>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    containers: Vec<ContainerUsage>,
    total: code_usage::Usage,
    /// When it was built (ms).
    updated_at: u64,
}

fn cache_path() -> PathBuf { crate::paths::storage_root().join("dashboard-cache.json") }

/// Builds the dashboard and keeps it as the cache for the next start.
pub fn build() -> Dashboard {
    let s = settings::load();
    let mut ids: Vec<(String, String, PathBuf)> = s.accounts.iter().filter(|(_, a)| a.container.is_some())
        .filter_map(|(id, a)| Some((id.clone(), a.label.clone().unwrap_or_else(|| id.clone()), container::claude_data_dir(&s, id).ok()?)))
        .collect();
    ids.sort_by(|a, b| a.1.cmp(&b.1));
    let sources: Vec<(PathBuf, PathBuf)> = ids.iter().map(|(id, _, d)| (d.clone(), container::usage_archive(id))).collect();
    let (each, total) = code_usage::scan_all(&sources);
    let containers = ids.into_iter().zip(each).map(|((id, name, dir), usage)| {
        let acc = s.account(&id);
        ContainerUsage { account: code_usage::account(&dir), limits: limits(&id, &dir), usage, blocked_soft: acc.blocked.is_some() && acc.blocked_soft, blocked: acc.blocked, id, name }
    }).collect();
    let dashboard = Dashboard { containers, total, updated_at: crate::paths::now_millis() };
    if let Err(e) = save(&dashboard) { crate::paths::append_log("hydra-error.log", &format!("dashboard cache: {e}")); }
    dashboard
}

fn save(d: &Dashboard) -> Result<(), String> {
    let path = cache_path();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(d).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    // Owner-only: it names the accounts' e-mail addresses.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// The last dashboard built, if any; a missing or unreadable cache is simply no cache.
pub fn cached() -> Option<Dashboard> {
    let d: Dashboard = serde_json::from_slice(&std::fs::read(cache_path()).ok()?).ok()?;
    // Accounts deleted since then must not reappear.
    let s = settings::load();
    let containers = d.containers.into_iter().filter(|c| s.accounts.get(&c.id).is_some_and(|a| a.container.is_some())).collect();
    Some(Dashboard { containers, ..d })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_the_latest_readings_in_the_history() {
        // Arrange
        let id = format!("ctr-limits-{}", uuid::Uuid::new_v4().simple());
        let dir = std::env::temp_dir().join(&id);
        std::fs::create_dir_all(&dir).unwrap();
        let now = crate::paths::now_millis();
        std::fs::write(dir.join(quota_history::LOG), format!(
            "{{\"at\":{},\"rateLimits\":{{\"five_hour\":{{\"used_percentage\":10,\"resets_at\":1794700000}}}}}}\n{{\"at\":{now},\"rateLimits\":{{\"five_hour\":{{\"used_percentage\":23.5,\"resets_at\":1794700000}},\"seven_day\":{{\"used_percentage\":41}}}}}}\n",
            now - 60_000)).unwrap();

        // Act
        let read = limits(&id, &dir);
        let none = limits(&format!("{id}-none"), &dir.join("missing"));

        // Assert
        assert_eq!(read, Some(Limits {
            recorded_at: now,
            five_hour: Some(Limit { used: 23.5, resets_at: Some(1_794_700_000_000) }),
            seven_day: Some(Limit { used: 41.0, resets_at: None }),
        }));
        assert!(none.is_none());
        quota_history::delete(&id);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dashboard_round_trips_through_json() {
        // Arrange
        let d = Dashboard {
            containers: vec![ContainerUsage { id: "ctr-1".into(), name: "工作".into(), account: Default::default(), usage: Default::default(), blocked: Some("x".into()), blocked_soft: true, limits: None }],
            total: Default::default(),
            updated_at: 42,
        };

        // Act
        let back: Dashboard = serde_json::from_slice(&serde_json::to_vec(&d).unwrap()).unwrap();

        // Assert
        assert_eq!(back.updated_at, 42);
        assert_eq!(back.containers[0].name, "工作");
        assert!(back.containers[0].blocked_soft);
    }
}
