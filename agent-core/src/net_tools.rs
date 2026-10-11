//! 网络工具：fetch（抓取网页转 Markdown）与 search_web（网页搜索）。
//!
//! 参考 Zed fetch_tool 的安全设计，按 astra 的宿主形态简化：
//! - **SSRF 防护**：每跳 URL 解析 DNS 后校验 IP，拦截 loopback / 私网 /
//!   链路本地 / ULA 地址（不是只看 hostname 字符串，防 DNS rebinding 的
//!   稳定解析情形；Zed 注明的「检查-连接窗口」残留在本实现同样存在）
//! - **重定向**：reqwest 关闭自动跟随，手动逐跳跟随（上限 20），每跳重过
//!   SSRF 门（对齐 Zed「每跳重新授权」——astra 无 per-host 授权体系，
//!   用静态 SSRF 门 + 全局 Confirm 替代）
//! - 仅 http/https；响应体上限 5MB；总超时 30s
//! - search_web 用 DuckDuckGo HTML lite 抓取（无需 API key），
//!   解析器是纯函数（可离线单测）

use crate::error::{Error, Result};
use crate::tool::{AgentTool, ToolContext, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::time::Duration;
use url::Url;
// bytes_stream 依赖 futures 的 StreamExt
use futures::StreamExt;

/// 每跳重定向上限（对齐 Zed MAX_REDIRECTS）
const MAX_REDIRECTS: usize = 20;
/// 响应体上限（字节）
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
/// 单请求总超时
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// 搜索结果条数上限
const SEARCH_MAX_RESULTS: usize = 8;

/// 无 scheme 时补 `https://`；已有任意 scheme 的原样保留（非 http/https
/// 会在后续 scheme 校验中被拒绝，而不是拼成畸形 URL）
fn normalize_url(url: &str) -> String {
    if url.contains("://") {
        url.to_string()
    } else {
        format!("https://{url}")
    }
}

/// SSRF 门的 IP 黑名单：loopback / 私网 / 链路本地 / ULA / 未指定地址。
/// 任一解析结果命中即拒绝（比「全部命中才拒绝」更保守）。
pub(crate) fn ip_is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.octets()[0] == 0 // 0.0.0.0/8 "this network"
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // IPv4-mapped：按内嵌 v4 再判一次
                || v6.to_ipv4_mapped().is_some_and(|v4| ip_is_forbidden(IpAddr::V4(v4)))
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 ULA
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        }
    }
}

/// 解析 host 并校验不在黑名单（DNS 后校验，不只看字符串）。
async fn verify_host_allowed(parsed: &Url) -> Result<()> {
    let host = parsed
        .host_str()
        .ok_or_else(|| Error::tool("fetch", format!("URL 无主机: {parsed}")))?;
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| Error::tool("fetch", format!("DNS 解析失败 {host}: {e}")))?
        .collect();
    if addrs.is_empty() {
        return Err(Error::tool("fetch", format!("{host} 没有解析到任何地址")));
    }
    for addr in &addrs {
        if ip_is_forbidden(addr.ip()) {
            return Err(Error::tool(
                "fetch",
                format!("{host} 解析到受限地址 {}（内网/回环），拒绝访问", addr.ip()),
            ));
        }
    }
    Ok(())
}

/// 单次请求（不跟随重定向）。返回重定向目标或最终文本。
enum FetchStep {
    Redirect(String),
    Complete(String),
}

