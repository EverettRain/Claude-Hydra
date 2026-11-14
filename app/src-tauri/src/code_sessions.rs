//! Claude Code sessions of a data folder: listing, reading, the recycle bin, moving them
//! between folders, and export / import.
//!
//! A session is `projects/<encoded working folder>/<id>.jsonl`, plus an optional folder of the
//! same name beside it (sub-agents, large tool results) and `file-history/<id>/` (checkpoints
//! for rewinding file edits). All three travel together.
use crate::claude_folders::{Folder, Paths};
use crate::claude_trash::{self, plain};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// `<project folder>/<session id>`.
    pub id: String,
    pub title: String,
    /// The working folder the session started in.
    pub cwd: Option<String>,
    pub last_activity_at: Option<u64>,
    pub turns: u64,
    pub model: Option<String>,
    pub size: u64,
}

/// Claude Code may be appending to a session written this recently.
const BUSY: Duration = Duration::from_secs(30);
/// Longest project folder name Claude Code writes without a hash suffix we cannot reproduce.
const MAX_PROJECT_NAME: usize = 200;

struct Files { project: String, session: String }

impl Files {
    fn parse(id: &str) -> Result<Files, String> {
        let (project, session) = id.split_once('/').ok_or("会话标识无效。")?;
        if !plain(project) || !plain(session) || session.contains('/') { return Err("会话标识无效。".into()); }
        Ok(Files { project: project.into(), session: session.into() })
    }
    fn transcript(&self) -> String { format!("projects/{}/{}.jsonl", self.project, self.session) }
    fn sidecar(&self) -> String { format!("projects/{}/{}", self.project, self.session) }
    fn history(&self) -> String { format!("file-history/{}", self.session) }
    fn all(&self) -> Vec<String> { vec![self.transcript(), self.sidecar(), self.history()] }
}

fn timestamp(v: &Value) -> Option<u64> {
    let date = time::OffsetDateTime::parse(v.as_str()?, &time::format_description::well_known::Rfc3339).ok()?;
    u64::try_from(date.unix_timestamp_nanos() / 1_000_000).ok()
}

/// The text a user typed, if this record is a prompt (not a tool result or injected context).
fn prompt_text(row: &Value) -> Option<String> {
    if row["type"] != "user" || row["isMeta"] == true || row["isCompactSummary"] == true { return None; }
    let content = &row["message"]["content"];
    if let Some(s) = content.as_str() { return Some(s.to_string()); }
    let blocks = content.as_array()?;
    if blocks.iter().any(|b| b["type"] == "tool_result") { return None; }
    let text: Vec<&str> = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
    (!text.is_empty()).then(|| text.join("\n"))
}

/// Slash commands and their output are stored as tagged text; they are not questions.
fn is_command(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<command-") || t.starts_with("<local-command-")
}

fn inner<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let start = text.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = text[start..].find(&format!("</{tag}>"))? + start;
    Some(&text[start..end])
}

/// A slash command or its output as a person reads it: `/resume`, not the tags around it.
fn command_text(text: &str) -> String {
    if let Some(name) = inner(text, "command-name") {
        let args = inner(text, "command-args").map(str::trim).filter(|a| !a.is_empty());
        return args.map(|a| format!("{name} {a}")).unwrap_or_else(|| name.to_string());
    }
    ["local-command-stdout", "local-command-stderr"].iter().filter_map(|t| inner(text, t)).collect::<Vec<_>>().join("\n").trim().to_string()
}

fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max { flat } else { flat.chars().take(max).collect::<String>() + "…" }
}

/// The fields a summary needs. Everything else, above all large tool output, is skipped while
/// parsing instead of being built into values: listing reads every transcript in full.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    is_sidechain: bool,
    #[serde(default)]
    is_meta: bool,
    #[serde(default)]
    is_compact_summary: bool,
    cwd: Option<String>,
    timestamp: Option<String>,
    uuid: Option<String>,
    message: Option<Message>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    summary: Option<String>,
}

#[derive(Deserialize)]
struct Message { model: Option<String>, content: Option<Content> }

#[derive(Deserialize)]
#[serde(untagged)]
enum Content { Text(String), Blocks(Vec<Block>) }

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

impl Row {
    /// The text a user typed, if this record is a prompt (not a tool result or injected context).
    fn prompt(&self) -> Option<String> {
        if self.kind.as_deref() != Some("user") || self.is_meta || self.is_compact_summary { return None; }
        match self.message.as_ref()?.content.as_ref()? {
            Content::Text(s) => Some(s.clone()),
            Content::Blocks(blocks) => {
                if blocks.iter().any(|b| b.kind.as_deref() == Some("tool_result")) { return None; }
                let text: Vec<&str> = blocks.iter().filter(|b| b.kind.as_deref() == Some("text")).filter_map(|b| b.text.as_deref()).collect();
                (!text.is_empty()).then(|| text.join("\n"))
            }
        }
    }
}

