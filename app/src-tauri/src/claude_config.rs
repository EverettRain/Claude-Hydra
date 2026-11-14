//! The configuration in a Claude Code data folder: `CLAUDE.md`, `settings.json`, skills, agents,
//! commands, MCP servers and plugins.
//!
//! Every write re-reads the file just before replacing it and replaces it atomically: Claude Code
//! may be running and rewriting the same files. Login credentials are never read.
use crate::claude_folders::Folder;
use crate::claude_trash::{self, copy_tree, plain};
use serde::Serialize;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

// ---- text files ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFile {
    text: String,
    /// Changes whenever the file does; a save carrying an old stamp is refused.
    stamp: String,
    exists: bool,
}

/// Files that may be read and edited, relative to the data folder.
fn editable(rel: &str) -> bool {
    matches!(rel, "CLAUDE.md" | "settings.json")
        || (plain(rel) && rel.ends_with(".md") && ["skills/", "agents/", "commands/"].iter().any(|p| rel.starts_with(p)) && !rel.starts_with("skills/synced/"))
}

/// A digest of the file's content: a modification time can stay the same across two quick
/// writes of the same length, a digest cannot. "" for a missing file.
fn stamp(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    fs::read(path).ok().map(|b| Sha256::digest(&b).iter().take(16).map(|x| format!("{x:02x}")).collect()).unwrap_or_default()
}

pub fn read_file(f: &Folder, rel: &str) -> Result<TextFile, String> {
    if !editable(rel) { return Err("不能读取这个文件。".into()); }
    let path = f.dir.join(rel);
    match fs::read(&path) {
        Ok(bytes) => Ok(TextFile { text: String::from_utf8_lossy(&bytes).into_owned(), stamp: stamp(&path), exists: true }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TextFile { text: String::new(), stamp: String::new(), exists: false }),
        Err(e) => Err(format!("无法读取 {rel}：{e}")),
    }
}

/// Replaces `path` with `bytes` through a temporary file, keeping the old file's permissions.
fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("路径无效。")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".{}.hydra-{}", path.file_name().and_then(|n| n.to_str()).unwrap_or("file"), uuid::Uuid::new_v4().simple()));
    let result = fs::write(&tmp, bytes).and_then(|_| {
        if let Ok(meta) = fs::metadata(path) { fs::set_permissions(&tmp, meta.permissions())?; }
        fs::rename(&tmp, path)
    });
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(format!("无法保存：{e}"));
    }
    Ok(())
}

/// Saves an edited file unless it changed since it was read; returns the new stamp.
pub fn save_file(f: &Folder, rel: &str, text: &str, read_stamp: &str) -> Result<String, String> {
    if !editable(rel) { return Err("不能修改这个文件。".into()); }
    if rel == "settings.json" && !serde_json::from_str::<Value>(text).is_ok_and(|v| v.is_object()) {
        return Err("settings.json 必须是一个合法的 JSON 对象，没有保存。".into());
    }
    let extension = rel.starts_with("skills/") || rel.starts_with("agents/") || rel.starts_with("commands/");
    if extension && !f.dir.join(rel).is_file() { return Err("这个文件已不存在，请刷新。".into()); }
    let path = f.dir.join(rel);
    if stamp(&path) != read_stamp { return Err("文件在别处被修改过，请重新载入后再编辑。".into()); }
    replace(&path, text.as_bytes())?;
    Ok(stamp(&path))
}

// ---- skills, agents, commands ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    /// `skill`, `agent` or `command`.
    kind: &'static str,
    name: String,
    description: Option<String>,
    /// What is copied or deleted: the skill's folder, or the agent's or command's file.
    path: String,
    /// The Markdown file shown and edited.
    file: String,
}

/// `name` and `description` from a Markdown file's YAML front matter (single-line values).
fn front_matter(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") { return (None, None); }
    let (mut name, mut description) = (None, None);
    for line in lines.take_while(|l| l.trim() != "---") {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'').trim();
        if value.is_empty() || value == ">" || value == "|" { continue; }
        match key.trim() { "name" => name = Some(value.to_string()), "description" => description = Some(value.to_string()), _ => {} }
    }
    (name, description)
}

