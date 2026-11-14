//! Claude Code container accounts on the user's own container runtime: OrbStack on macOS,
//! Docker Desktop on Windows (see `runtime`).
//!
//! A container account is an account entry with `AccountConfig::container` set. Instead of a
//! desktop profile it runs Claude Code in a Linux container, with the same network model as a
//! desktop account: its environment's node, its own local relay and the exit guard.
//!
//! Each container account has two containers and a volume:
//! - gateway `claude-hydra-<key>-gw` owns the network namespace. Every TCP connection becomes an
//!   HTTP CONNECT to the account's relay on this computer (`Runtime::host`), DNS goes over HTTPS
//!   through it, and everything else (UDP, IPv6, direct routes) is dropped
//!   (`container-images/gateway`). When the relay is paused by the guard, or Hydra is not
//!   running, the workspace has no network at all.
//! - workspace `claude-hydra-<key>` joins that namespace with no network capabilities and
//!   `no-new-privileges`; the user's folders appear under `/workspace/<name>`.
//! - Claude Code's data (login, settings, `.claude.json`) lives in a folder of this computer
//!   mounted at `/root/.claude` with `CLAUDE_CONFIG_DIR` pointing there: by default the account's
//!   folder in Hydra's data directory, or a folder the user picks. Visible and backed up here,
//!   kept across rebuilds and runtime resets.
//! - volume `claude-hydra-<key>-home` is the rest of `/root` (shell history and the like).
//! - Hydra's in-container tools (`hydra-audit`) are mounted read-only at `/opt/claude-hydra/bin`,
//!   so the container cannot alter the audit that judges it.
//!
//! Hydra opens a terminal running `claude-hydra --container <id>` (`enter`). It checks the route
//! through the relay, builds missing images with visible progress, starts both containers,
//! checks that the workspace really is in the gateway's namespace, and then hands the terminal
//! to a shell in the container that has the environment's timezone, language and variables.
use crate::runtime::{self, Docker, Runtime};
use crate::settings::{self, Settings};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const ID_PREFIX: &str = "ctr-";
const MAX_MOUNTS: usize = 32;

const LABEL_ID: &str = "claude-hydra.container";
const LABEL_ROLE: &str = "claude-hydra.role";
const LABEL_SPEC: &str = "claude-hydra.spec";
const LABEL_IMAGE: &str = "claude-hydra.image";

/// Proxy variables a docker client may add to every container it creates (its `proxies`
/// config, or Docker Desktop's proxy settings). The gateway already carries all traffic, and a
/// proxy address means nothing inside its network namespace; set empty, the client adds none.
const CLIENT_PROXY_ENV: [&str; 8] = ["HTTP_PROXY", "HTTPS_PROXY", "FTP_PROXY", "NO_PROXY", "http_proxy", "https_proxy", "ftp_proxy", "no_proxy"];

/// Linux capabilities the workspace keeps (file ownership for installs and builds); nothing
/// network-related, so the gateway's firewall cannot be changed from inside.
const WORKSPACE_CAPS: [&str; 7] = ["CHOWN", "DAC_OVERRIDE", "FOWNER", "FSETID", "SETUID", "SETGID", "SYS_CHROOT"];
/// OrbStack forwards the Mac's SSH agent at this path (other runtimes have no `Runtime::ssh_agent`).
const SSH_SOCK: &str = "/run/host-services/ssh-auth.sock";
/// Where Claude Code keeps its data in the container (`CLAUDE_CONFIG_DIR`).
const CLAUDE_DIR: &str = "/root/.claude";
const TOOLS_DIR: &str = "/opt/claude-hydra/bin";
/// Container layout of this Hydra version; a change makes existing workspaces need a rebuild.
const LAYOUT: &str = "2";

/// Tools installed into every workspace (read-only).
const TOOLS: [(&str, &str); 2] = [
    ("hydra-audit", include_str!("../container-images/tools/hydra-audit")),
    ("hydra-statusline", include_str!("../container-images/tools/hydra-statusline")),
];

/// Claude Code's status line in the container: shows the plan's limits and records them for the
/// dashboard (`dashboard::LIMITS_RECORD`). Claude Code hands them to status lines itself, so the
/// account's credentials are never touched.
const STATUSLINE: &str = "/opt/claude-hydra/bin/hydra-statusline";

