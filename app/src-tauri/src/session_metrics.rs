use serde_json::Value;
use std::{collections::{HashMap, HashSet}, fs, io::{BufRead, BufReader}, path::{Path, PathBuf}, sync::{Mutex, OnceLock}, time::SystemTime};

#[derive(Clone, Debug, Default)]
pub(super) struct Metrics {
    pub turns: Option<u64>,
    pub model: Option<String>,
    pub time: Option<u64>,
}

fn timestamp(value: &Value) -> Option<u64> {
    if let Some(n) = value.as_u64() { return Some(n); }
    let date = time::OffsetDateTime::parse(value.as_str()?, &time::format_description::well_known::Rfc3339).ok()?;
    u64::try_from(date.unix_timestamp_nanos() / 1_000_000).ok()
}

fn model(value: &Value) -> Option<String> {
    value.as_str().filter(|v| !v.trim().is_empty() && !v.starts_with('<')).map(str::to_owned)
}

pub(super) fn index_metrics(v: &Value) -> Metrics {
    let messages = v["chat_messages"].as_array();
    let latest = messages.into_iter().flatten().filter_map(|m| timestamp(&m["created_at"])).max();
    Metrics {
        turns: messages.map(|rows| rows.iter().filter(|m| m["sender"] == "human" || m["role"] == "user").count() as u64)
            .or_else(|| v["turnCount"].as_u64()),
        model: model(&v["model"]).or_else(|| messages?.iter().rev().find_map(|m| model(&m["model"]))),
        time: ["lastActivityAt", "updated_at", "createdAt", "created_at"].iter().find_map(|key| timestamp(&v[key])).into_iter().chain(latest).max(),
    }
}

fn transcript(reader: impl BufRead) -> Option<Metrics> {
    let mut count = 0;
    let mut seen = HashSet::new();
    let mut metrics = Metrics::default();
    let mut has_message = false;
    for line in reader.lines() {
        let line = line.ok()?;
        if line.trim().is_empty() { continue; }
        let row: Value = serde_json::from_str(&line).ok()?;
        if row["isSidechain"] == true || row["isMeta"] == true { continue; }
        if row["type"] != "assistant" && row["type"] != "user" { continue; }
        has_message = true;
        metrics.time = metrics.time.max(timestamp(&row["timestamp"]));
        if row["type"] == "assistant" {
            if let Some(m) = model(&row["message"]["model"]) { metrics.model = Some(m); }
            continue;
        }
        let content = &row["message"]["content"];
        let prompt = content.as_str().is_some_and(|s| !s.trim().is_empty()) ||
            content.as_array().is_some_and(|blocks| blocks.iter().any(|b| matches!(b["type"].as_str(), Some("text" | "image" | "document")))
                && !blocks.iter().any(|b| b["type"] == "tool_result"));
        if prompt && row["uuid"].as_str().is_none_or(|id| seen.insert(id.to_owned())) { count += 1; }
    }
    if has_message { metrics.turns = Some(count); Some(metrics) } else { None }
}

type Cached = (SystemTime, u64, Metrics);
fn cached_read(path: &Path) -> Option<Metrics> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Cached>>> = OnceLock::new();
    let meta = fs::metadata(path).ok()?;
    let stamp = meta.modified().ok()?;
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some((time, len, result)) = cache.lock().ok()?.get(path) {
        if *time == stamp && *len == meta.len() { return Some(result.clone()); }
    }
    let result = transcript(BufReader::new(fs::File::open(path).ok()?))?;
    let mut cache = cache.lock().ok()?;
    if cache.len() >= 2048 { cache.clear(); }
    cache.insert(path.to_owned(), (stamp, meta.len(), result.clone()));
    Some(result)
}

pub(super) fn lookup(ids: &HashSet<String>) -> HashMap<String, Metrics> {
    if ids.is_empty() { return HashMap::new(); }
    let config = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).or_else(|| {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|p| PathBuf::from(p).join(".claude"))
    });
    let Some(dirs) = config.and_then(|p| fs::read_dir(p.join("projects")).ok()) else { return HashMap::new(); };
    let mut paths: HashMap<String, Option<PathBuf>> = HashMap::new();
    for dir in dirs.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
        let Ok(files) = fs::read_dir(dir.path()) else { continue };
        for file in files.flatten() {
            let name = file.file_name();
            let Some(id) = name.to_str().and_then(|s| s.strip_suffix(".jsonl")) else { continue };
            if ids.contains(id) && file.file_type().is_ok_and(|t| t.is_file()) {
                paths.entry(id.to_owned()).and_modify(|p| *p = None).or_insert_with(|| Some(file.path()));
            }
        }
    }
    paths.into_iter().filter_map(|(id, path)| Some((id, cached_read(&path?)?))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_user_prompts_not_tool_returns_or_duplicates() {
        let log = [
            r#"{"type":"user","uuid":"a","message":{"content":"hello"}}"#,
            r#"{"type":"user","uuid":"a","message":{"content":"hello"}}"#,
            r#"{"type":"assistant","message":{"model":"opus"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result"}]}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"context"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"next"}]}}"#,
        ].join("\n");
        let stats = transcript(log.as_bytes()).unwrap();
        assert_eq!(stats.turns, Some(2));
        assert_eq!(stats.model.as_deref(), Some("opus"));
    }

    #[test]
    fn imported_chat_uses_message_dates_and_sender() {
        let v = serde_json::json!({"chat_messages":[{"sender":"human","created_at":"2026-01-01T00:00:00Z"},{"sender":"assistant","model":"sonnet"}]});
        let stats = index_metrics(&v);
        assert_eq!(stats.turns, Some(1));
        assert_eq!(stats.time, Some(1767225600000));
        assert_eq!(stats.model.as_deref(), Some("sonnet"));
    }
}
