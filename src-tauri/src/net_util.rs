//! 本机端点的 HTTP 客户端。
//!
//! 走 127.0.0.1 / localhost 的请求（本地 llama-server、Serve 宿主）必须绕开
//! 系统代理：reqwest 在 macOS 上读 SystemConfiguration 的代理设置时**不看
//! ExceptionList**，于是本机请求会被交给 Clash 之类的代理，代理回 502，
//! 表现就是「本地服务明明在跑，应用却连不上」。
//!
//! 远端端点保持默认（企业代理场景不受影响）。

/// 端点是否指向本机（回环）地址。端口 / userinfo / 路径不参与判断。
pub(crate) fn is_loopback_url(url: &str) -> bool {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    let host = host.trim().to_ascii_lowercase();
    host == "localhost" || host == "::1" || host == "0.0.0.0" || host.starts_with("127.")
}

/// 面向 `url` 的客户端构造器：本机端点已设好 `no_proxy()`，调用方继续加超时等配置。
pub(crate) fn client_builder_for(url: &str) -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    if is_loopback_url(url) {
        builder.no_proxy()
    } else {
        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_local_endpoints() {
        for url in [
            "http://127.0.0.1:8080/v1",
            "http://localhost:8080/v1",
            "http://127.0.0.1/v1",
            "http://[::1]:8080/v1",
            "http://0.0.0.0:8080",
            "http://user:pass@127.0.0.1:8080/v1",
        ] {
            assert!(is_loopback_url(url), "{url} 应识别为本机端点");
        }
    }

    #[test]
    fn keeps_remote_endpoints_on_default_proxy() {
        for url in [
            "https://api.openai.com/v1",
            "http://192.168.1.5:8080/v1",
            "https://localhost.example.com/v1",
            "https://api.example.com/localhost/v1",
        ] {
            assert!(!is_loopback_url(url), "{url} 不应绕过代理");
        }
    }
}
