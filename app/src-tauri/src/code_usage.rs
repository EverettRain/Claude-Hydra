//! Token usage and account details of a Claude Code data folder (`CLAUDE_CONFIG_DIR`).
//!
//! Everything here is read locally from the folder; nothing goes over the network and no
//! credential leaves this module (only the plan and display identity do).
//!
//! Usage comes from the transcripts (`projects/**.jsonl`, `transcripts/**.jsonl`): every
//! assistant response carries its `usage`. A response is written as several lines (one per
//! content block) and resumed sessions copy earlier responses into a new file, so responses
//! are counted once per `message.id` + `requestId` across all files. Parsed files are cached
//! by size and modification time. Costs are API-equivalent estimates (see `pricing`).
//!
//! Claude Code deletes old transcripts (30 days by default), so each account keeps a history
//! archive in the style of token-monitor's daily history: per UTC hour (the UI groups hours
//! into local days, so no per-timezone calendars are needed) one observation per model.
//! A rescan replaces an observation only when it has at least as many tokens (so prices are
//! refreshed); a smaller or missing one means its source was deleted and the archive keeps
//! the recorded value.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
/// After a response the session counts as active until the next one, for at most this long.
const IDLE_MS: u64 = 5 * MINUTE_MS;
const ARCHIVE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

impl Tokens {
    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_creation += o.cache_creation;
        self.cache_read += o.cache_read;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HourTokens {
    /// Start of the UTC hour (ms); the UI groups hours into local days.
    pub start: u64,
    pub tokens: Tokens,
    /// API-equivalent USD of the priced responses in this hour.
    pub cost: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTokens {
    pub model: String,
    pub tokens: Tokens,
    /// `None` when the model has no known price.
    pub cost: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub hours: Vec<HourTokens>,
    pub models: Vec<ModelTokens>,
    pub total: Tokens,
    pub responses: u64,
    /// API-equivalent USD of all priced responses.
    pub cost: f64,
    /// Tokens of responses from models without a known price (not in `cost`).
    pub unpriced: u64,
    /// Time spent in sessions, to the minute: each response's minute and the time until the
    /// next response (at most `IDLE_MS`).
    pub active_ms: u64,
}

#[derive(Clone, Debug)]
struct Entry {
    /// `message.id:requestId`; `None` when the record has neither (counted as is).
    key: Option<String>,
    at: u64,
    model: String,
    tokens: Tokens,
    cost: Option<f64>,
}

fn timestamp_ms(v: &Value) -> Option<u64> {
    let t = time::OffsetDateTime::parse(v.as_str()?, &time::format_description::well_known::Rfc3339).ok()?;
    u64::try_from(t.unix_timestamp_nanos() / 1_000_000).ok()
}

fn entry(line: &str) -> Option<Entry> {
    // Most lines are tool output or prompts; skip them before parsing.
    if !line.contains("\"usage\"") { return None; }
    let row: Value = serde_json::from_str(line).ok()?;
    if row["type"] != "assistant" { return None; }
    let message = &row["message"];
    let usage = message.get("usage")?;
    let model = message["model"].as_str().unwrap_or("").to_string();
    // Claude Code's own placeholder for locally generated errors, not an API response.
    if model == "<synthetic>" { return None; }
    let n = |k: &str| usage[k].as_u64().unwrap_or(0);
    let tokens = Tokens { input: n("input_tokens"), output: n("output_tokens"), cache_creation: n("cache_creation_input_tokens"), cache_read: n("cache_read_input_tokens") };
    if tokens == Tokens::default() { return None; }
    let id = message["id"].as_str().unwrap_or("");
    let request = row["requestId"].as_str().unwrap_or("");
    let key = (!id.is_empty() || !request.is_empty()).then(|| format!("{id}:{request}"));
    let write_1h = usage["cache_creation"]["ephemeral_1h_input_tokens"].as_u64().unwrap_or(0);
    let cost = crate::pricing::cost(&model, &tokens, write_1h, usage["speed"] == "fast");
    Some(Entry { key, at: timestamp_ms(&row["timestamp"])?, model, tokens, cost })
}

fn parse(reader: impl BufRead) -> Vec<Entry> {
    reader.lines().map_while(Result::ok).filter_map(|l| entry(&l)).collect()
}

fn transcripts(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(kind) = e.file_type() else { continue };
        // Links could leave the folder or loop; Claude Code does not create any here.
        if kind.is_symlink() { continue; }
        let path = e.path();
        if kind.is_dir() && depth < 6 { transcripts(&path, out, depth + 1); }
        else if kind.is_file() && path.extension().is_some_and(|x| x == "jsonl") { out.push(path); }
    }
}

type Cached = (u64, Option<SystemTime>, Vec<Entry>);

fn cache() -> &'static Mutex<HashMap<PathBuf, Cached>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, Cached>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn file_entries(path: &Path) -> Vec<Entry> {
    let Ok(meta) = std::fs::metadata(path) else { return Vec::new() };
    let stamp = (meta.len(), meta.modified().ok());
    if let Some((len, modified, entries)) = cache().lock().ok().and_then(|c| c.get(path).cloned()) {
        if (len, modified) == stamp { return entries; }
    }
    let entries = std::fs::File::open(path).map(|f| parse(BufReader::new(f))).unwrap_or_default();
    if let Ok(mut c) = cache().lock() { c.insert(path.to_path_buf(), (stamp.0, stamp.1, entries.clone())); }
    entries
}

