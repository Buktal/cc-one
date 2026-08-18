//! 密钥位置（密钥住哪）——单一事实来源。
//!
//! Provider 快照的凭据住在五个位置：`settingsConfig` 的 `env` 对象（claude
//! 的 `ANTHROPIC_*` / gemini 的 `GEMINI_API_KEY`）、`auth` 对象（codex 的
//! `OPENAI_API_KEY`，auth.json 的镜像）、`options.apiKey` 与
//! `options.headers` 认证头白名单（opencode，`Authorization` 等；元数据头
//! 如 `Helicone-*` 不是凭据，保留），以及 `meta.templateValues`（前端记录
//! 已填的 `${VAR}` 模板变量，Bedrock 预设的 AK/SK 走这里）。
//!
//! push / 导出剥（[`strip_settings_config`] / [`strip_meta`]），pull 回填
//! （[`restore_settings_config`] / [`restore_meta`]），三条消费路径共用这份
//! 位置清单——曾经 `Provider::redacted`、`provider::sync` 的 merge、
//! `provider::export_import` 各写各的，pull 侧漏了 `auth` /
//! `options.apiKey` / `options.headers` 三个位置，peer 的无密副本会静默
//! 覆盖本机凭据。
//!
//! 回填不变量（属性测试守住）：`restore(local, strip(local))` 恢复出原值——
//! strip 剥掉什么（含 `templateValues` 全剥空时整个移除），restore 就原样
//! 加回什么（被整体移除的位置重新创建）。其余结构 peer 胜出：strip 从不
//! 移除 `env` / `auth` / `options` / `headers` 对象本身，这些位置只在 peer
//! 对象存在时填充——peer 快照缺该位置（手工编辑的文件）→ 不发明结构。

use serde_json::Value;

use crate::error::{AppError, AppResult};

/// Secret env-var keys stripped from `settingsConfig` before it leaves this
/// device (the synced `providers.json` / an export): they live in the `env`
/// block (claude `ANTHROPIC_*` / gemini `GEMINI_API_KEY`), the `auth` object
/// (codex `OPENAI_API_KEY` — the auth.json mirror) and `meta.templateValues`
/// (filled `${VAR}` template variables, which is how the Bedrock presets
/// carry AK/SK) and must never enter the repo. `AWS_REGION` is deliberately
/// NOT here — it is a non-secret region code (or a `${VAR}` template-variable
/// placeholder), not a credential. This list is the single source of truth:
/// every strip / restore function in this module, `Provider::redacted`, the
/// sync merge (`provider::sync`) and the export path (`provider::export_import`)
/// all route through it.
pub const SECRET_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_ACCESS_KEY_ID",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
];

/// HTTP 认证头白名单（小写形式，匹配时大小写不敏感）：OpenCode 官方示例把
/// bearer token 放 `options.headers.Authorization`，是独立于 `options.apiKey`
/// 的认证路径，必须随同步投影一起剥掉 / 随 pull 一起回填。元数据头
/// （`Helicone-*` 等可观测性标签）不是凭据，不在此列，保留。
pub const SECRET_HEADER_KEYS: &[&str] = &["authorization", "x-api-key", "proxy-authorization"];

/// header 名是否命中认证头白名单（大小写不敏感——HTTP header 本就大小写
/// 不敏感）。
fn is_secret_header(name: &str) -> bool {
    SECRET_HEADER_KEYS.contains(&name.to_ascii_lowercase().as_str())
}