/// What reading a transcript up to `offset` found. Claude Code only appends to a transcript, so a
/// grown file is read on from `offset` instead of from the start.
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct Scan {
    offset: u64,
    /// Digest of the bytes just before `offset`: a file rewritten rather than appended to is read again.
    check: String,
    custom_title: Option<String>,
    ai_title: Option<String>,
    summary: Option<String>,
    first_prompt: Option<String>,
    cwd: Option<String>,
    time: Option<u64>,
    turns: u64,
    model: Option<String>,
    messages: bool,
    /// Prompts already counted: a resumed session can repeat earlier records.
    prompts: std::collections::BTreeSet<String>,
}

const CHECK_LEN: u64 = 64;

fn check_at(file: &mut fs::File, offset: u64) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::{Seek, SeekFrom};
    let start = offset.saturating_sub(CHECK_LEN);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; (offset - start) as usize];
    file.read_exact(&mut bytes)?;
    Ok(Sha256::digest(&bytes).iter().take(8).map(|b| format!("{b:02x}")).collect())
}

impl Scan {
    fn apply(&mut self, row: Row) {
        match row.kind.as_deref() {
            Some("custom-title") => self.custom_title = row.custom_title.clone().or(self.custom_title.take()),
            Some("ai-title") => self.ai_title = row.ai_title.clone().or(self.ai_title.take()),
            Some("summary") => self.summary = row.summary.clone().or(self.summary.take()),
            _ => {}
        }
        if row.is_sidechain { return; }
        if self.cwd.is_none() { self.cwd = row.cwd.clone(); }
        let kind = row.kind.as_deref();
        if kind != Some("user") && kind != Some("assistant") { return; }
        self.messages = true;
        self.time = self.time.max(row.timestamp.as_ref().and_then(|t| timestamp(&Value::String(t.clone()))));
        if kind == Some("assistant") {
            if let Some(m) = row.message.as_ref().and_then(|m| m.model.as_deref()).filter(|m| !m.starts_with('<')) { self.model = Some(m.to_owned()); }
        } else if let Some(text) = row.prompt().filter(|t| !t.trim().is_empty() && !is_command(t)) {
            if row.uuid.as_ref().is_none_or(|u| self.prompts.insert(u.clone())) { self.turns += 1; }
            if self.first_prompt.is_none() { self.first_prompt = Some(one_line(&text, 120)); }
        }
    }

    /// Reads the complete lines after `offset`; a line still being written waits for the next time.
    fn advance(&mut self, path: &Path) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom};
        let mut file = fs::File::open(path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = reader.read_until(b'\n', &mut line)?;
            if n == 0 || line.last() != Some(&b'\n') { break; }
            self.offset += n as u64;
            if let Ok(row) = serde_json::from_slice::<Row>(&line) { self.apply(row); }
        }
        self.check = check_at(reader.get_mut(), self.offset)?;
        Ok(())
    }

    fn session(&self, id: String, size: u64) -> Option<Session> {
        if !self.messages { return None; }
        let title = [&self.custom_title, &self.ai_title, &self.summary, &self.first_prompt].into_iter().flatten()
            .find(|t| !t.trim().is_empty()).map(|t| one_line(t, 120)).unwrap_or_else(|| "（没有标题）".into());
        Some(Session { id, title, cwd: self.cwd.clone(), last_activity_at: self.time, turns: self.turns, model: self.model.clone(), size })
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Indexed { modified: u64, len: u64, scan: Scan }

/// Summaries of every transcript read so far, kept on disk so a restart does not read them again.
#[derive(Serialize, Deserialize, Default)]
struct Index { version: u32, files: HashMap<String, Indexed> }

const INDEX_VERSION: u32 = 1;

fn index_path() -> PathBuf {
    #[cfg(test)]
    let base = std::env::temp_dir().join(format!("hydra-test-data-{}", std::process::id()));
    #[cfg(not(test))]
    let base = crate::paths::storage_root();
    base.join("claude-session-index.json")
}

fn index() -> &'static Mutex<Index> {
    static INDEX: OnceLock<Mutex<Index>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let saved = fs::read(index_path()).ok().and_then(|b| serde_json::from_slice::<Index>(&b).ok());
        Mutex::new(saved.filter(|i| i.version == INDEX_VERSION).unwrap_or(Index { version: INDEX_VERSION, files: HashMap::new() }))
    })
}

