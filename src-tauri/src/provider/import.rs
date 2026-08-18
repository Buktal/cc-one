//! 供应商导入的 store 层 seam：冲突规划（纯函数）+ 落库全流程。
//!
//! 三条导入路径共享同一骨架（归一化为 Provider → 冲突规划 → `save_provider`
//! 落库），曾各写各的冲突逻辑：导出文档导入（[`crate::provider::export_import`]）、
//! CC-Switch 导入（`commands::ccswitch` 曾把已就绪的 `Vec<Provider>` 序列化成
//! 导出文档文本再 re-parse 喂回 `apply_import`——纯文本 round-trip 绕道）、live
//! 导入（单激活应用按 name、opencode 按 liveKey 各写一套 upsert）。现在归一化
//! 在这里：**冲突键策略作参数**（[`ImportKeyStrategy`] 三态），三条路径共用
//! 同一份规划 + 落库代码。
//!
//! 策略（冲突键 = `(app, 键)`，app 隔离永远在键里——同 id / 同 name 在不同
//! 应用池是两个独立条目）：
//! - [`AppId`](ImportKeyStrategy::AppId)：键 = `(app, id)`（导出文档 /
//!   CC-Switch 导入）——merge = 冲突跳过（保留双方），overwrite = 冲突替换
//!   （本地独有保留，不做删除）。
//! - [`Name`](ImportKeyStrategy::Name)：键 = `(app, name)`（单激活应用 live
//!   导入）——冲突 = 原地更新 name / settings_config（保留 id / 展示字段 /
//!   meta），否则新建。
//! - [`LiveKey`](ImportKeyStrategy::LiveKey)：键 = `(app, meta.liveKey)`
//!   （opencode live 导入）——冲突 = 原地更新 name / settings_config / meta
//!   （meta 顶层按 incoming 胜出合并：liveKey / liveManaged 以本次为准，已有
//!   行的其它字段如 templateValues 保留），否则新建。
//!
//! **空键行（空 id / 空 name / 无 liveKey）不参与冲突判定**——一律视为新建
//! （空 id 由 `save_provider` 生成）。新建行追加在末尾、冲突行保留本地
//! `sort_index`（排序是本地偏好，导入不做重排——`save_provider` 语义）。
//!
//! 纯函数接缝：[`plan_import`]（existing + incoming + 策略 → 写入计划）；
//! store 层全流程 [`import_providers`]（读现有列表 → 规划 → 逐条写库），命令
//! 直接调它，测试也直接调它——测试跑的就是生产路径。只写 DB、不 emit
//! （`emit_providers_changed` 由命令层负责）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::Store;
use crate::error::AppResult;
use crate::model::Provider;
use crate::provider::live_opencode;

/// 导入冲突模式（导出文档 / CC-Switch 导入的 AppId 策略参数）：merge = 已有
/// `(app, id)` 跳过（保留双方，按 (app, id) 去重）；overwrite = 同 `(app, id)`
/// 以导入为准（后者胜），本地独有保留（不做删除——保守迁移）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ProviderImportMode {
    Merge,
    Overwrite,
}

/// 冲突键策略（三态）：定义「从 Provider 提取什么键」+「冲突时怎么处理」。
/// 三条导入路径各自声明自己的策略，冲突规划代码只有一份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKeyStrategy {
    /// 键 = `(app, id)`；merge 冲突跳过 / overwrite 冲突替换。导出文档与
    /// CC-Switch 导入用。
    AppId(ProviderImportMode),
    /// 键 = `(app, name)`；冲突 = 原地更新 name / settings_config（保留
    /// id / 展示字段 / meta）。单激活应用 live 导入用。
    Name,
    /// 键 = `(app, meta.liveKey)`；冲突 = 原地更新 name / settings_config /
    /// meta（meta 顶层按 incoming 胜出合并）。opencode live 导入用。
    LiveKey,
}