async fn fetch_step(client: &reqwest::Client, url: &str) -> Result<FetchStep> {
    let normalized = normalize_url(url);
    let parsed = Url::parse(&normalized)
        .map_err(|e| Error::tool("fetch", format!("URL 无效 {normalized}: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(Error::tool(
            "fetch",
            format!("仅支持 http/https，拒绝: {normalized}"),
        ));
    }
    verify_host_allowed(&parsed).await?;

    let resp = client
        .get(parsed.clone())
        .send()
        .await
        .map_err(|e| Error::tool("fetch", format!("请求失败 {normalized}: {e}")))?;

    // 重定向：取 Location，join 成绝对 URL（每跳由调用方重新过门）
    if resp.status().is_redirection() {
        let location = resp
            .headers()
            .get("location")
            .ok_or_else(|| Error::tool("fetch", "重定向响应缺少 Location 头"))?
            .to_str()
            .map_err(|_| Error::tool("fetch", "Location 头非合法 UTF-8"))?
            .to_string();
        let target = parsed
            .join(&location)
            .map_err(|e| Error::tool("fetch", format!("重定向目标无效 {location}: {e}")))?;
        if !matches!(target.scheme(), "http" | "https") {
            return Err(Error::tool(
                "fetch",
                format!("拒绝跟随到非 HTTP(S) 的重定向: {target}"),
            ));
        }
        return Ok(FetchStep::Redirect(target.to_string()));
    }

    let status = resp.status();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // 体量上限：先看 content-length，再分块读防超
    if let Some(len) = resp.content_length() {
        if len as usize > MAX_BODY_BYTES {
            return Err(Error::tool(
                "fetch",
                format!("响应体过大（{} MB），上限 5 MB", len / 1024 / 1024),
            ));
        }
    }
    let mut body: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| Error::tool("fetch", format!("读取响应失败: {e}")))?;
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(Error::tool("fetch", "响应体超过 5 MB 上限，已中止"));
        }
        body.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        return Err(Error::tool(
            "fetch",
            format!(
                "HTTP {}，响应片段: {}",
                status.as_u16(),
                &text[..text.len().min(500)]
            ),
        ));
    }

    let text = convert_body(&body, &content_type)?;
    Ok(FetchStep::Complete(text))
}

/// 按 content-type 把响应体转成文本（HTML → Markdown）
fn convert_body(body: &[u8], content_type: &str) -> Result<String> {
    let ct = content_type.to_lowercase();
    if ct.starts_with("text/plain") {
        return String::from_utf8(body.to_vec())
            .map_err(|_| Error::tool("fetch", "响应不是合法 UTF-8 文本"));
    }
    if ct.starts_with("application/json") {
        let v: Value = serde_json::from_slice(body)
            .map_err(|e| Error::tool("fetch", format!("JSON 解析失败: {e}")))?;
        let pretty = serde_json::to_string_pretty(&v)
            .map_err(|e| Error::tool("fetch", format!("JSON 序列化失败: {e}")))?;
        return Ok(format!("```json\n{pretty}\n```"));
    }
    // 其余（text/html 等）一律当 HTML 转 Markdown
    let html = String::from_utf8_lossy(body);
    htmd::convert(&html).map_err(|e| Error::tool("fetch", format!("HTML 转 Markdown 失败: {e}")))
}

pub struct FetchTool {
    client: reqwest::Client,
}

impl FetchTool {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(FETCH_TIMEOUT)
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36 AstraAgent/0.1")
            .build()
            .map_err(|e| Error::tool("fetch", format!("HTTP client 初始化失败: {e}")))?;
        Ok(Self { client })
    }
}

impl Default for FetchTool {
    fn default() -> Self {
        Self::new().expect("FetchTool::new 不应失败")
    }
}

#[async_trait]
impl AgentTool for FetchTool {
    fn name(&self) -> &str {
        "fetch"
    }
    fn description(&self) -> &str {
        "抓取一个网页并把内容转成 Markdown 返回。需要查阅网页文档、资料时使用；\
         内网/回环地址一律拒绝；重定向逐跳校验"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "要抓取的 URL（无 scheme 时默认 https://）" }
            },
            "required": ["url"]
        })
    }
    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::tool("fetch", "缺少 url"))?;

        let mut current = url.to_string();
        let mut redirects: usize = 0;
        let text = loop {
            if ctx.cancelled.is_cancelled() {
                return Err(Error::tool("fetch", "已取消"));
            }
            let step = tokio::select! {
                _ = ctx.cancelled.cancelled() => {
                    return Err(Error::tool("fetch", "已取消"));
                }
                r = fetch_step(&self.client, &current) => r?,
            };
            match step {
                FetchStep::Complete(text) => break text,
                FetchStep::Redirect(target) => {
                    redirects += 1;
                    if redirects > MAX_REDIRECTS {
                        return Err(Error::tool(
                            "fetch",
                            format!("重定向超过 {MAX_REDIRECTS} 跳上限"),
                        ));
                    }
                    current = target;
                }
            }
        };

        if text.trim().is_empty() {
            return Err(Error::tool("fetch", "页面没有可提取的文本内容"));
        }
        // 给模型的内容同样截断，避免一个页面撑爆上下文
        const FETCH_OUTPUT_CAP: usize = 40_000;
        let out = if text.chars().count() > FETCH_OUTPUT_CAP {
            let t: String = text.chars().take(FETCH_OUTPUT_CAP).collect();
            format!("{t}\n\n（内容过长已截断，可用更具体的页面或搜索引擎缩小范围）")
        } else {
            text
        };
        Ok(ToolOutput::from(out))
    }
}

