//! Reusable environment configs: proxy node, timezone, window language, launch args and env vars.
//!
//! An environment is saved once and assigned to any number of accounts; an account without one
//! connects directly with the system timezone. Older versions kept these settings per account;
//! `migrate` moves them into one environment per account.
use crate::node::{self, NodeConfig};
use crate::profiles;
use crate::settings::{self, Settings};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const LOCALES: [&str; 7] = ["", "en-US", "en-GB", "zh-CN", "zh-TW", "ja-JP", "ko-KR"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub name: String,
    /// Password sealed at rest (see `node::seal`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeConfig>,
    /// "" / "proxy" follow the node's exit region, "system" keeps the OS zone, else an IANA name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timezone: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub locale: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub args: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Result of the last manual connectivity check, shown again when the environment is edited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check: Option<NodeCheck>,
}

/// A successful connectivity check: exit info plus the residential/datacenter and risk notes.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeCheck {
    pub exit: node::ExitInfo,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub risk: String,
}

impl Environment {
    pub fn node(&self) -> Option<NodeConfig> {
        let mut n = self.node.clone()?;
        n.password = node::unseal(&n.password);
        n.validated().ok()
    }

    /// Window language plus the user's extra switches.
    pub fn launch_args(&self) -> String {
        let lang = (!self.locale.is_empty()).then(|| format!("--lang={}", self.locale));
        lang.into_iter().chain([self.args.replace(['\r', '\n'], " ").trim().to_string()]).filter(|a| !a.is_empty()).collect::<Vec<_>>().join(" ")
    }

    /// Whether this environment changes how Claude is launched at all.
    pub fn customizes_launch(&self) -> bool {
        self.node.is_some() || !matches!(self.timezone.as_str(), "" | "proxy" | "system") || !self.launch_args().is_empty() || !self.env.is_empty()
    }
}

/// What the UI edits: plain-text password and env lines.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInput {
    pub name: String,
    #[serde(default)]
    pub node: Option<NodeConfig>,
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub locale: String,
    #[serde(default)]
    pub args: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub last_check: Option<NodeCheck>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentView {
    pub id: String,
    #[serde(flatten)]
    pub input: EnvironmentInput,
    /// Names of the accounts that use this environment.
    pub used_by: Vec<String>,
}

fn account_name(s: &Settings, id: &str) -> String {
    s.account(id).label.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| id.to_string())
}

fn view(s: &Settings, id: &str, e: &Environment) -> EnvironmentView {
    EnvironmentView {
        id: id.to_string(),
        input: EnvironmentInput {
            name: e.name.clone(),
            node: e.node(),
            timezone: if e.timezone.is_empty() { "proxy".into() } else { e.timezone.clone() },
            locale: e.locale.clone(),
            args: e.args.clone(),
            env: e.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n"),
            last_check: e.last_check.clone(),
        },
        used_by: s.accounts.iter().filter(|(_, a)| a.environment.as_deref() == Some(id)).map(|(aid, _)| account_name(s, aid)).collect(),
    }
}

pub fn list() -> Vec<EnvironmentView> {
    let s = settings::load();
    let mut out: Vec<_> = s.environments.iter().map(|(id, e)| view(&s, id, e)).collect();
    out.sort_by(|a, b| a.input.name.cmp(&b.input.name));
    out
}

fn new_id(s: &Settings) -> String {
    let mut n = crate::paths::now_millis();
    loop {
        let id = format!("env-{n:x}");
        if !s.environments.contains_key(&id) { return id; }
        n += 1;
    }
}

/// Validates input into a stored environment (password sealed).
fn build(input: &EnvironmentInput) -> Result<Environment, String> {
    let name = input.name.trim();
    if name.is_empty() { return Err("请填写环境名称。".into()); }
    if name.chars().count() > 40 { return Err("环境名称最多 40 个字符。".into()); }
    let node = input.node.as_ref().map(|n| n.validated()).transpose()?;
    let timezone = profiles::normalize_timezone(&input.timezone)?;
    if !LOCALES.contains(&input.locale.as_str()) { return Err("不支持此窗口语言。".into()); }
    let args = input.args.replace(['\r', '\n'], " ").trim().to_string();
    if node.is_some() && node::args_conflict(&args) { return Err("请移除额外启动参数中的代理开关，代理由节点设置。".into()); }
    let env = profiles::parse_env_lines(&input.env)?;
    // A check result only describes a node; drop it when the environment has none.
    let last_check = node.as_ref().and(input.last_check.clone());
    Ok(Environment { name: name.to_string(), node: node.map(|mut n| { n.password = node::seal(&n.password); n }), timezone, locale: input.locale.clone(), args, env, last_check })
}

/// Creates (`id` = None) or updates an environment and returns its id.
pub fn save(install: &crate::install::Install, id: Option<&str>, input: &EnvironmentInput) -> Result<String, String> {
    let env = build(input)?;
    let mut s = settings::load();
    if let Some(id) = id { if !s.environments.contains_key(id) { return Err("这个环境配置已不存在。".into()); } }
    if s.environments.iter().any(|(k, e)| Some(k.as_str()) != id && e.name == env.name) { return Err(format!("已有名为“{}”的环境配置。", env.name)); }
    if install.is_msix() && env.customizes_launch() && id.is_some_and(|id| s.account(crate::paths::DEFAULT_ID).environment.as_deref() == Some(id)) {
        return Err("商店版默认账号正在使用此环境，而它无法应用代理、时区或启动参数。请先为默认账号取消此环境。".into());
    }
    let id = id.map(str::to_string).unwrap_or_else(|| new_id(&s));
    s.environments.insert(id.clone(), env);
    settings::save(&s)?;
    Ok(id)
}