/// 一次导入的写入计划：`to_save` 是需要落库的行（existing 里没变的不重写，
/// 避免 merge 导入把全部行的 `updated_at` 都刷新一遍），计数说明哪些导入行被
/// 应用、哪些被跳过。
pub struct ImportPlan {
    pub to_save: Vec<Provider>,
    pub imported: u32,
    pub skipped: u32,
}

/// 导入结果计数，前端 toast 展示「导入 N 个、跳过 M 个」。用 `u32` 而非
/// `usize`：本类型跨 Rust→JS 边界走 tauri-specta 的 typed 导出，specta 拒绝
/// BigInt 型（`usize`/`u64`/`i64`…）字段以避免 JS 精度损失——用 `usize`
/// 会让 bindings.ts 生成失败。计数是行数（一次导入顶多几条），`u32` 足够。
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportReport {
    /// 实际写入的行数（AppId-merge = 新 (app, id)；AppId-overwrite = 全部导入
    /// 行；Name / LiveKey = 处理的行数——更新与新建都算写入）。
    pub imported: u32,
    /// AppId-merge 模式下因 (app, id) 冲突被跳过的行数（其余策略恒为 0）。
    pub skipped: u32,
}

/// 从 Provider 提取该策略下的冲突键（app 独立提取，键由调用方与 app 拼）。
/// 空键（空 id / 空 name / 无 liveKey）→ 空串——调用方据此判「无冲突，视为
/// 新建」。
fn conflict_key(strategy: ImportKeyStrategy, p: &Provider) -> String {
    match strategy {
        ImportKeyStrategy::AppId(_) => p.id.clone(),
        ImportKeyStrategy::Name => p.name.clone(),
        ImportKeyStrategy::LiveKey => live_opencode::meta_live_key(&p.meta).unwrap_or_default(),
    }
}

/// 冲突规划（纯函数）：按策略把 incoming 并入 existing，产出要落库的行。
/// 见 [`ImportKeyStrategy`] 各变体的语义；空键行一律视为新建。已存在行保留
/// 本地 `sort_index`（排序是本地偏好，导入不做重排），导入的新行追加在末尾
/// （`save_provider` 语义）。
pub fn plan_import(
    existing: &[Provider],
    incoming: &[Provider],
    strategy: ImportKeyStrategy,
) -> ImportPlan {
    // overwrite 不看 existing：全部导入行直接落（同 (app, id) 由 save_provider
    // upsert 替换、保留本地 sort_index；本地独有行不在 incoming 里 → 保留）。
    if let ImportKeyStrategy::AppId(ProviderImportMode::Overwrite) = strategy {
        let to_save = incoming.to_vec();
        return ImportPlan {
            imported: to_save.len() as u32,
            to_save,
            skipped: 0,
        };
    }
    // existing 按 (app, 键) 索引——只索引有非空键的行（空键行不参与冲突）。
    let mut existing_by_key: HashMap<(String, String), &Provider> = HashMap::new();
    for p in existing {
        let key = conflict_key(strategy, p);
        if !key.is_empty() {
            existing_by_key.insert((p.app.as_str().to_string(), key), p);
        }
    }
    let mut to_save = Vec::new();
    let mut imported = 0;
    let mut skipped = 0;
    for p in incoming {
        let key = conflict_key(strategy, p);
        let matched = if key.is_empty() {
            None
        } else {
            existing_by_key
                .get(&(p.app.as_str().to_string(), key))
                .copied()
        };
        match (strategy, matched) {
            // AppId-merge：冲突跳过（existing 原样保留，不改写）。
            (ImportKeyStrategy::AppId(ProviderImportMode::Merge), Some(_)) => {
                skipped += 1;
            }
            (ImportKeyStrategy::AppId(ProviderImportMode::Merge), None) => {
                to_save.push(p.clone());
                imported += 1;
            }
            // Name / LiveKey：冲突 = 原地更新（保留 id / 展示字段），否则新建。
            (ImportKeyStrategy::Name, Some(existing)) => {
                to_save.push(Provider {
                    name: p.name.clone(),
                    settings_config: p.settings_config.clone(),
                    ..existing.clone()
                });
                imported += 1;
            }
            (ImportKeyStrategy::LiveKey, Some(existing)) => {
                to_save.push(Provider {
                    name: p.name.clone(),
                    settings_config: p.settings_config.clone(),
                    meta: merge_meta(&existing.meta, &p.meta),
                    ..existing.clone()
                });
                imported += 1;
            }
            (ImportKeyStrategy::Name | ImportKeyStrategy::LiveKey, None) => {
                to_save.push(p.clone());
                imported += 1;
            }
            (ImportKeyStrategy::AppId(ProviderImportMode::Overwrite), _) => {
                unreachable!("overwrite 提前返回")
            }
        }
    }
    ImportPlan {
        to_save,
        imported,
        skipped,
    }
}