fn save_index(index: &Index) {
    let path = index_path();
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec(index).map_err(|e| e.to_string())
        .and_then(|b| { if let Some(p) = path.parent() { let _ = fs::create_dir_all(p); } fs::write(&tmp, b).map_err(|e| e.to_string()) })
        .and_then(|_| fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = written { crate::paths::append_log("hydra-error.log", &format!("session index: {e}")); }
}

fn modified_ns(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

/// A transcript's summary from the index, reading only what changed since it was indexed.
/// Returns whether the index entry changed.
fn indexed(path: &Path, id: String) -> (Option<Session>, bool) {
    let Ok(meta) = fs::metadata(path) else { return (None, false) };
    let (modified, len) = (modified_ns(&meta), meta.len());
    let key = path.to_string_lossy().into_owned();
    let known = index().lock().ok().and_then(|i| i.files.get(&key).cloned());
    if let Some(k) = &known {
        if k.modified == modified && k.len == len { return (k.scan.session(id, len), false); }
    }
    // Continue from where the last read stopped if the file only grew; otherwise start over.
    let mut scan = known.filter(|k| len >= k.scan.offset && fs::File::open(path).and_then(|mut f| check_at(&mut f, k.scan.offset)).is_ok_and(|c| c == k.scan.check))
        .map(|k| k.scan).unwrap_or_default();
    if scan.advance(path).is_err() { return (None, false); }
    let session = scan.session(id, len);
    if let Ok(mut i) = index().lock() { i.files.insert(key, Indexed { modified, len, scan }); }
    (session, true)
}

fn list_dir(dir: &Path) -> Vec<Session> {
    let mut out = Vec::new();
    let mut changed = false;
    for project in fs::read_dir(dir.join("projects")).into_iter().flatten().flatten() {
        if !project.file_type().is_ok_and(|t| t.is_dir()) { continue; }
        let pname = project.file_name().to_string_lossy().into_owned();
        for file in fs::read_dir(project.path()).into_iter().flatten().flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".jsonl") else { continue };
            if !file.file_type().is_ok_and(|t| t.is_file()) { continue; }
            let (session, updated) = indexed(&file.path(), format!("{pname}/{stem}"));
            changed |= updated;
            out.extend(session);
        }
    }
    if changed {
        if let Ok(mut i) = index().lock() {
            // Deleted, moved and unpacked-then-removed transcripts leave the index here.
            i.files.retain(|path, _| Path::new(path).is_file());
            save_index(&i);
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.last_activity_at));
    out
}

pub fn list(f: &Folder) -> Vec<Session> { list_dir(&f.dir) }

// ---- reading ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// `user`, `assistant`, `thinking`, `tool`, `result`, `command`, `compact` or `system`.
    kind: &'static str,
    text: String,
    /// Tool name of a `tool` item.
    name: Option<String>,
    time: Option<u64>,
    /// Characters left out of a long text.
    omitted: usize,
    error: bool,
}

#[derive(Serialize)]
pub struct Transcript { items: Vec<Item>, total: usize }

const TEXT_LIMIT: usize = 20_000;
const TOOL_LIMIT: usize = 4_000;

