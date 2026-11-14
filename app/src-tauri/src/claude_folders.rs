//! The Claude Code data folders (`CLAUDE_CONFIG_DIR`) Hydra manages: the Mac's own `~/.claude`,
//! the folders of desktop accounts that keep Claude Code separate, and the folders of container
//! accounts.
//!
//! Sessions remember the working folder they ran in. A container sees the Mac's folders under
//! `/workspace/<name>`, so moving a session between the Mac and a container, or between two
//! containers, has to translate that path (`Paths`).
use crate::{container, install::Install, profiles, sandbox, settings};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// How working folders are written in a data folder's sessions.
#[derive(Clone, Debug, PartialEq)]
pub enum Paths {
    /// Paths on this computer.
    Host,
    /// Paths in a container: each `(name, Mac folder)` is shared as `/workspace/<name>`.
    Container(Vec<(String, PathBuf)>),
}

pub const WORKSPACE: &str = "/workspace";

impl Paths {
    /// The folder on this computer that `cwd` refers to, if it is one.
    pub fn to_host(&self, cwd: &str) -> Option<PathBuf> {
        match self {
            Paths::Host => Some(PathBuf::from(cwd)),
            Paths::Container(mounts) => {
                let rest = cwd.strip_prefix(WORKSPACE)?.strip_prefix('/')?;
                let (name, rest) = rest.split_once('/').unwrap_or((rest, ""));
                let (_, source) = mounts.iter().find(|(n, _)| n == name)?;
                Some(if rest.is_empty() { source.clone() } else { source.join(rest) })
            }
        }
    }

    /// How this side writes the folder `host` of this computer, if it can see it.
    pub fn path_of(&self, host: &Path) -> Option<String> {
        match self {
            Paths::Host => Some(host.to_string_lossy().into_owned()),
            Paths::Container(mounts) => {
                // The innermost shared folder wins when one is shared inside another.
                let (name, rest) = mounts.iter()
                    .filter_map(|(name, source)| Some((name, host.strip_prefix(source).ok()?)))
                    .min_by_key(|(_, rest)| rest.components().count())?;
                let rest = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
                Some(if rest.is_empty() { format!("{WORKSPACE}/{name}") } else { format!("{WORKSPACE}/{name}/{rest}") })
            }
        }
    }

    /// `cwd` of a session on side `self`, written for side `to`; `None` when `to` cannot see it.
    pub fn translate(&self, cwd: &str, to: &Paths) -> Option<String> {
        if self == to { return Some(cwd.to_string()); }
        to.path_of(&self.to_host(cwd)?)
    }

    /// Working folders offered when a path cannot be translated: a container's shared folders.
    pub fn choices(&self) -> Vec<String> {
        match self {
            Paths::Host => Vec::new(),
            Paths::Container(mounts) => mounts.iter().map(|(n, _)| format!("{WORKSPACE}/{n}")).chain([WORKSPACE.to_string()]).collect(),
        }
    }

    /// Whether `cwd` is a path this side can use as a working folder.
    pub fn accepts(&self, cwd: &str) -> bool {
        match self {
            Paths::Host => Path::new(cwd).is_absolute(),
            Paths::Container(_) => cwd == WORKSPACE || cwd.starts_with("/workspace/"),
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    /// `local`, `desktop:<account>` or `container:<account>`.
    pub id: String,
    /// `local`, `desktop` or `container`.
    pub kind: &'static str,
    pub name: String,
    pub path: String,
    /// A desktop account without a Claude Code folder of its own uses the Mac's `~/.claude`.
    pub shared_with_local: bool,
    /// Desktop account the folder belongs to (its Claude.app session list is managed separately).
    pub profile: Option<String>,
    #[serde(skip)]
    pub dir: PathBuf,
    /// Claude Code's global config (`.claude.json`: MCP servers, per-project state).
    #[serde(skip)]
    pub global_config: PathBuf,
    #[serde(skip)]
    pub paths: Paths,
}

fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

fn local() -> Folder {
    let dir = home().join(".claude");
    Folder {
        id: "local".into(), kind: "local", name: "本机 Claude Code".into(), path: dir.to_string_lossy().into_owned(),
        shared_with_local: false, profile: None, global_config: home().join(".claude.json"), dir, paths: Paths::Host,
    }
}

/// Every folder, the Mac's own first. Desktop accounts are listed only when Claude.app is installed.
pub fn list(install: Option<&Install>) -> Vec<Folder> {
    let mut out = vec![local()];
    let s = settings::load();
    if let Some(install) = install {
        for p in profiles::list(install, false) {
            // The same rule the launcher uses to point the account's Claude Code at its own folder.
            let e = s.effective(&p.id);
            let shared = !(e.separate_code && (!p.is_default || e.isolation == "sandboxie"));
            let own = sandbox::code_dir(&p.id);
            let base = if shared { local() } else {
                Folder { global_config: own.join(".claude.json"), path: own.to_string_lossy().into_owned(), dir: own, ..local() }
            };
            out.push(Folder { id: format!("desktop:{}", p.id), kind: "desktop", name: p.name, shared_with_local: shared, profile: Some(p.id), ..base });
        }
    }
    let mut containers: Vec<Folder> = s.accounts.iter().filter_map(|(id, a)| {
        let spec = a.container.as_ref()?;
        let dir = container::claude_data_dir(&s, id).ok()?;
        Some(Folder {
            id: format!("container:{id}"), kind: "container", name: a.label.clone().unwrap_or_else(|| id.clone()),
            path: dir.to_string_lossy().into_owned(), shared_with_local: false, profile: None,
            global_config: dir.join(".claude.json"), dir,
            paths: Paths::Container(spec.mounts.iter().map(|m| (m.name.clone(), PathBuf::from(&m.source))).collect()),
        })
    }).collect();
    containers.sort_by(|a, b| a.name.cmp(&b.name));
    out.extend(containers);
    out
}

/// A folder with what the location picker shows about it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    #[serde(flatten)]
    folder: Folder,
    exists: bool,
    sessions: usize,
    last_active_at: Option<u64>,
}

/// Counts sessions from file names and times alone, so many folders can be summarized at once.
pub fn summarize(folder: Folder) -> Summary {
    let mut sessions = 0;
    let mut last = None;
    for project in std::fs::read_dir(folder.dir.join("projects")).into_iter().flatten().flatten() {
        for file in std::fs::read_dir(project.path()).into_iter().flatten().flatten() {
            if !file.file_name().to_string_lossy().ends_with(".jsonl") { continue; }
            sessions += 1;
            let time = file.metadata().and_then(|m| m.modified()).ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
            last = last.max(time);
        }
    }
    Summary { exists: folder.dir.is_dir(), sessions, last_active_at: last, folder }
}

/// Opens the folder in Finder (Explorer on Windows).
pub fn reveal(f: &Folder) -> Result<(), String> {
    if !f.dir.is_dir() { return Err("这个目录还不存在：Claude Code 第一次在这里运行后才会创建。".into()); }
    std::process::Command::new(if cfg!(windows) { "explorer.exe" } else { "open" }).arg(&f.dir).spawn()
        .map(|_| ()).map_err(|e| format!("无法打开目录：{e}"))
}

pub fn find(install: Option<&Install>, id: &str) -> Result<Folder, String> {
    list(install).into_iter().find(|f| f.id == id).ok_or_else(|| "这个位置已不存在，请刷新。".into())
}

/// Key of a folder for Hydra's own records about it (trash): follows the folder, not the account.
pub fn key(f: &Folder) -> String { sandbox::account_key(&f.dir.to_string_lossy()) }

#[cfg(test)]
mod tests {
    use super::*;

