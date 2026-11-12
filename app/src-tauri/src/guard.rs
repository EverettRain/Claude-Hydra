//! Exit rules and the network kill switch.
//!
//! Every verified exit (at launch, in the account browser and periodically while an account
//! uses its relay) is checked against the account's environment:
//! - mainland-China exits are refused unless the environment allows them;
//! - an optional rule pins the exit to the first one seen (`exit_baseline`): same IP, city,
//!   region or country.
//!
//! A violation at launch refuses the launch. A violation while running pauses the account's
//! relay (see `relay::block`), which cuts the account off the network because it has no direct
//! route; it stays cut, across restarts, until the user resumes or rebinds.
use crate::environment::Environment;
use crate::node::ExitInfo;
use crate::settings;
use std::sync::OnceLock;

pub const RULES: [&str; 5] = ["", "static", "city", "region", "country"];

fn rule_name(rule: &str) -> &'static str {
    match rule {
        "static" => "固定 IP",
        "city" => "同城市",
        "region" => "同省/州",
        "country" => "同国家",
        _ => "不限",
    }
}

pub fn describe(x: &ExitInfo) -> String {
    let place: Vec<&str> = [x.country.as_str(), x.region.as_str(), x.city.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
    if place.is_empty() { x.ip.clone() } else { format!("{}（{}）", x.ip, place.join(" ")) }
}

/// Checks an exit against an environment's region block and exit rule.
pub fn check(env: &Environment, exit: &ExitInfo) -> Result<(), String> {
    if !env.allow_cn && exit.country.trim().eq_ignore_ascii_case("CN") {
        return Err(format!("出口 {} 位于中国大陆，Claude 不在该地区提供服务，已拦截。", describe(exit)));
    }
    let Some(base) = &env.exit_baseline else { return Ok(()) };
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    let country = same(&base.country, &exit.country);
    let region = country && same(&base.region, &exit.region);
    let ok = match env.exit_rule.as_str() {
        "static" => base.ip == exit.ip,
        "city" => region && same(&base.city, &exit.city),
        "region" => region,
        "country" => country,
        _ => true,
    };
    if ok { return Ok(()); }
    Err(format!("出口从 {} 变成了 {}，不符合“{}”规则。", describe(base), describe(exit), rule_name(&env.exit_rule)))
}

/// Applies the rules of environment `env_id` (none = defaults: only the region block) to a
/// verified exit, binding the baseline the first time a rule sees an exit.
pub fn verify(env_id: Option<&str>, exit: &ExitInfo) -> Result<(), String> {
    let s = settings::load();
    let env = env_id.and_then(|id| s.environments.get(id)).cloned().unwrap_or_default();
    check(&env, exit)?;
    if let Some(id) = env_id.filter(|_| !env.exit_rule.is_empty() && env.exit_baseline.is_none()) {
        settings::update(|s| if let Some(e) = s.environments.get_mut(id) { e.exit_baseline.get_or_insert_with(|| exit.clone()); })?;
    }
    Ok(())
}

/// Binds environment `env_id` to its node's current exit and lifts the block of its accounts.
pub fn rebind(env_id: &str) -> Result<ExitInfo, String> {
    let s = settings::load();
    let env = s.environments.get(env_id).cloned().ok_or("这个环境配置已不存在。")?;
    let node = env.node().ok_or("这个环境没有代理节点。")?;
    let (upstream, _core) = crate::clash::resolve(&node)?;
    let exit = crate::node::inspect(&upstream.client(std::time::Duration::from_secs(15))?, &node)?;
    check(&Environment { exit_baseline: None, ..env }, &exit)?;
    let accounts: Vec<String> = s.accounts.iter().filter(|(_, a)| a.environment.as_deref() == Some(env_id)).map(|(id, _)| id.clone()).collect();
    settings::update(|s| if let Some(e) = s.environments.get_mut(env_id) { e.exit_baseline = Some(exit.clone()); })?;
    for id in accounts { crate::relay::unblock(&id); }
    Ok(exit)
}

/// Re-checks a blocked account's exit and restores its network if the exit is acceptable again.
pub fn resume(id: &str) -> Result<ExitInfo, String> {
    let exit = crate::relay::probe(id)?;
    let env = settings::load().effective(id).environment;
    verify(env.as_deref(), &exit)?;
    crate::relay::unblock(id);
    Ok(exit)
}

type Notifier = Box<dyn Fn(&str, &str) + Send + Sync>;
static NOTIFY: OnceLock<Notifier> = OnceLock::new();

/// Registers how the host process tells the user an account was cut off (`id`, reason).
pub fn on_block(f: impl Fn(&str, &str) + Send + Sync + 'static) { let _ = NOTIFY.set(Box::new(f)); }

pub(crate) fn notify(id: &str, reason: &str) {
    if let Some(f) = NOTIFY.get() { f(id, reason); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(ip: &str, country: &str, region: &str, city: &str) -> ExitInfo {
        ExitInfo { ip: ip.into(), country: country.into(), region: region.into(), city: city.into(), ..Default::default() }
    }

    #[test]
    fn region_block_and_rules() {
        let tokyo = exit("1.1.1.1", "JP", "Tokyo", "Tokyo");
        let mut env = Environment::default();
        assert!(check(&env, &exit("2.2.2.2", "CN", "Beijing", "Beijing")).is_err());
        assert!(check(&env, &exit("2.2.2.2", "HK", "", "")).is_ok());
        env.allow_cn = true;
        assert!(check(&env, &exit("2.2.2.2", "CN", "", "")).is_ok());
        // A rule without a baseline yet accepts the exit (it becomes the baseline).
        env.exit_rule = "static".into();
        assert!(check(&env, &tokyo).is_ok());
        env.exit_baseline = Some(tokyo.clone());
        assert!(check(&env, &exit("1.1.1.2", "JP", "Tokyo", "Tokyo")).is_err());
        env.exit_rule = "city".into();
        assert!(check(&env, &exit("1.1.1.2", "jp", "tokyo", "Tokyo")).is_ok());
        assert!(check(&env, &exit("1.1.1.2", "JP", "Tokyo", "Shinjuku")).is_err());
        env.exit_rule = "region".into();
        assert!(check(&env, &exit("1.1.1.2", "JP", "Tokyo", "Shinjuku")).is_ok());
        assert!(check(&env, &exit("1.1.1.2", "JP", "Osaka", "Osaka")).is_err());
        env.exit_rule = "country".into();
        assert!(check(&env, &exit("1.1.1.2", "JP", "Osaka", "Osaka")).is_ok());
        assert!(check(&env, &exit("1.1.1.2", "US", "", "")).is_err());
        env.exit_rule.clear();
        assert!(check(&env, &exit("1.1.1.2", "US", "", "")).is_ok());
    }
}
