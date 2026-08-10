//! 模型列表获取（OpenAI 兼容 `GET /v1/models`）。
//!
//! WebView 里 fetch 会撞 CORS，所以请求必须由后端发（ureq，与
//! `pricing::fetch_litellm` 同一 HTTP 栈）。候选 URL 构造
//! （[`candidate_models_urls`]）是纯函数：modelsUrl 覆写 → baseURL 拼
//! `/v1/models` → 版本段识别（`/v1` 结尾拼 `/models`）→ 兼容子路径剥离 →
//! 去重保序最多 3 条——全部可单测，不碰网络。请求按候选顺序尝试，首个成功
//! 即返回。
//!
//! 错误串带稳定前缀标签，前端按标签分桶成 toast 提示（分桶函数
//! `bucketFetchModelsError` 与标签契约一一对应）：
//! - `AUTH_FAILED:` —— 401/403（认证失败）
//! - `ENDPOINT_CLOSED:` —— 404/405（这个 URL 不是模型端点）或全部候选失败
//!   （端点未开放）
//! - `TIMEOUT:` —— 请求超时
//! - `BAD_FORMAT:` —— 2xx 但响应不是 OpenAI 兼容的 `{ "data": [{ "id": … }] }`
//!   （格式不支持）
//! - `NETWORK:` —— 其余传输错误与其余 HTTP 状态码（兜底）
//!
//! 标签后的详情（如 `HTTP 401: <body>`）保留给用户看，不参与分桶。改标签
//! 必须同步改前端的 `bucketFetchModelsError`。

use std::time::Duration;

use crate::error::{AppError, AppResult};

/// 已知的「Anthropic 协议兼容子路径」后缀，按长度降序排列（长后缀优先）。
/// baseURL 命中任一后缀时，候选列表追加「剥离后缀再拼 /v1/models、/models」
/// 的版本——这些供应商的 OpenAI 兼容端点位于剥离后的根路径上（DeepSeek
/// `/anthropic`、智谱 `/api/anthropic`、百炼 `/apps/anthropic`、火山
/// `/api/coding`、StepFun `/step_plan` 等都因此兜底可用）。
const COMPAT_SUFFIXES: &[&str] = &[
    "/api/claudecode",
    "/api/anthropic",
    "/apps/anthropic",
    "/api/coding",
    "/claudecode",
    "/anthropic",
    "/step_plan",
    "/coding",
    "/claude",
];

/// 单个候选请求的超时。
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// 错误响应体截断长度：401/404 的 HTML 错误页不该整页进错误串。
const ERROR_BODY_MAX_CHARS: usize = 512;

/// 候选数量上限：最多尝试 3 条。
const MAX_CANDIDATES: usize = 3;

/// 错误串前缀标签——前端 `bucketFetchModelsError` 按这些前缀分桶，契约见
/// 模块文档。标签是前后端之间的稳定接口，改动必须两端同步。
const TAG_AUTH: &str = "AUTH_FAILED";
const TAG_ENDPOINT: &str = "ENDPOINT_CLOSED";
const TAG_TIMEOUT: &str = "TIMEOUT";
const TAG_FORMAT: &str = "BAD_FORMAT";
const TAG_NETWORK: &str = "NETWORK";

/// 构造带标签的模型获取错误。
fn fetch_err(tag: &str, detail: impl std::fmt::Display) -> AppError {
    AppError::FetchModels(format!("{tag}: {detail}"))
}

