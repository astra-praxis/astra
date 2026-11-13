//! 面向用户的错误文案。
//!
//! 服务商、shell、文件解析把英文原文一路带到界面上，用户只看得到
//! `HTTP 400 Bad Request [InvalidParameter] ... Request id: 0217...` 这种串，
//! 既看不懂，也分不清是密钥错了、模型不在端点范围内，还是网络断了。
//!
//! 这里的规矩（与 astra-bid 的 friendly 层同源）：
//!
//! * 原文里已经带中文的（我们自己写的提示，例如「模型连接超时」）原样返回；
//! * 其余按传输层关键词、HTTP 状态码、错误码归类成一句人话；
//! * 英文原文由**调用方**写进日志（`app_log!` + agent_events.log），界面不再
//!   显示请求编号、堆栈或 `tool 'x' failed` 这类内容；
//! * 回填给**模型**的工具错误仍是原文（见 `agent/run.rs` 的转发注释）：模型要
//!   靠具体原因换做法，只有界面卡片中文化。

/// 一段文字里是否已经有中文（≥2 个汉字）。
///
/// 用来判断「这是我们自己写的提示」还是「底层吐出来的英文原文」，
/// 避免把已经是中文的话再翻译一遍。
fn has_chinese(text: &str) -> bool {
    text.chars()
        .filter(|c| matches!(c, '\u{4e00}'..='\u{9fff}'))
        .take(2)
        .count()
        >= 2
}

fn contains_any(lower: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| lower.contains(needle))
}