/// LiveKey 冲突时 meta 的处理：顶层 JSON 对象合并、incoming 胜出。incoming 由
/// 调用方 `live_opencode::with_meta_live_state` 构造——liveKey / liveManaged
/// 以本次为准；已有行的其它 meta 字段（如 templateValues）保留。existing meta
/// 非对象（无法合并的结构）→ 直接用 incoming。
fn merge_meta(existing_meta: &str, incoming_meta: &str) -> String {
    let mut merged: serde_json::Value =
        serde_json::from_str(existing_meta.trim()).unwrap_or_else(|_| serde_json::json!({}));
    let Some(existing_obj) = merged.as_object_mut() else {
        return incoming_meta.to_string();
    };
    if let Ok(incoming) = serde_json::from_str::<serde_json::Value>(incoming_meta.trim()) {
        if let Some(incoming_obj) = incoming.as_object() {
            for (k, v) in incoming_obj {
                existing_obj.insert(k.clone(), v.clone());
            }
        }
    }
    serde_json::to_string(&merged).unwrap_or_else(|_| incoming_meta.to_string())
}

/// 导入全流程（store 层，命令直接调这个）：读现有列表 → 按策略规划 → 逐条
/// `save_provider` 写回本机 DB。只写 DB——不碰任何同步文件，导入的 key 只进
/// 本机库；`emit_providers_changed` 由命令层负责。
pub fn import_providers(
    store: &Store,
    incoming: &[Provider],
    strategy: ImportKeyStrategy,
) -> AppResult<ProviderImportReport> {
    let existing = store.list_providers()?;
    let plan = plan_import(&existing, incoming, strategy);
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
    use crate::provider::import_ccswitch::{collect_ccswitch_imports, CcSwitchProvider};
    use serde_json::json;

    fn provider(id: &str, name: &str, settings_config: &str) -> Provider {
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
            meta: "{}".into(),
            updated_at: String::new(),
        }
    }

    // ---------------- AppId（导出文档 / CC-Switch 导入）----------------

    #[test]
    fn plan_app_id_merge_skips_existing_ids_and_appends_new() {
        let existing = [provider("a", "Alpha", "{}"), provider("b", "Beta", "{}")];
        let incoming = [
            provider("a", "Alpha-renamed", r#"{"env":{}}"#), // 冲突：跳过
            provider("c", "Gamma", r#"{"env":{}}"#),         // 新 id：追加
        ];
        let plan = plan_import(
            &existing,
            &incoming,
            ImportKeyStrategy::AppId(ProviderImportMode::Merge),
        );
        assert_eq!(plan.imported, 1);
        assert_eq!(plan.skipped, 1);
        assert_eq!(plan.to_save.len(), 1);
        assert_eq!(plan.to_save[0].id, "c");
        assert_eq!(plan.to_save[0].name, "Gamma");
    }

    #[test]
    fn plan_app_id_merge_inserts_empty_id_rows_as_new() {
        let existing = [provider("a", "Alpha", "{}")];
        let incoming = [provider("", "Hand-made", r#"{"env":{}}"#)];
        let plan = plan_import(
            &existing,
            &incoming,
            ImportKeyStrategy::AppId(ProviderImportMode::Merge),
        );
        assert_eq!(plan.imported, 1);
        assert_eq!(plan.skipped, 0);
        assert_eq!(plan.to_save[0].id, "", "空 id 走 save_provider 生成新 id");
    }

    #[test]
    fn plan_app_id_overwrite_replaces_same_id_appends_new_keeps_local_only() {
        let existing = [
            provider("a", "Alpha-old", "old"),
            provider("b", "Beta", "{}"),
        ];
        let incoming = [
            provider("a", "Alpha-new", r#"{"env":{}}"#), // 同 id：覆盖
            provider("c", "Gamma", r#"{"env":{}}"#),     // 新 id：追加
        ];
        let plan = plan_import(
            &existing,
            &incoming,
            ImportKeyStrategy::AppId(ProviderImportMode::Overwrite),
        );
        assert_eq!(plan.imported, 2);
        assert_eq!(plan.skipped, 0);
        assert_eq!(plan.to_save.len(), 2);
        assert_eq!(plan.to_save[0].name, "Alpha-new", "同 id 后者胜");
        assert_eq!(plan.to_save[0].settings_config, r#"{"env":{}}"#);
        assert_eq!(plan.to_save[1].id, "c");
        // 本地独有 id "b" 不在写入计划里 → 保留（不删除）。
        assert!(!plan.to_save.iter().any(|p| p.id == "b"));
    }

    /// 冲突键是 (app, id)：同一 id 出现在两个应用池 → merge 不互相跳过，
    /// overwrite 也不互相覆盖。
    #[test]
    fn plan_app_id_keeps_same_id_across_apps_separate() {
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
        let merge = plan_import(
            &existing,
            &incoming,
            ImportKeyStrategy::AppId(ProviderImportMode::Merge),
        );
        assert_eq!(merge.imported, 1, "codex 池的 p1 是新条目");
        assert_eq!(merge.skipped, 1, "claude 池的 p1 冲突跳过");
        assert_eq!(merge.to_save[0].app, App::Codex);
        assert_eq!(merge.to_save[0].name, "Codex-pool");

        let overwrite = plan_import(
            &existing,
            &incoming,
            ImportKeyStrategy::AppId(ProviderImportMode::Overwrite),
        );
        assert_eq!(overwrite.imported, 2, "两个池的行各自按 (app, id) 落");
    }

    // ---------------- Name（单激活应用 live 导入）----------------

    /// Name 冲突 = 原地更新：保留已有行的 id / 展示字段 / meta，只更新
    /// name / settings_config（updated_at 由 save_provider 按结构变化刷新）。
    #[test]
    fn plan_name_conflict_updates_name_and_settings_keeps_rest() {
        let existing = [provider("p1", "moonshot", "old")];
        let incoming = [Provider {
            id: String::new(),
            name: "moonshot".into(),
            settings_config: r#"{"env":{"ANTHROPIC_MODEL":"kimi"}}"#.into(),
            ..provider("", "moonshot", "{}")
        }];
        let plan = plan_import(&existing, &incoming, ImportKeyStrategy::Name);
        assert_eq!(plan.imported, 1);
        assert_eq!(plan.skipped, 0);
        assert_eq!(plan.to_save.len(), 1);
        let row = &plan.to_save[0];
        assert_eq!(row.id, "p1", "冲突 → 保留已有 id");
        assert_eq!(row.name, "moonshot");
        assert_eq!(row.settings_config, r#"{"env":{"ANTHROPIC_MODEL":"kimi"}}"#);
        assert_eq!(row.website_url, "https://example.com", "展示字段保留");
        assert_eq!(row.meta, "{}", "meta 保留（Name 策略不更新 meta）");
    }

    /// 同 name 不同 app 池 = 独立条目（键含 app）：互不冲突。
    #[test]
    fn plan_name_same_name_across_apps_is_no_conflict() {
        let existing = [provider("p1", "moonshot", "old")];
        let incoming = [Provider {
            app: App::Codex,
            ..provider("", "moonshot", r#"{"env":{}}"#)
        }];
        let plan = plan_import(&existing, &incoming, ImportKeyStrategy::Name);
        assert_eq!(plan.imported, 1);
        assert_eq!(plan.to_save[0].app, App::Codex, "同 name 不同池 → 新建");
        assert_eq!(plan.to_save[0].id, "", "新建行空 id");
    }

    // ---------------- LiveKey（opencode live 导入）----------------

    /// LiveKey 冲突 = 原地更新：保留已有 id / 展示字段，更新 name /
    /// settings_config / meta——meta 顶层按 incoming 胜出合并，已有行的
    /// 非 live 状态字段（templateValues）保留。
    #[test]
    fn plan_live_key_conflict_updates_and_keeps_extra_meta_fields() {
        let existing = [Provider {
            meta: json!({"liveKey": "deepseek", "templateValues": {"X": "y"}}).to_string(),
            ..provider("p1", "DeepSeek", "old")
        }];
        let incoming = [Provider {
            id: String::new(),
            name: "DS".into(),
            settings_config: r#"{"npm":"@ai-sdk/openai-compatible"}"#.into(),
            meta: json!({"liveKey": "deepseek", "liveManaged": true}).to_string(),
            ..provider("", "DS", "{}")
        }];
        let plan = plan_import(&existing, &incoming, ImportKeyStrategy::LiveKey);
        assert_eq!(plan.imported, 1);
        assert_eq!(plan.to_save.len(), 1);
        let row = &plan.to_save[0];
        assert_eq!(row.id, "p1", "冲突 → 保留已有 id");
        assert_eq!(row.name, "DS");
        assert_eq!(
            row.settings_config,
            r#"{"npm":"@ai-sdk/openai-compatible"}"#
        );
        let meta: serde_json::Value = serde_json::from_str(&row.meta).unwrap();
        assert_eq!(meta["liveKey"], "deepseek", "liveKey 以 incoming 为准");
        assert_eq!(meta["liveManaged"], true);
        assert_eq!(meta["templateValues"]["X"], "y", "已有 meta 其它字段保留");
    }

    /// 无 liveKey 的行不可匹配：existing 无 liveKey → 同 (app, name) 也不冲突；
    /// incoming 无 liveKey → 一律新建。
    #[test]
    fn plan_live_key_rows_without_key_never_conflict() {
        let existing = [provider("p1", "DeepSeek", "old")];
        let incoming = [Provider {
            id: String::new(),
            name: "DeepSeek".into(),
            meta: json!({"liveKey": "deepseek", "liveManaged": true}).to_string(),
            ..provider("", "DeepSeek", r#"{"npm":"@ai-sdk/openai-compatible"}"#)
        }];
        let plan = plan_import(&existing, &incoming, ImportKeyStrategy::LiveKey);
        assert_eq!(plan.imported, 1, "无 liveKey 的已有行不可匹配 → 新建");
        assert_eq!(
            plan.to_save[0].id, "",
            "新建（空 id 交 save_provider 生成）"
        );

        let no_key_incoming = [provider("", "New", r#"{"npm":"x"}"#)];
        let plan2 = plan_import(&existing, &no_key_incoming, ImportKeyStrategy::LiveKey);
        assert_eq!(plan2.imported, 1);
        assert_eq!(plan2.to_save[0].name, "New", "incoming 无 liveKey → 新建");
    }

    // ---------------- 三路对比 + store 层 + ccswitch ----------------

    /// 三路去重语义在同一测试面对比：同一份「已有行 + 重复导入」夹具，三个
    /// 策略各自的行为一览——AppId-merge 跳过、AppId-overwrite 替换、Name /
    /// LiveKey 原地更新（保留 id / 展示字段）。任一路语义漂移，对比当场可见。
    #[test]
    fn import_strategies_compared_on_same_fixture() {
        // AppId：键 = (app, id)。merge 跳过，overwrite 整行替换。
        let existing = [provider("p1", "Alpha", "old")];
        let incoming = [provider("p1", "Alpha-new", r#"{"env":{}}"#)];
        let merge = plan_import(
            &existing,
            &incoming,
            ImportKeyStrategy::AppId(ProviderImportMode::Merge),
        );
        assert_eq!(merge.imported, 0, "merge：冲突跳过");
        assert_eq!(merge.skipped, 1);
        assert!(merge.to_save.is_empty());
        let overwrite = plan_import(
            &existing,
            &incoming,
            ImportKeyStrategy::AppId(ProviderImportMode::Overwrite),
        );
        assert_eq!(overwrite.imported, 1, "overwrite：冲突替换");
        assert_eq!(overwrite.to_save[0].name, "Alpha-new");

        // Name：键 = (app, name)。冲突 = 原地更新（保留已有 id）。
        let existing = [provider("p1", "moonshot", "old")];
        let incoming = [Provider {
            id: String::new(),
            name: "moonshot".into(),
            settings_config: r#"{"env":{"ANTHROPIC_MODEL":"m"}}"#.into(),
            ..provider("", "moonshot", "{}")
        }];
        let name = plan_import(&existing, &incoming, ImportKeyStrategy::Name);
        assert_eq!(name.imported, 1, "name：冲突原地更新（不算跳过）");
        assert_eq!(name.to_save[0].id, "p1", "保留已有 id");
        assert_eq!(
            name.to_save[0].settings_config,
            r#"{"env":{"ANTHROPIC_MODEL":"m"}}"#
        );

        // LiveKey：键 = (app, meta.liveKey)。冲突 = 原地更新。
        let existing = [Provider {
            meta: json!({"liveKey": "k1"}).to_string(),
            ..provider("p1", "K1", "old")
        }];
        let incoming = [Provider {
            id: String::new(),
            name: "K1-new".into(),
            settings_config: r#"{"npm":"x"}"#.into(),
            meta: json!({"liveKey": "k1", "liveManaged": true}).to_string(),
            ..provider("", "K1-new", "{}")
        }];
        let live_key = plan_import(&existing, &incoming, ImportKeyStrategy::LiveKey);
        assert_eq!(live_key.imported, 1, "liveKey：冲突原地更新（不算跳过）");
        assert_eq!(live_key.to_save[0].id, "p1", "保留已有 id");
        assert_eq!(live_key.to_save[0].name, "K1-new");
    }

    /// store 层 merge：冲突行跳过，`updated_at` 不被刷新。
    #[test]
    fn import_providers_app_id_merge_does_not_touch_existing_rows() {
        let s = mem();
        let alpha = s
            .save_provider(provider("", "Alpha", r#"{"env":{}}"#))
            .unwrap();
        let report = import_providers(
            &s,
            &[provider(&alpha.id, "Alpha-renamed", "new")],
            ImportKeyStrategy::AppId(ProviderImportMode::Merge),
        )
        .unwrap();
        assert_eq!(report.imported, 0);
        assert_eq!(report.skipped, 1);
        let row = s.get_provider(App::Claude, &alpha.id).unwrap().unwrap();
        assert_eq!(row.updated_at, alpha.updated_at, "merge 冲突行不得重写");
        assert_eq!(row.settings_config, r#"{"env":{}}"#);
    }

    /// store 层 overwrite：已存在行保留本地排序，新行追加在末尾。
    #[test]
    fn import_providers_app_id_overwrite_keeps_local_order_and_appends_new_rows() {
        let s = mem();
        let alpha = s
            .save_provider(provider("", "Alpha", r#"{"env":{}}"#))
            .unwrap();
        let beta = s
            .save_provider(provider("", "Beta", r#"{"env":{}}"#))
            .unwrap();
        s.reorder_providers(App::Claude, &[beta.id.clone(), alpha.id.clone()])
            .unwrap();
        let alpha_local = s.get_provider(App::Claude, &alpha.id).unwrap().unwrap();
        let incoming = [
            provider(
                &alpha.id,
                "Alpha-imported",
                r#"{"env":{"ANTHROPIC_MODEL":"m"}}"#,
            ),
            provider(&beta.id, "Beta-imported", r#"{"env":{}}"#),
            provider("gammagamma", "Gamma", r#"{"env":{}}"#),
        ];
        let report = import_providers(
            &s,
            &incoming,
            ImportKeyStrategy::AppId(ProviderImportMode::Overwrite),
        )
        .unwrap();
        assert_eq!(report.imported, 3);
        assert_eq!(report.skipped, 0);
        let after = s.list_providers().unwrap();
        let names: Vec<&str> = after.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Beta-imported", "Alpha-imported", "Gamma"]);
        let alpha_after = after.iter().find(|p| p.id == alpha.id).unwrap();
        assert_eq!(
            alpha_after.sort_index, alpha_local.sort_index,
            "已存在行保留本地 sort_index"
        );
        assert_eq!(
            after
                .iter()
                .find(|p| p.id == "gammagamma")
                .unwrap()
                .sort_index,
            2,
            "新行追加在末尾"
        );
    }

    /// ccswitch 冲突行为直接测 seam（不经导出文档序列化绕道）：转换出的
    /// Provider 直接喂 AppId 策略——merge 跳过同 (app, id)、overwrite 替换同
    /// (app, id)（保留本地排序），代理跳过明细不受影响。
    #[test]
    fn ccswitch_import_conflict_behavior_via_seam() {
        let s = mem();
        let first = s
            .save_provider(provider("abc", "My Kimi", r#"{"env":{}}"#))
            .unwrap();
        let cc = CcSwitchProvider {
            id: "abc".into(),
            name: "My Kimi".into(),
            app_type: "claude".into(),
            settings_config: json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.moonshot.cn/anthropic",
                    "ANTHROPIC_AUTH_TOKEN": "sk-xxx",
                    "ANTHROPIC_MODEL": "kimi-k2.7-code"
                }
            }),
            website_url: Some("https://platform.kimi.com".into()),
            category: Some("cn_official".into()),
            icon: Some("kimi".into()),
            icon_color: Some("#6366F1".into()),
            sort_index: Some(3),
            notes: None,
            meta: json!({}),
        };
        let (imported, skipped) =
            collect_ccswitch_imports(&[cc], &[], App::Claude, "2026-08-12T00:00:00Z");
        assert!(skipped.is_empty(), "无代理跳过");
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].id, "abc", "转换保留 CC-Switch 原 id");

        // merge：同 (app, id) 冲突 → 跳过，已有行不碰。
        let report = import_providers(
            &s,
            &imported,
            ImportKeyStrategy::AppId(ProviderImportMode::Merge),
        )
        .unwrap();
        assert_eq!(report.imported, 0);
        assert_eq!(report.skipped, 1);
        let row = s.get_provider(App::Claude, &first.id).unwrap().unwrap();
        assert_eq!(row.settings_config, r#"{"env":{}}"#);

        // overwrite：同 (app, id) → 替换（保留本地 sort_index）。
        let report = import_providers(
            &s,
            &imported,
            ImportKeyStrategy::AppId(ProviderImportMode::Overwrite),
        )
        .unwrap();
        assert_eq!(report.imported, 1);
        let row = s.get_provider(App::Claude, &first.id).unwrap().unwrap();
        let sc: serde_json::Value = serde_json::from_str(&row.settings_config).unwrap();
        assert_eq!(
            sc["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-xxx",
            "覆盖后 key 落库"
        );
        assert_eq!(row.sort_index, first.sort_index, "覆盖保留本地排序");
    }
}
