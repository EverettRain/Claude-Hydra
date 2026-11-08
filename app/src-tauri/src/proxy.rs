use std::{io::Read, net::IpAddr, time::Duration};

pub(crate) fn transport(endpoint: &str) -> Result<reqwest::Proxy, String> {
    if !crate::valid_proxy(endpoint) {
        return Err("请输入有效的 HTTP、HTTPS 或 SOCKS5 代理入口。".into());
    }
    // Resolve destination hostnames on the SOCKS server.
    let address = endpoint.replacen("socks5://", "socks5h://", 1);
    reqwest::Proxy::all(address).map_err(|_| "代理地址无效。".into())
}

/// Chromium switches that would override or bypass the managed proxy entrance.
const PROXY_SWITCHES: [&str; 5] = ["--proxy-server", "--no-proxy-server", "--proxy-pac-url", "--proxy-auto-detect", "--proxy-bypass-list"];

/// Whether extra launch arguments already contain a proxy switch (exact flag match only).
pub fn args_conflict(args: &str) -> bool {
    args.split_whitespace()
        .map(|a| a.trim_matches('"').split('=').next().unwrap_or_default().to_ascii_lowercase())
        .any(|flag| PROXY_SWITCHES.contains(&flag.as_str()))
}

fn check(endpoint: &str, destination: &str) -> Result<IpAddr, String> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .proxy(transport(endpoint)?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| "无法初始化代理连接。")?;
    let mut text = String::new();
    client
        .get(destination)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|_| "通过指定代理访问 IP 检测服务失败。")?
        .take(128)
        .read_to_string(&mut text)
        .map_err(|_| "无法读取出口 IP。")?;
    text.trim()
        .parse()
        .map_err(|_| "检测服务未返回有效的出口 IP。".into())
}

pub fn verify(endpoint: &str) -> Result<String, String> {
    check(endpoint, "https://api64.ipify.org").map(|ip| ip.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    fn proxy(ip: &'static str) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(&stream);
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
            assert!(line.starts_with("GET http://hydra.invalid/ "));
            loop {
                line.clear();
                if std::io::BufRead::read_line(&mut reader, &mut line).unwrap() == 0
                    || line == "\r\n"
                {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                ip.len(),
                ip
            )
            .unwrap();
        });
        (endpoint, thread)
    }

    #[test]
    fn only_real_proxy_switches_conflict() {
        for args in ["--proxy-server=http://a:1", "--lang=en \"--proxy-pac-url=x\"", "--NO-PROXY-SERVER", "--proxy-bypass-list=*"] {
            assert!(args_conflict(args), "{args}");
        }
        for args in ["", "--lang=zh-CN", "--no-proxy-xxx", "--proxy-serverless", "--disable-gpu --my-proxy=1"] {
            assert!(!args_conflict(args), "{args}");
        }
    }

    #[test]
    fn separate_entrances_and_closed_proxy() {
        let (a, ta) = proxy("192.0.2.1");
        let (b, tb) = proxy("192.0.2.2");
        assert_eq!(
            check(&a, "http://hydra.invalid/").unwrap().to_string(),
            "192.0.2.1"
        );
        ta.join().unwrap();
        assert!(check(&a, "http://hydra.invalid/").is_err());
        assert_eq!(
            check(&b, "http://hydra.invalid/").unwrap().to_string(),
            "192.0.2.2"
        );
        tb.join().unwrap();
    }
}
