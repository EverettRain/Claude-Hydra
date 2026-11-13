//! Remaining-quota history per account, in the style of token-monitor's limits history.
//!
//! Every successful reading is recorded per usage window. Consecutive unchanged readings of
//! the same cycle form one run (first and last observation, sample count); a change, a new
//! cycle (reset deadline) or a sampling gap starts the next run. Failed readings are not
//! recorded, so they never become measurements. The UI turns runs into the cycle's trend and
//! the daily consumption in local time (see `src/lib/quotaHistory.ts`).
//!
//! While Hydra runs, accounts that were read by hand at least once are re-read in the
//! background. Not on macOS desktop accounts: reading their credential asks the Keychain, which
//! may show a prompt each time.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

const VERSION: u32 = 1;
/// Runs older than this are dropped (the UI shows the current cycle and 14 days).
const KEEP_MS: u64 = 35 * 24 * 3_600_000;
/// Readings further apart than this are not one unchanged run.
const MAX_GAP_MS: u64 = 30 * 60_000;
/// Reset deadlines are rounded by the service; within this they are the same cycle.
const RESET_TOLERANCE_MS: u64 = 1_000;
pub const SAMPLE_EVERY: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    /// Percentage of the window used.
    pub utilization: f64,
    /// Reset deadline (ms) of the cycle, when known.
    pub resets_at: Option<u64>,
    /// First and last observation (ms).
    pub from: u64,
    pub to: u64,
    pub samples: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    version: u32,
    /// Window id (`five_hour`, `seven_day`, …) → runs, oldest first.
    pub windows: BTreeMap<String, Vec<Run>>,
}

impl Default for History {
    fn default() -> Self { History { version: VERSION, windows: BTreeMap::new() } }
}

fn reset_ms(text: &str) -> Option<u64> {
    let t = time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).ok()?;
    u64::try_from(t.unix_timestamp_nanos() / 1_000_000).ok()
}

fn same_cycle(a: Option<u64>, b: Option<u64>) -> bool {
    match (a, b) { (Some(a), Some(b)) => a.abs_diff(b) <= RESET_TOLERANCE_MS, (None, None) => true, _ => false }
}

impl History {
    fn add(&mut self, quota: &crate::quota::Quota) {
        let at = quota.fetched_at;
        for w in &quota.windows {
            let runs = self.windows.entry(w.id.clone()).or_default();
            let resets_at = w.resets_at.as_deref().and_then(reset_ms);
            match runs.last_mut() {
                Some(last) if at <= last.to => {} // an older or repeated reading never replaces a newer one
                Some(last) if (last.utilization - w.utilization).abs() < 0.05 && same_cycle(last.resets_at, resets_at) && at - last.to <= MAX_GAP_MS => {
                    last.to = at;
                    last.samples += 1;
                }
                _ => runs.push(Run { utilization: w.utilization, resets_at, from: at, to: at, samples: 1 }),
            }
        }
    }

    fn prune(&mut self, now: u64) {
        for runs in self.windows.values_mut() {
            // Keep the last run anyway: it is the baseline for the next reading.
            let keep_from = runs.iter().position(|r| now.saturating_sub(r.to) <= KEEP_MS).unwrap_or(runs.len().saturating_sub(1));
            runs.drain(..keep_from);
        }
    }
}

fn dir() -> PathBuf { crate::paths::storage_root().join("quota-history") }

fn path(id: &str) -> PathBuf {
    // Account ids are folder names or `ctr-…`; keep the file name portable regardless.
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let hash = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(id.as_bytes()));
    dir().join(format!("{safe}-{}.json", &hash[..8]))
}