/// 从 `HTTP 400 Bad Request ...` 这类文案里取出状态码。
///
/// 只认「http + 空格 + 数字」：传输层错误里的 `http://host/...` 是 `http:`，
/// 不会被误当成状态码（这类错误应按「无状态码 = 网络抖动」处理）。
fn http_status(lower: &str) -> Option<u16> {
    let start = lower.find("http ")? + 5;
    let digits: String = lower[start..]
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// 这次失败是否值得**自动重试**。
///
/// 4xx 里只有 408（请求超时）、425（Too Early）、429（限流）是瞬时的，
/// 其余 4xx（401/403/404/400/413/422）重试必然同样失败 —— 自动重试三次只是
/// 让用户多等 15 秒、并把排队消息一起烧掉。5xx 与传输层故障（无状态码）照旧
/// 重试，网络抖动是最常见的瞬时故障。
pub(crate) fn is_retryable(raw: &str) -> bool {
    match http_status(&raw.to_lowercase()) {
        Some(408) | Some(425) | Some(429) => true,
        Some(status) => !(400..500).contains(&status),
        None => true,
    }
}

/// 一轮运行失败（`run_failed`）。
///
/// 先处理运行层自己的英文错误（超时/预算），其余交给 [`model`]。
pub(crate) fn run(raw: &str) -> String {
    if has_chinese(raw) {
        return raw.trim().to_string();
    }
    let lower = raw.to_lowercase();
    if contains_any(
        &lower,
        &["run deadline exceeded", "run timed out", "budget exceeded"],
    ) {
        return "这次运行超时了，任务被中止。把需求拆小一点再试，或换个更快的模型。".to_string();
    }
    if contains_any(&lower, &["stream cancelled", "cancelled"]) {
        return "这次运行被取消了。".to_string();
    }
    model(raw)
}

/// 模型调用失败。覆盖：连不上、超时、上下文超长、密钥、权限、限流、服务端故障。
pub(crate) fn model(raw: &str) -> String {
    if has_chinese(raw) {
        return raw.trim().to_string();
    }
    let lower = raw.to_lowercase();

    // ---- 传输层 ----
    if contains_any(
        &lower,
        &[
            "error sending request",
            "failed to connect",
            "connection refused",
            "connection reset",
            "connection closed",
            "dns error",
            "no route to host",
            "host unreachable",
            "network is unreachable",
        ],
    ) {
        return "连不上模型服务。检查一下网络，或到「模型服务商」页确认地址填得对不对。"
            .to_string();
    }
    if contains_any(&lower, &["certificate", "ssl", "tls"]) {
        return "连模型服务时证书校验没通过，检查服务商地址，或先用浏览器确认这个地址能打开。"
            .to_string();
    }
    if contains_any(&lower, &["timed out", "timeout", "deadline exceeded"]) {
        return "模型服务迟迟没有响应。稍等一会儿再试，或换一个模型。".to_string();
    }
    if contains_any(
        &lower,
        &["stream interrupted", "stream closed", "body error"],
    ) {
        return "和服务商的连接中途断了，这一轮没写完。重试一次即可。".to_string();
    }

    // ---- 内容长度 ----
    if contains_any(
        &lower,
        &[
            "context length",
            "context_length_exceeded",
            "maximum context",
            "too many tokens",
            "token limit",
            "input is too long",
            "prompt is too long",
        ],
    ) {
        return "这次的内容太长，超过了模型能处理的上限。清一下当前对话，或换一个上下文更大的模型。"
            .to_string();
    }

    // ---- 凭证 / 额度 ----
    if contains_any(
        &lower,
        &[
            "invalid_api_key",
            "incorrect api key",
            "invalidapikey",
            "authentication",
            "unauthorized",
            "api key",
        ],
    ) {
        return "API Key 无效或已过期。到「模型服务商」页里重新填一次。".to_string();
    }
    if contains_any(
        &lower,
        &[
            "insufficient",
            "balance",
            "arrears",
            "quota exceeded",
            "no quota",
        ],
    ) {
        return "这个账号的余额或调用额度不够了。去服务商那边续一下，或换一个服务商。".to_string();
    }
    if contains_any(
        &lower,
        &[
            "rate limit",
            "ratelimit",
            "too many requests",
            "concurrency",
        ],
    ) {
        return "调用太频繁了，等一会儿（十几秒）再试。".to_string();
    }

    // ---- HTTP 状态码 ----
    match http_status(&lower) {
        Some(401) => return "API Key 不对或已过期。到「模型服务商」页里重新填一次。".into(),
        Some(403) => {
            return "当前账号没有这个模型的权限。换一个模型，或确认服务商那边有没有开通。".into()
        }
        Some(404) => {
            // 火山方舟的编码计划等专用端点只开放部分模型，其余会回 404
            // `UnsupportedModel ... does not support the coding plan feature`——
            // 模型本身是存在的，所以别让用户去改模型名。
            if contains_any(
                &lower,
                &["coding plan", "does not support", "unsupportedmodel"],
            ) {
                return "这个模型不在当前服务地址开放的范围内（例如编码计划等专用端点只开放\
                        部分模型）。换一个模型，或把服务地址改回通用接口再试。"
                    .into();
            }
            return "服务商那边没有这个模型。到「模型服务商」页检查一下模型名。".into();
        }
        Some(413) => return "这次提交的内容太大，服务商拒收了。少选几份材料再试。".into(),
        Some(429) => return "调用太频繁或额度用完了，等一会儿再试。".into(),
        Some(500..=599) => return "模型服务商那边出故障了，稍后重试一次。".into(),
        _ => {}
    }

    // ---- 400：服务商只说「参数不合法」，不点名是哪个 ----
    // 典型是温度这类参数超出服务商约束（kimi 只收 1.0、glm 只收两位小数），
    // 模型层已经做了「去掉温度重试一次」，仍失败才走到这里。
    if http_status(&lower) == Some(400)
        || contains_any(
            &lower,
            &["invalidparameter", "invalid_parameter", "invalid request"],
        )
    {
        return "服务商拒绝了这次请求（参数不被接受）。先换一个模型试试；如果还是一样，\
                把这一步的报错反馈给我们，我们按模型适配。"
            .to_string();
    }

    if contains_any(
        &lower,
        &[
            "model_not_available",
            "unsupportedmodel",
            "unsupported model",
        ],
    ) {
        return "服务商说这个模型用不了。到「模型服务商」页换一个可用的模型。".to_string();
    }

    "模型调用失败了。稍等一会儿重试；反复失败就换一个模型或服务商。".to_string()
}

/// 工具执行失败（工具卡片上显示的那句）。
///
/// 工具错误同时也回填给模型（原文），这里只负责界面卡片：读文件类的失败
/// 还能靠具体原因说清楚，其余归成一句「这一步没成功」。
pub(crate) fn tool(raw: &str) -> String {
    if has_chinese(raw) {
        return raw.trim().to_string();
    }
    let lower = raw.to_lowercase();

    if contains_any(
        &lower,
        &[
            "unknown input format",
            "unsupported format",
            "unsupported",
            "cannot convert",
            "can't convert",
            "pandoc failed",
            "conversion failed",
            "unrecognized",
            "not supported",
        ],
    ) {
        return "这个文件的格式暂时读不了。用 Office 或 WPS 另存为 pdf / docx 后再试一次。"
            .to_string();
    }
    if contains_any(
        &lower,
        &["password", "encrypted", "encryption", "protected"],
    ) {
        return "这个文件设了密码，先去掉密码再试一次。".to_string();
    }
    if contains_any(&lower, &["permission denied", "access denied"]) {
        return "没有权限操作这个文件，确认一下它是否被其他程序占用。".to_string();
    }
    if contains_any(&lower, &["not found", "no such file", "does not exist"]) {
        return "找不到这个文件或目录，确认一下它还在原来的位置。".to_string();
    }
    if contains_any(&lower, &["timed out", "timeout"]) {
        return "这一步超时了，内容可能太大，拆小一点再试。".to_string();
    }
    "这一步没成功，换个方式再试一次。".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ark_invalid_parameter_becomes_chinese_without_the_raw_noise() {
        let raw = "model request failed: HTTP 400 Bad Request [InvalidParameter]: \
                   A parameter specified in the request is not valid \
                   Request id: 0217944817917574d7552398ca1982f1d8c06d525f58d82968532";
        let text = model(raw);
        assert!(text.contains("服务商"), "{text}");
        assert!(!text.contains("InvalidParameter"), "{text}");
        assert!(!text.contains("Request id"), "{text}");
        assert!(!text.contains("HTTP"), "{text}");
    }

    #[test]
    fn transport_failures_read_as_network_problems() {
        let raw = "model request failed: error sending request for url \
                   (http://172.16.18.69/v1/chat/completions)";
        let text = model(raw);
        assert!(text.contains("连不上"), "{text}");
        assert!(!text.contains("172.16"), "{text}");
    }

    #[test]
    fn auth_and_rate_limit_have_their_own_advice() {
        assert!(model("HTTP 401 Unauthorized").contains("API Key"));
        assert!(model("HTTP 403 Forbidden [model_not_available]").contains("权限"));
        assert!(model("HTTP 404 Not Found").contains("模型"));
        assert!(model("HTTP 429 Too Many Requests").contains("频繁"));
        assert!(model("HTTP 502 Bad Gateway").contains("故障"));
    }

    #[test]
    fn context_overflow_is_explained_in_plain_chinese() {
        let text = model("HTTP 400 [InvalidParameter]: maximum context length exceeded");
        assert!(text.contains("太长"), "{text}");
    }

    #[test]
    fn our_own_chinese_messages_pass_through_untouched() {
        let raw = "模型连接超时（180s）";
        assert_eq!(model(raw), raw);
        assert_eq!(tool(raw), raw);
        assert_eq!(run(raw), raw);
    }

    #[test]
    fn pandoc_failures_no_longer_leak_the_english_tool_name() {
        let raw = "tool 'extract_document' failed: pandoc failed: Unknown input format 'pdf' \
                   Pandoc can convert from many formats";
        let text = tool(raw);
        assert!(text.contains("格式暂时读不了"), "{text}");
        assert!(!text.contains("pandoc"), "{text}");
        assert!(!text.contains("extract_document"), "{text}");
    }

    #[test]
    fn encrypted_and_missing_files_are_named() {
        assert!(tool("document is encrypted").contains("密码"));
        assert!(tool("No such file or directory").contains("找不到"));
        assert!(tool("Permission denied").contains("权限"));
    }

    #[test]
    fn plan_limited_endpoints_do_not_blame_the_model_name() {
        // 火山方舟专用（编码计划）端点：模型存在，只是不在该端点开放范围内
        let raw = "model request failed: HTTP 404 Not Found [UnsupportedModel]: The requested \
                   model does not support the coding plan feature. Please refer to the \
                   documentation to select a coding model";
        let text = model(raw);
        assert!(text.contains("开放"), "{text}");
        assert!(!text.contains("检查一下模型名"), "{text}");
    }

    #[test]
    fn tool_failures_fall_back_to_a_sentence_that_fits_any_tool() {
        assert_eq!(tool("missing command"), "这一步没成功，换个方式再试一次。");
        // 读文件类的失败仍然给具体原因
        assert!(tool("Unknown input format 'pdf'").contains("格式暂时读不了"));
    }

    #[test]
    fn run_level_english_errors_do_not_claim_a_model_failure() {
        let text = run("run deadline exceeded (max runtime 600s)");
        assert!(text.contains("超时"), "{text}");
        assert!(!text.contains("模型调用失败"), "{text}");
    }

    #[test]
    fn retryability_follows_the_4xx_rules() {
        // 瞬时：超时 / 早到 / 限流，以及 5xx
        assert!(is_retryable(
            "model request failed: HTTP 408 Request Timeout"
        ));
        assert!(is_retryable(
            "model request failed: HTTP 429 Too Many Requests"
        ));
        assert!(is_retryable("model request failed: HTTP 502 Bad Gateway"));
        // 网络抖动（无状态码）照旧重试
        assert!(is_retryable(
            "model request failed: error sending request for url (http://172.16.18.69/v1/chat/completions)"
        ));
        // 4xx 的确定性失败：重试只会白等 15s 并把队列烧掉
        assert!(!is_retryable(
            "model request failed: HTTP 400 Bad Request [InvalidParameter]"
        ));
        assert!(!is_retryable("model request failed: HTTP 401 Unauthorized"));
        assert!(!is_retryable(
            "model request failed: HTTP 404 Not Found [UnsupportedModel]"
        ));
        assert!(!is_retryable(
            "model request failed: HTTP 404 Not Found [UnsupportedModel]: does not support the coding plan feature"
        ));
        // 传输层错误里的 URL 不能被误读成状态码
        assert!(is_retryable(
            "connection refused (http://127.0.0.1:8080/v1)"
        ));
    }
}
