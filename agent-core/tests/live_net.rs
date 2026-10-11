//! 真实网络冒烟测试（默认忽略：CI 无网/不稳定环境不跑）。
//! 手动执行：cargo test --test live_net -- --ignored
//!
//! 验证点：htmd 转换在真实页面上不 panic、DDG 返回可解析的结果、
//! SSRF 门对 localhost 拦截生效。

use agent_core::net_tools::parse_ddg_results;
use agent_core::AgentTool;

#[tokio::test]
#[ignore]
async fn live_fetch_example_com() {
    let tool = agent_core::FetchTool::new().unwrap();
    let ctx = agent_core::ToolContext {
        project_root: None,
        bridge: std::sync::Arc::new(agent_core::AutoBridge(agent_core::Decision::Allow)),
        cancelled: tokio_util::sync::CancellationToken::new(),
    };
    let out = tool
        .call(serde_json::json!({"url": "https://example.com"}), &ctx)
        .await
        .expect("fetch example.com 应成功");
    assert!(
        out.content.to_lowercase().contains("example"),
        "实际: {}",
        out.content
    );
}

#[tokio::test]
#[ignore]
async fn live_search_web() {
    let tool = agent_core::WebSearchTool::new().unwrap();
    let ctx = agent_core::ToolContext {
        project_root: None,
        bridge: std::sync::Arc::new(agent_core::AutoBridge(agent_core::Decision::Allow)),
        cancelled: tokio_util::sync::CancellationToken::new(),
    };
    let out = tool
        .call(
            serde_json::json!({"query": "rust programming language"}),
            &ctx,
        )
        .await
        .expect("search_web 应成功");
    assert!(
        out.content.contains("http"),
        "结果应含链接: {}",
        out.content
    );
}

#[test]
#[ignore]
fn live_ddg_parser_on_real_page_shape() {
    // 离线验证解析器对典型 DDG 标记的容错（不联网，固定样例）
    let html = r##"<a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F">Rust</a>"##;
    let r = parse_ddg_results(html);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].url, "https://www.rust-lang.org/");
}

#[tokio::test]
#[ignore]
async fn live_fetch_localhost_blocked() {
    let tool = agent_core::FetchTool::new().unwrap();
    let ctx = agent_core::ToolContext {
        project_root: None,
        bridge: std::sync::Arc::new(agent_core::AutoBridge(agent_core::Decision::Allow)),
        cancelled: tokio_util::sync::CancellationToken::new(),
    };
    let err = tool
        .call(serde_json::json!({"url": "http://127.0.0.1:1/x"}), &ctx)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("受限"), "实际: {err}");
}