/// Points the account's Claude Code status line at `hydra-statusline`, unless it has one of its own.
fn ensure_statusline(claude: &Path) -> Result<(), String> {
    let path = claude.join("settings.json");
    let mut settings = crate::claude_config::read_json(&path)?;
    if settings.contains_key("statusLine") { return Ok(()); }
    settings.insert("statusLine".into(), serde_json::json!({ "type": "command", "command": STATUSLINE, "padding": 0 }));
    crate::claude_config::write_json(&path, &settings)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Mount {
    /// Absolute folder on this computer.
    pub source: String,
    /// Folder name under `/workspace` in the container.
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContainerSpec {
    /// "" = Hydra's built-in workspace image, else an image the user has in the runtime.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<Mount>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ssh_agent: bool,
    /// Folder for Claude Code's data; "" = the account's folder in Hydra's data directory.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub claude_dir: String,
}

/// What the UI edits.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerInput {
    pub name: String,
    /// Environment id; it must have a node.
    pub environment: String,
    #[serde(flatten)]
    pub spec: ContainerSpec,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerView {
    pub id: String,
    #[serde(flatten)]
    pub input: ContainerInput,
    pub environment_name: Option<String>,
    /// The folder that actually holds Claude Code's data.
    pub claude_dir_path: String,
    /// "running" | "stopped" | "absent" (created on first open) | "unknown" (runtime unavailable)
    pub state: &'static str,
    /// Why the container must be rebuilt before it can be opened again.
    pub rebuild: Option<String>,
    /// The configured image differs from the one the container was created from.
    pub image_update: bool,
    pub blocked: Option<String>,
    /// The cut comes from failing checks and lifts by itself once the exit verifies again.
    pub blocked_soft: bool,
    pub last_exit: Option<crate::node::ExitInfo>,
}

// ---- images ---------------------------------------------------------------------------

struct Image {
    repo: &'static str,
    files: &'static [(&'static str, &'static str)],
}

const GATEWAY: Image = Image {
    repo: "claude-hydra-gateway",
    files: &[
        ("Dockerfile", include_str!("../container-images/gateway/Dockerfile")),
        ("gateway.py", include_str!("../container-images/gateway/gateway.py")),
    ],
};

const WORKSPACE: Image = Image {
    repo: "claude-hydra-workspace",
    files: &[("Dockerfile", include_str!("../container-images/workspace/Dockerfile"))],
};

fn short_hash(parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    format!("{:x}", h.finalize())[..12].to_string()
}

impl Image {
    /// Tagged by content: a Hydra update that changes the build context builds a new image.
    fn tag(&self) -> String {
        let parts: Vec<&[u8]> = self.files.iter().flat_map(|(n, b)| [n.as_bytes(), b.as_bytes()]).collect();
        format!("{}:{}", self.repo, short_hash(&parts))
    }
}

/// The image a container account is meant to run (before any snapshot).
fn wanted_image(spec: &ContainerSpec) -> String {
    if spec.image.is_empty() { WORKSPACE.tag() } else { spec.image.clone() }
}

/// Settings that are fixed when the workspace is created; a change needs a rebuild.
fn fingerprint(spec: &ContainerSpec) -> String {
    let mut mounts: Vec<&Mount> = spec.mounts.iter().collect();
    mounts.sort_by(|a, b| a.name.cmp(&b.name));
    let mut parts: Vec<&[u8]> = mounts.iter().flat_map(|m| [m.source.as_bytes(), m.name.as_bytes()]).collect();
    parts.push(if spec.ssh_agent { b"ssh" } else { b"" });
    parts.push(spec.claude_dir.as_bytes());
    parts.push(LAYOUT.as_bytes());
    short_hash(&parts)
}

// ---- names and validation ----------------------------------------------------------------

struct Names {
    workspace: String,
    gateway: String,
    home: String,
    snapshot: String,
}

fn names(id: &str) -> Result<Names, String> {
    let key = id.strip_prefix(ID_PREFIX)
        .filter(|k| !k.is_empty() && k.len() <= 32 && k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
        .ok_or("容器账号标识无效。")?;
    Ok(Names {
        workspace: format!("claude-hydra-{key}"),
        gateway: format!("claude-hydra-{key}-gw"),
        home: format!("claude-hydra-{key}-home"),
        snapshot: format!("claude-hydra-snapshot:{key}"),
    })
}

fn new_id(s: &Settings) -> String {
    let mut n = crate::paths::now_millis();
    loop {
        let id = format!("{ID_PREFIX}{n:x}");
        if !s.accounts.contains_key(&id) { return id; }
        n += 1;
    }
}

/// Image references as docker accepts them; never an option or anything a shell would split.
fn valid_image_ref(image: &str) -> bool {
    !image.is_empty() && image.len() <= 255 && !image.starts_with(['-', '.', '/', ':'])
        && image.chars().all(|c| c.is_ascii_alphanumeric() || "._-/:@".contains(c))
}

/// How this computer writes folder paths, and the folders that may never be shared.
struct HostRules {
    /// Windows paths (`C:\…`, any case) instead of Unix ones.
    windows: bool,
    home: String,
    /// Hydra's data directory (it holds node credentials).
    storage: String,
    /// The runtime can forward this computer's SSH agent.
    ssh_agent: bool,
}

impl HostRules {
    fn here(ssh_agent: bool) -> HostRules {
        let windows = cfg!(windows);
        let home = std::env::var(if windows { "USERPROFILE" } else { "HOME" }).unwrap_or_else(|_| "/".into());
        let mut rules = HostRules { windows, home, storage: crate::paths::storage_root().to_string_lossy().into_owned(), ssh_agent };
        // Written the way `folder` writes paths, so `inside` compares like with like.
        if let Ok(h) = rules.folder(&rules.home) { rules.home = h; }
        if let Ok(st) = rules.folder(&rules.storage) { rules.storage = st; }
        rules
    }

    /// A complete folder path, written without a trailing separator.
    fn folder(&self, raw: &str) -> Result<String, String> {
        let raw = raw.trim();
        if raw.contains(['\n', '\r', '\0']) { return Err(format!("文件夹路径不能包含换行：{raw}")); }
        if self.windows {
            let path = raw.replace('/', "\\");
            if path.starts_with("\\\\") { return Err(format!("暂不支持网络路径，请先把它映射为盘符：{raw}")); }
            let drive = path.as_bytes();
            if drive.len() < 2 || !drive[0].is_ascii_alphabetic() || drive[1] != b':' || (drive.len() > 2 && drive[2] != b'\\') {
                return Err(format!("请填写完整的文件夹路径（如 D:\\Projects\\app）：{raw}"));
            }
            if path.split('\\').any(|c| c == ".." || c == ".") { return Err(format!("文件夹路径不能包含“..”：{raw}")); }
            let trimmed = path.trim_end_matches('\\');
            return Ok(if trimmed.len() == 2 { format!("{trimmed}\\") } else { trimmed.to_string() });
        }
        let path = if raw.len() > 1 { raw.trim_end_matches('/') } else { raw };
        if !path.starts_with('/') { return Err(format!("请填写完整的文件夹路径：{raw}")); }
        if Path::new(path).components().any(|c| matches!(c, std::path::Component::ParentDir)) { return Err(format!("文件夹路径不能包含“..”：{raw}")); }
        Ok(path.to_string())
    }

    /// Whether `path` is `folder` or inside it (both as `folder` writes them).
    fn inside(&self, path: &str, folder: &str) -> bool {
        if !self.windows { return Path::new(path).starts_with(folder); }
        let (path, folder) = (path.to_lowercase(), folder.to_lowercase());
        path == folder || path.starts_with(&if folder.ends_with('\\') { folder } else { format!("{folder}\\") })
    }

    fn join(&self, folder: &str, name: &str) -> String {
        let sep = if self.windows { '\\' } else { '/' };
        format!("{}{sep}{name}", folder.trim_end_matches(sep))
    }

    /// A folder that may be shared into a container. Refuses the root, the home folder and any
    /// folder containing them or Hydra's own data.
    fn source(&self, raw: &str) -> Result<String, String> {
        let source = self.folder(raw)?;
        if self.inside(&self.home, &source) || self.inside(&self.storage, &source) {
            return Err(format!("不能共享 {source}：它包含用户主目录或 Claude-Hydra 的数据目录（含节点凭据）。请选择具体的项目文件夹。"));
        }
        Ok(source)
    }
}

fn check_mount_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() { return Err("请填写容器内的文件夹名称。".into()); }
    if name.chars().count() > 80 { return Err(format!("容器内的文件夹名称最多 80 个字符：{name}")); }
    if name == "." || name == ".." || name.contains(['/', '\\', ':', '\n', '\r', '\0']) {
        return Err(format!("容器内的文件夹名称不能包含 / \\ : 或换行：{name}"));
    }
    Ok(name.to_string())
}

fn container_names(s: &Settings, except: Option<&str>) -> Vec<String> {
    s.accounts.iter()
        .filter(|(id, a)| a.container.is_some() && Some(id.as_str()) != except)
        .filter_map(|(_, a)| a.label.clone())
        .collect()
}

/// Validates input into (name, environment id, spec). `is_dir` checks that a source exists.
fn build(input: &ContainerInput, s: &Settings, id: Option<&str>, host: &HostRules, is_dir: impl Fn(&str) -> bool) -> Result<(String, String, ContainerSpec), String> {
    let name = input.name.trim();
    if name.is_empty() { return Err("请填写名称。".into()); }
    if name.chars().count() > 40 { return Err("名称最多 40 个字符。".into()); }
    if container_names(s, id).iter().any(|n| n.eq_ignore_ascii_case(name)) { return Err(format!("已有名为“{name}”的容器账号。")); }
    let env = s.environments.get(&input.environment).ok_or("请选择一个环境配置。")?;
    if env.node.is_none() { return Err(format!("环境“{}”没有代理节点。容器只经代理节点联网，请选择带节点的环境。", env.name)); }
    let image = input.spec.image.trim().to_string();
    if !image.is_empty() && !valid_image_ref(&image) { return Err(format!("镜像名称无效：{image}")); }
    if input.spec.mounts.len() > MAX_MOUNTS { return Err(format!("最多共享 {MAX_MOUNTS} 个文件夹。")); }
    if input.spec.ssh_agent && !host.ssh_agent { return Err("这台电脑上的容器不支持转发 SSH agent，请关闭此选项。".into()); }
    let mut mounts: Vec<Mount> = Vec::new();
    for m in &input.spec.mounts {
        let source = host.source(&m.source)?;
        if !is_dir(&source) { return Err(format!("找不到文件夹：{source}")); }
        let name = check_mount_name(&m.name)?;
        if mounts.iter().any(|o| o.name == name) { return Err(format!("容器内有两个文件夹都叫“{name}”，请改名。")); }
        mounts.push(Mount { source, name });
    }
    let claude_dir = match input.spec.claude_dir.trim() {
        "" => String::new(),
        dir => {
            let dir = host.source(dir)?;
            if host.inside(&dir, &host.join(&host.home, ".claude")) {
                return Err("不能使用本机自己的 .claude 文件夹：那里是本机 Claude Code 的登录与设置。请另选一个文件夹。".into());
            }
            if host.inside(&dir, &host.storage) {
                return Err("请选择 Claude-Hydra 数据目录以外的文件夹；不选则使用数据目录中的默认位置。".into());
            }
            if !is_dir(&dir) { return Err(format!("找不到文件夹：{dir}")); }
            let same = |other: &str| host.inside(other, &dir) && host.inside(&dir, other);
            let taken = s.accounts.iter().any(|(k, a)| Some(k.as_str()) != id && a.container.as_ref().is_some_and(|c| same(&c.claude_dir)));
            if taken { return Err("另一个容器账号已在使用这个 Claude 数据文件夹，每个账号需要各自的文件夹。".into()); }
            dir
        }
    };
    Ok((name.to_string(), input.environment.clone(), ContainerSpec { image, mounts, ssh_agent: input.spec.ssh_agent, claude_dir }))
}

fn account(s: &Settings, id: &str) -> Result<(String, ContainerSpec), String> {
    let acc = s.accounts.get(id).ok_or("这个容器账号已不存在。")?;
    let spec = acc.container.clone().ok_or("这不是容器账号。")?;
    Ok((acc.label.clone().unwrap_or_else(|| id.to_string()), spec))
}

/// Finds a container account by id or name.
pub fn resolve(name_or_id: &str) -> Option<String> {
    let s = settings::load();
    let containers = || s.accounts.iter().filter(|(_, a)| a.container.is_some());
    containers().find(|(id, _)| id.as_str() == name_or_id)
        .or_else(|| containers().find(|(_, a)| a.label.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(name_or_id))))
        .map(|(id, _)| id.clone())
}

fn state_dir(id: &str) -> PathBuf { crate::paths::storage_root().join("containers").join(id) }

/// The account's usage history (see `code_usage`); deleted with the account.
pub(crate) fn usage_archive(id: &str) -> PathBuf { state_dir(id).join("usage-history.json") }

/// The folder holding a container account's Claude Code data.
pub(crate) fn claude_data_dir(s: &Settings, id: &str) -> Result<PathBuf, String> {
    Ok(claude_dir(id, &account(s, id)?.1))
}

fn claude_dir(id: &str, spec: &ContainerSpec) -> PathBuf {
    if spec.claude_dir.is_empty() { state_dir(id).join("claude") } else { PathBuf::from(&spec.claude_dir) }
}

// ---- settings --------------------------------------------------------------------------

pub fn save(id: Option<&str>, input: &ContainerInput) -> Result<String, String> {
    let s = settings::load();
    if let Some(id) = id { account(&s, id)?; }
    let ssh_agent = runtime::current().is_some_and(|r| r.ssh_agent);
    let (name, environment, spec) = build(input, &s, id, &HostRules::here(ssh_agent), |p| Path::new(p).is_dir())?;
    let id = id.map(str::to_string).unwrap_or_else(|| new_id(&s));
    settings::update(|s| {
        let acc = s.account_mut(&id);
        // The last exit described the previous environment's node.
        if acc.environment.as_deref() != Some(environment.as_str()) { acc.last_exit = None; }
        acc.label = Some(name);
        acc.environment = Some(environment);
        acc.container = Some(spec);
    })?;
    Ok(id)
}

struct Live {
    running: bool,
    spec: String,
    image: String,
}

/// Workspace containers Hydra created, by account id.
fn live_workspaces(docker: &Docker) -> Result<HashMap<String, Live>, String> {
    let format = format!("{{{{.Label \"{LABEL_ID}\"}}}}\t{{{{.State}}}}\t{{{{.Label \"{LABEL_SPEC}\"}}}}\t{{{{.Label \"{LABEL_IMAGE}\"}}}}");
    let out = docker.run(["ps", "-a", "--filter", &format!("label={LABEL_ROLE}=workspace"), "--format", &format])?;
    Ok(parse_live(&out))
}

fn parse_live(out: &str) -> HashMap<String, Live> {
    out.lines().filter_map(|line| {
        let f: Vec<&str> = line.split('\t').collect();
        (f.len() == 4 && !f[0].is_empty()).then(|| (f[0].to_string(), Live { running: f[1] == "running", spec: f[2].to_string(), image: f[3].to_string() }))
    }).collect()
}

/// (reason the workspace must be rebuilt, whether its image is outdated)
fn pending(spec: &ContainerSpec, live: Option<&Live>) -> (Option<String>, bool) {
    let Some(live) = live else { return (None, false) };
    let rebuild = (live.spec != fingerprint(spec)).then(|| "容器设置已变化（共享文件夹、Claude 数据文件夹、SSH，或 Hydra 更新了容器结构），需要重建容器才能生效；登录数据会保留。".to_string());
    (rebuild, live.image != wanted_image(spec))
}

pub fn list() -> Vec<ContainerView> {
    let s = settings::load();
    let live = runtime::connect().and_then(|(d, _)| live_workspaces(&d)).ok();
    let mut out: Vec<ContainerView> = s.accounts.iter().filter_map(|(id, acc)| {
        let spec = acc.container.clone()?;
        let current = live.as_ref().and_then(|m| m.get(id));
        let (rebuild, image_update) = pending(&spec, current);
        let environment = acc.environment.clone().unwrap_or_default();
        Some(ContainerView {
            id: id.clone(),
            environment_name: s.environments.get(&environment).map(|e| e.name.clone()),
            claude_dir_path: claude_dir(id, &spec).to_string_lossy().to_string(),
            input: ContainerInput { name: acc.label.clone().unwrap_or_else(|| id.clone()), environment, spec },
            state: match (&live, current) {
                (None, _) => "unknown",
                (Some(_), None) => "absent",
                (Some(_), Some(l)) => if l.running { "running" } else { "stopped" },
            },
            rebuild,
            image_update,
            blocked: acc.blocked.clone(),
            blocked_soft: acc.blocked.is_some() && acc.blocked_soft,
            last_exit: acc.last_exit.clone(),
        })
    }).collect();
    out.sort_by(|a, b| a.input.name.cmp(&b.input.name));
    out
}

// ---- docker arguments ------------------------------------------------------------------

/// `--mount` binding `source` of this computer at `target`. Unlike `-v`, it takes any path:
/// drive letters, colons, and commas or quotes (CSV-quoted).
fn bind(source: &Path, target: &str, readonly: bool) -> [String; 2] {
    let field = |f: String| if f.contains([',', '"']) { format!("\"{}\"", f.replace('"', "\"\"")) } else { f };
    let mut spec = format!("type=bind,{},{}", field(format!("source={}", source.display())), field(format!("target={target}")));
    if readonly { spec.push_str(",readonly"); }
    ["--mount".into(), spec]
}

fn gateway_args(id: &str, n: &Names, upstream_file: &Path, image: &str, add_host: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = ["run", "-d", "--init", "--name", &n.gateway, "--hostname", "workstation"].map(String::from).to_vec();
    a.extend(["--label".into(), format!("{LABEL_ID}={id}"), "--label".into(), format!("{LABEL_ROLE}=gateway")]);
    a.extend([
        "--cap-drop", "ALL", "--cap-add", "NET_ADMIN", "--cap-add", "NET_BIND_SERVICE",
        "--security-opt", "no-new-privileges:true", "--read-only",
        "--tmpfs", "/run:rw,nosuid,nodev", "--tmpfs", "/tmp:rw,nosuid,nodev",
        "--sysctl", "net.ipv6.conf.all.disable_ipv6=1",
    ].map(String::from));
    if let Some(h) = add_host { a.extend(["--add-host".into(), h.into()]); }
    a.extend(bind(upstream_file, "/etc/hydra-gateway/upstream", true));
    a.push(image.to_string());
    a
}

/// Folders of this computer mounted into the workspace besides the shared ones.
struct HostDirs<'a> {
    claude: &'a Path,
    tools: &'a Path,
}

