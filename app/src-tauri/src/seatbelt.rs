//! macOS Seatbelt (`sandbox-exec`): a sandbox profile applied when a program starts and inherited
//! by every process it starts, here used to leave a program and its children only one way out:
//! the account's relay. Whatever ignores the proxy settings (its own sockets, UDP, the system's
//! DNS) then fails instead of going around the relay and its exit guard.
//!
//! Only network access is restricted (`allow default`); files, windows and the rest stay as they
//! are, so the program runs as it would outside. A Chromium-based app cannot keep its own sandbox
//! under it (macOS refuses a sandbox inside a sandbox), so this suits command-line programs such as
//! Claude Code and MCP servers rather than Claude's own window.

/// The profile for a program that may reach the network only through the relay on
/// `127.0.0.1:relay_port`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn network_profile(relay_port: u16) -> String {
    format!(
        r#"(version 1)
(allow default)
(deny network-outbound)
; Local connections between processes (not the network) stay open.
(allow network-outbound (remote unix-socket (path-regex #"^/")))
; The system's DNS service: a lookup there would send the name to the system's resolver.
(deny network-outbound (remote unix-socket (path-literal "/private/var/run/mDNSResponder")))
(deny mach-lookup (global-name "com.apple.dnssd.service"))
; The account's relay, the only way out.
(allow network-outbound (remote ip "localhost:{relay_port}"))
"#
    )
}

/// `program` with `args` under the network profile for `relay_port`.
#[cfg(target_os = "macos")]
pub fn command(program: &std::path::Path, args: &[String], relay_port: u16) -> std::process::Command {
    let mut cmd = std::process::Command::new("/usr/bin/sandbox-exec");
    cmd.arg("-p").arg(network_profile(relay_port)).arg(program).args(args);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_relay_is_reachable() {
        // Arrange
        let port = 57575;

        // Act
        let profile = network_profile(port);

        // Assert
        let rules: Vec<&str> = profile.lines().filter(|l| l.starts_with('(')).collect();
        let deny_all = rules.iter().position(|r| *r == "(deny network-outbound)").expect("everything else is denied");
        let relay = rules.iter().position(|r| r.contains(&format!("localhost:{port}"))).expect("the relay is allowed");
        let dns = rules.iter().position(|r| r.contains("mDNSResponder")).expect("the system DNS is denied");
        // Seatbelt applies the last matching rule, so each exception must follow the rule it narrows.
        assert!(deny_all < relay && deny_all < dns, "{profile}");
        assert!(rules.iter().all(|r| !r.contains("(allow network-outbound (remote ip \"*") && !r.contains("localhost:*")), "no other host or port: {profile}");
        assert_eq!(profile.matches('(').count(), profile.matches(')').count(), "balanced: {profile}");
    }
}
