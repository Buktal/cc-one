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
//! provider 列表，版本校验）、`plan_import`（merge / overwrite 冲突规划）。
//! `apply_import` 是 store 层全流程，命令直接调它，测试也直接调它——测试跑
//! 的就是生产路径。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::model::Provider;

/// 当前导出文档版本。导入只认这个版本——未来格式演进时，旧版 app 读到新文档
/// 会明确报错而不是静默错解。版本号**不因加 app 字段而升**：老文档（行无 app）
/// 与新文档（行带 app）都是 v1——serde 对未知字段忽略，新读旧 = 全归 claude
/// 池，旧读新 = app 字段被忽略；版本只在格式真的不兼容时才升。
pub const EXPORT_VERSION: u32 = 1;

/// 导入冲突模式：merge = 已有 `(app, id)` 跳过（保留双方，按 (app, id) 去重）；
/// overwrite = 同 `(app, id)` 以导入为准（后者胜），本地独有保留（不做删除——
/// 保守迁移）。两种模式都不还原导出方的排序：已存在行保留本地 `sort_index`
/// （排序是本地偏好，导入不做重排），导入的新行追加在末尾（`save_provider`
/// 语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ProviderImportMode {
    Merge,
    Overwrite,
}

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

/// 导入结果计数，前端 toast 展示「导入 N 个、跳过 M 个」。用 `u32` 而非
/// `usize`：本类型跨 Rust→JS 边界走 tauri-specta 的 typed 导出，specta 拒绝
/// BigInt 型（`usize`/`u64`/`i64`…）字段以避免 JS 精度损失——用 `usize`
/// 会让 bindings.ts 生成失败。计数是行数（一次导入顶多几条），`u32` 足够。
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportReport {
    /// 实际写入的行数（merge = 新 (app, id)；overwrite = 全部导入行）。
    pub imported: u32,
    /// merge 模式下因 (app, id) 冲突被跳过的行数（overwrite 恒为 0）。
    pub skipped: u32,
}