fn workspace_args(id: &str, n: &Names, spec: &ContainerSpec, dirs: &HostDirs, image: &str, image_label: &str, gateway_id: &str) -> Vec<String> {
    let mut a: Vec<String> = ["create", "--init", "--name", &n.workspace].map(String::from).to_vec();
    a.extend(["--network".into(), format!("container:{gateway_id}"), "--cap-drop".into(), "ALL".into()]);
    for cap in WORKSPACE_CAPS { a.extend(["--cap-add".into(), cap.into()]); }
    a.extend(["--security-opt".into(), "no-new-privileges:true".into()]);
    for k in CLIENT_PROXY_ENV { a.extend(["-e".into(), format!("{k}=")]); }
    for (k, v) in [(LABEL_ID, id), (LABEL_ROLE, "workspace"), (LABEL_SPEC, &fingerprint(spec)), (LABEL_IMAGE, image_label)] {
        a.extend(["--label".into(), format!("{k}={v}")]);
    }
    a.extend(["-v".into(), format!("{}:/root", n.home)]);
    a.extend(bind(dirs.claude, CLAUDE_DIR, false));
    a.extend(["-e".into(), format!("CLAUDE_CONFIG_DIR={CLAUDE_DIR}")]);
    a.extend(bind(dirs.tools, TOOLS_DIR, true));
    for m in &spec.mounts { a.extend(bind(Path::new(&m.source), &format!("/workspace/{}", m.name), false)); }
    if spec.ssh_agent {
        a.extend(["-v".into(), format!("{SSH_SOCK}:{SSH_SOCK}"), "-e".into(), format!("SSH_AUTH_SOCK={SSH_SOCK}")]);
    }
    a.extend(["-w", "/workspace", "--entrypoint", "sleep", image, "infinity"].map(String::from));
    a
}

/// "en-US" → "en_US.UTF-8"; "" (follow the system) → none.
fn posix_locale(locale: &str) -> Option<String> {
    (!locale.is_empty()).then(|| format!("{}.UTF-8", locale.replace('-', "_")))
}

/// What `hydra-audit` compares the container's traffic with.
struct AuditContext {
    /// Exits the account may use: the one verified just now, and the exit rule's baseline.
    exits: Vec<String>,
    country: String,
    /// This computer as the gateway reaches it.
    host_ip: String,
    relay_port: u16,
}

/// Variables for the container shell: the environment's own ones (without proxy variables, the
/// gateway is transparent, without `CLAUDE_CONFIG_DIR`, which is fixed at creation, and without
/// `HYDRA_*`) plus the timezone, language and audit context Hydra manages.
fn shell_env(env: &BTreeMap<String, String>, timezone: Option<&str>, locale: Option<&str>, audit: &AuditContext) -> Vec<(String, String)> {
    let managed = |k: &str| (timezone.is_some() && k.eq_ignore_ascii_case("TZ"))
        || (locale.is_some() && (k.eq_ignore_ascii_case("LANG") || k.eq_ignore_ascii_case("LC_ALL")));
    let mut out: Vec<(String, String)> = env.iter()
        .filter(|(k, _)| !crate::node::is_proxy_env(k) && !managed(k) && !k.eq_ignore_ascii_case("CLAUDE_CONFIG_DIR") && !k.to_ascii_uppercase().starts_with("HYDRA_"))
        .map(|(k, v)| (k.clone(), v.clone())).collect();
    if let Some(tz) = timezone { out.push(("TZ".into(), tz.into())); }
    if let Some(l) = locale { out.push(("LANG".into(), l.into())); out.push(("LC_ALL".into(), l.into())); }
    out.push(("HYDRA_EXPECTED_EXIT".into(), audit.exits.join(",")));
    out.push(("HYDRA_EXPECTED_COUNTRY".into(), audit.country.clone()));
    if let Some(tz) = timezone { out.push(("HYDRA_EXPECTED_TZ".into(), tz.into())); }
    out.push(("HYDRA_HOST_IP".into(), audit.host_ip.clone()));
    out.push(("HYDRA_RELAY".into(), format!("http://{}:{}", audit.host_ip, audit.relay_port)));
    out
}