/// 剥 `settingsConfig` 里的密钥（push / 导出共用）：`env` 与 `auth` 对象里的
/// [`SECRET_ENV_KEYS`]、`options.apiKey` 整个移除、`options.headers` 里命中
/// [`SECRET_HEADER_KEYS`] 的条目移除——密钥名绝不出现在投影里。非对象
/// `options` / `options.headers` 跳过不报错（非对象无法携带这些键，无泄露
/// 风险）；`env` / `auth` 非对象 → `Err`（标准密钥位置，无法证明密钥缺失，
/// 宁可不发布）。无键被剥 → 原文 verbatim（字节稳定）；有剥 → pretty
/// 重序列化（serde_json 的 `Value` map 按键排序，输出确定）。
/// `what` 是错误消息里的主体名（调用方传 "provider" 等）。
pub fn strip_settings_config(raw: &str, what: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(raw.to_string());
    }
    let mut v: Value = serde_json::from_str(trimmed)
        .map_err(|e| AppError::Config(format!("{what} settingsConfig is not valid JSON: {e}")))?;
    let obj = v.as_object_mut().ok_or_else(|| {
        AppError::Config(format!("{what} settingsConfig is not a JSON object"))
    })?;
    let mut stripped = false;
    for loc in ["env", "auth"] {
        if let Some(map) = obj.get_mut(loc) {
            let map = map.as_object_mut().ok_or_else(|| {
                AppError::Config(format!("{what} settingsConfig {loc} is not a JSON object"))
            })?;
            for key in SECRET_ENV_KEYS {
                if map.remove(*key).is_some() {
                    stripped = true;
                }
            }
        }
    }
    if let Some(options) = obj.get_mut("options").and_then(|o| o.as_object_mut()) {
        if options.remove("apiKey").is_some() {
            stripped = true;
        }
        if let Some(headers) = options.get_mut("headers").and_then(|h| h.as_object_mut()) {
            let to_strip: Vec<String> = headers
                .keys()
                .filter(|k| is_secret_header(k))
                .cloned()
                .collect();
            for k in to_strip {
                headers.remove(&k);
                stripped = true;
            }
        }
    }
    if stripped {
        Ok(serde_json::to_string_pretty(&v)?)
    } else {
        Ok(raw.to_string())
    }
}

/// 剥 meta 里的密钥：`templateValues` 对象里的 [`SECRET_ENV_KEYS`]（前端记录
/// 的已填 `${VAR}` 模板变量——Bedrock 预设的 AK/SK 走这里）；全剥空 → 整个
/// `templateValues` 移除（不发布 `{"templateValues":{}}` 噪音）。空白 meta →
/// `{}`（无记录即无密钥）；非 JSON → `Err`；非对象 → 原样通过（无
/// `templateValues` 字符串键即无可剥的密钥位置）。无剥 → 原文 verbatim。
pub fn strip_meta(raw: &str, what: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    let mut meta: Value = if trimmed.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(trimmed)
            .map_err(|e| AppError::Config(format!("{what} meta is not valid JSON: {e}")))?
    };
    let mut stripped = false;
    if let Some(values) = meta
        .get_mut("templateValues")
        .and_then(|tv| tv.as_object_mut())
    {
        for key in SECRET_ENV_KEYS {
            if values.remove(*key).is_some() {
                stripped = true;
            }
        }
        if stripped && values.is_empty() {
            if let Some(meta_obj) = meta.as_object_mut() {
                meta_obj.remove("templateValues");
            }
        }
    }
    if stripped {
        Ok(serde_json::to_string_pretty(&meta)?)
    } else {
        Ok(raw.to_string())
    }
}

