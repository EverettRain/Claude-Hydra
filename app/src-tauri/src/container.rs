//! Claude Code container accounts on the user's own OrbStack (macOS).
//!
//! A container account is an account entry with `AccountConfig::container` set. Instead of a
//! desktop profile it runs Claude Code in a Linux container, with the same network model as a
//! desktop account: its environment's node, its own local relay and the exit guard.
//!
//! Each container account has two containers and a volume:
//! - gateway `claude-hydra-<key>-gw` owns the network namespace. Every TCP connection becomes an
//!   HTTP CONNECT to the account's relay on the Mac (`orbstack::HOST_IP`), DNS goes over HTTPS
//!   through it, and everything else (UDP, IPv6, direct routes) is dropped
//!   (`container-images/gateway`). When the relay is paused by the guard, or Hydra is not
//!   running, the workspace has no network at all.
//! - workspace `claude-hydra-<key>` joins that namespace with no network capabilities and
//!   `no-new-privileges`; the user's folders appear under `/workspace/<name>`.
//! - Claude Code's data (login, settings, `.claude.json`) lives in a Mac folder mounted at
//!   `/root/.claude` with `CLAUDE_CONFIG_DIR` pointing there: by default the account's folder in
//!   Hydra's data directory, or a folder the user picks. Visible and backed up on the Mac, kept
//!   across rebuilds and OrbStack resets.
//! - volume `claude-hydra-<key>-home` is the rest of `/root` (shell history and the like).
//! - Hydra's in-container tools (`hydra-audit`) are mounted read-only at `/opt/claude-hydra/bin`,
//!   so the container cannot alter the audit that judges it.
//!
//! Hydra opens a terminal running `claude-hydra --container <id>` (`enter`). It checks the route
//! through the relay, builds missing images with visible progress, starts both containers,
//! checks that the workspace really is in the gateway's namespace, and then replaces itself
//! with a shell in the container that has the environment's timezone, language and variables.
use crate::orbstack::{self, Docker};
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

/// Linux capabilities the workspace keeps (file ownership for installs and builds); nothing
/// network-related, so the gateway's firewall cannot be changed from inside.
const WORKSPACE_CAPS: [&str; 7] = ["CHOWN", "DAC_OVERRIDE", "FOWNER", "FSETID", "SETUID", "SETGID", "SYS_CHROOT"];
/// OrbStack forwards the Mac's SSH agent at this path.
const SSH_SOCK: &str = "/run/host-services/ssh-auth.sock";
/// Where Claude Code keeps its data in the container (`CLAUDE_CONFIG_DIR`).
const CLAUDE_DIR: &str = "/root/.claude";
const TOOLS_DIR: &str = "/opt/claude-hydra/bin";
/// Container layout of this Hydra version; a change makes existing workspaces need a rebuild.
const LAYOUT: &str = "2";