/// Variables describing the terminal Hydra was opened in. `docker exec` would otherwise report a
/// plain `xterm`: Claude Code reads them to pick colours and keys, and to tell the user how to
/// copy text when the terminal cannot take the clipboard from it (Terminal.app ignores OSC 52).
const TERMINAL_ENV: [&str; 6] = ["TERM", "COLORTERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "LC_TERMINAL", "LC_TERMINAL_VERSION"];

/// What a Windows console supports when it does not say: Windows Terminal and the console host
/// both take 256 and 24-bit colours.
fn console_default(name: &str) -> Option<String> {
    if !cfg!(windows) { return None; }
    match name { "TERM" => Some("xterm-256color".into()), "COLORTERM" => Some("truecolor".into()), _ => None }
}

/// The terminal's own variables, unless the environment config sets them.
fn terminal_env(set: &[(String, String)], read: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    TERMINAL_ENV.iter()
        .filter(|k| !set.iter().any(|(s, _)| s.eq_ignore_ascii_case(k)))
        .filter_map(|k| read(k).filter(|v| !v.is_empty()).map(|v| (k.to_string(), v)))
        .collect()
}

fn exec_args(workspace: &str, env: &[(String, String)], launch: Option<&Launch>) -> Vec<String> {
    let dir = launch.map(|l| l.cwd.as_str()).unwrap_or(crate::claude_folders::WORKSPACE);
    let mut a: Vec<String> = ["exec", "-it", "-w", dir].map(String::from).to_vec();
    for (k, v) in env { a.extend(["-e".into(), format!("{k}={v}")]); }
    // Claude Code first, when asked for; leaving it drops into a shell in the same folder.
    let claude = match launch {
        Some(Launch { resume: Some(session), .. }) => format!("claude --resume {session}; "),
        Some(_) => "claude; ".to_string(),
        None => String::new(),
    };
    let shell = format!("export PATH=\"{TOOLS_DIR}:$PATH\"; {claude}if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi");
    a.extend([workspace.to_string(), "sh".into(), "-c".into(), shell]);
    a
}

/// This computer's own timezone, for environments set to follow the system.
fn host_timezone() -> Option<String> {
    iana_time_zone::get_timezone().ok().filter(|tz| crate::node::valid_timezone(tz))
}

// ---- lifecycle -------------------------------------------------------------------------

fn say(text: &str) {
    println!("{text}");
    let _ = std::io::stdout().flush();
}

/// Daemon errors are not an absent container.
fn exists(docker: &Docker, name: &str) -> Result<bool, String> {
    Ok(!docker.run(["ps", "-aq", "--filter", &format!("name=^/{name}$")])?.is_empty())
}

fn no_node() -> String { "容器账号需要一个带代理节点的环境。请在容器设置中选择环境。".into() }

/// The environment names a node that cannot be read; refusing keeps the account off the
/// network instead of looking like it has none.
fn check_node(effective: &settings::Effective) -> Result<crate::node::NodeConfig, String> {
    if effective.unusable_node {
        return Err("这个容器账号的代理节点无法读取（密码不能在本机解密，或节点配置已损坏）。请打开它的环境配置，重新填写节点并保存。".into());
    }
    effective.node.clone().ok_or_else(no_node)
}

fn blocked(reason: &str, soft: bool) -> String {
    if soft {
        return format!("出口保护暂时无法核验这个容器账号的出口，已暂停它的网络——{reason}\nHydra 会自动重试，核验通过后自动恢复；也可以在容器页选择“恢复网络”立即重新检测。");
    }
    format!("出口保护已切断这个容器账号的网络——{reason}\n请在 Claude-Hydra 的容器页选择“恢复网络”，或在环境配置中重新绑定出口。")
}

fn ensure_image(docker: &Docker, image: &Image) -> Result<String, String> {
    let tag = image.tag();
    if docker.ok(["image", "inspect", &tag]) { return Ok(tag); }
    let dir = crate::paths::storage_root().join("container-build").join(tag.replace(':', "-"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建镜像构建目录：{e}"))?;
    for (name, body) in image.files {
        std::fs::write(dir.join(name), body).map_err(|e| format!("无法写入镜像构建文件：{e}"))?;
    }
    say(&format!("[镜像] 首次使用，正在构建 {tag}（需要几分钟）…"));
    let built = docker.attached(["build", "-t", &tag, &dir.to_string_lossy()])?;
    let _ = std::fs::remove_dir_all(&dir);
    if !built { return Err(format!("镜像 {tag} 构建失败，详情见上方输出。")); }
    Ok(tag)
}

fn running(docker: &Docker, name: &str) -> Result<bool, String> {
    Ok(docker.run(["container", "inspect", "--format", "{{.State.Running}}", name])? == "true")
}

/// Waits for the gateway's firewall and relay, and returns its ready line.
fn wait_gateway(docker: &Docker, gateway: &str) -> Result<String, String> {
    for _ in 0..50 {
        if let Ok(line) = docker.run(["exec", gateway, "cat", "/run/gateway-ready"]) { return Ok(line); }
        if !running(docker, gateway)? {
            let logs = docker.run(["logs", "--tail", "20", gateway]).unwrap_or_default();
            return Err(format!("网关容器启动失败，工作容器不会启动。\n{logs}"));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err("网关未就绪，工作容器不会启动。".into())
}

/// The address the gateway reports using (`upstream=<ip>:<port>`), when it is relay `port` on
/// `host`, an IP or a name the gateway resolved.
fn gateway_upstream(ready: &str, host: &str, port: u16) -> Option<String> {
    let (ip, p) = ready.trim().strip_prefix("upstream=")?.rsplit_once(':')?;
    let ip: std::net::Ipv4Addr = ip.parse().ok()?;
    let same_host = host.parse::<std::net::Ipv4Addr>().map_or(true, |h| h == ip);
    (p == port.to_string() && same_host).then(|| ip.to_string())
}

/// Creates or starts the account's gateway, pointed at relay `port`; returns its container id
/// and this computer's address as the gateway reaches it.
fn ensure_gateway(docker: &Docker, rt: &Runtime, id: &str, n: &Names, port: u16, image: &str) -> Result<(String, String), String> {
    let upstream = format!("{}:{port}", rt.host);
    let dir = state_dir(id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建容器状态目录：{e}"))?;
    let file = dir.join("upstream");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(upstream.as_str()) {
        std::fs::write(&file, &upstream).map_err(|e| format!("无法写入网关配置：{e}"))?;
    }
    if !exists(docker, &n.gateway)? {
        say("[网关] 创建网络网关…");
        docker.run(gateway_args(id, n, &file, image, rt.add_host))?;
    } else if !running(docker, &n.gateway)? {
        docker.run(["start", &n.gateway])?;
    }
    let host_ip = match gateway_upstream(&wait_gateway(docker, &n.gateway)?, rt.host, port) {
        Some(ip) => ip,
        None => {
            // The relay moved to another port; a restart makes the gateway read the new one, but
            // a running workspace would stay in the old network namespace.
            if exists(docker, &n.workspace)? && running(docker, &n.workspace)? {
                return Err("本机中继的端口已变化。请先在 Claude-Hydra 的容器页停止此容器，再重新打开（数据不会丢失）。".into());
            }
            docker.run(["restart", &n.gateway])?;
            gateway_upstream(&wait_gateway(docker, &n.gateway)?, rt.host, port).ok_or("网关没有使用当前的本机中继，工作容器不会启动。")?
        }
    };
    Ok((docker.run(["container", "inspect", "--format", "{{.Id}}", &n.gateway])?, host_ip))
}

/// Writes Hydra's in-container tools to the account's state folder (mounted read-only), so a
/// Hydra update brings the new versions without a rebuild.
fn prepare_tools(id: &str) -> Result<PathBuf, String> {
    let dir = state_dir(id).join("bin");
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法写入容器工具：{e}"))?;
    for (name, body) in TOOLS {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(body) {
            std::fs::write(&path, body).map_err(|e| format!("无法写入容器工具：{e}"))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("无法写入容器工具：{e}"))?;
        }
    }
    Ok(dir)
}

/// The folder for Claude Code's data: the default one is created owner-only (it holds the login
/// in plain text; on Windows the user's own AppData already is); a chosen one must exist.
fn prepare_claude_dir(id: &str, spec: &ContainerSpec) -> Result<PathBuf, String> {
    let dir = claude_dir(id, spec);
    if spec.claude_dir.is_empty() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建 Claude 数据文件夹：{e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
    } else if !dir.is_dir() {
        return Err(format!("找不到 Claude 数据文件夹：{}。请在容器设置中重新选择。", dir.display()));
    }
    Ok(dir)
}

/// Creates or starts the workspace in the gateway's namespace and verifies it.
fn ensure_workspace(docker: &Docker, id: &str, n: &Names, spec: &ContainerSpec, base: &str, gateway_id: &str) -> Result<(), String> {
    if !exists(docker, &n.workspace)? {
        // A rebuild that kept the container's changes left a snapshot to continue from.
        let (image, label) = if docker.ok(["image", "inspect", &n.snapshot]) {
            let label = docker.run(["image", "inspect", "--format", &format!("{{{{index .Config.Labels \"{LABEL_IMAGE}\"}}}}"), &n.snapshot]).unwrap_or_default();
            (n.snapshot.clone(), if label.is_empty() || label == "<no value>" { base.to_string() } else { label })
        } else {
            (base.to_string(), base.to_string())
        };
        let claude = prepare_claude_dir(id, spec)?;
        let tools = state_dir(id).join("bin");
        say("[容器] 创建工作容器…");
        docker.run(workspace_args(id, n, spec, &HostDirs { claude: &claude, tools: &tools }, &image, &label, gateway_id))?;
    }
    let info = docker.run(["container", "inspect", "--format", &format!("{{{{.HostConfig.NetworkMode}}}}|{{{{index .Config.Labels \"{LABEL_SPEC}\"}}}}"), &n.workspace])?;
    let (mode, fp) = info.split_once('|').unwrap_or((info.as_str(), ""));
    if mode != format!("container:{gateway_id}") {
        return Err("工作容器没有绑定当前的网关（网关可能被删除过）。请在 Claude-Hydra 的容器页“重建”此容器，登录数据会保留。".into());
    }
    if fp != fingerprint(spec) {
        return Err("共享文件夹或 SSH 设置已修改。请在 Claude-Hydra 的容器页“重建”此容器后再打开。".into());
    }
    if !running(docker, &n.workspace)? {
        say("[容器] 启动工作容器…");
        docker.run(["start", &n.workspace])?;
    }
    let gateway_net = docker.run(["exec", &n.gateway, "readlink", "/proc/self/ns/net"])?;
    let workspace_net = docker.run(["exec", &n.workspace, "readlink", "/proc/self/ns/net"])?;
    if gateway_net.is_empty() || gateway_net != workspace_net {
        return Err("工作容器仍在网关旧的网络空间中（网关重启过）。请在 Claude-Hydra 的容器页停止此容器，再重新打开，数据不会丢失。".into());
    }
    if spec.ssh_agent && !docker.ok(["exec", &n.workspace, "sh", "-c", "test -S \"$SSH_AUTH_SOCK\""]) {
        return Err("本机的 SSH agent 不可用。请确认 ssh-agent 正在运行并已添加密钥，或在容器设置中关闭 SSH agent 转发。".into());
    }
    Ok(())
}

/// Terminal entry (`claude-hydra --container <id> [--launch <token>]`): prepares everything, then
/// hands the terminal to a shell in the container, or Claude Code in a project when the app asked
/// for one. Every check runs first: a failed exit check starts nothing. On Unix this process is
/// replaced and returns only on failure; on Windows it returns the session's exit code.
pub fn enter(id: &str, token: Option<&str>) -> Result<i32, String> {
    let s = settings::load();
    let (name, spec) = account(&s, id)?;
    let launch = token.map(|t| take_launch(id, t)).transpose()?;
    if let Some(l) = &launch { check_launch(&spec, l)?; }
    let n = names(id)?;
    let (docker, rt) = runtime::connect()?;
    let effective = s.effective(id);
    let node = check_node(&effective)?;
    if let Some(reason) = s.account(id).blocked { return Err(blocked(&reason, s.account(id).blocked_soft)); }
    let environment = effective.environment.as_deref().and_then(|e| s.environments.get(e));

    say(&format!("[出口] 正在经本机中继检测“{name}”的出口…"));
    let (port, exit) = crate::launcher::verify_route(id, effective.environment.as_deref(), &node)?;
    let timezone = match effective.timezone.as_str() {
        "" | "proxy" => Some(crate::launcher::exit_timezone(&exit)?),
        "system" => host_timezone(),
        tz => Some(tz.to_string()),
    };
    let locale = environment.and_then(|e| posix_locale(&e.locale));

    let gateway_image = ensure_image(&docker, &GATEWAY)?;
    let base = wanted_image(&spec);
    if spec.image.is_empty() {
        ensure_image(&docker, &WORKSPACE)?;
    } else if !docker.ok(["image", "inspect", &base]) {
        return Err(format!("{} 中没有镜像 {base}。请先执行 docker pull {base}（或自行构建），也可以在容器设置中改用内置镜像。", rt.name));
    }
    prepare_tools(id)?;
    let (gateway_id, host_ip) = ensure_gateway(&docker, rt, id, &n, port, &gateway_image)?;
    ensure_workspace(&docker, id, &n, &spec, &base, &gateway_id)?;
    // Only the dashboard's limits depend on it; entering goes ahead either way.
    if let Err(e) = ensure_statusline(&claude_dir(id, &spec)) { crate::paths::append_log("hydra-error.log", &format!("status line: {e}")); }

    let folders: Vec<String> = spec.mounts.iter().map(|m| format!("/workspace/{}", m.name)).collect();
    say("====================================");
    say(&format!("  Claude-Hydra 容器：{name}"));
    say("====================================");
    say(&format!("[出口]  {}", crate::guard::describe(&exit)));
    say(&format!("[时区]  {}", timezone.as_deref().unwrap_or("镜像默认")));
    say(&format!("[语言]  {}", locale.as_deref().unwrap_or("镜像默认")));
    say(&format!("[目录]  {}", if folders.is_empty() { "（未共享文件夹）".into() } else { folders.join("  ") }));
    say("[网络]  仅经本账号的代理节点（UDP / IPv6 / 直连均被阻止）");
    say("[审计]  hydra-audit 检查出口、直连与泄露并打分；hydra-audit watch 持续监测");
    match &launch {
        Some(l) => {
            say(&format!("[项目]  {}", l.cwd));
            say(if l.resume.is_some() { "出口检测通过，正在继续会话。登录时显示的链接请在与该账号同出口的浏览器中打开。" }
                else { "出口检测通过，正在启动 Claude Code。登录时显示的链接请在与该账号同出口的浏览器中打开。" });
        }
        None => say("输入 claude 启动 Claude Code。登录时显示的链接请在与该账号同出口的浏览器中打开。"),
    }
    say("");

    let baseline = environment.and_then(|e| e.exit_baseline.as_ref()).map(|b| b.ip.clone());
    let audit = AuditContext {
        exits: std::iter::once(exit.ip.clone()).chain(baseline.filter(|b| *b != exit.ip)).collect(),
        country: exit.country.clone(),
        host_ip,
        relay_port: port,
    };
    let mut env = shell_env(&effective.env, timezone.as_deref(), locale.as_deref(), &audit);
    env.extend(terminal_env(&env, |k| std::env::var(k).ok().or_else(|| console_default(k))));
    let args = exec_args(&n.workspace, &env, launch.as_ref());
    #[cfg(unix)]
    { Err(docker.exec(args)) }
    #[cfg(not(unix))]
    { docker.interactive(args) }
}

/// The script (macOS) or shortcut (Windows) that enters the account.
fn write_entry(path: &Path, id: &str, token: Option<&str>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法获取当前程序路径：{e}"))?;
    let launch = token.map(|t| format!(" --launch {t}")).unwrap_or_default();
    crate::sys::write_shortcut(path, &exe, &format!("--container {id}{launch}"), "Claude Code 容器")
}

/// A terminal window that runs `enter`. On Windows this program opens its own console window
/// (in the system's default terminal), so nothing passes through a shell command line.
fn start_terminal(s: &Settings, dir: &Path, id: &str, token: Option<&str>) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = (s, dir);
        let exe = std::env::current_exe().map_err(|e| format!("无法获取当前程序路径：{e}"))?;
        let mut c = std::process::Command::new(exe);
        c.args(["--container", id]);
        if let Some(t) = token { c.args(["--launch", t]); }
        // CREATE_NEW_CONSOLE: a development build (console program) gets its own window too.
        c.creation_flags(0x0000_0010).spawn().map(|_| ()).map_err(|e| format!("无法打开容器终端：{e}"))
    }
    #[cfg(not(windows))]
    {
        let script = dir.join(match token { Some(t) => format!("enter-{t}.command"), None => "enter.command".into() });
        write_entry(&script, id, token)?;
        crate::terminal::open(&s.terminal, &script)
    }
}

/// Quick checks, then a terminal window that runs `enter`, in a project when `launch` says so.
pub fn open(id: &str, launch: Option<Launch>) -> Result<(), String> {
    let s = settings::load();
    let (_, spec) = account(&s, id)?;
    if let Some(l) = &launch { check_launch(&spec, l)?; }
    let (docker, _, _) = runtime::connect_with(s.docker_path.as_deref())?;
    check_node(&s.effective(id))?;
    if let Some(reason) = s.account(id).blocked { return Err(blocked(&reason, s.account(id).blocked_soft)); }
    if let Some(reason) = pending(&spec, live_workspaces(&docker)?.get(id)).0 { return Err(reason); }
    let dir = state_dir(id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建容器状态目录：{e}"))?;
    let Some(launch) = launch else { return start_terminal(&s, &dir, id, None) };
    clear_stale_launches(&dir);
    // The request travels in a file and the terminal gets only its token, so folder names with
    // spaces or quotes never pass through a shell command line.
    let token = uuid::Uuid::new_v4().simple().to_string();
    let request = serde_json::to_vec(&launch).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("launch-{token}.json")), request).map_err(|e| format!("无法保存打开请求：{e}"))?;
    start_terminal(&s, &dir, id, Some(&token))
}

// ---- projects --------------------------------------------------------------------------

/// What a terminal opened from the app does once every check has passed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Launch {
    /// Working folder in the container, inside a shared folder.
    pub cwd: String,
    /// Claude Code session to resume; a new session otherwise.
    pub resume: Option<String>,
}

/// A launch request is used once, by the terminal it was made for; one left behind (a terminal
/// that never started) is useless after a while.
const LAUNCH_TTL: std::time::Duration = std::time::Duration::from_secs(600);

fn clear_stale_launches(dir: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("launch-") || name.starts_with("enter-")) { continue; }
        let old = e.metadata().and_then(|m| m.modified()).ok()
            .and_then(|t| std::time::SystemTime::now().duration_since(t).ok()).is_some_and(|age| age > LAUNCH_TTL);
        if old { let _ = std::fs::remove_file(e.path()); }
    }
}