    fn container(mounts: &[(&str, &str)]) -> Paths {
        Paths::Container(mounts.iter().map(|(n, s)| (n.to_string(), PathBuf::from(s))).collect())
    }

    #[test]
    fn container_paths_go_through_the_shared_mac_folder() {
        // Arrange
        let a = container(&[("app", "/Users/u/code/app")]);
        let b = container(&[("code", "/Users/u/code")]);

        // Act
        let to_mac = a.translate("/workspace/app/src", &Paths::Host);
        let to_other = a.translate("/workspace/app/src", &b);
        let from_mac = Paths::Host.translate("/Users/u/code/app", &a);

        // Assert
        assert_eq!(to_mac.map(PathBuf::from), Some(Path::new("/Users/u/code/app").join("src")));
        assert_eq!(to_other.as_deref(), Some("/workspace/code/app/src"));
        assert_eq!(from_mac.as_deref(), Some("/workspace/app"));
    }

    #[test]
    fn unshared_folders_have_no_translation() {
        // Arrange
        let a = container(&[("app", "/Users/u/code/app")]);
        let b = container(&[("docs", "/Users/u/docs")]);

        // Act
        let outside = a.translate("/workspace", &Paths::Host);
        let unknown_mount = a.translate("/workspace/other", &Paths::Host);
        let not_shared = a.translate("/workspace/app", &b);
        let sibling = Paths::Host.translate("/Users/u/code/application", &a);

        // Assert
        assert_eq!((outside, unknown_mount, not_shared, sibling), (None, None, None, None));
    }

    #[test]
    fn the_innermost_shared_folder_is_used() {
        // Arrange
        let a = container(&[("code", "/Users/u/code"), ("app", "/Users/u/code/app")]);

        // Act
        let inner = Paths::Host.translate("/Users/u/code/app/x", &a);

        // Assert
        assert_eq!(inner.as_deref(), Some("/workspace/app/x"));
    }

    #[test]
    fn summary_counts_transcripts_only() {
        // Arrange
        let dir = std::env::temp_dir().join(format!("hydra-folder-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(dir.join("projects/-a/s1")).unwrap();
        std::fs::write(dir.join("projects/-a/s1.jsonl"), "{}").unwrap();
        std::fs::write(dir.join("projects/-a/s2.jsonl"), "{}").unwrap();
        std::fs::write(dir.join("projects/-a/bridge-pointer.json"), "{}").unwrap();
        let f = Folder { id: "local".into(), kind: "local", name: "t".into(), path: String::new(), shared_with_local: false, profile: None,
            global_config: dir.join(".claude.json"), dir: dir.clone(), paths: Paths::Host };

        // Act
        let s = summarize(f);

        // Assert
        assert!(s.exists);
        assert_eq!(s.sessions, 2);
        assert!(s.last_active_at.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn choices_and_accepted_paths_follow_the_side() {
        // Arrange
        let a = container(&[("app", "/Users/u/code/app")]);

        // Act
        let choices = a.choices();

        // Assert
        assert_eq!(choices, vec!["/workspace/app".to_string(), "/workspace".to_string()]);
        assert!(choices.iter().all(|c| a.accepts(c)));
        assert!(!a.accepts("/Users/u/code/app"));
        assert!(!a.accepts("/workspacefoo"));
        assert!(Paths::Host.choices().is_empty());
    }
}