/// 把 local 的密钥回填进 peer 的去密 `settingsConfig`（pull 侧密钥守卫）：
/// peer 结构胜出，但 local 在密钥位置的值加回 peer 投影——导入可更新结构、
/// 永不把本机凭据覆盖成 peer 的无密副本。回填位置与 [`strip_settings_config`]
/// 剥的位置一一对应（`env` / `auth` / `options.apiKey` /
/// `options.headers` 白名单）。strip 从不移除 `env` / `auth` / `options`
/// 对象本身，故这些位置只在 peer 对象存在时填充——peer 快照缺该位置（手工
/// 编辑的文件）→ 结构以 peer 为准，不发明结构。
///
/// peer 的 `env` / `auth` / `options` 全缺 → 不解析 local（没有位置可回填
/// 就无需看 local——local 怎么坏都不阻塞结构导入）；否则 local 必须可解析为
/// 对象（local 密钥位置不可见就不替换，调用方跳过导入）。local 的 `env` /
/// `auth` / `options` 缺失或非对象 → 贡献零密钥，不报错。结果总是 pretty
/// 重序列化。
pub fn restore_settings_config(local: &str, peer: &str) -> AppResult<String> {
    let mut peer_value: Value = serde_json::from_str(peer.trim()).map_err(|e| {
        AppError::Config(format!("peer provider settingsConfig is not valid JSON: {e}"))
    })?;
    let peer_obj = peer_value.as_object_mut().ok_or_else(|| {
        AppError::Config("peer provider settingsConfig is not a JSON object".into())
    })?;
    // 有可回填的位置才需要看 local。
    if ["env", "auth", "options"]
        .iter()
        .any(|loc| peer_obj.get(*loc).is_some_and(Value::is_object))
    {
        let local_value: Value = serde_json::from_str(local.trim()).map_err(|e| {
            AppError::Config(format!("local provider settingsConfig is not valid JSON: {e}"))
        })?;
        let local_obj = local_value.as_object().ok_or_else(|| {
            AppError::Config("local provider settingsConfig is not a JSON object".into())
        })?;
        for loc in ["env", "auth"] {
            let Some(peer_map) = peer_obj.get_mut(loc).and_then(|v| v.as_object_mut()) else {
                continue;
            };
            let Some(local_map) = local_obj.get(loc).and_then(|v| v.as_object()) else {
                continue;
            };
            for key in SECRET_ENV_KEYS {
                if let Some(v) = local_map.get(*key) {
                    peer_map.insert((*key).to_string(), v.clone());
                }
            }
        }
        if let Some(peer_options) = peer_obj.get_mut("options").and_then(|o| o.as_object_mut()) {
            if let Some(local_options) = local_obj.get("options").and_then(|o| o.as_object()) {
                if let Some(api_key) = local_options.get("apiKey") {
                    peer_options.insert("apiKey".to_string(), api_key.clone());
                }
                if let Some(peer_headers) = peer_options
                    .get_mut("headers")
                    .and_then(|h| h.as_object_mut())
                {
                    if let Some(local_headers) =
                        local_options.get("headers").and_then(|h| h.as_object())
                    {
                        for (k, v) in local_headers {
                            if is_secret_header(k) {
                                peer_headers.insert(k.clone(), v.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(serde_json::to_string_pretty(&peer_value)?)
}

/// 把 local 的密钥回填进 peer 的去密 meta（pull 侧密钥守卫，与
/// [`restore_settings_config`] 同一角色）：`templateValues` 里的
/// [`SECRET_ENV_KEYS`] 从 local 加回 peer。peer 没有 `templateValues` →
/// 重建——strip 全剥空时会把空记录整个移除，回填必须把该位置建回来（否则
/// 全密钥的模板记录会在 sync 往返中丢失）。peer meta 非 JSON / 非对象 →
/// `Err`（无法证明其无密钥，宁可不导入）；local meta 非 JSON → `Err`（local
/// 密钥位置不可见就不替换）；local 无 `templateValues` 或非对象 → 贡献零
/// 密钥。无回填 → peer 原文 verbatim（不重写）；有回填 → pretty 重序列化。
pub fn restore_meta(local: &str, peer: &str) -> AppResult<String> {
    let parse = |raw: &str, what: &str| -> AppResult<Value> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(serde_json::json!({}));
        }
        serde_json::from_str(trimmed)
            .map_err(|e| AppError::Config(format!("{what} meta is not valid JSON: {e}")))
    };
    let mut peer_meta = parse(peer, "peer provider")?;
    let peer_obj = peer_meta.as_object_mut().ok_or_else(|| {
        AppError::Config("peer provider meta is not a JSON object".into())
    })?;
    let local_meta = parse(local, "local provider")?;
    let mut changed = false;
    if let Some(local_values) = local_meta
        .get("templateValues")
        .and_then(|tv| tv.as_object())
    {
        let peer_values = peer_obj
            .entry("templateValues")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(peer_values) = peer_values.as_object_mut() {
            for key in SECRET_ENV_KEYS {
                if let Some(v) = local_values.get(*key) {
                    peer_values.insert((*key).to_string(), v.clone());
                    changed = true;
                }
            }
        }
    }
    if changed {
        Ok(serde_json::to_string_pretty(&peer_meta)?)
    } else {
        Ok(peer.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    /// 属性不变量：restore(local, strip(local)) 恢复出原值（按解析后的 JSON
    /// 比较——strip / restore 的重序列化只改字节布局，不改结构）。三种 app
    /// 形状全测：claude 的 `env` + `meta.templateValues`、codex 的 `auth`、
    /// opencode 的 `options.apiKey` + `options.headers` 白名单（含大小写
    /// 变体与元数据头）。顺带守住 strip 幂等（剥过的再剥 → 字节不变）。
    #[test]
    fn restore_roundtrips_strip_for_every_key_location() {
        let cases: &[(&str, &str)] = &[
            // claude：env 密钥 + 非密钥 env 键；meta.templateValues 同清单。
            (
                r#"{"env":{"ANTHROPIC_BASE_URL":"https://x.dev","ANTHROPIC_AUTH_TOKEN":"sk-1","ANTHROPIC_API_KEY":"sk-2","AWS_ACCESS_KEY_ID":"AKIA1","AWS_SECRET_ACCESS_KEY":"sk-3","AWS_REGION":"us-east-1","ANTHROPIC_MODEL":"m"},"includeCoAuthoredBy":false}"#,
                r#"{"templateValues":{"AWS_REGION":"us-east-1","AWS_ACCESS_KEY_ID":"AKIA1","AWS_SECRET_ACCESS_KEY":"sk-3","ANTHROPIC_AUTH_TOKEN":"sk-1"}}"#,
            ),
            // codex：auth 对象里的 OPENAI_API_KEY（auth.json 镜像）。
            (
                r#"{"auth":{"OPENAI_API_KEY":"sk-codex"},"config":"model = \"gpt-5.6\""}"#,
                "{}",
            ),
            // opencode：options.apiKey + options.headers 白名单（大小写不
            // 敏感）；Helicone-Auth 元数据头不是凭据，保留。
            (
                r#"{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://api.deepseek.com","apiKey":"sk-opencode","headers":{"Authorization":"Bearer tok","x-api-key":"k2","PROXY-AUTHORIZATION":"p","Helicone-Auth":"meta"}}}"#,
                "{}",
            ),
        ];
        for (settings_config, meta) in cases {
            let stripped = strip_settings_config(settings_config, "provider").unwrap();
            assert_ne!(
                value(&stripped),
                value(settings_config),
                "有密钥时投影必须重写"
            );
            assert_eq!(
                value(&restore_settings_config(settings_config, &stripped).unwrap()),
                value(settings_config),
                "settingsConfig 往返恢复原值"
            );
            assert_eq!(
                strip_settings_config(&stripped, "provider").unwrap(),
                stripped,
                "strip 幂等且字节稳定"
            );
            assert_eq!(
                value(&restore_meta(meta, &strip_meta(meta, "provider").unwrap()).unwrap()),
                value(meta),
                "meta 往返恢复原值"
            );
        }
    }

    /// strip 把全密钥的 templateValues 整体移除后，restore 必须把位置建
    /// 回来——否则全是密钥的模板记录在 sync 往返中丢失（旧 merge 只在 peer
    /// 也有 templateValues 时才回填，正是这个缺口丢 Bedrock 的 AK/SK）。
    #[test]
    fn restore_recreates_template_values_strip_dropped_whole() {
        let meta = r#"{"templateValues":{"AWS_ACCESS_KEY_ID":"AKIA1","AWS_SECRET_ACCESS_KEY":"sk-3"}}"#;
        let stripped = strip_meta(meta, "provider").unwrap();
        assert_eq!(
            value(&stripped),
            serde_json::json!({}),
            "全密钥 → 空记录整体移除"
        );
        assert_eq!(
            value(&restore_meta(meta, &stripped).unwrap()),
            value(meta),
            "重建 templateValues，密钥原样回来"
        );
        // 与密钥无关的 meta 字段不受影响。
        let other = r#"{"foo":"bar","templateValues":{"AWS_SECRET_ACCESS_KEY":"sk"}}"#;
        let stripped_other = strip_meta(other, "provider").unwrap();
        assert_eq!(value(&stripped_other), serde_json::json!({"foo": "bar"}));
        assert_eq!(
            value(&restore_meta(other, &stripped_other).unwrap()),
            value(other)
        );
    }

    /// restore 只回填密钥位置：非密钥键不发明（peer 结构胜出），被剥的结构
    /// （strip 从不移除对象本身——缺了就是 peer 真没有）不重建。
    #[test]
    fn restore_fills_only_secret_locations_and_keeps_peer_structure() {
        // env：local 的非密钥键（ANTHROPIC_MODEL）不回填，密钥回填。
        let merged = restore_settings_config(
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://old.dev","ANTHROPIC_MODEL":"m","ANTHROPIC_AUTH_TOKEN":"sk-1"}}"#,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://new.dev"}}"#,
        )
        .unwrap();
        let v = value(&merged);
        assert_eq!(
            v["env"]["ANTHROPIC_BASE_URL"], "https://new.dev",
            "结构以 peer 为准"
        );
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-1", "密钥位置回填");
        assert!(
            v["env"].get("ANTHROPIC_MODEL").is_none(),
            "非密钥键不发明"
        );

        // peer 没有 options（strip 从不移除 options 对象 → 只可能是手工文件）
        // → 不回填 local 的 options.apiKey。
        let no_options = restore_settings_config(
            r#"{"options":{"apiKey":"sk"}}"#,
            r#"{"npm":"@ai-sdk/openai-compatible"}"#,
        )
        .unwrap();
        assert!(
            value(&no_options).get("options").is_none(),
            "peer 缺位置 → 不发明结构"
        );
    }
}
