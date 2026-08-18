//! 供应商导出 / 导入（手动迁移）：全部供应商序列化为一份 JSON 文档（可选剔除
//! API key），或从文档按「合并 / 覆盖」模式写回本机 DB。供换设备迁移 / 留档用。
//!
//! **应用维度**：文档每行（Provider 序列化）自带 `app` 字段；冲突规划键从
//! `id` 变为 `(app, id)`——同一 id 在不同应用池是两个条目，互不冲突。旧文档
//! （应用维度之前导出的）行没有 `app` 字段，读为 claude（serde default）——
//! 存量数据全部归入 Claude 池。
//!
//! 本模块**不经过 git 同步**：导入只走 `store.save_provider` 写 DB，绝不写
//! providers.json 同步文件——导入的 key 只进本机库。「不触发同步写」是结构性的
//! （本模块没有同步文件路径，命令只调 `apply_import`），不用测试守。
//!
//! 纯函数（测试接缝）：`export_document`（provider 列表 → 文档文本，可选走
//! [`Provider::redacted`] 剥密钥）、`parse_export_document`（文档文本 →
//! provider 列表，版本校验）。`apply_import` 是文档导入的 store 层入口：解析
//! 文档 → 冲突规划——规划本身是 [`crate::provider::import`] 的 store 层 seam
//! （AppId 策略，导出文档 / CC-Switch / live 三条路径共用同一份冲突代码）。
//! 命令直接调它，测试也直接调它——测试跑的就是生产路径。

use serde::{Deserialize, Serialize};

use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::model::Provider;
use crate::provider::import::{
    import_providers, ImportKeyStrategy, ProviderImportMode, ProviderImportReport,
};

/// 当前导出文档版本。导入只认这个版本——未来格式演进时，旧版 app 读到新文档
/// 会明确报错而不是静默错解。版本号**不因加 app 字段而升**：老文档（行无 app）
/// 与新文档（行带 app）都是 v1——serde 对未知字段忽略，新读旧 = 全归 claude
/// 池，旧读新 = app 字段被忽略；版本只在格式真的不兼容时才升。
pub const EXPORT_VERSION: u32 = 1;

/// 导出文档：版本号 + 导出时间 + provider 列表。Provider 自身序列化已有
/// `rename_all = "camelCase"`，直接复用（每行自带 `app` 字段）。不跨
/// Rust→JS 边界（导出返回 JSON 文本、导入收 JSON 文本），所以不需要
/// `specta::Type`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderExportDocument {
    pub version: u32,
    pub exported_at: String,
    pub providers: Vec<Provider>,
}

/// 全部供应商 → 导出文档 JSON 文本。`include_keys=false` 时对每个 provider
/// 调 [`Provider::redacted`]（密钥位置清单在 [`crate::provider::keys`]：剥
/// settingsConfig 的 `env` / `auth`、opencode 的 `options.apiKey` /
/// `options.headers` 认证头白名单与 meta.templateValues；剥不了 → `Err` 拒绝
/// 导出——宁可不导，不能导出无法证明无密钥的配置）；`include_keys=true` 时
/// settingsConfig 原样保留（往返后字节一致）。
pub fn export_document(
    providers: &[Provider],
    include_keys: bool,
    exported_at: &str,
) -> AppResult<String> {
    let providers: Vec<Provider> = if include_keys {
        providers.to_vec()
    } else {
        providers
            .iter()
            .map(|p| p.redacted())
            .collect::<AppResult<Vec<Provider>>>()?
    };
    let doc = ProviderExportDocument {
        version: EXPORT_VERSION,
        exported_at: exported_at.to_string(),
        providers,
    };
    Ok(serde_json::to_string_pretty(&doc)?)
}