/// 一次导入的写入计划：`to_save` 是需要落库的行（existing 里没变的不重写，
/// 避免 merge 导入把全部行的 `updated_at` 都刷新一遍），计数说明哪些导入行被
/// 应用、哪些被跳过。
pub struct ImportPlan {
    pub to_save: Vec<Provider>,
    pub imported: u32,
    pub skipped: u32,
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

/// 冲突规划（纯函数）：按模式把 incoming 并入 existing，产出要落库的行。
/// 冲突键是 `(app, id)`——同一 id 在不同应用池是两个独立条目。
/// - merge：incoming 里 `(app, id)` 已存在 → 跳过（existing 原样保留，不
///   改写）；其余（新键、空 id）→ 追加。空 id 行视为新建——没有冲突，由
///   `save_provider` 生成新 id。
/// - overwrite：同 `(app, id)` → 用 incoming 整行替换；新键 / 空 id → 追加；
///   本地独有 → 保留（「覆盖 = 后者胜」按 upsert 实现，不做删除）。
pub fn plan_import(
    existing: &[Provider],
    incoming: &[Provider],
    mode: ProviderImportMode,
) -> ImportPlan {
    match mode {
        ProviderImportMode::Merge => {
            let existing_keys: HashSet<(String, String)> = existing
                .iter()
                .map(|p| (p.app.as_str().to_string(), p.id.clone()))
                .collect();
            let mut to_save = Vec::new();
            let mut imported = 0;
            let mut skipped = 0;
            for p in incoming {
                let key = (p.app.as_str().to_string(), p.id.clone());
                if !p.id.is_empty() && existing_keys.contains(&key) {
                    skipped += 1;
                    continue;
                }
                to_save.push(p.clone());
                imported += 1;
            }
            ImportPlan {
                to_save,
                imported,
                skipped,
            }
        }
        ProviderImportMode::Overwrite => {
            let to_save = incoming.to_vec();
            ImportPlan {
                imported: to_save.len() as u32,
                to_save,
                skipped: 0,
            }
        }
    }
}

/// 导入全流程（store 层，命令直接调这个）：解析文档 → 读现有列表 → 按模式
/// 规划 → 逐条 `save_provider` 写回本机 DB。只写 DB——不碰任何同步文件，
/// 导入的 key 只进本机库。
pub fn apply_import(
    store: &Store,
    json: &str,
    mode: ProviderImportMode,
) -> AppResult<ProviderImportReport> {
    let incoming = parse_export_document(json)?;
    let existing = store.list_providers()?;
    let plan = plan_import(&existing, &incoming, mode);
    for p in &plan.to_save {
        store.save_provider(p.clone())?;
    }
    Ok(ProviderImportReport {
        imported: plan.imported,
        skipped: plan.skipped,
    })
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

    #[test]
    fn plan_import_merge_skips_existing_ids_and_appends_new() {
        let existing = [provider("a", "Alpha", "{}"), provider("b", "Beta", "{}")];
        let incoming = [
            provider("a", "Alpha-renamed", r#"{"env":{}}"#), // 冲突：跳过
            provider("c", "Gamma", r#"{"env":{}}"#),         // 新 id：追加
        ];
        let plan = plan_import(&existing, &incoming, ProviderImportMode::Merge);
        assert_eq!(plan.imported, 1);
        assert_eq!(plan.skipped, 1);
        assert_eq!(plan.to_save.len(), 1);
        assert_eq!(plan.to_save[0].id, "c");
        assert_eq!(plan.to_save[0].name, "Gamma");
    }

    #[test]
    fn plan_import_merge_inserts_empty_id_rows_as_new() {
        let existing = [provider("a", "Alpha", "{}")];
        let incoming = [provider("", "Hand-made", r#"{"env":{}}"#)];
        let plan = plan_import(&existing, &incoming, ProviderImportMode::Merge);
        assert_eq!(plan.imported, 1);
        assert_eq!(plan.skipped, 0);
        assert_eq!(plan.to_save[0].id, "", "空 id 走 save_provider 生成新 id");
    }

    #[test]
    fn plan_import_overwrite_replaces_same_id_appends_new_keeps_local_only() {
        let existing = [
            provider("a", "Alpha-old", "old"),
            provider("b", "Beta", "{}"),
        ];
        let incoming = [
            provider("a", "Alpha-new", r#"{"env":{}}"#), // 同 id：覆盖
            provider("c", "Gamma", r#"{"env":{}}"#),     // 新 id：追加
        ];
        let plan = plan_import(&existing, &incoming, ProviderImportMode::Overwrite);
        assert_eq!(plan.imported, 2);
        assert_eq!(plan.skipped, 0);
        assert_eq!(plan.to_save.len(), 2);
        assert_eq!(plan.to_save[0].name, "Alpha-new", "同 id 后者胜");
        assert_eq!(plan.to_save[0].settings_config, r#"{"env":{}}"#);
        assert_eq!(plan.to_save[1].id, "c");
        // 本地独有 id "b" 不在写入计划里 → 保留（不删除）。
        assert!(!plan.to_save.iter().any(|p| p.id == "b"));
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

    /// 冲突键是 (app, id)：同一 id 出现在两个应用池 → merge 不互相跳过，
    /// overwrite 也不互相覆盖。
    #[test]
    fn plan_import_keeps_same_id_across_apps_separate() {
        fn provider_for(app: App, id: &str, name: &str) -> Provider {
            Provider {
                app,
                ..provider(id, name, r#"{"env":{}}"#)
            }
        }
        let existing = [provider_for(App::Claude, "p1", "Claude-pool")];
        let incoming = [
            // 同 (app, id)：merge 跳过。
            provider_for(App::Claude, "p1", "Claude-renamed"),
            // 同 id、不同池：是独立条目，merge 追加。
            provider_for(App::Codex, "p1", "Codex-pool"),
        ];
        let merge = plan_import(&existing, &incoming, ProviderImportMode::Merge);
        assert_eq!(merge.imported, 1, "codex 池的 p1 是新条目");
        assert_eq!(merge.skipped, 1, "claude 池的 p1 冲突跳过");
        assert_eq!(merge.to_save[0].app, App::Codex);
        assert_eq!(merge.to_save[0].name, "Codex-pool");

        let overwrite = plan_import(&existing, &incoming, ProviderImportMode::Overwrite);
        assert_eq!(overwrite.imported, 2, "两个池的行各自按 (app, id) 落");
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