/// One model's usage within an hour (token-monitor's "observation").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    tokens: Tokens,
    /// `None` for models without a known price.
    cost: Option<f64>,
    responses: u64,
}

impl Observation {
    fn total(&self) -> u64 { self.tokens.input + self.tokens.output + self.tokens.cache_creation + self.tokens.cache_read }

    fn add(&mut self, tokens: &Tokens, cost: Option<f64>, responses: u64) {
        let first = self.responses == 0;
        self.tokens.add(tokens);
        self.cost = match (first, self.cost, cost) { (true, _, c) => c, (false, Some(a), Some(b)) => Some(a + b), _ => None };
        self.responses += responses;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hour {
    /// Bit `n` set: minute `n` of the hour was active. Unions (archive, several accounts)
    /// are bitwise ors, so overlapping sessions count once.
    active_minutes: u64,
    models: BTreeMap<String, Observation>,
}

/// Usage hour by hour; persisted per account as its history archive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    version: u32,
    /// Start of the UTC hour (ms) → usage.
    hours: BTreeMap<u64, Hour>,
}

impl Default for Ledger {
    fn default() -> Self { Ledger { version: ARCHIVE_VERSION, hours: BTreeMap::new() } }
}

fn mark_active(hours: &mut BTreeMap<u64, Hour>, from: u64, until: u64) {
    let mut minute = from / MINUTE_MS;
    loop {
        let at = minute * MINUTE_MS;
        hours.entry(at / HOUR_MS * HOUR_MS).or_default().active_minutes |= 1 << ((at % HOUR_MS) / MINUTE_MS);
        minute += 1;
        if minute * MINUTE_MS >= until { break; }
    }
}

fn ledger<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Ledger {
    let mut seen = HashSet::new();
    let mut unique: Vec<&Entry> = entries.into_iter().filter(|e| e.key.as_ref().is_none_or(|k| seen.insert(k.as_str()))).collect();
    unique.sort_by_key(|e| e.at);
    let mut out = Ledger::default();
    for (i, e) in unique.iter().enumerate() {
        let model = if e.model.is_empty() { "unknown".to_string() } else { e.model.clone() };
        out.hours.entry(e.at / HOUR_MS * HOUR_MS).or_default().models.entry(model).or_default().add(&e.tokens, e.cost, 1);
        let next = unique.get(i + 1).map(|n| n.at).unwrap_or(e.at);
        mark_active(&mut out.hours, e.at, e.at + (next - e.at).min(IDLE_MS));
    }
    out
}

/// The archive updated with a fresh scan (see the module notes).
fn retain(mut archived: Ledger, fresh: &Ledger) -> Ledger {
    archived.version = ARCHIVE_VERSION;
    for (start, hour) in &fresh.hours {
        let kept = archived.hours.entry(*start).or_default();
        kept.active_minutes |= hour.active_minutes;
        for (model, obs) in &hour.models {
            if kept.models.get(model).is_none_or(|old| obs.total() >= old.total()) {
                kept.models.insert(model.clone(), obs.clone());
            }
        }
    }
    archived
}

fn combine<'a>(ledgers: impl IntoIterator<Item = &'a Ledger>) -> Ledger {
    let mut out = Ledger::default();
    for l in ledgers {
        for (start, hour) in &l.hours {
            let sum = out.hours.entry(*start).or_default();
            sum.active_minutes |= hour.active_minutes;
            for (model, obs) in &hour.models {
                sum.models.entry(model.clone()).or_default().add(&obs.tokens, obs.cost, obs.responses);
            }
        }
    }
    out
}