fn clip(text: String, max: usize) -> (String, usize) {
    let count = text.chars().count();
    if count <= max { (text, 0) } else { (text.chars().take(max).collect(), count - max) }
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks.iter().map(|b| match b["type"].as_str() {
            Some("text") => b["text"].as_str().unwrap_or_default().to_string(),
            Some("image") => "［图片］".into(),
            _ => String::new(),
        }).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

fn items_of(row: &Value) -> Vec<Item> {
    let time = timestamp(&row["timestamp"]);
    let item = |kind, text: String, name: Option<String>, error: bool| {
        let text = if matches!(kind, "tool" | "result") { text } else { text.trim().to_string() };
        let (text, omitted) = clip(text, if matches!(kind, "tool" | "result") { TOOL_LIMIT } else { TEXT_LIMIT });
        Item { kind, text, name, time, omitted, error }
    };
    let command = |text: &str| Some(command_text(text)).filter(|t| !t.is_empty()).map(|t| vec![item("command", t, None, false)]).unwrap_or_default();
    if row["isSidechain"] == true || row["isMeta"] == true { return Vec::new(); }
    match row["type"].as_str() {
        Some("user") => {
            if row["isCompactSummary"] == true { return vec![item("compact", result_text(&row["message"]["content"]), None, false)]; }
            if let Some(text) = prompt_text(row) {
                return if is_command(&text) { command(&text) } else { vec![item("user", text, None, false)] };
            }
            row["message"]["content"].as_array().into_iter().flatten()
                .filter(|b| b["type"] == "tool_result")
                .map(|b| item("result", result_text(&b["content"]), None, b["is_error"] == true)).collect()
        }
        Some("assistant") => row["message"]["content"].as_array().into_iter().flatten().filter_map(|b| match b["type"].as_str() {
            Some("text") => b["text"].as_str().filter(|t| !t.trim().is_empty()).map(|t| item("assistant", t.into(), None, false)),
            Some("thinking") => b["thinking"].as_str().filter(|t| !t.trim().is_empty()).map(|t| item("thinking", t.into(), None, false)),
            Some("tool_use") => Some(item("tool", serde_json::to_string_pretty(&b["input"]).unwrap_or_default(), b["name"].as_str().map(str::to_owned), false)),
            _ => None,
        }).collect(),
        Some("system") if row["subtype"] == "compact_boundary" => vec![item("compact", "上下文已压缩".into(), None, false)],
        Some("system") if row["subtype"] == "local_command" => command(row["content"].as_str().unwrap_or_default()),
        // Connection notices of Remote Control repeat on every reconnect and say nothing about the work.
        Some("system") if row["subtype"] == "bridge_status" => Vec::new(),
        Some("system") => row["content"].as_str().filter(|t| !t.trim().is_empty())
            .map(|t| vec![item("system", t.into(), None, row["level"] == "error")]).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// `limit` items of a session's conversation, starting at item `offset`.
pub fn read(f: &Folder, id: &str, offset: usize, limit: usize) -> Result<Transcript, String> {
    let files = Files::parse(id)?;
    let file = fs::File::open(f.dir.join(files.transcript())).map_err(|_| "找不到这个会话，请刷新。")?;
    let mut items = Vec::new();
    let mut total = 0;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        let Ok(row) = serde_json::from_str::<Value>(&line) else { continue };
        for it in items_of(&row) {
            if total >= offset && items.len() < limit { items.push(it); }
            total += 1;
        }
    }
    Ok(Transcript { items, total })
}

// ---- recycle bin ----

fn busy(path: &Path) -> bool {
    fs::metadata(path).and_then(|m| m.modified()).ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < BUSY)
}

fn check_idle(f: &Folder, files: &Files) -> Result<(), String> {
    if busy(&f.dir.join(files.transcript())) {
        return Err("这个会话 30 秒内还有写入，可能正在使用。请先结束这个会话，再删除或移动。".into());
    }
    Ok(())
}

pub fn trash(f: &Folder, ids: &[String]) -> Result<usize, String> {
    let sessions = list(f);
    let mut done = 0;
    for id in ids {
        let files = Files::parse(id)?;
        check_idle(f, &files)?;
        let s = sessions.iter().find(|s| &s.id == id);
        claude_trash::put(f, "session", s.map(|s| s.title.as_str()).unwrap_or(&files.session), s.and_then(|s| s.cwd.clone()), &files.all())?;
        done += 1;
    }
    Ok(done)
}

// ---- moving between folders ----

/// Where sessions come from: a data folder, or an unpacked export.
pub struct Side<'a> { pub dir: &'a Path, pub paths: &'a Paths }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    /// A working folder of the selected sessions, as the source wrote it.
    cwd: String,
    sessions: usize,
    /// The same folder as the target writes it, when Hydra can tell.
    target: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    roots: Vec<Root>,
    /// Folders offered for a working folder the target cannot see (a container's shared folders).
    choices: Vec<String>,
    /// The target uses paths of this computer, picked with a folder dialog.
    host_target: bool,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Outcome { pub copied: usize, pub skipped: usize, pub failed: usize, pub errors: Vec<String> }

fn selected(src: &Side, ids: &[String]) -> Vec<Session> {
    list_dir(src.dir).into_iter().filter(|s| ids.contains(&s.id)).collect()
}

pub fn plan(src: &Side, ids: &[String], dst: &Folder) -> Plan {
    let mut roots: BTreeMap<String, usize> = BTreeMap::new();
    for s in selected(src, ids) { if let Some(c) = s.cwd { *roots.entry(c).or_default() += 1; } }
    Plan {
        roots: roots.into_iter().map(|(cwd, sessions)| Root { target: src.paths.translate(&cwd, &dst.paths), cwd, sessions }).collect(),
        choices: dst.paths.choices(),
        host_target: dst.paths == Paths::Host,
    }
}

/// The project folder name Claude Code uses for a working folder: every character other than an
/// ASCII letter or digit (each UTF-16 unit of it, as JavaScript counts) becomes `-`.
pub fn project_name(cwd: &str) -> String {
    cwd.chars().flat_map(|c| {
        let n = if c.is_ascii_alphanumeric() { 0 } else { c.len_utf16() };
        std::iter::repeat_n('-', n).chain((n == 0).then_some(c))
    }).collect()
}

/// `path` with its leading `from` replaced by `to`, when it is `from` or inside it.
fn rebase(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from { return Some(to.to_string()); }
    let rest = path.strip_prefix(from)?;
    let sep = rest.chars().next()?;
    if sep != '/' && sep != '\\' { return None; }
    // A container path is POSIX even when the session came from Windows.
    Some(if to.starts_with('/') { format!("{to}{}", rest.replace('\\', "/")) } else { format!("{to}{rest}") })
}

/// A transcript line with its `cwd` moved from `from` to `to`; other lines and fields are kept
/// byte for byte, since altering messages can invalidate their thinking signatures.
fn rewrite_line(line: &str, from: &str, to: &str) -> String {
    if from == to || !line.contains("\"cwd\"") { return line.to_string(); }
    let Ok(mut row) = serde_json::from_str::<Value>(line) else { return line.to_string() };
    let Some(Value::String(cwd)) = row.get_mut("cwd") else { return line.to_string() };
    let Some(moved) = rebase(cwd, from, to) else { return line.to_string() };
    *cwd = moved;
    serde_json::to_string(&row).unwrap_or_else(|_| line.to_string())
}

fn rewrite_file(from_path: &Path, from: &str, to: &str) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    for line in BufReader::new(fs::File::open(from_path)?).split(b'\n') {
        let line = line?;
        match std::str::from_utf8(&line) {
            Ok(text) => out.extend_from_slice(rewrite_line(text, from, to).as_bytes()),
            Err(_) => out.extend_from_slice(&line),
        }
        out.push(b'\n');
    }
    Ok(out)
}