// ===== search_web =====

/// 一条搜索结果（解析器输出，纯数据便于单测）
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// 去掉片段中的 HTML 标签并解码常见实体（纯函数）
pub(crate) fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// 从 DuckDuckGo HTML lite 结果页提取结果（纯函数，供离线单测）。
/// 结果链接形如 //duckduckgo.com/l/?uddg=<urlencode>&rut=...，需解出真实 URL；
/// 部分结果也可能直接是 http(s) 链接。
pub fn parse_ddg_results(html: &str) -> Vec<SearchResult> {
    use percent_encoding::percent_decode_str;

    let link_re =
        regex::Regex::new(r#"(?s)<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#)
            .expect("内置正则必须可编译");
    let snippet_re = regex::Regex::new(r#"(?s)<a[^>]*class="result__snippet"[^>]*>(.*?)</a>"#)
        .expect("内置正则必须可编译");

    let snippets: Vec<String> = snippet_re
        .captures_iter(html)
        .map(|c| strip_tags(&c[1]).trim().to_string())
        .collect();

    link_re
        .captures_iter(html)
        .enumerate()
        .map(|(i, c)| {
            let raw_href = &c[1];
            let url = if let Some(idx) = raw_href.find("uddg=") {
                let enc = raw_href[idx + 5..].split('&').next().unwrap_or_default();
                percent_decode_str(enc)
                    .decode_utf8()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| enc.to_string())
            } else if raw_href.starts_with("//") {
                format!("https:{raw_href}")
            } else {
                raw_href.to_string()
            };
            SearchResult {
                title: strip_tags(&c[2]).trim().to_string(),
                url,
                snippet: snippets.get(i).cloned().unwrap_or_default(),
            }
        })
        .collect()
}

/// 从 SearXNG JSON API 响应提取结果（纯函数，供离线单测）。
/// 响应形状：{"results": [{"title": ..., "url": ..., "content": 摘要}, ...]}
pub fn parse_searx_json(json: &Value) -> Vec<SearchResult> {
    json.get("results")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    let title = r.get("title")?.as_str()?.trim().to_string();
                    let url = r.get("url")?.as_str()?.trim().to_string();
                    if title.is_empty() || url.is_empty() {
                        return None;
                    }
                    Some(SearchResult {
                        title,
                        url,
                        snippet: r
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub struct WebSearchTool {
    client: reqwest::Client,
    /// 可选：SearXNG 实例的 base URL（如 https://searx.example.org）。
    /// 设置后走 SearXNG JSON API（format=json）；不设置用 DuckDuckGo HTML lite
    /// 抓取（无需 key，但部分网络环境不可达）。搜索后端可配置的原因：
    /// 各地区网络对 DDG 的可达性差异大，SearXNG 可自托管/换实例。
    endpoint: Option<String>,
}

impl WebSearchTool {
    pub fn new() -> Result<Self> {
        Self::with_endpoint(None)
    }

    pub fn with_endpoint(endpoint: Option<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36")
            .build()
            .map_err(|e| Error::tool("search_web", format!("HTTP client 初始化失败: {e}")))?;
        Ok(Self { client, endpoint })
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new().expect("WebSearchTool::new 不应失败")
    }
}

#[async_trait]
impl AgentTool for WebSearchTool {
    fn name(&self) -> &str {
        "search_web"
    }
    fn description(&self) -> &str {
        "用关键词搜索网页，返回标题、链接与摘要。需要实时信息、外部资料时使用；\
         拿到结果后可用 fetch 抓取具体页面"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "搜索词或问题" }
            },
            "required": ["query"]
        })
    }
    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::tool("search_web", "缺少 query"))?
            .trim();
        if query.is_empty() {
            return Err(Error::tool("search_web", "query 为空"));
        }

        let encoded =
            percent_encoding::utf8_percent_encode(query, percent_encoding::NON_ALPHANUMERIC);
        let results = match &self.endpoint {
            Some(base) => {
                let url = format!("{base}/search?q={encoded}&format=json");
                self.search_searx(&url, ctx).await?
            }
            None => {
                let url = format!("https://html.duckduckgo.com/html/?q={encoded}");
                self.search_ddg(&url, ctx).await?
            }
        };
        if results.is_empty() {
            return Err(Error::tool(
                "search_web",
                "未找到搜索结果（可能被搜索引擎限流或查询过冷门；可改用 fetch 直接抓取已知页面）",
            ));
        }

        let mut out = String::new();
        for (i, r) in results.iter().take(SEARCH_MAX_RESULTS).enumerate() {
            out.push_str(&format!("{}. {}\n   {}\n", i + 1, r.title, r.url));
            if !r.snippet.is_empty() {
                out.push_str(&format!("   {}\n", r.snippet));
            }
        }
        Ok(ToolOutput::from(out))
    }
}