/// 构造「模型列表端点」的候选 URL 列表（纯函数，不碰网络）。
///
/// 候选顺序：
/// 1. `models_url` 覆写非空 → 只返回它（精确指路：个别预设的端点拼不出
///    正确候选，如火山 `/api/compatible` 不在剥离清单里，预设自带覆写）；
/// 2. baseURL（trim + 去尾部斜杠）已以版本段 `/v{N}` 结尾（`/v1`、智谱
///    `/api/coding/paas/v4` 等）→ 拼 `/models`——版本号已在路径里，再补
///    `/v1` 会得到不存在的 `.../v4/v1/models`；版本段非 `/v1` 时追加
///    `/v1/models` 作为兜底次候选（正确路径在前）；
/// 3. 未以版本段结尾 → 拼 `/v1/models`；
/// 4. baseURL 命中 [`COMPAT_SUFFIXES`] → 剥离后缀（长后缀优先）再拼
///    `/v1/models`、`/models`（剥离后无 scheme 或为空 → 跳过）。
///
/// 结果去重保序、最多 [`MAX_CANDIDATES`] 条。baseURL 为空 → 空列表（调用方
/// 报错，前端预检已挡）。
pub fn candidate_models_urls(base_url: &str, models_url: Option<&str>) -> Vec<String> {
    if let Some(raw) = models_url {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return vec![trimmed.to_string()];
        }
    }
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    if ends_with_version_segment(base) {
        candidates.push(format!("{base}/models"));
        if !base.ends_with("/v1") {
            candidates.push(format!("{base}/v1/models"));
        }
    } else {
        candidates.push(format!("{base}/v1/models"));
    }
    if let Some(stripped) = strip_compat_suffix(base) {
        let root = stripped.trim_end_matches('/');
        if !root.is_empty() && root.contains("://") {
            candidates.push(format!("{root}/v1/models"));
            candidates.push(format!("{root}/models"));
        }
    }
    dedup_keep_first(candidates, MAX_CANDIDATES)
}

/// 去重保序 + 数量上限。候选空间小，线性 `contains` 即可。独立成函数让
/// 去重 / 上限有直接测试入口——当前 9 个后缀下自然输入碰不到重复，但清单
/// 后续扩展（如补 `/v1`、`/compatible`）时去重必须仍成立。
fn dedup_keep_first(candidates: Vec<String>, max: usize) -> Vec<String> {
    let mut unique: Vec<String> = Vec::new();
    for url in candidates {
        if unique.len() >= max {
            break;
        }
        if !unique.contains(&url) {
            unique.push(url);
        }
    }
    unique
}

/// 判断 baseURL 是否以版本段 `/v{N}` 结尾（N 为一个或多个数字），如 `/v1`、
/// `.../coding/paas/v4`。这类 URL 版本号已在路径里，模型端点应为
/// `{base}/models`，不能再补 `/v1`。
fn ends_with_version_segment(url: &str) -> bool {
    let last = url.rsplit('/').next().unwrap_or("");
    last.strip_prefix('v')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// 若 baseURL 以任一 [`COMPAT_SUFFIXES`] 结尾，返回剥离后缀后的剩余部分；
/// 否则 `None`。清单按长度降序排列，保证最长后缀优先命中（否则 `/anthropic`
/// 会提前匹配掉 `/api/anthropic` 的场景）。
fn strip_compat_suffix(base_url: &str) -> Option<&str> {
    for suffix in COMPAT_SUFFIXES {
        if let Some(stripped) = base_url.strip_suffix(suffix) {
            return Some(stripped);
        }
    }
    None
}

/// 获取供应商的可用模型列表（模型 id，按 id 排序）。按候选顺序尝试，首个
/// 成功立即返回。失败错误串带模块文档里的前缀标签，前端按标签分桶提示。
pub fn fetch_models(
    base_url: &str,
    api_key: &str,
    models_url: Option<&str>,
) -> AppResult<Vec<String>> {
    fetch_models_with_timeout(base_url, api_key, models_url, FETCH_TIMEOUT)
}

/// `fetch_models` 的可测内层：超时作为参数注入，让超时用例不必真等 10 秒。
fn fetch_models_with_timeout(
    base_url: &str,
    api_key: &str,
    models_url: Option<&str>,
    timeout: Duration,
) -> AppResult<Vec<String>> {
    if api_key.trim().is_empty() {
        return Err(fetch_err(TAG_AUTH, "api key is empty"));
    }
    let candidates = candidate_models_urls(base_url, models_url);
    if candidates.is_empty() {
        return Err(fetch_err(TAG_ENDPOINT, "base url is empty"));
    }
    let mut last_not_found = String::new();
    let trimmed_key = api_key.trim();
    for url in &candidates {
        let request = ureq::get(url)
            .timeout(timeout)
            .set("Authorization", &format!("Bearer {trimmed_key}"))
            .set(
                "User-Agent",
                &format!("cc one/{}", env!("CARGO_PKG_VERSION")),
            );
        match request.call() {
            Ok(response) => {
                let body = response.into_string().unwrap_or_default();
                return parse_models_response(&body);
            }
            Err(ureq::Error::Status(status, response)) => {
                let body = truncate_body(&response.into_string().unwrap_or_default());
                // 401/403 = 认证失败；404/405 = 这个 URL 不是模型端点，试下一个；
                // 其余状态码立即失败（详情进 NETWORK 兜底桶）。
                match status {
                    401 | 403 => return Err(fetch_err(TAG_AUTH, format!("HTTP {status}: {body}"))),
                    404 | 405 => {
                        last_not_found = format!("HTTP {status}: {body}");
                        continue;
                    }
                    _ => return Err(fetch_err(TAG_NETWORK, format!("HTTP {status}: {body}"))),
                }
            }
            Err(e) => {
                if is_timeout(&e) {
                    return Err(fetch_err(TAG_TIMEOUT, e));
                }
                return Err(fetch_err(TAG_NETWORK, e));
            }
        }
    }
    Err(fetch_err(
        TAG_ENDPOINT,
        format!("all candidates failed: {last_not_found}"),
    ))
}

/// 解析 OpenAI 兼容的 /v1/models 响应体（`{ "data": [{ "id": … }] }`），按 id
/// 排序后返回。2xx 但解析不了 / 形状不对 → `BAD_FORMAT`（格式不支持）——
/// 不猜测其他响应形状，OpenAI 兼容格式就是 `data` 数组。
fn parse_models_response(body: &str) -> AppResult<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    let parsed: ModelsResponse =
        serde_json::from_str(body).map_err(|e| fetch_err(TAG_FORMAT, format!("{e}")))?;
    let mut models: Vec<String> = parsed.data.into_iter().map(|m| m.id).collect();
    models.sort();
    Ok(models)
}