fn load(id: &str) -> Result<History, String> {
    match std::fs::read(path(id)) {
        Ok(bytes) => serde_json::from_slice::<History>(&bytes).map_err(|e| e.to_string())
            .and_then(|h| if h.version > VERSION { Err(format!("version {} is newer than this build", h.version)) } else { Ok(h) }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(History::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// Records a successful reading. An unreadable history file is left untouched (and logged).
pub fn record(id: &str, quota: &crate::quota::Quota) {
    let result = load(id).and_then(|mut h| {
        h.add(quota);
        h.prune(crate::paths::now_millis());
        let p = path(id);
        std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&h).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
    });
    if let Err(e) = result { crate::paths::append_log("hydra-error.log", &format!("quota history {id}: {e}")); }
}

pub fn read(id: &str) -> Result<History, String> {
    load(id).map_err(|e| format!("额度历史无法读取：{e}"))
}

pub fn delete(id: &str) { let _ = std::fs::remove_file(path(id)); }

fn sampled(id: &str) -> bool { path(id).is_file() }

/// Host process: re-reads the quota of accounts that have a history, every `SAMPLE_EVERY`.
/// `notify` receives each new reading (account id, quota).
pub fn start_sampler(notify: impl Fn(&str, &crate::quota::Quota) + Send + 'static) {
    let _ = std::thread::Builder::new().name("quota-sampler".into()).spawn(move || loop {
        std::thread::sleep(SAMPLE_EVERY);
        let s = crate::settings::load();
        if s.quota_sampling_off { continue; }
        for (id, acc) in &s.accounts {
            if acc.blocked.is_some() || !sampled(id) { continue; }
            let reading = if acc.container.is_some() {
                crate::quota::fetch_container(id)
            } else if cfg!(target_os = "macos") {
                continue;
            } else {
                crate::install::resolve().and_then(|i| {
                    let p = crate::profiles::resolve(&i, id, false).ok_or("账号不存在")?;
                    crate::quota::fetch(&i, &p)
                })
            };
            // A failed reading is not a measurement; the next round tries again.
            if let Ok(q) = reading {
                record(id, &q);
                notify(id, &q);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::{Quota, Window};

    fn quota(at: u64, five_hour: f64, resets: &str) -> Quota {
        Quota { fetched_at: at, windows: vec![Window { id: "five_hour".into(), utilization: five_hour, resets_at: Some(resets.into()) }] }
    }

    const R1: &str = "2026-11-13T22:40:00Z";
    const R2: &str = "2026-11-14T03:40:00Z";
    const MIN: u64 = 60_000;

    #[test]
    fn unchanged_readings_extend_one_run() {
        // Arrange
        let mut h = History::default();

        // Act
        for i in 0..3 { h.add(&quota(i * 10 * MIN, 20.0, R1)); }

        // Assert
        let runs = &h.windows["five_hour"];
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].from, runs[0].to, runs[0].samples), (0, 20 * MIN, 3));
    }

    #[test]
    fn change_reset_or_gap_starts_a_new_run() {
        // Arrange
        let mut h = History::default();
        h.add(&quota(0, 20.0, R1));

        // Act
        h.add(&quota(10 * MIN, 25.0, R1)); // used more
        h.add(&quota(20 * MIN, 0.0, R2)); // new cycle
        h.add(&quota(20 * MIN + MAX_GAP_MS + 1, 0.0, R2)); // after a sampling gap

        // Assert
        assert_eq!(h.windows["five_hour"].len(), 4);
    }

    #[test]
    fn rounded_reset_deadline_is_the_same_cycle() {
        // Arrange
        let mut h = History::default();
        h.add(&quota(0, 20.0, "2026-11-13T22:40:00.000Z"));

        // Act
        h.add(&quota(10 * MIN, 20.0, "2026-11-13T22:40:00.400Z"));

        // Assert
        assert_eq!(h.windows["five_hour"].len(), 1);
    }

    #[test]
    fn older_readings_never_replace_newer_ones() {
        // Arrange
        let mut h = History::default();
        h.add(&quota(20 * MIN, 30.0, R1));

        // Act
        h.add(&quota(10 * MIN, 10.0, R1));

        // Assert
        assert_eq!(h.windows["five_hour"], vec![Run { utilization: 30.0, resets_at: reset_ms(R1), from: 20 * MIN, to: 20 * MIN, samples: 1 }]);
    }

    #[test]
    fn prune_drops_old_runs_but_keeps_the_last_baseline() {
        // Arrange
        let mut h = History::default();
        h.add(&quota(0, 10.0, R1));
        h.add(&quota(10 * MIN, 20.0, R1));
        let mut only_old = h.clone();

        // Act
        h.prune(KEEP_MS + 5 * MIN);
        only_old.prune(KEEP_MS * 3);

        // Assert
        assert_eq!(h.windows["five_hour"].len(), 1);
        assert_eq!(h.windows["five_hour"][0].utilization, 20.0);
        assert_eq!(only_old.windows["five_hour"].len(), 1, "the last run stays as the baseline");
    }

    #[test]
    fn history_round_trips_through_json() {
        // Arrange
        let mut h = History::default();
        h.add(&quota(0, 12.5, R1));

        // Act
        let back: History = serde_json::from_slice(&serde_json::to_vec(&h).unwrap()).unwrap();

        // Assert
        assert_eq!(back, h);
    }

    #[test]
    fn file_names_are_portable_and_distinct() {
        // Arrange
        let ids = ["Account 2", "Account_2", "ctr-18f3", "工作"];

        // Act
        let names: Vec<String> = ids.iter().map(|id| path(id).file_name().unwrap().to_string_lossy().to_string()).collect();

        // Assert
        assert!(names.iter().all(|n| n.is_ascii() && !n.contains(' ')));
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
    }
}