impl WebSearchTool {
    async fn search_ddg(&self, url: &str, ctx: &ToolContext) -> Result<Vec<SearchResult>> {
        let resp = tokio::select! {
            _ = ctx.cancelled.cancelled() => {
                return Err(Error::tool("search_web", "已取消"));
            }
            r = self.client.get(url).send() => r
                .map_err(|e| Error::tool("search_web", format!("搜索请求失败: {e}")))?,
        };
        if !resp.status().is_success() {
            return Err(Error::tool(
                "search_web",
                format!(
                    "搜索服务返回 HTTP {}（可能被限流，请稍后重试）",
                    resp.status().as_u16()
                ),
            ));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| Error::tool("search_web", format!("读取搜索结果失败: {e}")))?;
        Ok(parse_ddg_results(&html))
    }

    async fn search_searx(&self, url: &str, ctx: &ToolContext) -> Result<Vec<SearchResult>> {
        let resp = tokio::select! {
            _ = ctx.cancelled.cancelled() => {
                return Err(Error::tool("search_web", "已取消"));
            }
            r = self.client.get(url).send() => r
                .map_err(|e| Error::tool("search_web", format!("搜索请求失败: {e}")))?,
        };
        if !resp.status().is_success() {
            return Err(Error::tool(
                "search_web",
                format!(
                    "搜索服务返回 HTTP {}（检查 SearXNG 实例地址是否正确、是否允许 JSON 输出）",
                    resp.status().as_u16()
                ),
            ));
        }
        let json: Value = tokio::select! {
            _ = ctx.cancelled.cancelled() => {
                return Err(Error::tool("search_web", "已取消"));
            }
            r = resp.json() => r
                .map_err(|e| Error::tool("search_web", format!("搜索结果非 JSON（SearXNG 需开启 format=json）: {e}")))?,
        };
        Ok(parse_searx_json(&json))
    }
}

/// 注册网络工具（不依赖项目根，纯聊天也可用）。search_endpoint 见 WebSearchTool。
pub fn register_with_search(
    reg: crate::tool::ToolRegistry,
    search_endpoint: Option<String>,
) -> crate::tool::ToolRegistry {
    reg.register(FetchTool::new().expect("FetchTool 构造失败"))
        .register(WebSearchTool::with_endpoint(search_endpoint).expect("WebSearchTool 构造失败"))
}