/// Tools installed into every workspace (read-only).
const TOOLS: [(&str, &str); 1] = [("hydra-audit", include_str!("../container-images/tools/hydra-audit"))];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Mount {
    /// Absolute folder on the Mac.
    pub source: String,
    /// Folder name under `/workspace` in the container.
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContainerSpec {
    /// "" = Hydra's built-in workspace image, else an image the user has in OrbStack.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<Mount>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ssh_agent: bool,
    /// Mac folder for Claude Code's data; "" = the account's folder in Hydra's data directory.
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
    /// The Mac folder that actually holds Claude Code's data.
    pub claude_dir_path: String,
    /// "running" | "stopped" | "absent" (created on first open) | "unknown" (OrbStack unavailable)
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

/// A Mac folder that may be shared into a container. Refuses `/`, the home folder and any folder
/// containing Hydra's own data (it holds node credentials).
fn check_source(source: &str, home: &Path, storage: &Path) -> Result<String, String> {
    let trimmed = source.trim();
    let source = if trimmed.len() > 1 { trimmed.trim_end_matches('/') } else { trimmed };
    if !source.starts_with('/') { return Err(format!("请填写完整的文件夹路径：{source}")); }
    if source.contains([':', '\n', '\r', '\0']) { return Err(format!("文件夹路径不能包含冒号或换行：{source}")); }
    let path = Path::new(source);
    if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) { return Err(format!("文件夹路径不能包含“..”：{source}")); }
    if home.starts_with(path) || storage.starts_with(path) {
        return Err(format!("不能共享 {source}：它包含用户主目录或 Claude-Hydra 的数据目录（含节点凭据）。请选择具体的项目文件夹。"));
    }
    Ok(source.to_string())
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
fn build(input: &ContainerInput, s: &Settings, id: Option<&str>, home: &Path, storage: &Path, is_dir: impl Fn(&str) -> bool) -> Result<(String, String, ContainerSpec), String> {
    let name = input.name.trim();
    if name.is_empty() { return Err("请填写名称。".into()); }
    if name.chars().count() > 40 { return Err("名称最多 40 个字符。".into()); }
    if container_names(s, id).iter().any(|n| n.eq_ignore_ascii_case(name)) { return Err(format!("已有名为“{name}”的容器账号。")); }
    let env = s.environments.get(&input.environment).ok_or("请选择一个环境配置。")?;
    if env.node.is_none() { return Err(format!("环境“{}”没有代理节点。容器只经代理节点联网，请选择带节点的环境。", env.name)); }
    let image = input.spec.image.trim().to_string();
    if !image.is_empty() && !valid_image_ref(&image) { return Err(format!("镜像名称无效：{image}")); }
    if input.spec.mounts.len() > MAX_MOUNTS { return Err(format!("最多共享 {MAX_MOUNTS} 个文件夹。")); }
    let mut mounts: Vec<Mount> = Vec::new();
    for m in &input.spec.mounts {
        let source = check_source(&m.source, home, storage)?;
        if !is_dir(&source) { return Err(format!("找不到文件夹：{source}")); }
        let name = check_mount_name(&m.name)?;
        if mounts.iter().any(|o| o.name == name) { return Err(format!("容器内有两个文件夹都叫“{name}”，请改名。")); }
        mounts.push(Mount { source, name });
    }
    let claude_dir = match input.spec.claude_dir.trim() {
        "" => String::new(),
        dir => {
            let dir = check_source(dir, home, storage)?;
            let path = Path::new(&dir);
            if path.starts_with(home.join(".claude")) {
                return Err("不能使用 Mac 自己的 ~/.claude：那里是本机 Claude Code 的登录与设置。请另选一个文件夹。".into());
            }
            if path.starts_with(storage) {
                return Err("请选择 Claude-Hydra 数据目录以外的文件夹；不选则使用数据目录中的默认位置。".into());
            }
            if !is_dir(&dir) { return Err(format!("找不到文件夹：{dir}")); }
            let taken = s.accounts.iter().any(|(k, a)| Some(k.as_str()) != id && a.container.as_ref().is_some_and(|c| c.claude_dir == dir));
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

/// The Mac folder holding a container account's Claude Code data.
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
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    let (name, environment, spec) = build(input, &s, id, &home, &crate::paths::storage_root(), |p| Path::new(p).is_dir())?;
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
    let live = orbstack::connect().and_then(|d| live_workspaces(&d)).ok();
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

fn gateway_args(id: &str, n: &Names, upstream_file: &Path, image: &str) -> Vec<String> {
    let mut a: Vec<String> = ["run", "-d", "--init", "--name", &n.gateway, "--hostname", "workstation"].map(String::from).to_vec();
    a.extend(["--label".into(), format!("{LABEL_ID}={id}"), "--label".into(), format!("{LABEL_ROLE}=gateway")]);
    a.extend([
        "--cap-drop", "ALL", "--cap-add", "NET_ADMIN", "--cap-add", "NET_BIND_SERVICE",
        "--security-opt", "no-new-privileges:true", "--read-only",
        "--tmpfs", "/run:rw,nosuid,nodev", "--tmpfs", "/tmp:rw,nosuid,nodev",
        "--sysctl", "net.ipv6.conf.all.disable_ipv6=1",
    ].map(String::from));
    a.extend(["-v".into(), format!("{}:/etc/hydra-gateway/upstream:ro", upstream_file.display()), image.to_string()]);
    a
}

/// Mac folders mounted into the workspace besides the shared ones.
struct HostDirs<'a> {
    claude: &'a Path,
    tools: &'a Path,
}

fn workspace_args(id: &str, n: &Names, spec: &ContainerSpec, dirs: &HostDirs, image: &str, image_label: &str, gateway_id: &str) -> Vec<String> {
    let mut a: Vec<String> = ["create", "--init", "--name", &n.workspace].map(String::from).to_vec();
    a.extend(["--network".into(), format!("container:{gateway_id}"), "--cap-drop".into(), "ALL".into()]);
    for cap in WORKSPACE_CAPS { a.extend(["--cap-add".into(), cap.into()]); }
    a.extend(["--security-opt".into(), "no-new-privileges:true".into()]);
    for (k, v) in [(LABEL_ID, id), (LABEL_ROLE, "workspace"), (LABEL_SPEC, &fingerprint(spec)), (LABEL_IMAGE, image_label)] {
        a.extend(["--label".into(), format!("{k}={v}")]);
    }
    a.extend(["-v".into(), format!("{}:/root", n.home)]);
    a.extend(["-v".into(), format!("{}:{CLAUDE_DIR}", dirs.claude.display()), "-e".into(), format!("CLAUDE_CONFIG_DIR={CLAUDE_DIR}")]);
    a.extend(["-v".into(), format!("{}:{TOOLS_DIR}:ro", dirs.tools.display())]);
    for m in &spec.mounts { a.extend(["-v".into(), format!("{}:/workspace/{}", m.source, m.name)]); }
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
    out.push(("HYDRA_HOST_IP".into(), orbstack::HOST_IP.into()));
    out.push(("HYDRA_RELAY".into(), format!("http://{}:{}", orbstack::HOST_IP, audit.relay_port)));
    out
}

fn exec_args(workspace: &str, env: &[(String, String)]) -> Vec<String> {
    let mut a: Vec<String> = ["exec", "-it", "-w", "/workspace"].map(String::from).to_vec();
    for (k, v) in env { a.extend(["-e".into(), format!("{k}={v}")]); }
    let shell = format!("export PATH=\"{TOOLS_DIR}:$PATH\"; if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi");
    a.extend([workspace.to_string(), "sh".into(), "-c".into(), shell]);
    a
}

/// The Mac's own timezone, for environments set to follow the system.
fn host_timezone() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let tz = target.to_string_lossy().split_once("zoneinfo/")?.1.to_string();
    crate::node::valid_timezone(&tz).then_some(tz)
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

/// Creates or starts the account's gateway, pointed at relay `port`; returns its container id.
fn ensure_gateway(docker: &Docker, id: &str, n: &Names, port: u16, image: &str) -> Result<String, String> {
    let upstream = format!("{}:{port}", orbstack::HOST_IP);
    let dir = state_dir(id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建容器状态目录：{e}"))?;
    let file = dir.join("upstream");
    if file.to_string_lossy().contains(':') { return Err(format!("Claude-Hydra 数据目录的路径不能包含冒号：{}", dir.display())); }
    if std::fs::read_to_string(&file).ok().as_deref() != Some(upstream.as_str()) {
        std::fs::write(&file, &upstream).map_err(|e| format!("无法写入网关配置：{e}"))?;
    }
    if !exists(docker, &n.gateway)? {
        say("[网关] 创建网络网关…");
        docker.run(gateway_args(id, n, &file, image))?;
    } else if !running(docker, &n.gateway)? {
        docker.run(["start", &n.gateway])?;
    }
    let expected = format!("upstream={upstream}");
    if wait_gateway(docker, &n.gateway)? != expected {
        // The relay moved to another port; a restart makes the gateway read the new one, but
        // a running workspace would stay in the old network namespace.
        if exists(docker, &n.workspace)? && running(docker, &n.workspace)? {
            return Err("本机中继的端口已变化。请先在 Claude-Hydra 的容器页停止此容器，再重新打开（数据不会丢失）。".into());
        }
        docker.run(["restart", &n.gateway])?;
        if wait_gateway(docker, &n.gateway)? != expected { return Err("网关没有使用当前的本机中继，工作容器不会启动。".into()); }
    }
    docker.run(["container", "inspect", "--format", "{{.Id}}", &n.gateway])
}

/// Writes Hydra's in-container tools to the account's state folder (mounted read-only), so a
/// Hydra update brings the new versions without a rebuild.
fn prepare_tools(id: &str) -> Result<PathBuf, String> {
    let dir = state_dir(id).join("bin");
    if dir.to_string_lossy().contains(':') { return Err(format!("Claude-Hydra 数据目录的路径不能包含冒号：{}", dir.display())); }
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

/// The Mac folder for Claude Code's data: the default one is created owner-only (it holds the
/// login in plain text); a chosen one must exist.
fn prepare_claude_dir(id: &str, spec: &ContainerSpec) -> Result<PathBuf, String> {
    let dir = claude_dir(id, spec);
    if dir.to_string_lossy().contains(':') { return Err(format!("Claude 数据文件夹的路径不能包含冒号：{}", dir.display())); }
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
        return Err("Mac 的 SSH agent 不可用。请确认 ssh-agent 正在运行并已添加密钥，或在容器设置中关闭 SSH agent 转发。".into());
    }
    Ok(())
}

/// Terminal entry (`claude-hydra --container <id>`): prepares everything, then replaces this
/// process with a shell in the container. Returns only on failure (or, off Unix, on exit).
pub fn enter(id: &str) -> Result<i32, String> {
    let s = settings::load();
    let (name, spec) = account(&s, id)?;
    let n = names(id)?;
    let docker = orbstack::connect()?;
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
        return Err(format!("OrbStack 中没有镜像 {base}。请先执行 docker pull {base}（或自行构建），也可以在容器设置中改用内置镜像。"));
    }
    prepare_tools(id)?;
    let gateway_id = ensure_gateway(&docker, id, &n, port, &gateway_image)?;
    ensure_workspace(&docker, id, &n, &spec, &base, &gateway_id)?;

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
    say("输入 claude 启动 Claude Code。登录时显示的链接请在与该账号同出口的浏览器中打开。");
    say("");

    let baseline = environment.and_then(|e| e.exit_baseline.as_ref()).map(|b| b.ip.clone());
    let audit = AuditContext {
        exits: std::iter::once(exit.ip.clone()).chain(baseline.filter(|b| *b != exit.ip)).collect(),
        country: exit.country.clone(),
        relay_port: port,
    };
    let args = exec_args(&n.workspace, &shell_env(&effective.env, timezone.as_deref(), locale.as_deref(), &audit));
    #[cfg(unix)]
    { Err(docker.exec(args)) }
    #[cfg(not(unix))]
    { docker.attached(args).map(|ok| if ok { 0 } else { 1 }) }
}

/// The script a terminal runs to enter the account (also used for desktop shortcuts).
fn write_entry(path: &Path, id: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法获取当前程序路径：{e}"))?;
    crate::sys::write_shortcut(path, &exe, &format!("--container {id}"), "")
}

/// Quick checks, then a terminal window that runs `enter`.
pub fn open(id: &str) -> Result<(), String> {
    let s = settings::load();
    let (_, spec) = account(&s, id)?;
    let (docker, _) = orbstack::connect_with(s.docker_path.as_deref())?;
    check_node(&s.effective(id))?;
    if let Some(reason) = s.account(id).blocked { return Err(blocked(&reason, s.account(id).blocked_soft)); }
    if let Some(reason) = pending(&spec, live_workspaces(&docker)?.get(id)).0 { return Err(reason); }
    let dir = state_dir(id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建容器状态目录：{e}"))?;
    let script = dir.join("enter.command");
    write_entry(&script, id)?;
    crate::terminal::open(&s.terminal, &script)
}

/// Stops the workspace and its gateway (open terminals in it end). Nothing is deleted.
pub fn stop(id: &str) -> Result<(), String> {
    account(&settings::load(), id)?;
    let n = names(id)?;
    let docker = orbstack::connect()?;
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
    let docker = orbstack::connect()?;
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
/// folder. Shared folders and a Claude data folder the user chose are untouched. Returns a note when OrbStack could not be cleaned up.
pub fn delete(id: &str) -> Result<Option<String>, String> {
    account(&settings::load(), id)?;
    let n = names(id)?;
    let note = match orbstack::connect() {
        Ok(docker) => {
            for name in [&n.workspace, &n.gateway] {
                if exists(&docker, name)? { docker.run(["rm", "-f", name.as_str()])?; }
            }
            if docker.ok(["volume", "inspect", &n.home]) { docker.run(["volume", "rm", &n.home])?; }
            if docker.ok(["image", "inspect", &n.snapshot]) { docker.run(["image", "rm", &n.snapshot])?; }
            None
        }
        Err(e) => Some(format!("{e}\n已删除账号记录，但没能清理 OrbStack 中的 {}、{}、数据卷 {}，请稍后在 OrbStack 中手动删除。", n.workspace, n.gateway, n.home)),
    };
    settings::update(|s| s.remove_account(id))?;
    let _ = std::fs::remove_dir_all(state_dir(id));
    Ok(note)
}

pub fn create_shortcut(id: &str) -> Result<PathBuf, String> {
    let (name, _) = account(&settings::load(), id)?;
    let desktop = crate::paths::desktop_dir().ok_or("无法定位桌面目录。")?;
    let safe: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let path = desktop.join(format!("Claude Code - {safe}.command"));
    write_entry(&path, id)?;
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

    fn validate(s: &Settings, input: &ContainerInput) -> Result<(String, String, ContainerSpec), String> {
        build(input, s, None, Path::new(HOME), Path::new(STORAGE), |_| true)
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
        let sources = ["/", "/Users", HOME, "/Users/u/Library", STORAGE, "relative/path", "/Users/u/a:b", "/Users/u/code/../.."];

        // Act
        let results: Vec<_> = sources.iter().map(|src| validate(&s, &input(&[(src, "x")]))).collect();

        // Assert
        for (src, r) in sources.iter().zip(results) { assert!(r.is_err(), "{src} should be refused"); }
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
        let r = build(&input(&[("/Users/u/gone", "gone")]), &s, None, Path::new(HOME), Path::new(STORAGE), |_| false);

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
        let same = build(&input(&[]), &s, Some("ctr-1"), Path::new(HOME), Path::new(STORAGE), |_| true);
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
        let spec = ContainerSpec { mounts: vec![Mount { source: "/Users/u/Workspace Of Claude".into(), name: "Workspace Of Claude".into() }], ssh_agent: true, ..Default::default() };

        let dirs = HostDirs { claude: Path::new("/Users/u/claude data"), tools: Path::new("/state/bin") };

        // Act
        let a = workspace_args("ctr-1a", &n, &spec, &dirs, "img:1", "img:1", "gw-id");

        // Assert
        let has = |pair: [&str; 2]| a.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(has(["--network", "container:gw-id"]));
        assert!(has(["--cap-drop", "ALL"]));
        assert!(has(["--security-opt", "no-new-privileges:true"]));
        assert!(has(["-v", "claude-hydra-1a-home:/root"]));
        assert!(has(["-v", "/Users/u/claude data:/root/.claude"]));
        assert!(has(["-e", "CLAUDE_CONFIG_DIR=/root/.claude"]));
        assert!(has(["-v", "/state/bin:/opt/claude-hydra/bin:ro"]), "tools are read-only inside the container");
        assert!(has(["-v", "/Users/u/Workspace Of Claude:/workspace/Workspace Of Claude"]));
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
        let a = gateway_args("ctr-1a", &n, Path::new("/state/ctr-1a/upstream"), "gw:1");

        // Assert
        assert!(a.windows(2).any(|w| w == ["-v", "/state/ctr-1a/upstream:/etc/hydra-gateway/upstream:ro"]));
        assert!(a.iter().any(|x| x == "--read-only"));
        assert!(a.windows(2).any(|w| w == ["--sysctl", "net.ipv6.conf.all.disable_ipv6=1"]));
        assert_eq!(a.last().unwrap(), "gw:1");
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

        let audit = AuditContext { exits: vec!["203.0.113.9".into(), "203.0.113.10".into()], country: "US".into(), relay_port: 4321 };

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
        assert_eq!(get(&managed, "HYDRA_RELAY"), [format!("http://{}:4321", orbstack::HOST_IP)], "the environment cannot point the audit elsewhere");
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
        let a = exec_args("claude-hydra-1a", &env);

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
