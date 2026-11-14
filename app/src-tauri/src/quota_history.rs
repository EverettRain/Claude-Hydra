//! Remaining-quota history per container account, in the style of token-monitor's limits history.
//!
//! The readings come from the container's own Claude Code: it hands the plan's limits to its
//! status line, and `hydra-statusline` appends them to `LOG` in the Claude data folder (on a
//! change, else at most every few minutes). Hydra replays that log into the history whenever it
//! is read, so readings taken while Hydra was closed still count. No credentials are involved.
//!
//! Consecutive unchanged readings of the same cycle form one run (first and last observation,
//! sample count); a change, a new cycle (reset deadline) or a gap starts the next run. The UI
//! turns runs into the cycle's trend and the daily consumption in local time
//! (see `src/lib/quotaHistory.ts`).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Readings `hydra-statusline` appends, one JSON object per line: `{"at": ms, "rateLimits": {…}}`
/// with the status line's `rate_limits` as Claude Code passed it.
pub const LOG: &str = ".hydra-rate-limits.jsonl";

const VERSION: u32 = 1;
/// Runs older than this are dropped (the UI shows the current cycle and 14 days).
const KEEP_MS: u64 = 35 * 24 * 3_600_000;
/// Readings further apart than this are not one unchanged run.
const MAX_GAP_MS: u64 = 30 * 60_000;
/// Reset deadlines are rounded by the service; within this they are the same cycle.
const RESET_TOLERANCE_MS: u64 = 1_000;

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

/// One window of a reading: share used (0-100) and reset deadline (ms).
#[derive(Clone, Debug, PartialEq)]
pub struct Window { pub id: String, pub used: f64, pub resets_at: Option<u64> }

#[derive(Clone, Debug, PartialEq)]
pub struct Reading { pub at: u64, pub windows: Vec<Window> }

/// A log line, or nothing when it is not one (a line cut off mid-write, say).
fn parse(line: &str) -> Option<Reading> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let windows = v["rateLimits"].as_object()?.iter().filter_map(|(id, w)| {
        let used = w["used_percentage"].as_f64().filter(|n| n.is_finite() && *n >= 0.0)?;
        // The status line gets reset deadlines in seconds.
        Some(Window { id: id.clone(), used: used.min(100.0), resets_at: w["resets_at"].as_u64().map(|s| s * 1000) })
    }).collect();
    Some(Reading { at: v["at"].as_u64()?, windows })
}

fn same_cycle(a: Option<u64>, b: Option<u64>) -> bool {
    match (a, b) { (Some(a), Some(b)) => a.abs_diff(b) <= RESET_TOLERANCE_MS, (None, None) => true, _ => false }
}

impl History {
    /// Whether the reading changed anything.
    fn add(&mut self, reading: &Reading) -> bool {
        let at = reading.at;
        let mut changed = false;
        for w in &reading.windows {
            let runs = self.windows.entry(w.id.clone()).or_default();
            match runs.last_mut() {
                Some(last) if at <= last.to => continue, // an older or repeated reading never replaces a newer one
                Some(last) if (last.utilization - w.used).abs() < 0.05 && same_cycle(last.resets_at, w.resets_at) && at - last.to <= MAX_GAP_MS => {
                    last.to = at;
                    last.samples += 1;
                }
                _ => runs.push(Run { utilization: w.used, resets_at: w.resets_at, from: at, to: at, samples: 1 }),
            }
            changed = true;
        }
        changed
    }

    /// The newest reading of each window, with when it was taken.
    pub fn latest(&self, id: &str) -> Option<&Run> { self.windows.get(id)?.last() }

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

fn save(id: &str, h: &History) -> Result<(), String> {
    let p = path(id);
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(h).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

/// The account's history, with the readings its status line logged since the last time.
pub fn sync(id: &str, claude: &Path) -> Result<History, String> {
    let mut h = load(id).map_err(|e| format!("额度历史无法读取：{e}"))?;
    let text = match std::fs::read(claude.join(LOG)) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(h),
        Err(e) => return Err(format!("额度记录无法读取：{e}")),
    };
    let mut readings: Vec<Reading> = text.lines().filter_map(parse).collect();
    readings.sort_by_key(|r| r.at);
    let mut changed = false;
    for r in &readings { changed |= h.add(r); }
    if changed {
        h.prune(crate::paths::now_millis());
        // Not saving only means the same lines are replayed next time.
        if let Err(e) = save(id, &h) { crate::paths::append_log("hydra-error.log", &format!("quota history {id}: {e}")); }
    }
    Ok(h)
}

pub fn delete(id: &str) { let _ = std::fs::remove_file(path(id)); }

#[cfg(test)]
mod tests {
    use super::*;

    fn quota(at: u64, five_hour: f64, resets: u64) -> Reading {
        Reading { at, windows: vec![Window { id: "five_hour".into(), used: five_hour, resets_at: Some(resets) }] }
    }

    const R1: u64 = 1_794_700_000_000;
    const R2: u64 = R1 + 5 * 3_600_000;
    const MIN: u64 = 60_000;

    #[test]
    fn log_lines_become_readings_in_milliseconds() {
        // Arrange
        let line = r#"{"at":1000,"rateLimits":{"five_hour":{"used_percentage":23.5,"resets_at":1794700000},"seven_day":{"used_percentage":120},"bad":{"used_percentage":"x"}}}"#;

        // Act
        let r = parse(line).unwrap();

        // Assert
        assert_eq!(r, Reading { at: 1000, windows: vec![
            Window { id: "five_hour".into(), used: 23.5, resets_at: Some(R1) },
            Window { id: "seven_day".into(), used: 100.0, resets_at: None },
        ] });
        assert!(parse(r#"{"at":1000,"rateLim"#).is_none(), "a line cut off mid-write is skipped");
    }

    #[test]
    fn sync_replays_the_log_once_and_keeps_what_it_learned() {
        // Arrange
        let id = format!("ctr-test-{}", uuid::Uuid::new_v4().simple());
        let claude = std::env::temp_dir().join(&id);
        std::fs::create_dir_all(&claude).unwrap();
        let now = crate::paths::now_millis();
        let line = |at: u64, used: f64| format!(r#"{{"at":{at},"rateLimits":{{"five_hour":{{"used_percentage":{used},"resets_at":{}}}}}}}"#, R1 / 1000);
        std::fs::write(claude.join(LOG), [line(now - 20 * MIN, 10.0), line(now - 10 * MIN, 10.0), line(now, 30.0), "{\"at\":".into()].join("\n")).unwrap();

        // Act
        let first = sync(&id, &claude).unwrap();
        std::fs::remove_file(claude.join(LOG)).unwrap();
        let again = sync(&id, &claude).unwrap();

        // Assert
        let runs = &first.windows["five_hour"];
        assert_eq!(runs.iter().map(|r| (r.utilization, r.samples)).collect::<Vec<_>>(), vec![(10.0, 2), (30.0, 1)]);
        assert_eq!(again, first, "the history outlives the log");
        assert_eq!(first.latest("five_hour").map(|r| r.to), Some(now));
        delete(&id);
        let _ = std::fs::remove_dir_all(claude);
    }
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
        h.add(&quota(0, 20.0, R1));

        // Act
        h.add(&quota(10 * MIN, 20.0, R1 + 400));

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
        assert_eq!(h.windows["five_hour"], vec![Run { utilization: 30.0, resets_at: Some(R1), from: 20 * MIN, to: 20 * MIN, samples: 1 }]);
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
