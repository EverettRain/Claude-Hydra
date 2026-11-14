//! Command-line interface for headless account launching and management.

use crate::{install, launcher, paths, profiles, sys, watchdog};

const HELP: &str = "Claude-Hydra - 同时运行多个 Claude 桌面端账号

用法：claude-hydra [选项]
  （无参数）           打开主窗口
  --tray               只启动到系统托盘
  --launch <账号>      启动账号；已在运行则切到它的窗口
  --close <账号>       关闭账号的 Claude
  --restart <账号>     重启账号的 Claude（让新的启动参数生效）
  --add-account        新建账号并打开登录界面
  --list               列出账号及运行状态
  --container <名称>   在当前终端进入 Claude Code 容器账号（macOS + OrbStack）
  --watch              执行一次“保持运行”检查
  --version            打印版本号";

/// Returns `Some(exit_code)` when a CLI subcommand was executed, or `None` to start the GUI/tray.
pub fn run(args: &[String]) -> Option<i32> {
    let first = args.first()?.as_str();
    if first == "--login-callback" {
        let result = args.get(1).ok_or("缺少登录回调地址。".to_string()).and_then(|url| crate::login::route(url));
        return Some(match result {
            Ok(()) => 0,
            Err(e) => { sys::error_box(&e); 1 }
        });
    }
    if first == "--open-url" {
        let result = args.get(1).ok_or("缺少链接地址。".to_string()).and_then(|url| crate::browser::route_external(url));
        return Some(match result {
            Ok(()) => 0,
            Err(e) => { sys::error_box(&e); 1 }
        });
    }
    if first == "--tray" {
        return None;
    }
    if first == "--container" {
        let result = args.get(1).ok_or("--container 需要一个容器账号名称。".to_string())
            .and_then(|n| crate::container::resolve(n).ok_or_else(|| format!("没有名为“{n}”的容器账号。")))
            .and_then(|id| {
                let token = (args.get(2).map(String::as_str) == Some("--launch")).then(|| args.get(3).map(String::as_str).unwrap_or_default());
                crate::container::enter(&id, token)
            });
        return Some(match result {
            Ok(code) => code,
            Err(e) => {
                paths::append_log("hydra-error.log", &format!("--container: {e}"));
                eprintln!("\n[错误] {e}");
                // The terminal window may close as soon as this process ends.
                use std::io::IsTerminal;
                if std::io::stdin().is_terminal() {
                    eprint!("\n按回车键关闭…");
                    let _ = std::io::stdin().read_line(&mut String::new());
                }
                1
            }
        });
    }
    let console = sys::attach_console();
    let value = args.get(1).map(String::as_str);
    let result = match (first, value) {
        ("--version" | "-v", _) => Ok(env!("CARGO_PKG_VERSION").to_string()),
        ("--help" | "-h", _) => Ok(HELP.to_string()),
        ("--list", _) => list(),
        ("--launch", Some(n)) => with_profile(n, |i, p| launcher::launch(i, p).map(|o| format!("{}：{o}", p.name))),
        ("--close", Some(n)) => with_profile(n, |i, p| launcher::close(i, p).map(|o| format!("{}：{o}", p.name))),
        ("--restart", Some(n)) => with_profile(n, |i, p| launcher::restart(i, p).map(|o| format!("{}：{o}", p.name))),
        ("--add-account", _) => install::resolve()
            .and_then(|i| launcher::add_account(&i))
            .map(|p| format!("已创建 {}", p.name)),
        ("--watch", _) => install::resolve().map(|i| {
            let names = watchdog::run_once(&i);
            if names.is_empty() {
                "没有需要重新打开的账号。".into()
            } else {
                format!("已重新打开：{}", names.join("、"))
            }
        }),
        ("--launch" | "--close" | "--restart", None) => Err(format!("{first} 需要一个账号名称。")),
        _ => Err(format!("未知参数 {first}\n\n{HELP}")),
    };
    match result {
        Ok(text) => {
            if console {
                println!("{text}");
            }
            Some(0)
        }
        Err(e) => {
            paths::append_log("hydra-error.log", &format!("{}: {e}", args.join(" ")));
            if console {
                eprintln!("{e}");
            } else {
                sys::error_box(&e);
            }
            Some(1)
        }
    }
}

fn with_profile(
    name: &str,
    f: impl FnOnce(&install::Install, &profiles::Profile) -> Result<String, String>,
) -> Result<String, String> {
    let i = install::resolve()?;
    let p = profiles::resolve(&i, name, false).ok_or_else(|| format!("没有名为“{name}”的账号。"))?;
    f(&i, &p)
}

fn list() -> Result<String, String> {
    let i = install::resolve()?;
    let rows: Vec<String> = profiles::list(&i, true)
        .into_iter()
        .map(|p| {
            let state = p.pid.map(|pid| format!("运行中 (pid {pid})")).unwrap_or_else(|| "未运行".into());
            let name = if p.name != p.id { format!("{} [{}]", p.name, p.id) } else { p.name };
            format!("{name:<28} {state:<20} {}", p.path)
        })
        .collect();
    Ok(rows.join("\n"))
}