fn markdown_files(dir: &Path, rel: &str, out: &mut Vec<String>) {
    for e in fs::read_dir(dir.join(rel)).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') { continue; }
        let child = format!("{rel}/{name}");
        match e.file_type() {
            Ok(t) if t.is_dir() => markdown_files(dir, &child, out),
            Ok(t) if t.is_file() && name.ends_with(".md") => out.push(child),
            _ => {}
        }
    }
}

pub fn extensions(f: &Folder) -> Vec<Extension> {
    let mut out = Vec::new();
    let mut skills: Vec<_> = fs::read_dir(f.dir.join("skills")).into_iter().flatten().flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.file_name().to_string_lossy().into_owned())
        // `synced` holds the skills the account syncs from claude.ai; they are managed there.
        .filter(|n| !n.starts_with('.') && n != "synced").collect();
    skills.sort();
    for dir in skills {
        let file = format!("skills/{dir}/SKILL.md");
        let Ok(text) = fs::read_to_string(f.dir.join(&file)) else { continue };
        let (name, description) = front_matter(&text);
        out.push(Extension { kind: "skill", name: name.unwrap_or_else(|| dir.clone()), description, path: format!("skills/{dir}"), file });
    }
    for (kind, root) in [("agent", "agents"), ("command", "commands")] {
        let mut files = Vec::new();
        markdown_files(&f.dir, root, &mut files);
        files.sort();
        for file in files {
            let text = fs::read_to_string(f.dir.join(&file)).unwrap_or_default();
            let (name, description) = front_matter(&text);
            // A command's name is its path: `commands/git/commit.md` is `/git:commit`.
            let fallback = file[root.len() + 1..].trim_end_matches(".md").replace('/', ":");
            let name = if kind == "agent" { name.unwrap_or(fallback) } else { fallback };
            out.push(Extension { kind, name, description, path: file.clone(), file });
        }
    }
    out
}

fn extension_kind(path: &str) -> Option<&'static str> {
    if !plain(path) { return None; }
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["skills", name] if *name != "synced" => Some("skill"),
        ["agents", .., last] if last.ends_with(".md") => Some("agent"),
        ["commands", .., last] if last.ends_with(".md") => Some("command"),
        _ => None,
    }
}