/// Copies files missing from `to`, moving the working folder in transcripts; nothing is replaced.
fn copy_missing(from: &Path, to: &Path, cwd: (&str, &str)) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.is_dir() {
        for e in fs::read_dir(from)? {
            let e = e?;
            copy_missing(&e.path(), &to.join(e.file_name()), cwd)?;
        }
    } else if meta.is_file() && !to.exists() {
        if let Some(p) = to.parent() { fs::create_dir_all(p)?; }
        if from.extension().is_some_and(|x| x == "jsonl") { fs::write(to, rewrite_file(from, cwd.0, cwd.1)?)?; } else { fs::copy(from, to)?; }
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<bool> {
    if let Some(p) = path.parent() { fs::create_dir_all(p)?; }
    let mut file = match fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    };
    if let Err(e) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(e);
    }
    Ok(true)
}

fn copy_session(src: &Side, s: &Session, dst: &Folder, target_cwd: &str) -> Result<bool, String> {
    let files = Files::parse(&s.id)?;
    let cwd = s.cwd.as_deref().ok_or("没有记录工作目录")?;
    let project = project_name(target_cwd);
    if project.len() > MAX_PROJECT_NAME { return Err("目标工作目录的路径太长".into()); }
    let target = Files { project, session: files.session.clone() };
    if dst.dir.join(target.transcript()).exists() { return Ok(false); }
    // The transcript goes last, so a session only appears once everything it refers to is there.
    for (from, to) in [(files.sidecar(), target.sidecar()), (files.history(), target.history())] {
        if src.dir.join(&from).exists() {
            copy_missing(&src.dir.join(&from), &dst.dir.join(&to), (cwd, target_cwd)).map_err(|e| e.to_string())?;
        }
    }
    let bytes = rewrite_file(&src.dir.join(files.transcript()), cwd, target_cwd).map_err(|e| e.to_string())?;
    write_new(&dst.dir.join(target.transcript()), &bytes).map_err(|e| e.to_string())
}

/// Copies sessions into `dst`, each working folder moved as `mapping` says. With `remove`, a
/// copied (or already present) session is then moved to the source folder's recycle bin.
pub fn transfer(src: &Side, ids: &[String], dst: &Folder, mapping: &HashMap<String, String>, remove: Option<&Folder>) -> Result<Outcome, String> {
    if src.dir == dst.dir { return Err("来源和目标是同一个数据目录。".into()); }
    let sessions = selected(src, ids);
    if let Some(f) = remove {
        for s in &sessions { check_idle(f, &Files::parse(&s.id)?)?; }
    }
    let mut out = Outcome::default();
    for s in &sessions {
        let target = s.cwd.as_ref().and_then(|c| mapping.get(c)).filter(|t| dst.paths.accepts(t));
        let result = match target {
            Some(t) => copy_session(src, s, dst, t),
            None => Err("没有指定目标工作目录".into()),
        };
        match result {
            Ok(copied) => {
                if copied { out.copied += 1 } else { out.skipped += 1 }
                if let Some(f) = remove {
                    if let Err(e) = claude_trash::put(f, "session", &s.title, s.cwd.clone(), &Files::parse(&s.id)?.all()) {
                        out.errors.push(format!("「{}」已复制，但没能从来源移除：{e}", s.title));
                    }
                }
            }
            Err(e) => { out.failed += 1; out.errors.push(format!("「{}」：{e}", s.title)); }
        }
    }
    out.failed += ids.len().saturating_sub(sessions.len());
    Ok(out)
}

// ---- export / import ----

const EXPORT_FORMAT: &str = "claude-hydra-sessions";
const MANIFEST: &str = "hydra-export.json";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: String,
    version: u32,
    source: String,
    /// The source container's shared folders `(name, Mac folder)`; absent for paths of a Mac.
    mounts: Option<Vec<(String, String)>>,
    exported_at: u64,
}