fn usage(l: &Ledger) -> Usage {
    let mut usage = Usage::default();
    let mut models: BTreeMap<&str, Observation> = BTreeMap::new();
    for (start, hour) in &l.hours {
        let mut tokens = Tokens::default();
        let mut cost = 0.0;
        for (model, obs) in &hour.models {
            tokens.add(&obs.tokens);
            cost += obs.cost.unwrap_or(0.0);
            match obs.cost { Some(c) => usage.cost += c, None => usage.unpriced += obs.total() }
            usage.responses += obs.responses;
            models.entry(model).or_default().add(&obs.tokens, obs.cost, obs.responses);
        }
        usage.total.add(&tokens);
        usage.active_ms += u64::from(hour.active_minutes.count_ones()) * MINUTE_MS;
        if tokens != Tokens::default() { usage.hours.push(HourTokens { start: *start, tokens, cost }); }
    }
    usage.models = models.into_iter().map(|(model, o)| ModelTokens { model: model.to_string(), tokens: o.tokens, cost: o.cost }).collect();
    usage.models.sort_by_key(|m| std::cmp::Reverse(m.tokens.input + m.tokens.output + m.tokens.cache_creation + m.tokens.cache_read));
    usage
}

fn dir_entries(dir: &Path) -> Vec<Entry> {
    let mut files = Vec::new();
    for lane in ["projects", "transcripts"] { transcripts(&dir.join(lane), &mut files, 0); }
    files.sort();
    files.iter().flat_map(|f| file_entries(f)).collect()
}

/// Reads an archive; `Ok(None)` when there is none yet.
fn read_archive(path: &Path) -> Result<Option<Ledger>, String> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let ledger: Ledger = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if ledger.version > ARCHIVE_VERSION { return Err(format!("archive version {} is newer than this build", ledger.version)); }
    Ok(Some(ledger))
}

fn write_archive(path: &Path, ledger: &Ledger) -> Result<(), String> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(ledger).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// A data folder's usage merged into its history archive, which is updated when it changed.
/// An unreadable archive is left untouched (and logged); the fresh scan is used alone.
fn collect(dir: &Path, archive: &Path) -> Ledger {
    let fresh = ledger(&dir_entries(dir));
    let (merged, previous) = match read_archive(archive) {
        Ok(previous) => (previous.clone().map(|a| retain(a, &fresh)).unwrap_or_else(|| fresh.clone()), previous),
        Err(e) => {
            crate::paths::append_log("hydra-error.log", &format!("usage archive {}: {e}", archive.display()));
            return fresh;
        }
    };
    let changed = previous.as_ref() != Some(&merged) && !(previous.is_none() && merged.hours.is_empty());
    if changed {
        if let Err(e) = write_archive(archive, &merged) {
            crate::paths::append_log("hydra-error.log", &format!("usage archive {}: {e}", archive.display()));
        }
    }
    merged
}

/// Usage of several accounts (data folder, history archive), each and combined.
pub fn scan_all(sources: &[(PathBuf, PathBuf)]) -> (Vec<Usage>, Usage) {
    let ledgers: Vec<Ledger> = sources.iter().map(|(dir, archive)| collect(dir, archive)).collect();
    (ledgers.iter().map(usage).collect(), usage(&combine(&ledgers)))
}