fn take_launch(id: &str, token: &str) -> Result<Launch, String> {
    if token.is_empty() || !token.chars().all(|c| c.is_ascii_hexdigit()) { return Err("打开请求无效。".into()); }
    let dir = state_dir(id);
    let file = dir.join(format!("launch-{token}.json"));
    let raw = std::fs::read(&file).map_err(|_| "打开请求已失效，请回到 Claude-Hydra 重新打开。".to_string())?;
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_file(dir.join(format!("enter-{token}.command")));
    serde_json::from_slice(&raw).map_err(|_| "打开请求无效。".to_string())
}

fn folder_paths(spec: &ContainerSpec) -> crate::claude_folders::Paths {
    crate::claude_folders::Paths::Container(spec.mounts.iter().map(|m| (m.name.clone(), PathBuf::from(&m.source))).collect())
}

/// A working folder must be inside a shared folder and exist here, or be `/workspace` itself
/// (where sessions started outside any shared folder resume); a session id is passed to a shell,
/// so it may only hold the characters of one.
fn check_launch(spec: &ContainerSpec, l: &Launch) -> Result<(), String> {
    let paths = folder_paths(spec);
    let inside = l.cwd.starts_with("/workspace/") && !l.cwd.split('/').any(|p| p == ".." || p == ".");
    let root = l.cwd == crate::claude_folders::WORKSPACE;
    if !root && (!inside || !paths.to_host(&l.cwd).is_some_and(|h| h.is_dir())) {
        return Err(format!("{} 不在这个容器的共享文件夹里，或文件夹已不存在。", l.cwd));
    }
    if l.resume.as_deref().is_some_and(|r| r.is_empty() || !r.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
        return Err("会话标识无效。".into());
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// The folder in the container.
    cwd: String,
    /// The same folder on this computer.
    host: String,
    sessions: usize,
    last_active_at: Option<u64>,
}

#[derive(Serialize)]
pub struct Projects {
    /// Folders Claude Code has worked in, most recent first.
    recent: Vec<Project>,
    /// The shared folders themselves, to start somewhere new.
    shared: Vec<Project>,
}

pub fn projects(id: &str) -> Result<Projects, String> {
    let (_, spec) = account(&settings::load(), id)?;
    let paths = folder_paths(&spec);
    let folder = crate::claude_folders::find(None, &format!("container:{id}"))?;
    let mut recent: BTreeMap<String, Project> = BTreeMap::new();
    for s in crate::code_sessions::list(&folder) {
        let Some(cwd) = s.cwd else { continue };
        let Some(host) = paths.to_host(&cwd).filter(|h| h.is_dir()) else { continue };
        let p = recent.entry(cwd.clone()).or_insert(Project { cwd, host: host.to_string_lossy().into_owned(), sessions: 0, last_active_at: None });
        p.sessions += 1;
        p.last_active_at = p.last_active_at.max(s.last_activity_at);
    }
    let mut recent: Vec<Project> = recent.into_values().collect();
    recent.sort_by_key(|p| std::cmp::Reverse(p.last_active_at));
    let shared = spec.mounts.iter().map(|m| {
        let cwd = format!("{}/{}", crate::claude_folders::WORKSPACE, m.name);
        let inside = |p: &&Project| p.cwd == cwd || p.cwd.starts_with(&format!("{cwd}/"));
        Project {
            sessions: recent.iter().filter(inside).map(|p| p.sessions).sum(),
            last_active_at: recent.iter().filter(inside).filter_map(|p| p.last_active_at).max(),
            host: m.source.clone(), cwd,
        }
    }).collect();
    Ok(Projects { recent, shared })
}

#[derive(Serialize)]
pub struct Subfolder { name: String, cwd: String }

/// The folders inside `cwd`, a folder of a shared folder; hidden ones are left out.
pub fn subfolders(id: &str, cwd: &str) -> Result<Vec<Subfolder>, String> {
    let (_, spec) = account(&settings::load(), id)?;
    check_launch(&spec, &Launch { cwd: cwd.to_string(), resume: None })?;
    let host = folder_paths(&spec).to_host(cwd).ok_or("文件夹不在共享范围内。")?;
    let mut out: Vec<Subfolder> = std::fs::read_dir(&host).map_err(|e| format!("无法读取文件夹：{e}"))?.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .map(|name| Subfolder { cwd: format!("{cwd}/{name}"), name }).collect();
    out.sort_by_key(|f| f.name.to_lowercase());
    Ok(out)
}

/// Stops the workspace and its gateway (open terminals in it end). Nothing is deleted.
pub fn stop(id: &str) -> Result<(), String> {
    account(&settings::load(), id)?;
    let n = names(id)?;
    let (docker, _) = runtime::connect()?;
    for name in [&n.workspace, &n.gateway] {
        if exists(&docker, name)? { docker.run(["stop", "-t", "5", name.as_str()])?; }
    }
    Ok(())
}

/// Removes the workspace and gateway so the next open creates them from the saved settings.
/// The home volume (Claude Code login) always stays. `keep_changes` snapshots the workspace
/// first so software installed inside it carries over.
pub fn rebuild(id: &str, keep_changes: bool) -> Result<(), String> {
    account(&settings::load(), id)?;
    let n = names(id)?;
    let (docker, _) = runtime::connect()?;
    if exists(&docker, &n.workspace)? {
        docker.run(["stop", "-t", "5", &n.workspace])?;
        if keep_changes {
            let label = docker.run(["container", "inspect", "--format", &format!("{{{{index .Config.Labels \"{LABEL_IMAGE}\"}}}}"), &n.workspace])?;
            docker.run(["commit", "--change", &format!("LABEL {LABEL_IMAGE}={label}"), &n.workspace, &n.snapshot])
                .map_err(|e| format!("无法保存容器快照：{e}"))?;
        }
        docker.run(["rm", &n.workspace])?;
    }
    if !keep_changes && docker.ok(["image", "inspect", &n.snapshot]) {
        docker.run(["image", "rm", &n.snapshot])?;
    }
    if exists(&docker, &n.gateway)? { docker.run(["rm", "-f", &n.gateway])?; }
    Ok(())
}

/// Deletes the account with its containers, home volume, snapshot and default Claude data
/// folder. Shared folders and a Claude data folder the user chose are untouched. Returns a note
/// when the runtime could not be cleaned up.
pub fn delete(id: &str) -> Result<Option<String>, String> {
    account(&settings::load(), id)?;
    let n = names(id)?;
    let note = match runtime::connect() {
        Ok((docker, _)) => {
            for name in [&n.workspace, &n.gateway] {
                if exists(&docker, name)? { docker.run(["rm", "-f", name.as_str()])?; }
            }
            if docker.ok(["volume", "inspect", &n.home]) { docker.run(["volume", "rm", &n.home])?; }
            if docker.ok(["image", "inspect", &n.snapshot]) { docker.run(["image", "rm", &n.snapshot])?; }
            None
        }
        Err(e) => {
            let rt = runtime::current().map(|r| r.name).unwrap_or("容器运行时");
            Some(format!("{e}\n已删除账号记录，但没能清理 {rt} 中的 {}、{}、数据卷 {}，请稍后在 {rt} 中手动删除。", n.workspace, n.gateway, n.home))
        }
    };
    settings::update(|s| s.remove_account(id))?;
    let _ = std::fs::remove_dir_all(state_dir(id));
    Ok(note)
}

pub fn create_shortcut(id: &str) -> Result<PathBuf, String> {
    let (name, _) = account(&settings::load(), id)?;
    let desktop = crate::paths::desktop_dir().ok_or("无法定位桌面目录。")?;
    let safe: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let path = desktop.join(format!("Claude Code - {safe}.{}", if cfg!(windows) { "lnk" } else { "command" }));
    write_entry(&path, id, None)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::Environment;
    use crate::node::NodeConfig;
    use crate::settings::AccountConfig;

    const HOME: &str = "/Users/u";
    const STORAGE: &str = "/Users/u/Library/Application Support/ClaudeHydra";

    fn settings_with_envs() -> Settings {
        let mut s = Settings::default();
        let node = NodeConfig { kind: "http".into(), host: "h".into(), port: 1, ..Default::default() };
        s.environments.insert("with-node".into(), Environment { name: "美国".into(), node: Some(node), ..Default::default() });
        s.environments.insert("direct".into(), Environment { name: "直连".into(), ..Default::default() });
        s
    }

    fn input(mounts: &[(&str, &str)]) -> ContainerInput {
        ContainerInput {
            name: "工作".into(),
            environment: "with-node".into(),
            spec: ContainerSpec { mounts: mounts.iter().map(|(s, n)| Mount { source: s.to_string(), name: n.to_string() }).collect(), ..Default::default() },
        }
    }

    fn mac() -> HostRules { HostRules { windows: false, home: HOME.into(), storage: STORAGE.into(), ssh_agent: true } }

    fn windows() -> HostRules {
        HostRules { windows: true, home: r"C:\Users\u".into(), storage: r"C:\Users\u\AppData\Roaming\ClaudeHydra".into(), ssh_agent: false }
    }

    fn validate(s: &Settings, input: &ContainerInput) -> Result<(String, String, ContainerSpec), String> {
        build(input, s, None, &mac(), |_| true)
    }

    #[test]
    fn valid_input_is_normalised() {
        // Arrange
        let s = settings_with_envs();
        let input = input(&[("/Users/u/code/app/", " app ")]);

        // Act
        let (name, env, spec) = validate(&s, &input).unwrap();

        // Assert
        assert_eq!((name.as_str(), env.as_str()), ("工作", "with-node"));
        assert_eq!(spec.mounts, vec![Mount { source: "/Users/u/code/app".into(), name: "app".into() }]);
    }

    #[test]
    fn environment_must_have_a_node() {
        // Arrange
        let s = settings_with_envs();
        let direct = ContainerInput { environment: "direct".into(), ..input(&[]) };
        let missing = ContainerInput { environment: "gone".into(), ..input(&[]) };

        // Act
        let results = [validate(&s, &direct), validate(&s, &missing)];

        // Assert
        assert!(results[0].as_ref().unwrap_err().contains("没有代理节点"));
        assert!(results[1].is_err());
    }

    #[test]
    fn broad_or_sensitive_folders_are_refused() {
        // Arrange
        let s = settings_with_envs();
        let sources = ["/", "/Users", HOME, "/Users/u/Library", STORAGE, "relative/path", "/Users/u/code/../.."];

        // Act
        let results: Vec<_> = sources.iter().map(|src| validate(&s, &input(&[(src, "x")]))).collect();

        // Assert
        for (src, r) in sources.iter().zip(results) { assert!(r.is_err(), "{src} should be refused"); }
    }

    #[test]
    fn windows_folders_are_drive_paths_outside_the_user_and_hydra_folders() {
        // Arrange
        let s = settings_with_envs();
        let host = windows();
        let accepted = [r"D:\Projects\app\", "d:/projects/a,b (1)", r"C:\Users\u\code\app"];
        let refused = [r"C:\", "C:", r"c:\users", r"C:\USERS\U", r"C:\Users\u\AppData", r"\\server\share\x", r"Projects\app", r"D:\a\..\..", "/Users/u/code"];

        // Act
        let ok: Vec<_> = accepted.iter().map(|src| build(&input(&[(src, "x")]), &s, None, &host, |_| true)).collect();
        let bad: Vec<_> = refused.iter().map(|src| build(&input(&[(src, "x")]), &s, None, &host, |_| true)).collect();

        // Assert
        let sources: Vec<String> = ok.into_iter().map(|r| r.unwrap().2.mounts[0].source.clone()).collect();
        assert_eq!(sources, [r"D:\Projects\app", r"d:\projects\a,b (1)", r"C:\Users\u\code\app"]);
        for (src, r) in refused.iter().zip(bad) { assert!(r.is_err(), "{src} should be refused"); }
    }

    #[test]
    fn windows_claude_folder_rules_ignore_case() {
        // Arrange
        let mut s = settings_with_envs();
        s.accounts.insert("ctr-1".into(), AccountConfig { label: Some("其他".into()), container: Some(ContainerSpec { claude_dir: r"D:\Claude\a".into(), ..Default::default() }), ..Default::default() });
        let with_dir = |dir: &str| ContainerInput { spec: ContainerSpec { claude_dir: dir.into(), ..Default::default() }, ..input(&[]) };

        // Act
        let results: Vec<_> = [r"c:\users\U\.CLAUDE", r"d:\claude\A", r"D:\Claude\b"].iter().map(|d| build(&with_dir(d), &s, None, &windows(), |_| true)).collect();

        // Assert
        assert!(results[0].is_err(), "the computer's own .claude");
        assert!(results[1].is_err(), "another account's folder in other case");
        assert_eq!(results[2].as_ref().unwrap().2.claude_dir, r"D:\Claude\b");
    }

    #[test]
    fn ssh_agent_needs_a_runtime_that_forwards_it() {
        // Arrange
        let s = settings_with_envs();
        let wanted = ContainerInput { spec: ContainerSpec { ssh_agent: true, ..Default::default() }, ..input(&[]) };

        // Act
        let on_mac = build(&wanted, &s, None, &mac(), |_| true);
        let on_windows = build(&wanted, &s, None, &windows(), |_| true);

        // Assert
        assert!(on_mac.is_ok());
        assert!(on_windows.unwrap_err().contains("SSH agent"));
    }

    #[test]
    fn mount_names_must_be_single_unique_folders() {
        // Arrange
        let s = settings_with_envs();
        let cases = [vec![("/Users/u/a", "..")], vec![("/Users/u/a", "x/y")], vec![("/Users/u/a", " ")], vec![("/Users/u/a", "x"), ("/Users/u/b", "x")]];

        // Act
        let results: Vec<_> = cases.iter().map(|m| validate(&s, &input(m))).collect();

        // Assert
        assert!(results.iter().all(Result::is_err));
    }

    #[test]
    fn claude_data_folder_is_private_to_one_account() {
        // Arrange
        let mut s = settings_with_envs();
        s.accounts.insert("ctr-1".into(), AccountConfig { label: Some("其他".into()), container: Some(ContainerSpec { claude_dir: "/Users/u/claude-a".into(), ..Default::default() }), ..Default::default() });
        let with_dir = |dir: &str| ContainerInput { spec: ContainerSpec { claude_dir: dir.into(), ..Default::default() }, ..input(&[]) };

        // Act
        let own = validate(&s, &with_dir("/Users/u/claude-b/"));
        let refused: Vec<_> = ["/Users/u/.claude", "/Users/u/.claude/projects", "/Users/u/claude-a", &format!("{STORAGE}/containers/ctr-1/claude"), HOME]
            .iter().map(|d| validate(&s, &with_dir(d))).collect();

        // Assert
        assert_eq!(own.unwrap().2.claude_dir, "/Users/u/claude-b");
        for r in refused { assert!(r.is_err()); }
    }

    #[test]
    fn default_claude_data_folder_is_inside_the_account_state() {
        // Arrange
        let custom = ContainerSpec { claude_dir: "/Users/u/claude".into(), ..Default::default() };

        // Act
        let (default_dir, custom_dir) = (claude_dir("ctr-1a", &ContainerSpec::default()), claude_dir("ctr-1a", &custom));

        // Assert
        assert_eq!(default_dir, state_dir("ctr-1a").join("claude"));
        assert_eq!(custom_dir, PathBuf::from("/Users/u/claude"));
    }

    #[test]
    fn missing_source_folder_is_refused() {
        // Arrange
        let s = settings_with_envs();

        // Act
        let r = build(&input(&[("/Users/u/gone", "gone")]), &s, None, &mac(), |_| false);

        // Assert
        assert!(r.unwrap_err().contains("找不到文件夹"));
    }

    #[test]
    fn names_are_unique_among_containers_except_itself() {
        // Arrange
        let mut s = settings_with_envs();
        s.accounts.insert("ctr-1".into(), AccountConfig { label: Some("工作".into()), container: Some(ContainerSpec::default()), ..Default::default() });
        s.accounts.insert("Account 2".into(), AccountConfig { label: Some("桌面".into()), ..Default::default() });

        // Act
        let clash = validate(&s, &input(&[]));
        let same = build(&input(&[]), &s, Some("ctr-1"), &mac(), |_| true);
        let desktop_name = validate(&s, &ContainerInput { name: "桌面".into(), ..input(&[]) });

        // Assert
        assert!(clash.is_err());
        assert!(same.is_ok());
        assert!(desktop_name.is_ok());
    }

    #[test]
    fn image_references_cannot_carry_options() {
        // Arrange
        let refs = ["cloaked-claude:latest", "ghcr.io/me/img@sha256:abc", "-v", "a b", "x;rm", ""];

        // Act
        let ok: Vec<bool> = refs.iter().map(|r| valid_image_ref(r)).collect();

        // Assert
        assert_eq!(ok, [true, true, false, false, false, false]);
    }

    #[test]
    fn docker_names_derive_from_the_id_only() {
        // Arrange
        let ids = ["ctr-18f3a2b", "Account 2", "ctr-", "ctr-AB", "ctr-a;b"];

        // Act
        let results: Vec<_> = ids.iter().map(|id| names(id)).collect();

        // Assert
        let n = results[0].as_ref().unwrap();
        assert_eq!((n.workspace.as_str(), n.gateway.as_str(), n.home.as_str()), ("claude-hydra-18f3a2b", "claude-hydra-18f3a2b-gw", "claude-hydra-18f3a2b-home"));
        assert!(results[1..].iter().all(Result::is_err));
    }

    #[test]
    fn fingerprint_follows_mounts_and_ssh_but_not_order_or_image() {
        // Arrange
        let a = Mount { source: "/a".into(), name: "a".into() };
        let b = Mount { source: "/b".into(), name: "b".into() };
        let base = ContainerSpec { mounts: vec![a.clone(), b.clone()], ..Default::default() };

        // Act
        let reordered = ContainerSpec { mounts: vec![b.clone(), a.clone()], image: "other:1".into(), ..Default::default() };
        let with_ssh = ContainerSpec { ssh_agent: true, ..base.clone() };
        let fewer = ContainerSpec { mounts: vec![a], ..Default::default() };
        let own_data = ContainerSpec { claude_dir: "/Users/u/claude".into(), ..base.clone() };

        // Assert
        assert_eq!(fingerprint(&base), fingerprint(&reordered));
        assert_ne!(fingerprint(&base), fingerprint(&with_ssh));
        assert_ne!(fingerprint(&base), fingerprint(&fewer));
        assert_ne!(fingerprint(&base), fingerprint(&own_data));
    }

    #[test]
    fn pending_reports_changed_settings_and_images() {
        // Arrange
        let spec = ContainerSpec { mounts: vec![Mount { source: "/a".into(), name: "a".into() }], ..Default::default() };
        let current = Live { running: true, spec: fingerprint(&spec), image: wanted_image(&spec) };
        let stale = Live { running: true, spec: "old".into(), image: "claude-hydra-workspace:old".into() };

        // Act
        let fresh = pending(&spec, Some(&current));
        let outdated = pending(&spec, Some(&stale));
        let absent = pending(&spec, None);

        // Assert
        assert_eq!(fresh, (None, false));
        assert!(outdated.0.is_some() && outdated.1);
        assert_eq!(absent, (None, false));
    }

    #[test]
    fn workspace_joins_the_gateway_without_network_powers() {
        // Arrange
        let n = names("ctr-1a").unwrap();
        let spec = ContainerSpec { mounts: vec![
            Mount { source: "/Users/u/Workspace Of Claude".into(), name: "Workspace Of Claude".into() },
            Mount { source: r"D:\a,b\x".into(), name: "a,b".into() },
        ], ssh_agent: true, ..Default::default() };

        let dirs = HostDirs { claude: Path::new("/Users/u/claude data"), tools: Path::new("/state/bin") };

        // Act
        let a = workspace_args("ctr-1a", &n, &spec, &dirs, "img:1", "img:1", "gw-id");

        // Assert
        let has = |pair: [&str; 2]| a.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(has(["--network", "container:gw-id"]));
        assert!(has(["--cap-drop", "ALL"]));
        assert!(has(["--security-opt", "no-new-privileges:true"]));
        assert!(has(["-v", "claude-hydra-1a-home:/root"]));
        assert!(has(["--mount", "type=bind,source=/Users/u/claude data,target=/root/.claude"]));
        assert!(has(["-e", "CLAUDE_CONFIG_DIR=/root/.claude"]));
        for k in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "NO_PROXY"] { assert!(has(["-e", &format!("{k}=")]), "{k} cannot come from the docker client"); }
        assert!(has(["--mount", "type=bind,source=/state/bin,target=/opt/claude-hydra/bin,readonly"]), "tools are read-only inside the container");
        assert!(has(["--mount", "type=bind,source=/Users/u/Workspace Of Claude,target=/workspace/Workspace Of Claude"]));
        assert!(has(["--mount", r#"type=bind,"source=D:\a,b\x","target=/workspace/a,b""#]), "commas are quoted as CSV fields");
        assert!(has(["-e", "SSH_AUTH_SOCK=/run/host-services/ssh-auth.sock"]));
        for cap in ["NET_ADMIN", "NET_RAW", "SYS_ADMIN"] { assert!(!a.iter().any(|x| x == cap), "{cap}"); }
        assert!(!a.iter().any(|x| x == "--privileged" || x.contains("docker.sock")));
        assert_eq!(a[a.len() - 2..], ["img:1".to_string(), "infinity".to_string()]);
    }

    #[test]
    fn gateway_reads_its_upstream_from_a_read_only_file() {
        // Arrange
        let n = names("ctr-1a").unwrap();

        // Act
        let a = gateway_args("ctr-1a", &n, Path::new("/state/ctr-1a/upstream"), "gw:1", None);
        let named = gateway_args("ctr-1a", &n, Path::new(r"C:\state\upstream"), "gw:1", runtime::DOCKER_DESKTOP.add_host);

        // Assert
        assert!(a.windows(2).any(|w| w == ["--mount", "type=bind,source=/state/ctr-1a/upstream,target=/etc/hydra-gateway/upstream,readonly"]));
        assert!(!a.iter().any(|x| x == "--add-host"));
        assert!(named.windows(2).any(|w| w == ["--add-host", "hydra-host:host-gateway"]));
        assert!(named.windows(2).any(|w| w == ["--mount", r"type=bind,source=C:\state\upstream,target=/etc/hydra-gateway/upstream,readonly"]));
        assert!(a.iter().any(|x| x == "--read-only"));
        assert!(a.windows(2).any(|w| w == ["--sysctl", "net.ipv6.conf.all.disable_ipv6=1"]));
        assert_eq!(a.last().unwrap(), "gw:1");
    }

    #[test]
    fn the_gateway_must_report_the_relay_it_was_given() {
        // Arrange
        let cases = [
            ("upstream=0.250.250.254:4321\n", "0.250.250.254", 4321, Some("0.250.250.254")),
            ("upstream=192.168.65.254:4321", "hydra-host", 4321, Some("192.168.65.254")),
            ("upstream=192.168.65.254:4000", "hydra-host", 4321, None),
            ("upstream=10.0.0.1:4321", "0.250.250.254", 4321, None),
            ("upstream=hydra-host:4321", "hydra-host", 4321, None),
            ("", "hydra-host", 4321, None),
        ];

        for (ready, host, port, expected) in cases {
            // Act
            let ip = gateway_upstream(ready, host, port);

            // Assert
            assert_eq!(ip.as_deref(), expected, "{ready:?}");
        }
    }

    #[test]
    fn the_status_line_is_set_once_and_never_over_the_accounts_own() {
        // Arrange
        let dir = std::env::temp_dir().join(format!("hydra-statusline-{}", uuid::Uuid::new_v4().simple()));
        let (fresh, own) = (dir.join("fresh"), dir.join("own"));
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(fresh.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        std::fs::write(own.join("settings.json"), r#"{"statusLine":{"type":"command","command":"~/my-line.sh"}}"#).unwrap();

        // Act
        ensure_statusline(&fresh).unwrap();
        ensure_statusline(&own).unwrap();
        ensure_statusline(&dir.join("missing")).unwrap_or_default();

        // Assert
        let read = |p: &Path| serde_json::from_slice::<serde_json::Value>(&std::fs::read(p.join("settings.json")).unwrap()).unwrap();
        assert_eq!((read(&fresh)["statusLine"]["command"].as_str(), read(&fresh)["model"].as_str()), (Some(STATUSLINE), Some("opus")));
        assert_eq!(read(&own)["statusLine"]["command"], "~/my-line.sh");
        assert!(STATUSLINE.starts_with(TOOLS_DIR) && TOOLS.iter().any(|(name, _)| STATUSLINE.ends_with(name)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_launch_starts_claude_in_its_folder_then_leaves_a_shell() {
        // Arrange
        let new = Launch { cwd: "/workspace/my app".into(), resume: None };
        let resume = Launch { cwd: "/workspace/app".into(), resume: Some("1b2c-3d".into()) };

        // Act
        let plain = exec_args("ws", &[], None);
        let started = exec_args("ws", &[], Some(&new));
        let resumed = exec_args("ws", &[], Some(&resume));

        // Assert
        assert_eq!(plain[3], "/workspace");
        assert!(!plain.last().unwrap().contains("claude;") && !plain.last().unwrap().contains("claude --"));
        assert_eq!(started[3], "/workspace/my app", "the folder is an argument of its own, never part of the shell command");
        assert!(started.last().unwrap().contains("claude; if command -v bash"));
        assert!(resumed.last().unwrap().contains("claude --resume 1b2c-3d; "));
    }

    #[test]
    fn launches_stay_inside_existing_shared_folders() {
        // Arrange
        let root = std::env::temp_dir().join(format!("hydra-launch-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        let spec = ContainerSpec { mounts: vec![Mount { source: root.to_string_lossy().into(), name: "app".into() }], ..Default::default() };
        let at = |cwd: &str, resume: Option<&str>| check_launch(&spec, &Launch { cwd: cwd.into(), resume: resume.map(Into::into) });

        // Act
        let inside = [at("/workspace/app", None), at("/workspace/app/src", Some("a1-b2")), at("/workspace", Some("a1"))];
        let outside = [at("/workspace/app/missing", None), at("/workspace/app/../app", None), at("/workspacex", None),
            at("/workspace/other", None), at("/workspace/app", Some("x; rm -rf /")), at("/workspace/app", Some(""))];

        // Assert
        assert!(inside.iter().all(|r| r.is_ok()));
        assert!(outside.iter().all(|r| r.is_err()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_variables_pass_through_unless_configured() {
        // Arrange
        let host = |k: &str| match k { "TERM" => Some("xterm-256color".to_string()), "TERM_PROGRAM" => Some("Apple_Terminal".to_string()), "COLORTERM" => Some(String::new()), _ => None };
        let configured = vec![("term".to_string(), "screen".to_string())];

        // Act
        let passed = terminal_env(&configured, host);

        // Assert
        assert_eq!(passed, vec![("TERM_PROGRAM".to_string(), "Apple_Terminal".to_string())]);
    }

    #[test]
    fn shell_env_drops_proxy_variables_and_applies_managed_ones() {
        // Arrange
        let env = BTreeMap::from([
            ("HTTPS_PROXY".to_string(), "http://127.0.0.1:1".to_string()),
            ("TZ".to_string(), "Asia/Tokyo".to_string()),
            ("CLAUDE_CONFIG_DIR".to_string(), "/tmp/elsewhere".to_string()),
            ("HYDRA_RELAY".to_string(), "http://attacker:1".to_string()),
            ("FOO".to_string(), "1".to_string()),
        ]);

        let audit = AuditContext { exits: vec!["203.0.113.9".into(), "203.0.113.10".into()], country: "US".into(), host_ip: "192.168.65.254".into(), relay_port: 4321 };

        // Act
        let managed = shell_env(&env, Some("America/Los_Angeles"), posix_locale("en-US").as_deref(), &audit);
        let unmanaged = shell_env(&env, None, None, &audit);

        // Assert
        let get = |v: &[(String, String)], k: &str| v.iter().filter(|(n, _)| n == k).map(|(_, x)| x.clone()).collect::<Vec<_>>();
        assert_eq!(get(&managed, "TZ"), ["America/Los_Angeles"]);
        assert_eq!(get(&managed, "LANG"), ["en_US.UTF-8"]);
        assert!(get(&managed, "HTTPS_PROXY").is_empty());
        assert!(get(&unmanaged, "CLAUDE_CONFIG_DIR").is_empty());
        assert_eq!(get(&managed, "HYDRA_EXPECTED_EXIT"), ["203.0.113.9,203.0.113.10"]);
        assert_eq!(get(&managed, "HYDRA_RELAY"), ["http://192.168.65.254:4321"], "the environment cannot point the audit elsewhere");
        assert_eq!(get(&managed, "HYDRA_HOST_IP"), ["192.168.65.254"]);
        assert_eq!(get(&managed, "HYDRA_EXPECTED_TZ"), ["America/Los_Angeles"]);
        assert!(get(&unmanaged, "HYDRA_EXPECTED_TZ").is_empty());
        assert_eq!(get(&managed, "FOO"), ["1"]);
        assert_eq!(get(&unmanaged, "TZ"), ["Asia/Tokyo"]);
        assert!(get(&unmanaged, "LANG").is_empty());
    }

    #[test]
    fn every_window_language_has_a_built_in_locale() {
        // Arrange
        let dockerfile = WORKSPACE.files[0].1;

        // Act
        let missing: Vec<&str> = crate::environment::LOCALES.iter().filter(|l| !l.is_empty())
            .filter(|l| !dockerfile.contains(posix_locale(l).unwrap().trim_end_matches(".UTF-8")))
            .copied().collect();

        // Assert
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn exec_passes_each_variable_as_one_argument() {
        // Arrange
        let env = vec![("A".to_string(), "x y; z".to_string())];

        // Act
        let a = exec_args("claude-hydra-1a", &env, None);

        // Assert
        assert!(a.windows(2).any(|w| w == ["-e", "A=x y; z"]));
        assert!(a.last().unwrap().starts_with("export PATH=\"/opt/claude-hydra/bin:"), "hydra-audit is on the shell's PATH");
        assert!(a.iter().any(|x| x == "claude-hydra-1a"));
    }

    #[test]
    fn audit_tool_is_a_python_script() {
        // Arrange
        let (name, body) = TOOLS[0];

        // Act
        let shebang = body.lines().next().unwrap_or("");

        // Assert
        assert_eq!(name, "hydra-audit");
        assert_eq!(shebang, "#!/usr/bin/env python3");
    }

    #[test]
    fn image_tags_follow_their_build_context() {
        // Arrange
        let changed = Image { repo: GATEWAY.repo, files: &[("Dockerfile", "FROM scratch")] };

        // Act
        let (a, b) = (GATEWAY.tag(), changed.tag());

        // Assert
        assert!(a.starts_with("claude-hydra-gateway:"));
        assert_ne!(a, b);
        assert_eq!(a, GATEWAY.tag());
    }

    #[test]
    fn live_listing_is_parsed_by_account() {
        // Arrange
        let out = "ctr-1\trunning\tabc\timg:1\nctr-2\texited\tdef\timg:2\n\tcreated\tx\ty\nbroken line";

        // Act
        let live = parse_live(out);

        // Assert
        assert_eq!(live.len(), 2);
        assert!(live["ctr-1"].running);
        assert!(!live["ctr-2"].running);
        assert_eq!(live["ctr-2"].spec, "def");
    }
}