/// Deletes an environment; accounts using it fall back to a direct connection.
pub fn delete(id: &str) -> Result<(), String> {
    settings::update(|s| {
        s.environments.remove(id);
        for acc in s.accounts.values_mut().filter(|a| a.environment.as_deref() == Some(id)) {
            acc.environment = None;
            acc.relay_port = None;
            acc.last_exit = None;
        }
    })
}

/// One-time move of per-account settings (v3.0) into environments.
pub fn migrate() {
    let mut s = settings::load();
    let pending: Vec<String> = s.accounts.iter()
        .filter(|(_, a)| a.environment.is_none() && (a.node.is_some() || !a.proxy_endpoint.is_empty() || !a.args.is_empty() || !a.env.is_empty() || !matches!(a.timezone.as_str(), "" | "proxy")))
        .map(|(id, _)| id.clone()).collect();
    if pending.is_empty() { return; }
    for id in pending {
        let acc = s.account(&id);
        // Keep the sealed password as-is; convert a legacy entrance and seal nothing new.
        let node = acc.node.clone().or_else(|| NodeConfig::from_legacy(&acc.proxy_endpoint));
        let mut locale = String::new();
        let args: Vec<&str> = acc.args.split_whitespace().filter(|a| {
            match a.strip_prefix("--lang=") { Some(l) if LOCALES.contains(&l) && locale.is_empty() => { locale = l.to_string(); false } _ => true }
        }).collect();
        let mut name = format!("{} 的环境", account_name(&s, &id));
        while s.environments.values().any(|e| e.name == name) { name.push('′'); }
        let env = Environment { name, node, timezone: acc.timezone.clone(), locale, args: args.join(" "), env: acc.env.clone(), last_check: None };
        let env_id = new_id(&s);
        s.environments.insert(env_id.clone(), env);
        let a = s.account_mut(&id);
        a.environment = Some(env_id);
        a.node = None;
        a.proxy_endpoint.clear();
        a.args.clear();
        a.env.clear();
        a.timezone.clear();
    }
    let _ = settings::save(&s);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AccountConfig;

    #[test]
    fn effective_config_prefers_assigned_environment() {
        let mut s = Settings::default();
        s.environments.insert("e1".into(), Environment {
            name: "东京".into(),
            node: Some(NodeConfig { kind: "socks5".into(), host: "jp.example".into(), port: 1080, password: node::seal("pw"), username: "u".into(), ..Default::default() }),
            timezone: "Asia/Tokyo".into(), locale: "ja-JP".into(), args: "--disable-gpu".into(),
            env: BTreeMap::from([("FOO".into(), "1".into())]), last_check: None,
        });
        s.accounts.insert("a".into(), AccountConfig { environment: Some("e1".into()), args: "--ignored".into(), ..Default::default() });
        let e = s.effective("a");
        assert_eq!(e.node.unwrap().password, "pw");
        assert_eq!(e.timezone, "Asia/Tokyo");
        assert_eq!(e.args, "--lang=ja-JP --disable-gpu");
        assert_eq!(e.env.get("FOO").map(String::as_str), Some("1"));
        // A missing environment means a plain, direct launch.
        s.accounts.insert("b".into(), AccountConfig { environment: Some("gone".into()), ..Default::default() });
        assert!(s.effective("b").node.is_none());
    }

    #[test]
    fn build_validates_input() {
        assert!(build(&EnvironmentInput { name: " ".into(), ..Default::default() }).is_err());
        assert!(build(&EnvironmentInput { name: "x".into(), locale: "xx".into(), ..Default::default() }).is_err());
        let node = NodeConfig { kind: "http".into(), host: "h".into(), port: 1, ..Default::default() };
        assert!(build(&EnvironmentInput { name: "x".into(), node: Some(node.clone()), args: "--proxy-server=a".into(), ..Default::default() }).is_err());
        let ok = build(&EnvironmentInput { name: "x".into(), node: Some(NodeConfig { password: "p".into(), username: "u".into(), ..node }), env: "A=1".into(), ..Default::default() }).unwrap();
        assert_ne!(ok.node.unwrap().password, "p");
        assert_eq!(ok.env.get("A").map(String::as_str), Some("1"));
    }

    #[test]
    fn last_check_kept_only_with_node() {
        let check = NodeCheck { exit: node::ExitInfo { ip: "1.2.3.4".into(), timezone: "Asia/Tokyo".into(), checked_at: 1, ..Default::default() }, quality: None, risk: "r".into() };
        let node = NodeConfig { kind: "http".into(), host: "h".into(), port: 1, ..Default::default() };
        let with = build(&EnvironmentInput { name: "x".into(), node: Some(node), last_check: Some(check.clone()), ..Default::default() }).unwrap();
        assert_eq!(with.last_check, Some(check.clone()));
        let without = build(&EnvironmentInput { name: "x".into(), last_check: Some(check), ..Default::default() }).unwrap();
        assert_eq!(without.last_check, None);
    }
}