pub fn register(reg: crate::tool::ToolRegistry) -> crate::tool::ToolRegistry {
    register_with_search(reg, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== SSRF 门 =====

    #[test]
    fn forbidden_ips_cover_local_ranges() {
        for ip in [
            "127.0.0.1",
            "127.8.8.8",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.1.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(ip_is_forbidden(ip), "应拦截: {ip}");
        }
    }

    #[test]
    fn public_ips_allowed() {
        for ip in ["8.8.8.8", "1.1.1.1", "172.32.0.1", "2606:4700::1111"] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(!ip_is_forbidden(ip), "不应拦截: {ip}");
        }
    }

    // ===== normalize_url =====

    #[test]
    fn normalize_adds_https_default() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(normalize_url("http://a.b"), "http://a.b");
        assert_eq!(normalize_url("https://a.b"), "https://a.b");
        // 已有其他 scheme 原样保留，交给 scheme 校验拒绝
        assert_eq!(normalize_url("ftp://a.b/f"), "ftp://a.b/f");
    }

    // ===== DDG 结果解析 =====

    #[test]
    fn parse_ddg_extracts_title_url_snippet_and_decodes_uddg() {
        let html = r##"
        <div class="result">
          <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2Flearn&rut=abc">&amp;Rust 官方<b>教程</b></a>
          <a class="result__snippet" href="#">学习 <strong>Rust</strong> 的入门资料</a>
        </div>
        <div class="result">
          <a rel="nofollow" class="result__a" href="https://doc.rust-lang.org/book/">The Book</a>
          <a class="result__snippet" href="#">Rust &amp; 编程指南</a>
        </div>"##;
        let results = parse_ddg_results(html);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://rust-lang.org/learn");
        assert_eq!(results[0].title, "&Rust 官方教程");
        assert_eq!(results[0].snippet, "学习 Rust 的入门资料");
        assert_eq!(results[1].url, "https://doc.rust-lang.org/book/");
        assert_eq!(results[1].snippet, "Rust & 编程指南");
    }

    #[test]
    fn parse_ddg_empty_html_yields_no_results() {
        assert!(parse_ddg_results("<html><body>无结果</body></html>").is_empty());
        assert!(parse_ddg_results("").is_empty());
    }

    #[test]
    fn parse_searx_extracts_title_url_content() {
        let json: Value = serde_json::from_str(
            r##"{"results": [
                {"title": "Rust", "url": "https://rust-lang.org", "content": "官方站点 & 文档"},
                {"title": "", "url": "https://empty-title.example"},
                {"url": "https://no-title.example"}
            ]}"##,
        )
        .unwrap();
        let r = parse_searx_json(&json);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "Rust");
        assert_eq!(r[0].url, "https://rust-lang.org");
        assert_eq!(r[0].snippet, "官方站点 & 文档");
        // 缺 results 字段 → 空
        assert!(parse_searx_json(&serde_json::json!({})).is_empty());
    }

    // ===== convert_body =====

    #[test]
    fn json_body_pretty_printed_in_code_block() {
        let out = convert_body(br#"{"a":1}"#, "application/json; charset=utf-8").unwrap();
        assert!(out.starts_with("```json"));
        assert!(out.contains("\"a\": 1"));
    }

    #[test]
    fn plaintext_body_passes_through() {
        let out = convert_body(b"hello\nworld", "text/plain; charset=utf-8").unwrap();
        assert_eq!(out, "hello\nworld");
    }

    #[test]
    fn html_body_converted_to_markdown() {
        let out = convert_body(
            b"<html><body><h1>Title</h1><p>para</p></body></html>",
            "text/html",
        )
        .unwrap();
        assert!(out.contains("Title"), "实际: {out}");
        assert!(out.contains("para"));
    }

    // ===== 工具参数校验（不发真实请求） =====

    fn ctx() -> ToolContext {
        ToolContext {
            project_root: None,
            bridge: std::sync::Arc::new(crate::permissions::AutoBridge(
                crate::permissions::Decision::Allow,
            )),
            cancelled: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn search_requires_query() {
        let tool = WebSearchTool::new().unwrap();
        let err = tool.call(json!({}), &ctx()).await.unwrap_err();
        assert!(err.to_string().contains("query"));

        let err = tool.call(json!({"query": "  "}), &ctx()).await.unwrap_err();
        assert!(err.to_string().contains("query"));
    }

    #[tokio::test]
    async fn fetch_requires_url() {
        let tool = FetchTool::new().unwrap();
        let err = tool.call(json!({}), &ctx()).await.unwrap_err();
        assert!(err.to_string().contains("url"));
    }

    #[tokio::test]
    async fn fetch_rejects_private_address_even_by_name() {
        // DNS 稳定解析到回环的域名（localhost）→ SSRF 门拦截
        let tool = FetchTool::new().unwrap();
        let err = tool
            .call(json!({"url": "http://localhost:9/x"}), &ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("受限") || err.to_string().contains("DNS"));
    }

    #[tokio::test]
    async fn fetch_rejects_non_http_scheme_before_dns() {
        let tool = FetchTool::new().unwrap();
        // ftp 无法被 normalize 成 https（有 scheme），URL 合法但 scheme 拒绝
        let err = tool
            .call(json!({"url": "ftp://example.com/file"}), &ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("仅支持 http/https"));
    }
}