/// 截断响应体到 [`ERROR_BODY_MAX_CHARS`] 字符，避免 HTML 错误页占满错误串。
fn truncate_body(body: &str) -> String {
    if body.chars().count() <= ERROR_BODY_MAX_CHARS {
        body.to_string()
    } else {
        let mut s: String = body.chars().take(ERROR_BODY_MAX_CHARS).collect();
        s.push('…');
        s
    }
}

/// ureq 传输错误是否超时：ureq 2.12 把超时包装成 `ErrorKind::Io` 的
/// `Transport`，原始 `io::Error` 的 kind 是 `TimedOut`（连接超时与读超时都
/// 走这条路径）。其余传输错误按 NETWORK 处理。
fn is_timeout(e: &ureq::Error) -> bool {
    let ureq::Error::Transport(t) = e else {
        return false;
    };
    if t.kind() != ureq::ErrorKind::Io {
        return false;
    }
    std::error::Error::source(t)
        .and_then(|src| src.downcast_ref::<std::io::Error>())
        .is_some_and(|ioe| ioe.kind() == std::io::ErrorKind::TimedOut)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

    use super::*;

    // ---------------- candidate_models_urls ----------------

    #[test]
    fn override_wins_and_ignores_base() {
        let c = candidate_models_urls(
            "https://api.deepseek.com/anthropic",
            Some("https://api.deepseek.com/models"),
        );
        assert_eq!(c, vec!["https://api.deepseek.com/models"]);
    }

    #[test]
    fn override_blank_falls_through() {
        let c = candidate_models_urls("https://api.siliconflow.cn", Some("   "));
        assert_eq!(c, vec!["https://api.siliconflow.cn/v1/models"]);
    }

    #[test]
    fn plain_root_appends_v1_models() {
        let c = candidate_models_urls("https://api.siliconflow.cn", None);
        assert_eq!(c, vec!["https://api.siliconflow.cn/v1/models"]);
    }

    #[test]
    fn trailing_slash_is_trimmed() {
        let c = candidate_models_urls("https://api.example.com/", None);
        assert_eq!(c, vec!["https://api.example.com/v1/models"]);
    }

    #[test]
    fn v1_ending_appends_models() {
        let c = candidate_models_urls("https://api.example.com/v1", None);
        assert_eq!(c, vec!["https://api.example.com/v1/models"]);
    }

    #[test]
    fn non_v1_version_segment_appends_models_then_v1_models() {
        // 智谱 Coding Plan 端点以 /v4 版本段结尾：模型端点是 {base}/models，
        // 正确路径必须排在 .../v4/v1/models（404）之前。
        let c = candidate_models_urls("https://open.bigmodel.cn/api/coding/paas/v4", None);
        assert_eq!(
            c,
            vec![
                "https://open.bigmodel.cn/api/coding/paas/v4/models",
                "https://open.bigmodel.cn/api/coding/paas/v4/v1/models",
            ]
        );
    }

    #[test]
    fn version_segment_detection() {
        assert!(ends_with_version_segment("https://x.com/v1"));
        assert!(ends_with_version_segment(
            "https://open.bigmodel.cn/api/coding/paas/v4"
        ));
        assert!(ends_with_version_segment("https://x.com/v10"));
        assert!(!ends_with_version_segment("https://x.com/api"));
        assert!(!ends_with_version_segment("https://x.com/vX"));
        assert!(!ends_with_version_segment("https://x.com/models"));
        assert!(!ends_with_version_segment("https://api.siliconflow.cn"));
    }

    #[test]
    fn empty_base_yields_no_candidates() {
        assert!(candidate_models_urls("", None).is_empty());
        assert!(candidate_models_urls("   ", None).is_empty());
    }

    /// 9 种兼容后缀各一个典型 case：期望顺序恒为
    /// [base/v1/models, root/v1/models, root/models]。
    #[test]
    fn each_compat_suffix_strips_to_root() {
        let cases: &[(&str, &str)] = &[
            (
                "https://api.deepseek.com/anthropic",
                "https://api.deepseek.com",
            ),
            (
                "https://open.bigmodel.cn/api/anthropic",
                "https://open.bigmodel.cn",
            ),
            (
                "https://coding.dashscope.aliyuncs.com/apps/anthropic",
                "https://coding.dashscope.aliyuncs.com",
            ),
            (
                "https://ark.cn-beijing.volces.com/api/coding",
                "https://ark.cn-beijing.volces.com",
            ),
            (
                "https://www.right.codes/claudecode",
                "https://www.right.codes",
            ),
            (
                "https://api.example.com/api/claudecode",
                "https://api.example.com",
            ),
            (
                "https://api.stepfun.com/step_plan",
                "https://api.stepfun.com",
            ),
            ("https://api.kimi.com/coding", "https://api.kimi.com"),
            ("https://www.right.codes/claude", "https://www.right.codes"),
        ];
        for (base, root) in cases {
            assert_eq!(
                candidate_models_urls(base, None),
                vec![
                    format!("{base}/v1/models"),
                    format!("{root}/v1/models"),
                    format!("{root}/models"),
                ],
                "case: {base}"
            );
        }
    }

    #[test]
    fn longest_suffix_wins() {
        // /api/anthropic 结尾应剥离整个 /api/anthropic，而不是只剥 /anthropic
        // （那样会得到残缺的 .../api 根）。
        let c = candidate_models_urls("https://api.z.ai/api/anthropic", None);
        assert_eq!(
            c,
            vec![
                "https://api.z.ai/api/anthropic/v1/models",
                "https://api.z.ai/v1/models",
                "https://api.z.ai/models",
            ]
        );
    }

    #[test]
    fn no_suffix_means_no_strip() {
        // OpenRouter 的 /api 不在剥离清单里——它本身是 OpenAI 兼容根。
        let c = candidate_models_urls("https://openrouter.ai/api", None);
        assert_eq!(c, vec!["https://openrouter.ai/api/v1/models"]);
    }

    #[test]
    fn dedup_keeps_first_occurrence_and_caps_at_three() {
        // 直接喂重复候选（生产路径里的去重 + 上限步骤）：保序去重，最多 3 条。
        let c = dedup_keep_first(
            vec![
                "https://a/v1/models".into(),
                "https://b/models".into(),
                "https://a/v1/models".into(),
                "https://c/models".into(),
            ],
            MAX_CANDIDATES,
        );
        assert_eq!(
            c,
            vec![
                "https://a/v1/models".to_string(),
                "https://b/models".to_string(),
                "https://c/models".to_string(),
            ]
        );
    }

    #[test]
    fn version_segment_prevents_suffix_strip() {
        // 版本段是 URL 最后一段，后缀剥离要求 URL 以已知后缀结尾——两者互斥：
        // /v4 命中版本段分支后不会再剥 /coding，只产出 {base}/models 与
        // {base}/v1/models 两个候选（自然输入上限其实到不了 3 条，去重 /
        // 上限步骤由 dedup_keep_first 的直接用例守住）。
        let c = candidate_models_urls("https://x.com/coding/v4", None);
        assert_eq!(
            c,
            vec![
                "https://x.com/coding/v4/models",
                "https://x.com/coding/v4/v1/models",
            ]
        );
    }

    // ---------------- fetch loop（本地真实 HTTP 服务器，跑完整生产路径）----

    /// 取错误串（断言全部失败路径都是 FetchModels 变体）。
    fn fetch_err_msg(r: AppResult<Vec<String>>) -> String {
        match r.unwrap_err() {
            AppError::FetchModels(msg) => msg,
            other => panic!("expected AppError::FetchModels, got {other}"),
        }
    }

    /// 极简 HTTP 测试服务器：按请求路径返回固定状态码 + 响应体，每连接处理
    /// 一个请求后关闭。真实 HTTP 栈（ureq）打它跟打真端点无异——测试覆盖
    /// 「候选遍历 → 状态分桶 → 解析」的完整生产路径。
    struct TestServer {
        url: String,
    }

    impl TestServer {
        fn start(routes: &[(&str, u16, &str)]) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
            let url = format!("http://{}", listener.local_addr().expect("local addr"));
            let routes: Arc<Vec<(String, u16, String)>> = Arc::new(
                routes
                    .iter()
                    .map(|(p, s, b)| (p.to_string(), *s, b.to_string()))
                    .collect(),
            );
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    let routes = Arc::clone(&routes);
                    thread::spawn(move || handle_request(stream, &routes));
                }
            });
            Self { url }
        }

        /// 该服务器上某路径的完整 URL。
        fn endpoint(&self, path: &str) -> String {
            format!("{}{}", self.url, path)
        }
    }

    fn handle_request(mut stream: std::net::TcpStream, routes: &[(String, u16, String)]) {
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let text = String::from_utf8_lossy(&buf);
        let path = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/");
        let (status, body) = routes
            .iter()
            .find(|(p, _, _)| p == path)
            .map(|(_, s, b)| (*s, b.clone()))
            .unwrap_or((404, "not found".to_string()));
        let reason = match status {
            200 => "OK",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            _ => "Error",
        };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
    }

    /// 两条模型，故意乱序，断言按 id 排序返回。
    const MODELS_JSON: &str = r#"{"object":"list","data":[{"id":"b-model","object":"model"},{"id":"a-model","object":"model","owned_by":"test"}]}"#;

    #[test]
    fn fetch_success_returns_sorted_ids() {
        let server = TestServer::start(&[("/v1/models", 200, MODELS_JSON)]);
        let models = fetch_models(&server.endpoint("/v1"), "sk-test", None).unwrap();
        assert_eq!(models, vec!["a-model", "b-model"]);
    }

    #[test]
    fn fetch_401_maps_to_auth_tag() {
        let server = TestServer::start(&[("/v1/models", 401, "{\"error\":\"invalid key\"}")]);
        let msg = fetch_err_msg(fetch_models(&server.endpoint(""), "sk-test", None));
        assert!(msg.starts_with("AUTH_FAILED: HTTP 401: "), "got: {msg}");
        assert!(msg.contains("invalid key"), "got: {msg}");
    }

    #[test]
    fn fetch_403_maps_to_auth_tag() {
        let server = TestServer::start(&[("/v1/models", 403, "forbidden")]);
        let msg = fetch_err_msg(fetch_models(&server.endpoint(""), "sk-test", None));
        assert!(msg.starts_with("AUTH_FAILED: HTTP 403: "), "got: {msg}");
    }

    #[test]
    fn fetch_continues_past_404_to_next_candidate() {
        // 首个候选（/anthropic/v1/models）404 → 试第二个（根 /v1/models）→ 成功。
        let server = TestServer::start(&[
            ("/anthropic/v1/models", 404, "<html>not found</html>"),
            ("/v1/models", 200, MODELS_JSON),
        ]);
        let models = fetch_models(&server.endpoint("/anthropic"), "sk-test", None).unwrap();
        assert_eq!(models, vec!["a-model", "b-model"]);
    }

    #[test]
    fn fetch_all_candidates_404_maps_to_endpoint_tag() {
        let server = TestServer::start(&[
            ("/anthropic/v1/models", 404, "nope"),
            ("/v1/models", 404, "nope"),
            ("/models", 404, "nope"),
        ]);
        let msg = fetch_err_msg(fetch_models(
            &server.endpoint("/anthropic"),
            "sk-test",
            None,
        ));
        assert!(
            msg.starts_with("ENDPOINT_CLOSED: all candidates failed: "),
            "got: {msg}"
        );
        assert!(msg.contains("HTTP 404"), "got: {msg}");
    }

    #[test]
    fn fetch_405_maps_to_endpoint_tag() {
        let server = TestServer::start(&[("/v1/models", 405, "no get")]);
        let msg = fetch_err_msg(fetch_models(&server.endpoint(""), "sk-test", None));
        assert!(
            msg.starts_with("ENDPOINT_CLOSED: all candidates failed: HTTP 405"),
            "got: {msg}"
        );
    }

    #[test]
    fn fetch_other_status_maps_to_network_tag() {
        let server = TestServer::start(&[("/v1/models", 500, "boom")]);
        let msg = fetch_err_msg(fetch_models(&server.endpoint(""), "sk-test", None));
        assert!(msg.starts_with("NETWORK: HTTP 500: "), "got: {msg}");
    }

    #[test]
    fn fetch_garbage_body_maps_to_format_tag() {
        let server = TestServer::start(&[("/v1/models", 200, "<html>captive portal</html>")]);
        let msg = fetch_err_msg(fetch_models(&server.endpoint(""), "sk-test", None));
        assert!(msg.starts_with("BAD_FORMAT: "), "got: {msg}");
    }

    #[test]
    fn fetch_body_without_data_maps_to_format_tag() {
        let server = TestServer::start(&[("/v1/models", 200, "{\"object\":\"list\"}")]);
        let msg = fetch_err_msg(fetch_models(&server.endpoint(""), "sk-test", None));
        assert!(msg.starts_with("BAD_FORMAT: "), "got: {msg}");
    }

    #[test]
    fn fetch_missing_api_key_maps_to_auth_tag() {
        let msg = fetch_err_msg(fetch_models("https://example.com", "", None));
        assert!(msg.starts_with("AUTH_FAILED: "), "got: {msg}");
    }

    #[test]
    fn fetch_empty_base_maps_to_endpoint_tag() {
        let msg = fetch_err_msg(fetch_models("", "sk-test", None));
        assert!(msg.starts_with("ENDPOINT_CLOSED: "), "got: {msg}");
    }

    #[test]
    fn fetch_timeout_maps_to_timeout_tag() {
        // 服务器接受连接但永不响应，逼客户端在注入的短超时内失败。
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind timeout server");
        let url = format!("http://{}", listener.local_addr().expect("local addr"));
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                thread::sleep(Duration::from_secs(30));
            }
        });
        let msg = fetch_err_msg(fetch_models_with_timeout(
            &url,
            "sk-test",
            None,
            Duration::from_millis(200),
        ));
        assert!(msg.starts_with("TIMEOUT: "), "got: {msg}");
    }

    #[test]
    fn truncate_body_keeps_head_and_marks_ellipsis() {
        let long = "x".repeat(ERROR_BODY_MAX_CHARS + 100);
        let out = truncate_body(&long);
        assert_eq!(out.chars().count(), ERROR_BODY_MAX_CHARS + 1);
        assert!(out.ends_with('…'));
        assert_eq!(truncate_body("short"), "short");
    }
}