/// Who is signed in, from what Claude Code records about the account in `.claude.json`. The
/// credentials file is never opened: a third-party app may not collect Claude sign-in tokens.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub signed_in: bool,
    /// The plan as Claude names it, e.g. "Max".
    pub plan: Option<String>,
    pub email: Option<String>,
    pub organization: Option<String>,
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn account(dir: &Path) -> Account {
    let text = |v: &Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    // With CLAUDE_CONFIG_DIR, `.claude.json` lives inside the folder.
    let profile = read_json(&dir.join(".claude.json")).map(|v| v["oauthAccount"].clone()).unwrap_or(Value::Null);
    Account {
        signed_in: text(&profile["accountUuid"]).or_else(|| text(&profile["emailAddress"])).is_some(),
        plan: text(&profile["planDisplayName"]),
        email: text(&profile["emailAddress"]),
        organization: text(&profile["organizationName"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, request: &str, at: &str, model: &str, input: u64, output: u64) -> String {
        serde_json::json!({
            "type": "assistant", "requestId": request, "timestamp": at,
            "message": { "id": id, "model": model, "usage": { "input_tokens": input, "output_tokens": output, "cache_creation_input_tokens": 10, "cache_read_input_tokens": 100 } }
        }).to_string()
    }

    #[test]
    fn responses_count_once_across_lines_and_files() {
        // Arrange
        let first = [line("m1", "r1", "2026-11-13T01:10:00Z", "claude-opus-5-5", 5, 50), line("m1", "r1", "2026-11-13T01:10:01Z", "claude-opus-5-5", 5, 50)].join("\n");
        let resumed = [line("m1", "r1", "2026-11-13T01:10:00Z", "claude-opus-5-5", 5, 50), line("m2", "r2", "2026-11-13T03:00:00Z", "claude-sonnet-5-5", 1, 2)].join("\n");

        // Act
        let entries: Vec<Entry> = [first, resumed].iter().flat_map(|f| parse(f.as_bytes())).collect();
        let usage = usage(&ledger(&entries));

        // Assert
        assert_eq!(usage.responses, 2);
        assert_eq!(usage.total, Tokens { input: 6, output: 52, cache_creation: 20, cache_read: 200 });
        assert_eq!(usage.hours.len(), 2);
        assert_eq!(usage.models[0].model, "claude-opus-5-5");
    }

    #[test]
    fn active_time_caps_idle_gaps_and_overlaps_count_once() {
        // Arrange: 2 minutes until the next response, then a long break (capped), then one more.
        let a = [line("a1", "1", "2026-11-13T01:00:00Z", "claude-opus-5-5", 1, 1), line("a2", "2", "2026-11-13T01:02:00Z", "claude-opus-5-5", 1, 1), line("a3", "3", "2026-11-13T03:00:00Z", "claude-opus-5-5", 1, 1)].join("\n");
        let b = line("b1", "1", "2026-11-13T01:01:30Z", "claude-opus-5-5", 1, 1);
        let (la, lb) = (ledger(&parse(a.as_bytes())), ledger(&parse(b.as_bytes())));

        // Act
        let own = usage(&la);
        let both = usage(&combine([&la, &lb]));

        // Assert
        assert_eq!(own.active_ms, 2 * MINUTE_MS + IDLE_MS + MINUTE_MS);
        assert_eq!(both.active_ms, own.active_ms);
    }

    #[test]
    fn unpriced_models_are_kept_out_of_the_cost() {
        // Arrange
        let text = [line("p", "1", "2026-11-13T01:00:00Z", "claude-opus-5-5", 1000, 1000), line("u", "2", "2026-11-13T01:00:00Z", "claude-sonnet-4-5", 1000, 1000)].join("\n");

        // Act
        let usage = usage(&ledger(&parse(text.as_bytes())));

        // Assert
        assert!(usage.cost > 0.0);
        assert_eq!(usage.unpriced, 1000 + 1000 + 10 + 100);
        assert!(usage.models.iter().any(|m| m.model == "claude-sonnet-4-5" && m.cost.is_none()));
    }

    #[test]
    fn non_usage_records_are_ignored() {
        // Arrange
        let text = [
            r#"{"type":"user","message":{"content":"hi \"usage\""}}"#.to_string(),
            line("m3", "r3", "2026-11-13T01:00:00Z", "<synthetic>", 9, 9),
            "not json \"usage\"".to_string(),
            line("m4", "r4", "not a time", "claude-opus-5-5", 1, 1),
        ].join("\n");

        // Act
        let entries = parse(text.as_bytes());

        // Assert
        assert!(entries.is_empty());
    }

    #[test]
    fn hours_are_utc_hour_starts() {
        // Arrange
        let text = line("m5", "r5", "2026-11-13T01:59:59.900Z", "claude-opus-5-5", 1, 1);

        // Act
        let e = &parse(text.as_bytes())[0];

        // Assert
        let usage = usage(&ledger([e]));
        assert_eq!(usage.hours[0].start, timestamp_ms(&Value::from("2026-11-13T01:00:00Z")).unwrap());
    }

    #[test]
    fn account_comes_from_the_profile_never_the_credentials() {
        // Arrange
        let dir = std::env::temp_dir().join(format!("hydra-code-usage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".credentials.json"), r#"{"claudeAiOauth":{"accessToken":"sk-secret","subscriptionType":"pro"}}"#).unwrap();
        std::fs::write(dir.join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"u-1","emailAddress":"a@example.com","organizationName":"Org","planDisplayName":"Max"}}"#).unwrap();
        let empty = dir.join("signed-out");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join(".credentials.json"), r#"{"claudeAiOauth":{"accessToken":"sk-other"}}"#).unwrap();

        // Act
        let a = account(&dir);
        let none = account(&empty);

        // Assert
        assert!(a.signed_in);
        assert_eq!((a.plan.as_deref(), a.email.as_deref(), a.organization.as_deref()), (Some("Max"), Some("a@example.com"), Some("Org")));
        assert!(!none.signed_in, "a credentials file alone is not read");
        assert!(!serde_json::to_string(&a).unwrap().contains("sk-"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_walks_project_and_subagent_folders() {
        // Arrange
        let dir = std::env::temp_dir().join(format!("hydra-code-scan-{}", std::process::id()));
        let sub = dir.join("projects/-workspace-app/session/subagents");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("projects/-workspace-app/session.jsonl"), line("a", "1", "2026-11-13T01:00:00Z", "claude-opus-5-5", 1, 1)).unwrap();
        std::fs::write(sub.join("agent.jsonl"), line("b", "2", "2026-11-13T01:00:00Z", "claude-opus-5-5", 1, 1)).unwrap();

        // Act
        let (each, total) = scan_all(&[(dir.clone(), dir.join("history.json"))]);

        // Assert
        assert_eq!((each[0].responses, total.responses), (2, 2));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hydra-{name}-{}-{}", std::process::id(), crate::paths::now_millis()));
        std::fs::create_dir_all(dir.join("projects/p")).unwrap();
        dir
    }

    #[test]
    fn archive_keeps_usage_whose_transcripts_were_deleted_or_shrank() {
        // Arrange
        let dir = temp("archive-retain");
        let archive = dir.join("history.json");
        let file = dir.join("projects/p/s.jsonl");
        std::fs::write(&file, [line("a", "1", "2026-10-01T01:00:00Z", "claude-opus-5-5", 100, 100), line("b", "2", "2026-11-13T01:00:00Z", "claude-opus-5-5", 5, 5)].join("\n")).unwrap();
        let first = usage(&collect(&dir, &archive));
        // Claude Code cleaned the old part of the transcript and recorded a new response.
        std::fs::write(&file, [line("b", "2", "2026-11-13T01:00:00Z", "claude-opus-5-5", 5, 5), line("c", "3", "2026-11-13T01:30:00Z", "claude-opus-5-5", 7, 7)].join("\n")).unwrap();

        // Act
        let second = usage(&collect(&dir, &archive));

        // Assert
        assert_eq!(first.responses, 2);
        assert_eq!(second.hours.len(), 2, "the October hour survives its deleted transcript");
        assert_eq!(second.total.input, 100 + 5 + 7);
        assert_eq!(second.responses, 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rescan_with_fewer_tokens_keeps_the_recorded_observation() {
        // Arrange
        let mut archived = Ledger::default();
        archived.hours.entry(0).or_default().models.insert("m".into(), Observation { tokens: Tokens { input: 10, ..Default::default() }, cost: Some(1.0), responses: 2 });
        let mut fresh = Ledger::default();
        fresh.hours.entry(0).or_default().models.insert("m".into(), Observation { tokens: Tokens { input: 4, ..Default::default() }, cost: Some(0.4), responses: 1 });
        let mut repriced = archived.clone();
        repriced.hours.get_mut(&0).unwrap().models.get_mut("m").unwrap().cost = Some(2.0);

        // Act
        let kept = retain(archived.clone(), &fresh);
        let updated = retain(archived, &repriced);

        // Assert
        assert_eq!(kept.hours[&0].models["m"].responses, 2);
        assert_eq!(updated.hours[&0].models["m"].cost, Some(2.0));
    }

    #[test]
    fn unreadable_archive_is_not_overwritten() {
        // Arrange
        let dir = temp("archive-corrupt");
        let archive = dir.join("history.json");
        std::fs::write(&archive, "{not json").unwrap();
        std::fs::write(dir.join("projects/p/s.jsonl"), line("a", "1", "2026-11-13T01:00:00Z", "claude-opus-5-5", 1, 1)).unwrap();

        // Act
        let fresh = usage(&collect(&dir, &archive));

        // Assert
        assert_eq!(fresh.responses, 1);
        assert_eq!(std::fs::read_to_string(&archive).unwrap(), "{not json");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn archive_round_trips_through_its_file() {
        // Arrange
        let dir = temp("archive-roundtrip");
        let archive = dir.join("history.json");
        std::fs::write(dir.join("projects/p/s.jsonl"), [line("a", "1", "2026-11-13T01:00:00Z", "claude-opus-5-5", 1, 1), line("b", "2", "2026-11-13T01:01:00Z", "claude-sonnet-4-5", 1, 1)].join("\n")).unwrap();

        // Act
        let written = collect(&dir, &archive);
        let read = read_archive(&archive).unwrap().unwrap();

        // Assert
        assert_eq!(read, written);
        assert_eq!(read.version, ARCHIVE_VERSION);
        let _ = std::fs::remove_dir_all(dir);
    }
}
