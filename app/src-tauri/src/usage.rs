//! Read-only, best-effort Claude Code usage from local transcripts.
use serde::Serialize;
use std::{collections::HashMap, io::{BufRead, BufReader}, path::Path};

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    messages: u64,
    files: u64,
    unreadable_files: u64,
    path: String,
}

fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() { walk(&entry.path(), files)?; }
        else if kind.is_file() && entry.path().extension().is_some_and(|e| e == "jsonl") { files.push(entry.path()); }
    }
    Ok(())
}

pub fn read() -> Result<Usage, String> {
    let root = std::env::var_os("CLAUDE_CONFIG_DIR").map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|p| std::path::PathBuf::from(p).join(".claude")))
        .ok_or("无法确定 Claude Code 配置目录。")?.join("projects");
    read_root(&root)
}

fn read_root(root: &Path) -> Result<Usage, String> {
    let mut total = Usage { path: root.to_string_lossy().into_owned(), ..Usage::default() };
    if !root.exists() { return Ok(total); }
    let mut files = Vec::new();
    walk(&root, &mut files).map_err(|e| format!("无法扫描 Claude Code 日志：{e}"))?;
    let mut messages: HashMap<String, [u64; 4]> = HashMap::new();
    for file in files {
        let Ok(f) = std::fs::File::open(&file) else { total.unreadable_files += 1; continue };
        total.files += 1;
        for (line_no, line) in BufReader::new(f).lines().enumerate() {
            let Ok(line) = line else { total.unreadable_files += 1; break };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            if v["type"].as_str() != Some("assistant") { continue; }
            let m = &v["message"];
            let u = &m["usage"];
            if !u.is_object() { continue; }
            let key = match m["id"].as_str() {
                Some(id) => format!("{}:{}", file.display(), id),
                None => format!("{}:line:{line_no}", file.display()),
            };
            let values = ["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"].map(|k| u[k].as_u64().unwrap_or(0));
            messages.entry(key).and_modify(|old| { for i in 0..4 { old[i] = old[i].max(values[i]); } }).or_insert(values);
        }
    }
    total.messages = messages.len() as u64;
    for v in messages.values() {
        total.input_tokens = total.input_tokens.saturating_add(v[0]);
        total.output_tokens = total.output_tokens.saturating_add(v[1]);
        total.cache_creation_tokens = total.cache_creation_tokens.saturating_add(v[2]);
        total.cache_read_tokens = total.cache_read_tokens.saturating_add(v[3]);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    #[test]
    fn combines_streamed_message_updates_without_double_counting() {
        let dir = std::env::temp_dir().join(format!("hydra-usage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.jsonl"), concat!(
            "{\"type\":\"assistant\",\"message\":{\"id\":\"msg\",\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n",
            "{\"type\":\"assistant\",\"message\":{\"id\":\"msg\",\"usage\":{\"input_tokens\":10,\"output_tokens\":8}}}\n",
            "{\"type\":\"user\"}\n",
        )).unwrap();
        let usage = super::read_root(&dir).unwrap();
        assert_eq!((usage.messages, usage.input_tokens, usage.output_tokens), (1, 10, 8));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