pub fn delete_extension(f: &Folder, path: &str) -> Result<(), String> {
    let kind = extension_kind(path).ok_or("不能删除这一项。")?;
    let name = path.rsplit('/').next().unwrap_or(path).trim_end_matches(".md").to_string();
    claude_trash::put(f, kind, &name, Some(path.to_string()), &[path.to_string()])
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyResult {
    target: String,
    /// `copied`, `exists` (left alone) or `failed`.
    status: &'static str,
    error: Option<String>,
}

fn copy_result(target: &Folder, r: Result<bool, String>) -> CopyResult {
    match r {
        Ok(true) => CopyResult { target: target.name.clone(), status: "copied", error: None },
        Ok(false) => CopyResult { target: target.name.clone(), status: "exists", error: None },
        Err(e) => CopyResult { target: target.name.clone(), status: "failed", error: Some(e) },
    }
}

/// Copies a skill, agent or command into other folders. An existing one is replaced only with
/// `overwrite`, and then goes to that folder's recycle bin first.
pub fn copy_extension(f: &Folder, path: &str, targets: &[Folder], overwrite: bool) -> Result<Vec<CopyResult>, String> {
    let kind = extension_kind(path).ok_or("不能复制这一项。")?;
    let from = f.dir.join(path);
    if !from.exists() { return Err("这一项已不存在，请刷新。".into()); }
    Ok(targets.iter().filter(|t| t.dir != f.dir).map(|t| {
        let to = t.dir.join(path);
        copy_result(t, (|| {
            if fs::symlink_metadata(&to).is_ok() {
                if !overwrite { return Ok(false); }
                let name = path.rsplit('/').next().unwrap_or(path).trim_end_matches(".md");
                claude_trash::put(t, kind, name, Some(path.to_string()), &[path.to_string()])?;
            }
            copy_tree(&from, &to).map_err(|e| e.to_string())?;
            Ok(true)
        })())
    }).collect())
}

// ---- MCP servers ----

fn read_json(path: &Path) -> Result<Map<String, Value>, String> {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(Value::Object(m)) => Ok(m),
            _ => Err(format!("{} 不是合法的 JSON，没有修改。", path.display())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(format!("无法读取 {}：{e}", path.display())),
    }
}

fn write_json(path: &Path, m: &Map<String, Value>) -> Result<(), String> {
    replace(path, &serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    name: String,
    /// The project it is limited to; `None` for a server available everywhere.
    project: Option<String>,
    transport: String,
    /// Command line or URL.
    summary: String,
    /// The configuration, with environment and header values hidden.
    config: Value,
}

const HIDDEN: &str = "••••••";

fn masked(config: &Value) -> Value {
    let mut c = config.clone();
    for key in ["env", "headers"] {
        if let Some(Value::Object(m)) = c.get_mut(key) {
            for v in m.values_mut() { *v = Value::String(HIDDEN.into()); }
        }
    }
    c
}

fn server(name: &str, project: Option<String>, config: &Value) -> McpServer {
    let command = config["command"].as_str();
    let args: Vec<&str> = config["args"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let summary = match (command, config["url"].as_str()) {
        (Some(c), _) => std::iter::once(c).chain(args).collect::<Vec<_>>().join(" "),
        (None, Some(u)) => u.to_string(),
        _ => String::new(),
    };
    let transport = config["type"].as_str().map(str::to_owned).unwrap_or_else(|| if command.is_some() { "stdio" } else { "http" }.into());
    McpServer { name: name.into(), project, transport, summary, config: masked(config) }
}

pub fn mcp_servers(f: &Folder) -> Result<Vec<McpServer>, String> {
    let root = read_json(&f.global_config)?;
    let mut out: Vec<McpServer> = root.get("mcpServers").and_then(Value::as_object).into_iter().flatten()
        .map(|(n, c)| server(n, None, c)).collect();
    for (project, state) in root.get("projects").and_then(Value::as_object).into_iter().flatten() {
        for (n, c) in state.get("mcpServers").and_then(Value::as_object).into_iter().flatten() {
            out.push(server(n, Some(project.clone()), c));
        }
    }
    Ok(out)
}

fn servers_of<'a>(root: &'a mut Map<String, Value>, project: Option<&str>) -> Option<&'a mut Map<String, Value>> {
    let holder = match project {
        None => root,
        Some(p) => root.get_mut("projects")?.as_object_mut()?.get_mut(p)?.as_object_mut()?,
    };
    holder.get_mut("mcpServers")?.as_object_mut()
}

pub fn delete_mcp(f: &Folder, project: Option<&str>, name: &str) -> Result<(), String> {
    let mut root = read_json(&f.global_config)?;
    servers_of(&mut root, project).and_then(|s| s.remove(name)).ok_or("这个 MCP 服务器已不存在，请刷新。")?;
    write_json(&f.global_config, &root)
}

/// Copies a server available everywhere into the other folders' own lists.
pub fn copy_mcp(f: &Folder, name: &str, targets: &[Folder], overwrite: bool) -> Result<Vec<CopyResult>, String> {
    let mut root = read_json(&f.global_config)?;
    let config = servers_of(&mut root, None).and_then(|s| s.get(name)).cloned().ok_or("这个 MCP 服务器已不存在，请刷新。")?;
    Ok(targets.iter().filter(|t| t.global_config != f.global_config).map(|t| copy_result(t, (|| {
        let mut root = read_json(&t.global_config)?;
        let servers = root.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
        let servers = servers.as_object_mut().ok_or("目标的 mcpServers 格式无效")?;
        if servers.contains_key(name) && !overwrite { return Ok(false); }
        servers.insert(name.into(), config.clone());
        write_json(&t.global_config, &root)?;
        Ok(true)
    })())).collect())
}

// ---- plugins ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    /// `name@marketplace`.
    id: String,
    name: String,
    marketplace: Option<String>,
    version: Option<String>,
    enabled: bool,
    installed: bool,
}