fn add_tree(zip: &mut zip::ZipWriter<fs::File>, dir: &Path, rel: &str) -> Result<(), String> {
    let path = dir.join(rel);
    let Ok(meta) = fs::symlink_metadata(&path) else { return Ok(()) };
    if meta.is_dir() {
        for e in fs::read_dir(&path).map_err(|e| e.to_string())?.flatten() {
            add_tree(zip, dir, &format!("{rel}/{}", e.file_name().to_string_lossy()))?;
        }
    } else if meta.is_file() {
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(meta.len() > u32::MAX as u64);
        zip.start_file(rel, options).map_err(|e| e.to_string())?;
        std::io::copy(&mut fs::File::open(&path).map_err(|e| e.to_string())?, zip).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Writes the sessions to a zip laid out like a data folder, plus a manifest.
pub fn export(f: &Folder, ids: &[String], dest: &Path) -> Result<usize, String> {
    if ids.is_empty() { return Err("请选择会话。".into()); }
    let tmp = dest.with_extension("zip.part");
    let mut zip = zip::ZipWriter::new(fs::File::create(&tmp).map_err(|e| format!("无法创建文件：{e}"))?);
    let manifest = Manifest {
        format: EXPORT_FORMAT.into(), version: 1, source: f.name.clone(), exported_at: crate::paths::now_millis(),
        mounts: match &f.paths { Paths::Host => None, Paths::Container(m) => Some(m.iter().map(|(n, s)| (n.clone(), s.to_string_lossy().into_owned())).collect()) },
    };
    let result = (|| {
        zip.start_file(MANIFEST, zip::write::SimpleFileOptions::default()).map_err(|e| e.to_string())?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        for id in ids { for rel in Files::parse(id)?.all() { add_tree(&mut zip, &f.dir, &rel)?; } }
        zip.finish().map_err(|e| e.to_string())?;
        fs::rename(&tmp, dest).map_err(|e| e.to_string())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(format!("导出失败：{e}"));
    }
    Ok(ids.len())
}

/// An export unpacked into a scratch folder, removed when dropped.
pub struct Unpacked { pub dir: PathBuf, pub paths: Paths, pub source: String }

impl Drop for Unpacked {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.dir); }
}

pub fn unpack(file: &Path) -> Result<Unpacked, String> {
    let mut archive = zip::ZipArchive::new(fs::File::open(file).map_err(|e| format!("无法打开文件：{e}"))?)
        .map_err(|_| "这不是 Hydra 导出的会话文件（zip）。")?;
    let manifest: Manifest = {
        let mut m = archive.by_name(MANIFEST).map_err(|_| "这不是 Hydra 导出的会话文件：缺少 hydra-export.json。")?;
        let mut raw = Vec::new();
        m.read_to_end(&mut raw).map_err(|e| e.to_string())?;
        serde_json::from_slice(&raw).map_err(|_| "导出文件的清单无效。")?
    };
    if manifest.format != EXPORT_FORMAT || manifest.version != 1 { return Err("不支持这个导出文件的版本。".into()); }
    let unpacked = Unpacked {
        dir: scratch().join(format!("import-{}", uuid::Uuid::new_v4().simple())),
        paths: manifest.mounts.map(|m| Paths::Container(m.into_iter().map(|(n, s)| (n, PathBuf::from(s))).collect())).unwrap_or(Paths::Host),
        source: manifest.source,
    };
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() { continue; }
        let Some(rel) = entry.enclosed_name() else { continue };
        let rel_text = rel.to_string_lossy().replace('\\', "/");
        if !(rel_text.starts_with("projects/") || rel_text.starts_with("file-history/")) || !plain(&rel_text) { continue; }
        let to = unpacked.dir.join(&rel_text);
        if let Some(p) = to.parent() { fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        std::io::copy(&mut entry, &mut fs::File::create(&to).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    Ok(unpacked)
}

fn scratch() -> PathBuf {
    #[cfg(test)]
    return std::env::temp_dir().join(format!("hydra-test-data-{}", std::process::id()));
    #[cfg(not(test))]
    crate::paths::storage_root().join("tmp")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview { source: String, sessions: Vec<Session>, plan: Plan }

pub fn import_preview(file: &Path, dst: &Folder) -> Result<ImportPreview, String> {
    let u = unpack(file)?;
    let sessions = list_dir(&u.dir);
    let ids: Vec<String> = sessions.iter().map(|s| s.id.clone()).collect();
    let plan = plan(&Side { dir: &u.dir, paths: &u.paths }, &ids, dst);
    Ok(ImportPreview { source: u.source.clone(), sessions, plan })
}

pub fn import(file: &Path, dst: &Folder, mapping: &HashMap<String, String>) -> Result<Outcome, String> {
    let u = unpack(file)?;
    let ids: Vec<String> = list_dir(&u.dir).into_iter().map(|s| s.id).collect();
    if ids.is_empty() { return Err("导出文件里没有会话。".into()); }
    transfer(&Side { dir: &u.dir, paths: &u.paths }, &ids, dst, mapping, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(dir: &Path, paths: Paths) -> Folder {
        Folder { id: "t".into(), kind: "local", name: "t".into(), path: String::new(), shared_with_local: false, profile: None,
            global_config: dir.join(".claude.json"), dir: dir.to_path_buf(), paths }
    }

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("hydra-sessions-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    const SESSION: &str = "11111111-2222-3333-4444-555555555555";

    /// An absolute folder of this computer: tests run on Windows CI too.
    fn host_root() -> String {
        if cfg!(windows) { r"C:\Users\u\app".into() } else { "/Users/u/app".into() }
    }

    fn sub(dir: &str, name: &str) -> String {
        format!("{dir}{}{name}", if dir.starts_with('/') { '/' } else { '\\' })
    }

    fn write_session(dir: &Path, cwd: &str) -> String {
        use serde_json::json;
        let project = project_name(cwd);
        let lines = [
            json!({"type":"user","cwd":cwd,"uuid":"a","timestamp":"2026-01-01T00:00:00Z","message":{"role":"user","content":"修一下登录"}}),
            json!({"type":"assistant","cwd":sub(cwd, "src"),"uuid":"b","timestamp":"2026-01-01T00:00:05Z","message":{"model":"claude-opus-5-5","content":[
                {"type":"thinking","thinking":"t","signature":"sig"},{"type":"tool_use","name":"Read","input":{"file_path":sub(cwd, "a.rs")}}]}}),
            json!({"type":"user","uuid":"c","message":{"content":[{"type":"tool_result","content":"ok","is_error":false}]}}),
            json!({"type":"ai-title","aiTitle":"登录修复"}),
        ].map(|v| v.to_string());
        fs::create_dir_all(dir.join("projects").join(&project).join(SESSION).join("subagents")).unwrap();
        fs::write(dir.join("projects").join(&project).join(format!("{SESSION}.jsonl")), lines.join("\n") + "\n").unwrap();
        fs::write(dir.join("projects").join(&project).join(SESSION).join("subagents/agent-1.jsonl"), json!({"cwd":cwd}).to_string() + "\n").unwrap();
        fs::create_dir_all(dir.join("file-history").join(SESSION)).unwrap();
        fs::write(dir.join("file-history").join(SESSION).join("x@v1"), "old").unwrap();
        format!("{project}/{SESSION}")
    }

    #[test]
    fn project_names_follow_claude_code() {
        // Arrange
        let cases = [("/Users/u/my app", "-Users-u-my-app"), ("/workspace/文档", "-workspace---"), ("/a/😀", "-a---")];

        // Act
        let names: Vec<String> = cases.iter().map(|(c, _)| project_name(c)).collect();

        // Assert
        assert_eq!(names, cases.iter().map(|(_, n)| n.to_string()).collect::<Vec<_>>());
    }

    #[test]
    fn summary_uses_the_title_and_counts_prompts() {
        // Arrange
        let dir = temp();
        let id = write_session(&dir, &host_root());

        // Act
        let sessions = list_dir(&dir);

        // Assert
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, id);
        assert_eq!(sessions[0].title, "登录修复");
        assert_eq!(sessions[0].cwd, Some(host_root()));
        assert_eq!(sessions[0].turns, 1);
        assert_eq!(sessions[0].model.as_deref(), Some("claude-opus-5-5"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn appended_lines_are_read_on_and_rewritten_files_again() {
        // Arrange
        let dir = temp();
        let id = write_session(&dir, &host_root());
        let file = dir.join("projects").join(id.replace('/', std::path::MAIN_SEPARATOR_STR) + ".jsonl");
        let before = list_dir(&dir);
        let prompt = |text: &str, uuid: &str| serde_json::json!({"type":"user","uuid":uuid,"timestamp":"2026-01-02T00:00:00Z","message":{"content":text}}).to_string();

        // Act
        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{}", prompt("再试一次", "d")).unwrap();
        write!(f, "{{\"type\":\"user\",\"partial").unwrap();
        drop(f);
        let appended = list_dir(&dir);
        fs::write(&file, prompt("重写后的第一句", "z") + "\n").unwrap();
        let rewritten = list_dir(&dir);

        // Assert
        assert_eq!((before[0].turns, appended[0].turns), (1, 2));
        assert_eq!(appended[0].title, "登录修复");
        assert_eq!((rewritten[0].turns, rewritten[0].title.as_str()), (1, "重写后的第一句"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn reading_lists_the_conversation_in_order() {
        // Arrange
        let dir = temp();
        let id = write_session(&dir, &host_root());
        let f = folder(&dir, Paths::Host);

        // Act
        let t = read(&f, &id, 0, 100).unwrap();
        let page = read(&f, &id, 1, 1).unwrap();

        // Assert
        let kinds: Vec<&str> = t.items.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, ["user", "thinking", "tool", "result"]);
        assert_eq!(t.items[2].name.as_deref(), Some("Read"));
        assert_eq!((page.total, page.items.len(), page.items[0].kind), (4, 1, "thinking"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn commands_show_without_their_tags_and_empty_output_is_dropped() {
        // Arrange
        let rows = [
            r#"{"type":"user","message":{"content":"<command-name>/rename</command-name>\n<command-message>rename</command-message>\n<command-args>登录修复</command-args>"}}"#,
            r#"{"type":"system","subtype":"local_command","content":"<local-command-stdout></local-command-stdout>"}"#,
            r#"{"type":"system","subtype":"local_command","content":"<local-command-stdout>Session renamed</local-command-stdout>"}"#,
            r#"{"type":"system","subtype":"bridge_status","content":"/remote-control is active"}"#,
        ];

        // Act
        let items: Vec<Item> = rows.iter().flat_map(|r| items_of(&serde_json::from_str(r).unwrap())).collect();

        // Assert
        let shown: Vec<(&str, &str)> = items.iter().map(|i| (i.kind, i.text.as_str())).collect();
        assert_eq!(shown, [("command", "/rename 登录修复"), ("command", "Session renamed")]);
    }

    #[test]
    fn transfer_moves_the_working_folder_but_keeps_messages() {
        // Arrange
        let (a, b) = (temp(), temp());
        let id = write_session(&a, &host_root());
        let src = folder(&a, Paths::Host);
        let dst = folder(&b, Paths::Container(vec![("app".into(), host_root().into())]));
        let side = Side { dir: &src.dir, paths: &src.paths };

        // Act
        let plan = plan(&side, std::slice::from_ref(&id), &dst);
        let mapping = HashMap::from([(host_root(), plan.roots[0].target.clone().unwrap())]);
        let first = transfer(&side, std::slice::from_ref(&id), &dst, &mapping, None).unwrap();
        let again = transfer(&side, std::slice::from_ref(&id), &dst, &mapping, None).unwrap();

        // Assert
        assert_eq!(plan.roots[0].target.as_deref(), Some("/workspace/app"));
        assert_eq!((first.copied, again.skipped), (1, 1));
        let copied = fs::read_to_string(b.join("projects/-workspace-app").join(format!("{SESSION}.jsonl"))).unwrap();
        assert!(copied.contains(r#""cwd":"/workspace/app""#) && copied.contains(r#""cwd":"/workspace/app/src""#));
        assert!(copied.contains(&format!(r#""file_path":{}"#, serde_json::json!(sub(&host_root(), "a.rs")))), "message content is not rewritten");
        assert!(copied.contains(r#""signature":"sig""#));
        let agent = fs::read_to_string(b.join("projects/-workspace-app").join(SESSION).join("subagents/agent-1.jsonl")).unwrap();
        assert!(agent.contains("/workspace/app"));
        assert!(b.join("file-history").join(SESSION).join("x@v1").is_file());
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }

    #[test]
    fn unmapped_sessions_fail_and_moving_needs_an_idle_session() {
        // Arrange
        let (a, b) = (temp(), temp());
        let id = write_session(&a, &host_root());
        let src = folder(&a, Paths::Host);
        let dst = folder(&b, Paths::Container(Vec::new()));
        let side = Side { dir: &src.dir, paths: &src.paths };
        let outside = HashMap::from([(host_root(), host_root())]);

        // Act
        let unmapped = transfer(&side, std::slice::from_ref(&id), &dst, &HashMap::new(), None).unwrap();
        let wrong_side = transfer(&side, std::slice::from_ref(&id), &dst, &outside, None).unwrap();
        let moving = transfer(&side, std::slice::from_ref(&id), &dst, &outside, Some(&src));

        // Assert
        assert_eq!((unmapped.failed, wrong_side.failed), (1, 1));
        assert!(moving.is_err(), "a session written just now is not moved");
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }

    #[test]
    fn export_and_import_round_trip_with_the_source_paths() {
        // Arrange
        let (a, b) = (temp(), temp());
        let id = write_session(&a, "/workspace/app");
        let src = folder(&a, Paths::Container(vec![("app".into(), host_root().into())]));
        let dst = folder(&b, Paths::Host);
        let file = a.join("out.zip");

        // Act
        export(&src, std::slice::from_ref(&id), &file).unwrap();
        let preview = import_preview(&file, &dst).unwrap();
        let mapping = HashMap::from([("/workspace/app".to_string(), preview.plan.roots[0].target.clone().unwrap())]);
        let result = import(&file, &dst, &mapping).unwrap();

        // Assert
        assert_eq!(preview.sessions.len(), 1);
        assert_eq!(preview.plan.roots[0].target, Some(host_root()));
        assert_eq!(result.copied, 1);
        assert!(b.join("projects").join(project_name(&host_root())).join(format!("{SESSION}.jsonl")).is_file());
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }
}