/// 文档 JSON 文本 → provider 列表。非法 JSON / 非对象 / 版本不认识 → `Err`
/// （宁可不导，不能错解）。
pub fn parse_export_document(json: &str) -> AppResult<Vec<Provider>> {
    let doc: ProviderExportDocument = serde_json::from_str(json)
        .map_err(|e| AppError::Config(format!("provider export is not valid JSON: {e}")))?;
    if doc.version != EXPORT_VERSION {
        return Err(AppError::Config(format!(
            "unsupported provider export version {} (expected {EXPORT_VERSION})",
            doc.version
        )));
    }
    Ok(doc.providers)
}

/// 导入全流程（store 层，命令直接调这个）：解析文档 → 冲突规划（store 层
/// seam 的 AppId 策略，merge / overwrite 语义见 [`crate::provider::import`]）
/// → 逐条 `save_provider` 写回本机 DB。只写 DB——不碰任何同步文件，导入的
/// key 只进本机库。
pub fn apply_import(
    store: &Store,
    json: &str,
    mode: ProviderImportMode,
) -> AppResult<ProviderImportReport> {
    let incoming = parse_export_document(json)?;
    import_providers(store, &incoming, ImportKeyStrategy::AppId(mode))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::mem;
    use crate::model::{App, ProviderCategory};
    use crate::provider::keys::SECRET_ENV_KEYS;

    /// 构造一份带 env 密钥的 settingsConfig 文本（含非密钥 env 键和顶层字段，
    /// 模拟真实快照）。
    fn config_with_secrets() -> String {
        r#"{
  "env": {
    "ANTHROPIC_BASE_URL": "https://api.example.com",
    "ANTHROPIC_AUTH_TOKEN": "sk-secret-1",
    "ANTHROPIC_API_KEY": "sk-secret-2",
    "AWS_ACCESS_KEY_ID": "AKIA123",
    "AWS_SECRET_ACCESS_KEY": "wxyz",
    "KEEP_ME": "1"
  },
  "includeCoAuthoredBy": false
}"#
        .to_string()
    }

    fn provider(id: &str, name: &str, settings_config: &str) -> Provider {
        provider_with_meta(id, name, settings_config, r#"{}"#)
    }

    fn provider_with_meta(id: &str, name: &str, settings_config: &str, meta: &str) -> Provider {
        Provider {
            id: id.into(),
            name: name.into(),
            website_url: "https://example.com".into(),
            category: ProviderCategory::Custom,
            app: App::Claude,
            icon: String::new(),
            icon_color: String::new(),
            sort_index: 0,
            notes: String::new(),
            settings_config: settings_config.into(),
            meta: meta.into(),
            updated_at: String::new(),
        }
    }

    fn parsed(s: &str) -> ProviderExportDocument {
        serde_json::from_str(s).unwrap()
    }

    fn env_of(doc: &ProviderExportDocument, name: &str) -> serde_json::Value {
        let cfg: serde_json::Value = serde_json::from_str(
            &doc.providers
                .iter()
                .find(|p| p.name == name)
                .unwrap()
                .settings_config,
        )
        .unwrap();
        cfg["env"].clone()
    }

    #[test]
    fn export_without_keys_strips_secret_env_keys_keeps_rest() {
        let ps = [
            provider("a", "Alpha", &config_with_secrets()),
            provider("b", "Beta", r#"{"env":{"ANTHROPIC_MODEL":"m"}}"#),
        ];
        let doc = parsed(&export_document(&ps, false, "2026-08-07T00:00:00Z").unwrap());
        assert_eq!(doc.version, 1);
        assert_eq!(doc.exported_at, "2026-08-07T00:00:00Z");
        let env = env_of(&doc, "Alpha");
        assert!(
            env.get("ANTHROPIC_AUTH_TOKEN").is_none(),
            "AUTH_TOKEN 必须被剥"
        );
        assert!(env.get("ANTHROPIC_API_KEY").is_none(), "API_KEY 必须被剥");
        assert!(env.get("AWS_ACCESS_KEY_ID").is_none());
        assert!(env.get("AWS_SECRET_ACCESS_KEY").is_none());
        assert_eq!(
            env["ANTHROPIC_BASE_URL"],
            serde_json::json!("https://api.example.com")
        );
        assert_eq!(env["KEEP_ME"], serde_json::json!("1"));
        assert_eq!(
            env_of(&doc, "Beta"),
            serde_json::json!({"ANTHROPIC_MODEL": "m"}),
            "不含密钥的配置原样保留"
        );
    }

    #[test]
    fn export_without_keys_keeps_top_level_fields() {
        let ps = [provider("a", "Alpha", &config_with_secrets())];
        let doc = parsed(&export_document(&ps, false, "ts").unwrap());
        let cfg: serde_json::Value =
            serde_json::from_str(&doc.providers[0].settings_config).unwrap();
        assert_eq!(cfg["includeCoAuthoredBy"], serde_json::json!(false));
    }

    #[test]
    fn export_with_keys_keeps_settings_config_verbatim() {
        let ps = [provider("a", "Alpha", &config_with_secrets())];
        let doc = parsed(&export_document(&ps, true, "ts").unwrap());
        assert_eq!(
            doc.providers[0].settings_config,
            config_with_secrets(),
            "含 key 导出必须逐字节保留 settingsConfig"
        );
    }

    #[test]
    fn export_without_keys_strips_secret_template_values_from_meta() {
        // Bedrock 供应商的 AK/SK 走 `${VAR}` 模板变量，填值记录在
        // meta.templateValues——导出剥 key 时必须一并剥掉。
        let ps = [provider_with_meta(
            "a",
            "Bedrock",
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://bedrock-runtime.${AWS_REGION}.amazonaws.com","AWS_REGION":"us-east-1"}}"#,
            r#"{"templateValues":{"AWS_REGION":"us-east-1","AWS_ACCESS_KEY_ID":"AKIA123","AWS_SECRET_ACCESS_KEY":"top-secret"}}"#,
        )];
        let doc = parsed(&export_document(&ps, false, "ts").unwrap());
        let meta: serde_json::Value = serde_json::from_str(&doc.providers[0].meta).unwrap();
        let values = &meta["templateValues"];
        assert!(values.get("AWS_ACCESS_KEY_ID").is_none(), "AK 必须被剥");
        assert!(values.get("AWS_SECRET_ACCESS_KEY").is_none(), "SK 必须被剥");
        assert_eq!(values["AWS_REGION"], serde_json::json!("us-east-1"));
        // 密钥名不出现在整个导出文档里。
        let text = export_document(&ps, false, "ts").unwrap();
        for key in SECRET_ENV_KEYS {
            assert!(!text.contains(key), "{key} 不得出现在导出文档");
        }
    }

    /// 导出剥 key 走与同步投影相同的五处清单：codex 的 auth 与 opencode 的
    /// options.apiKey / 认证头白名单同样被剥——清单已收敛到 provider::keys，
    /// redacted 是唯一剥点（曾只覆盖 env 与 templateValues）。
    #[test]
    fn export_without_keys_strips_codex_and_opencode_locations() {
        let ps = [
            Provider {
                app: App::Codex,
                ..provider(
                    "a",
                    "Codex-A",
                    r#"{"auth":{"OPENAI_API_KEY":"sk-codex"},"config":"model = \"gpt-5.6\""}"#,
                )
            },
            Provider {
                app: App::OpenCode,
                ..provider(
                    "b",
                    "OpenCode-B",
                    r#"{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://api.deepseek.com","apiKey":"sk-opencode","headers":{"Authorization":"Bearer tok","Helicone-Auth":"meta"}}}"#,
                )
            },
        ];
        let text = export_document(&ps, false, "ts").unwrap();
        assert!(!text.contains("sk-codex"));
        assert!(!text.contains("sk-opencode"));
        assert!(!text.contains("Bearer tok"));
        let doc = parsed(&text);
        let codex: serde_json::Value = serde_json::from_str(
            &doc.providers
                .iter()
                .find(|p| p.app == App::Codex)
                .unwrap()
                .settings_config,
        )
        .unwrap();
        assert!(codex["auth"].get("OPENAI_API_KEY").is_none(), "codex auth 必须剥");
        assert_eq!(
            codex["config"],
            serde_json::json!("model = \"gpt-5.6\""),
            "非密钥字段保留"
        );
        let opencode: serde_json::Value = serde_json::from_str(
            &doc.providers
                .iter()
                .find(|p| p.app == App::OpenCode)
                .unwrap()
                .settings_config,
        )
        .unwrap();
        assert!(opencode["options"].get("apiKey").is_none(), "opencode apiKey 必须剥");
        assert!(
            opencode["options"]["headers"].get("Authorization").is_none(),
            "认证头必须剥"
        );
        assert_eq!(
            opencode["options"]["headers"]["Helicone-Auth"], "meta",
            "元数据头保留"
        );
    }

    #[test]
    fn export_without_keys_rejects_unparseable_meta() {
        // 剥不了（无法证明无密钥）→ 拒绝导出，宁可不导。
        let ps = [provider_with_meta("a", "Broken", r#"{"env":{}}"#, "{oops")];
        assert!(export_document(&ps, false, "ts").is_err());
        // 含 key 导出不剥 meta，原样放行。
        assert!(export_document(&ps, true, "ts").is_ok());
    }

    #[test]
    fn parse_export_document_round_trips_providers() {
        let ps = [provider("a", "Alpha", &config_with_secrets())];
        let text = export_document(&ps, true, "ts").unwrap();
        let got = parse_export_document(&text).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "a");
        assert_eq!(got[0].settings_config, config_with_secrets());
    }

    #[test]
    fn parse_export_document_rejects_unknown_version_and_garbage() {
        assert!(matches!(
            parse_export_document(r#"{"version": 99, "exportedAt": "ts", "providers": []}"#),
            Err(AppError::Config(_))
        ));
        assert!(matches!(
            parse_export_document("{nope"),
            Err(AppError::Config(_))
        ));
        assert!(matches!(
            parse_export_document(r#"[1,2,3]"#),
            Err(AppError::Config(_))
        ));
    }

    /// 往返一致：导出（含 key）→ 清空 → 导入 → 列表与导出前一致（`updated_at`
    /// 除外——导入是重新写盘，刷新写时间）。
    #[test]
    fn export_import_round_trip_restores_provider_list() {
        let s = mem();
        let alpha = s
            .save_provider(provider("", "Alpha", &config_with_secrets()))
            .unwrap();
        let beta = s
            .save_provider(provider("", "Beta", r#"{"env":{"ANTHROPIC_MODEL":"m"}}"#))
            .unwrap();
        let before = s.list_providers().unwrap();

        let doc = export_document(&before, true, "2026-08-07T00:00:00Z").unwrap();
        // 清空：模拟换机后空库。
        s.delete_provider(App::Claude, &alpha.id).unwrap();
        s.delete_provider(App::Claude, &beta.id).unwrap();
        assert!(s.list_providers().unwrap().is_empty());

        let report = apply_import(&s, &doc, ProviderImportMode::Overwrite).unwrap();
        assert_eq!(report.imported, 2);
        assert_eq!(report.skipped, 0);

        let after = s.list_providers().unwrap();
        assert_eq!(after.len(), 2);
        for (a, b) in before.iter().zip(&after) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.name, b.name);
            assert_eq!(a.website_url, b.website_url);
            assert_eq!(a.category, b.category);
            assert_eq!(a.sort_index, b.sort_index);
            assert_eq!(a.notes, b.notes);
            assert_eq!(
                a.settings_config, b.settings_config,
                "含 key 往返逐字节一致"
            );
            assert_eq!(a.meta, b.meta);
        }
    }

    /// overwrite 导入已存在行时保留本地排序：不还原导出方的 `sort_index`
    /// （排序是本地偏好），本地独有行保留，导入的新行追加在末尾。
    #[test]
    fn apply_import_overwrite_keeps_local_order_and_appends_new_rows() {
        let s = mem();
        let alpha = s
            .save_provider(provider("", "Alpha", r#"{"env":{}}"#))
            .unwrap();
        let beta = s
            .save_provider(provider("", "Beta", r#"{"env":{}}"#))
            .unwrap();
        // 本地顺序：Beta 在前、Alpha 在后。
        s.reorder_providers(App::Claude, &[beta.id.clone(), alpha.id.clone()])
            .unwrap();
        let alpha_local = s.get_provider(App::Claude, &alpha.id).unwrap().unwrap();
        assert_eq!(alpha_local.sort_index, 1, "本地顺序已生效");

        // 「另一台设备」的导出：同 id 但携带不同的 sort_index，外加一个本地
        // 没有的新 id。
        let doc = export_document(
            &[
                provider(&alpha.id, "Alpha-imported", r#"{"env":{}}"#),
                provider(&beta.id, "Beta-imported", r#"{"env":{}}"#),
                provider("gammagamma", "Gamma", r#"{"env":{}}"#),
            ],
            true,
            "ts",
        )
        .unwrap();

        let report = apply_import(&s, &doc, ProviderImportMode::Overwrite).unwrap();
        assert_eq!(report.imported, 3);
        assert_eq!(report.skipped, 0);

        let after = s.list_providers().unwrap();
        let names: Vec<&str> = after.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["Beta-imported", "Alpha-imported", "Gamma"],
            "已存在行保留本地顺序，新行追加在末尾"
        );
        let alpha_after = after.iter().find(|p| p.id == alpha.id).unwrap();
        assert_eq!(
            alpha_after.sort_index, alpha_local.sort_index,
            "已存在行保留本地 sort_index，不还原导出方排序"
        );
        let gamma_after = after.iter().find(|p| p.id == "gammagamma").unwrap();
        assert_eq!(gamma_after.sort_index, 2, "新行追加在末尾");
    }

    /// merge 导入不碰已有行：冲突行跳过，`updated_at` 不被刷新。
    #[test]
    fn apply_import_merge_does_not_touch_existing_rows() {
        let s = mem();
        let alpha = s
            .save_provider(provider("", "Alpha", &config_with_secrets()))
            .unwrap();
        let doc = export_document(&s.list_providers().unwrap(), true, "ts").unwrap();
        let report = apply_import(&s, &doc, ProviderImportMode::Merge).unwrap();
        assert_eq!(report.imported, 0);
        assert_eq!(report.skipped, 1);
        let row = s.get_provider(App::Claude, &alpha.id).unwrap().unwrap();
        assert_eq!(row.updated_at, alpha.updated_at, "merge 冲突行不得重写");
        assert_eq!(row.settings_config, alpha.settings_config);
    }

    /// 导出文档每行带 app 字段（版本号不升——v1 兼容新旧格式）；旧文档行
    /// 无 app → 读为 claude。
    #[test]
    fn export_carries_app_per_line_and_old_doc_reads_as_claude() {
        let ps = [
            provider("a", "Claude-A", r#"{"env":{}}"#),
            Provider {
                app: App::Codex,
                ..provider("b", "Codex-B", r#"{"env":{}}"#)
            },
        ];
        let text = export_document(&ps, true, "ts").unwrap();
        // to_string_pretty 在冒号后留空格，故匹配 "\"app\": \"claude\""。
        assert!(text.contains(r#""app": "claude""#), "claude 行带 app");
        assert!(text.contains(r#""app": "codex""#), "codex 行带 app");
        let got = parse_export_document(&text).unwrap();
        assert_eq!(got.len(), 2);

        // 应用维度之前的旧文档：行没有 app 字段 → 全归 claude 池。
        let old_doc = r#"{"version":1,"exportedAt":"ts","providers":[{"id":"a","name":"Old","websiteUrl":"","category":"custom","icon":"","iconColor":"","sortIndex":0,"notes":"","settingsConfig":"{}","meta":"{}","updatedAt":"2026-08-01T00:00:00Z"}]}"#;
        let old = parse_export_document(old_doc).unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].app, App::Claude, "旧行归 claude 池");
    }
}