fn settings_path(f: &Folder) -> PathBuf { f.dir.join("settings.json") }

pub fn plugins(f: &Folder) -> Vec<Plugin> {
    let installed = read_json(&f.dir.join("plugins/installed_plugins.json")).unwrap_or_default();
    let installed = installed.get("plugins").and_then(Value::as_object).cloned().unwrap_or_default();
    let settings = read_json(&settings_path(f)).unwrap_or_default();
    let enabled = settings.get("enabledPlugins").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut ids: Vec<&String> = installed.keys().chain(enabled.keys()).collect();
    ids.sort();
    ids.dedup();
    ids.into_iter().map(|id| {
        // Version 2 keeps a list of installations per plugin, version 1 a single record.
        let record = installed.get(id).map(|r| r.as_array().and_then(|a| a.first()).unwrap_or(r));
        let (name, marketplace) = id.split_once('@').map(|(n, m)| (n.to_string(), Some(m.to_string()))).unwrap_or((id.clone(), None));
        Plugin {
            id: id.clone(), name, marketplace,
            version: record.and_then(|r| r["version"].as_str()).map(str::to_owned),
            enabled: enabled.get(id).and_then(Value::as_bool).unwrap_or(false),
            installed: record.is_some(),
        }
    }).collect()
}

pub fn set_plugin(f: &Folder, id: &str, enabled: bool) -> Result<(), String> {
    let path = settings_path(f);
    let mut root = read_json(&path)?;
    let map = root.entry("enabledPlugins").or_insert_with(|| Value::Object(Map::new()));
    map.as_object_mut().ok_or("settings.json 的 enabledPlugins 格式无效。")?.insert(id.into(), Value::Bool(enabled));
    write_json(&path, &root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_folders::Paths;

    fn folder(name: &str) -> Folder {
        let dir = std::env::temp_dir().join(format!("hydra-config-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        Folder { id: name.into(), kind: "container", name: name.into(), path: String::new(), shared_with_local: false, profile: None,
            global_config: dir.join(".claude.json"), dir, paths: Paths::Host }
    }

    #[test]
    fn saving_refuses_stale_edits_and_invalid_settings() {
        // Arrange
        let f = folder("a");
        fs::write(f.dir.join("CLAUDE.md"), "old").unwrap();
        let read = read_file(&f, "CLAUDE.md").unwrap();

        // Act
        let saved = save_file(&f, "CLAUDE.md", "new", &read.stamp).unwrap();
        // Same length, right after: only the content tells the two versions apart.
        let stale = save_file(&f, "CLAUDE.md", "odd", &read.stamp);
        let invalid = save_file(&f, "settings.json", "[1]", "");
        let outside = read_file(&f, ".credentials.json");

        // Assert
        assert_eq!(fs::read_to_string(f.dir.join("CLAUDE.md")).unwrap(), "new");
        assert_ne!(saved, read.stamp);
        assert!(stale.is_err() && invalid.is_err() && outside.is_err());
        let _ = fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn extensions_are_listed_with_their_front_matter() {
        // Arrange
        let f = folder("a");
        fs::create_dir_all(f.dir.join("skills/pdf")).unwrap();
        fs::write(f.dir.join("skills/pdf/SKILL.md"), "---\nname: pdf\ndescription: \"读 PDF\"\n---\nbody").unwrap();
        fs::create_dir_all(f.dir.join("skills/synced/x")).unwrap();
        fs::create_dir_all(f.dir.join("commands/git")).unwrap();
        fs::write(f.dir.join("commands/git/commit.md"), "提交").unwrap();
        fs::create_dir_all(f.dir.join("agents")).unwrap();
        fs::write(f.dir.join("agents/review.md"), "---\nname: reviewer\n---\n").unwrap();

        // Act
        let list = extensions(&f);

        // Assert
        let names: Vec<(&str, &str)> = list.iter().map(|e| (e.kind, e.name.as_str())).collect();
        assert_eq!(names, [("skill", "pdf"), ("agent", "reviewer"), ("command", "git:commit")]);
        assert_eq!(list[0].description.as_deref(), Some("读 PDF"));
        let _ = fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn copying_an_extension_keeps_existing_ones_unless_asked() {
        // Arrange
        let (a, b) = (folder("a"), folder("b"));
        fs::create_dir_all(a.dir.join("skills/s")).unwrap();
        fs::write(a.dir.join("skills/s/SKILL.md"), "from a").unwrap();
        fs::create_dir_all(b.dir.join("skills/s")).unwrap();
        fs::write(b.dir.join("skills/s/SKILL.md"), "b's own").unwrap();

        // Act
        let kept = copy_extension(&a, "skills/s", std::slice::from_ref(&b), false).unwrap();
        let kept_text = fs::read_to_string(b.dir.join("skills/s/SKILL.md")).unwrap();
        let replaced = copy_extension(&a, "skills/s", std::slice::from_ref(&b), true).unwrap();

        // Assert
        assert_eq!((kept[0].status, kept_text.as_str()), ("exists", "b's own"));
        assert_eq!(replaced[0].status, "copied");
        assert_eq!(fs::read_to_string(b.dir.join("skills/s/SKILL.md")).unwrap(), "from a");
        assert_eq!(claude_trash::list(&b).len(), 1, "the replaced skill is recoverable");
        assert!(copy_extension(&a, "skills/../x", std::slice::from_ref(&b), true).is_err());
        let _ = fs::remove_dir_all(&a.dir);
        let _ = fs::remove_dir_all(&b.dir);
    }

    #[test]
    fn mcp_servers_hide_secrets_and_copy_keeps_other_state() {
        // Arrange
        let (a, b) = (folder("a"), folder("b"));
        fs::write(&a.global_config, r#"{"mcpServers":{"gh":{"command":"npx","args":["gh-mcp"],"env":{"TOKEN":"secret"}}},
            "projects":{"/w":{"mcpServers":{"db":{"type":"http","url":"https://db","headers":{"Authorization":"Bearer x"}}}}}}"#).unwrap();
        fs::write(&b.global_config, r#"{"numStartups":3}"#).unwrap();

        // Act
        let listed = mcp_servers(&a).unwrap();
        let copied = copy_mcp(&a, "gh", std::slice::from_ref(&b), false).unwrap();
        delete_mcp(&a, Some("/w"), "db").unwrap();

        // Assert
        assert_eq!(listed[0].summary, "npx gh-mcp");
        assert_eq!(listed[0].config["env"]["TOKEN"], HIDDEN);
        assert_eq!((listed[1].transport.as_str(), listed[1].config["headers"]["Authorization"].as_str()), ("http", Some(HIDDEN)));
        assert_eq!(copied[0].status, "copied");
        let target: Value = serde_json::from_slice(&fs::read(&b.global_config).unwrap()).unwrap();
        assert_eq!((target["numStartups"].as_u64(), target["mcpServers"]["gh"]["env"]["TOKEN"].as_str()), (Some(3), Some("secret")));
        assert_eq!(mcp_servers(&a).unwrap().len(), 1);
        let _ = fs::remove_dir_all(&a.dir);
        let _ = fs::remove_dir_all(&b.dir);
    }

    #[test]
    fn plugins_combine_installs_and_switches() {
        // Arrange
        let f = folder("a");
        fs::create_dir_all(f.dir.join("plugins")).unwrap();
        fs::write(f.dir.join("plugins/installed_plugins.json"), r#"{"version":2,"plugins":{"fmt@tools":[{"version":"1.2.0"}]}}"#).unwrap();
        fs::write(f.dir.join("settings.json"), r#"{"model":"opus","enabledPlugins":{"fmt@tools":true}}"#).unwrap();

        // Act
        let before = plugins(&f);
        set_plugin(&f, "fmt@tools", false).unwrap();
        let after = plugins(&f);

        // Assert
        assert_eq!((before[0].name.as_str(), before[0].version.as_deref(), before[0].enabled), ("fmt", Some("1.2.0"), true));
        assert!(!after[0].enabled);
        let settings: Value = serde_json::from_slice(&fs::read(f.dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["model"], "opus");
        let _ = fs::remove_dir_all(&f.dir);
    }
}
