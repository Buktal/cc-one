//! 供应商激活编排（架构审查候选③）：「切换供应商」这条全后端最深的业务流
//! 此前住在 `switch_provider_cmd` 的命令闭包里——不可测（「写盘成功才落激
//! 活态」的顺序不变量只存在于行序），且附加模式分支反向借用 commands 兄弟
//! 模块的 `ensure_opencode_in_live`，providers ↔ live_import 双向互依。下沉
//! 到 provider 域后：命令层只剩 spawn_blocking 薄壳 + emit；依赖注入与
//! [`App::write_live`] 的 paths 参数同一形状，编排可用内存库 +
//! `ConfigStore::for_test` + 临时 live 目录直测（见本模块测试组）。
//!
//! 附加模式「加入 live」与「移出 live」是同一编排的两半，都收在本模块
//! （[`ensure_opencode_in_live`] / [`remove_from_live`]）——撤除半边曾散在
//! commands 层两处（停用路径带 meta 半边、删除路径只撤文件），现在对称归位。
//!
//! 片段合并层的 per-app 分派（ADR-0010，含 settings_config 层的合并域）收口
//! 在 [`crate::provider::live_adapter`]；本模块是它们的组合次序权威。

use std::path::{Path, PathBuf};

use crate::config::ConfigStore;
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::model::{App, Provider};
use crate::provider::live_adapter::SnippetLayer;
use crate::provider::{live, live_opencode};

/// 激活动作涉及的 live 文件路径集。生产由 [`resolve_paths`] 从真实 home 解析；
/// 测试注入临时目录。形状与 [`App::write_live`] 的 paths 参数一致（主配置在
/// `[0]`、副文件在 `[1]`）。
pub(crate) struct ActivatePaths {
    /// 单激活应用的 live 文件路径集（[`App::live_paths`]：claude/grok 一份、
    /// codex/gemini 两份）。附加模式的写盘不经 [`App::write_live`]，不用此字段
    /// （live_paths 对附加模式也返回 opencode.json，供反向导入读，不进写盘）。
    pub(crate) single: Vec<PathBuf>,
    /// opencode.json 路径（附加模式的写入目标）；单激活忽略。
    pub(crate) opencode_config: PathBuf,
}

/// 解析真实的 live 路径集（生产入口一次解析、编排全程持有）。
pub(crate) fn resolve_paths(app: App) -> AppResult<ActivatePaths> {
    Ok(ActivatePaths {
        single: app.live_paths()?,
        opencode_config: live_opencode::opencode_config_path()?,
    })
}

/// 激活编排（supplier 侧唯一业务入口，命令层薄壳直接调用）：单激活 =
/// 「切换」——按应用的 ADR-0010 策略分派片段合并层后受控写盘，**写盘成功才**
/// `set_active_provider`（重启后激活态指向没写成的配置 = 废状态，故反序绝
/// 不允许）；附加模式（OpenCode）= ensure-in-live——写进 opencode.json + 设
/// `meta.liveManaged = true`，**不取消其它 provider、不碰 active_providers**
/// （附加模式无唯一激活）。两条路都返回入库后的 provider。
pub(crate) fn activate(
    store: &Store,
    config: &ConfigStore,
    app: App,
    id: &str,
    paths: &ActivatePaths,
) -> AppResult<Provider> {
    let provider = store
        .get_provider(app, id)?
        .ok_or_else(|| AppError::Config(format!("provider not found in {app:?} pool: {id}")))?;
    if app.is_additive_mode() {
        return ensure_opencode_in_live(store, provider, &paths.opencode_config);
    }
    // 单激活：片段按 provider 归属的应用读取（claude 池读 claude 片段）。读
    // guard 随语句结束释放。
    let snippet_record = config.get().snippet_for(app);
    let write_provider = match app.snippet_layer() {
        // settings_config 层（claude/gemini）：片段按该 app 的合并域（受控区
        // 形状，随层声明）并入供应商配置，再随受控写盘落地。claude 的
        // settings.json 是字面量 JSON：${VAR} 占位符会原样写进 live = 废配置，
        // 切换前拦下（gemini 的 .env 由 dotenv 展开 ${VAR} 是合法引用，不拦
        // ——见 App::validates_template_vars）。
        SnippetLayer::SettingsConfig(domain) => {
            let settings_config = crate::provider::snippet::apply_snippet(
                &provider.settings_config,
                &snippet_record.content,
                snippet_record.enabled,
                domain,
            )?;
            if app.validates_template_vars() {
                crate::provider::live::validate_no_unfilled_template_vars(&settings_config)?;
            }
            Provider {
                settings_config,
                ..provider.clone()
            }
        }
        // 写盘层（codex/grok）与无片段（opencode，先于此处返回）：供应商配置
        // 原样进写盘，片段随 write_snippet 走写盘层补缺失。
        SnippetLayer::WriteLayer | SnippetLayer::NoSnippet => provider.clone(),
    };
    // 写盘层片段（codex/grok）：启用 → 片段内容，否则空串（switch_*_live 空
    // 串即无操作）。settings_config 层应用一律空串（其片段已在上面并入供应商
    // 配置）。
    let write_snippet = match app.snippet_layer() {
        SnippetLayer::WriteLayer if snippet_record.enabled => snippet_record.content.clone(),
        _ => String::new(),
    };
    app.write_live(&paths.single, &write_provider, &write_snippet)?;
    config.update(|c| c.set_active_provider(app, id))?;
    Ok(provider)
}

/// 附加模式核心动作（OpenCode）:把 provider ensure-in-live——写进 opencode.json
/// 同时设 `meta.liveManaged = true` 并落库。key 由 `live_opencode::derive_live_key`
/// 派生（优先沿用 meta.liveKey，改名不重算；首次按 name slugify，空 → 回落
/// id）。路径由调用方给定（生产 [`resolve_paths`]，测试注入临时目录）。
pub(crate) fn ensure_opencode_in_live(
    store: &Store,
    provider: Provider,
    path: &Path,
) -> AppResult<Provider> {
    let live_text = live::read_live_settings(path)?;
    let key =
        live_opencode::derive_live_key(&provider.name, &provider.id, &provider.meta, &live_text);
    live_opencode::set_opencode_provider(path, &key, &provider.settings_config)?;
    let updated = Provider {
        meta: live_opencode::with_meta_live_state(&provider.meta, &key, true)?,
        ..provider
    };
    store.save_provider(updated)
}

/// 附加模式移除半边（[`ensure_opencode_in_live`] 的对称物，停用与删除路径
/// 共用同一入口）：已托管（`meta.liveManaged = true`）→ 从 live 配置移除该键；
/// 随后 `meta.liveManaged = false` 落库。**liveKey 保留**——key 稳定才不弄断
/// 用户顶层 `model: "<key>/<model>"` 引用，再加回来时沿用原 key。无 liveKey
/// （从未写盘）→ 显式无操作、原样返回；重复移除幂等（未托管不碰文件，meta
/// 值不变 → `save_provider` 判无结构变化、不刷新 `updated_at`）。路径由调用
/// 方给定（生产 [`resolve_paths`]，测试注入临时目录）。
pub(crate) fn remove_from_live(
    store: &Store,
    provider: Provider,
    path: &Path,
) -> AppResult<Provider> {
    let Some(key) = live_opencode::meta_live_key(&provider.meta) else {
        return Ok(provider);
    };
    if live_opencode::meta_live_managed(&provider.meta) == Some(true) {
        live_opencode::remove_opencode_provider(path, &key)?;
    }
    let updated = Provider {
        meta: live_opencode::with_meta_live_state(&provider.meta, &key, false)?,
        ..provider
    };
    store.save_provider(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigData, Paths};
    use crate::db::testutil::mem;

    /// 内存库 + 临时目录上的 config 店（真实 update/get 代码路径写
    /// config.json）+ 指向临时目录的 live 路径集。
    struct Fixture {
        store: crate::db::Store,
        config: ConfigStore,
        paths: ActivatePaths,
        _tmp: tempfile::TempDir,
    }

    fn fixture(single: Vec<PathBuf>, opencode_json: PathBuf) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let store = mem();
        let config = ConfigStore::for_test(
            Paths::resolve(tmp.path()),
            ConfigData {
                device_id: "0123456789ab".into(),
                ..Default::default()
            },
        );
        Fixture {
            store,
            config,
            paths: ActivatePaths {
                single,
                opencode_config: opencode_json,
            },
            _tmp: tmp,
        }
    }

    fn save(store: &Store, app: App, id: &str, name: &str, settings_config: &str) -> Provider {
        store
            .save_provider(crate::provider::testutil::provider(
                app,
                id,
                name,
                settings_config,
            ))
            .unwrap()
    }

    #[test]
    fn switch_single_activate_writes_controlled_keys_and_sets_active() {
        let settings = tempfile::tempdir().unwrap();
        let settings_path = settings.path().join("settings.json");
        std::fs::write(&settings_path, r#"{"hooks":{"x":1},"model":"keep-me"}"#).unwrap();
        let f = fixture(
            vec![settings_path.clone()],
            PathBuf::from("/unused/opencode.json"),
        );
        save(
            &f.store,
            App::Claude,
            "p1",
            "A",
            r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-1","ANTHROPIC_BASE_URL":"https://a"}}"#,
        );

        activation_ok(&f, App::Claude, "p1");

        // 顺序不变量的正向半边：写盘完成 → 激活态落位。
        assert_eq!(
            f.config.get().active_provider_id_for(App::Claude),
            Some("p1".into())
        );
        let merged = std::fs::read_to_string(&settings_path).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // 受控 env 整块替换 + 非受控键原地保留。
        assert_eq!(doc["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-1");
        assert_eq!(doc["hooks"]["x"], 1, "非受控 hooks 原地保留");
        assert_eq!(doc["model"], "keep-me", "非受控 model 原地保留");
    }

    #[test]
    fn unfilled_template_var_fails_before_write_and_never_sets_active() {
        let settings = tempfile::tempdir().unwrap();
        let settings_path = settings.path().join("settings.json");
        std::fs::write(&settings_path, r#"{"keep":"original"}"#).unwrap();
        let f = fixture(
            vec![settings_path.clone()],
            PathBuf::from("/unused/opencode.json"),
        );
        save(
            &f.store,
            App::Claude,
            "p2",
            "B",
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://${REGION}/api"}}"#,
        );

        let err = activation(&f, App::Claude, "p2").expect_err("未填模板变量必须失败");

        // 顺序不变量的负向半边：任何 Err ⇒ 激活态不动、live 文件不被污染。
        assert!(
            err.to_string().contains("template variable"),
            "应因模板变量拦截而失败: {err}"
        );
        assert_eq!(f.config.get().active_provider_id_for(App::Claude), None);
        assert_eq!(
            std::fs::read_to_string(&settings_path).unwrap(),
            r#"{"keep":"original"}"#,
            "失败路径不得改写 live"
        );
    }

    #[test]
    fn missing_provider_fails_without_touching_activation_state() {
        let f = fixture(
            vec![PathBuf::from("/unused/settings.json")],
            PathBuf::from("/unused/opencode.json"),
        );
        assert!(activation(&f, App::Claude, "ghost").is_err());
        assert_eq!(f.config.get().active_provider_id_for(App::Claude), None);
    }

    #[test]
    fn write_layer_app_merges_enabled_snippet_into_live_and_sets_active() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("config.toml");
        let auth_path = dir.path().join("auth.json");
        let f = fixture(
            vec![toml_path.clone(), auth_path],
            PathBuf::from("/unused/opencode.json"),
        );
        save(
            &f.store,
            App::Codex,
            "c1",
            "C",
            r#"{"auth":{"OPENAI_API_KEY":"sk-c"},"config":"model = \"m\""}"#,
        );
        // 启用的 codex 写盘层片段（ADR-0010：写盘层补缺失进 live 文件）。
        f.config
            .update(|c| {
                c.set_snippet(
                    App::Codex,
                    crate::model::CommonConfigSnippet {
                        enabled: true,
                        content: "[tui]\ntheme = \"dark\"\n".into(),
                    },
                )
            })
            .unwrap();

        activation_ok(&f, App::Codex, "c1");

        assert_eq!(
            f.config.get().active_provider_id_for(App::Codex),
            Some("c1".into())
        );
        let toml = std::fs::read_to_string(&toml_path).unwrap();
        assert!(toml.contains("[tui]"), "写盘层片段应补进 live TOML: {toml}");
        // 认证副文件：provider.auth 里的 key 原样落地（受控身份键）。
        let auth = std::fs::read_to_string(dir.path().join("auth.json")).unwrap();
        assert!(auth.contains("sk-c"), "认证键应写入 auth.json: {auth}");
    }

    #[test]
    fn additive_mode_ensures_in_live_and_never_touches_active_providers() {
        let dir = tempfile::tempdir().unwrap();
        let opencode_json = dir.path().join("opencode.json");
        std::fs::write(&opencode_json, r#"{"theme":"dark"}"#).unwrap();
        let f = fixture(vec![], opencode_json.clone());
        save(
            &f.store,
            App::OpenCode,
            "o1",
            "O",
            r#"{"options":{"apiKey":"sk-o","baseURL":"https://o.dev"}}"#,
        );

        let updated = activation(&f, App::OpenCode, "o1").expect("附加模式 ensure-in-live 应成功");

        // 附加模式两条不变量：live 已写入 managed 条目；active_providers 不动。
        assert_eq!(
            f.config.get().active_provider_id_for(App::OpenCode),
            None,
            "附加模式无唯一激活"
        );
        let live_doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&opencode_json).unwrap()).unwrap();
        let entry = &live_doc["provider"]["o"];
        assert_eq!(entry["options"]["apiKey"], "sk-o");
        assert!(
            updated.meta.contains("\"liveManaged\""),
            "meta 记录 managed: {}",
            updated.meta
        );
        assert!(f.store.list_providers_for(App::OpenCode).unwrap()[0]
            .meta
            .contains("\"liveKey\""));
        let _ = dir; // 保活
    }

    #[test]
    fn remove_from_live_withdraws_entry_clears_managed_keeps_live_key() {
        let dir = tempfile::tempdir().unwrap();
        let opencode_json = dir.path().join("opencode.json");
        std::fs::write(&opencode_json, r#"{"model":"x/y","provider":{"o":{"npm":"@ai-sdk/openai-compatible","options":{"apiKey":"sk-o"}}},"theme":"dark"}"#).unwrap();
        let f = fixture(vec![], opencode_json.clone());
        // 已托管的 provider：meta.liveKey = o、liveManaged = true。
        let managed = crate::provider::testutil::provider_with_meta(
            App::OpenCode,
            "o1",
            "O",
            r#"{"options":{"apiKey":"sk-o"}}"#,
            r#"{"liveKey":"o","liveManaged":true}"#,
        );

        let updated = remove_from_live(&f.store, managed, &opencode_json).unwrap();

        // live 撤除：该键消失，其它 provider 条目与用户顶层字段语义保留。
        let live_doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&opencode_json).unwrap()).unwrap();
        assert!(
            live_doc["provider"].get("o").is_none(),
            "provider.<key> 移除"
        );
        assert_eq!(live_doc["model"], "x/y", "用户顶层字段保留");
        // meta 半边：liveManaged=false、liveKey 保留（再加回来时 key 稳定）。
        assert_eq!(
            live_opencode::meta_live_key(&updated.meta).as_deref(),
            Some("o"),
            "liveKey 保留"
        );
        assert_eq!(live_opencode::meta_live_managed(&updated.meta), Some(false));
        assert_eq!(
            f.store
                .get_provider(App::OpenCode, "o1")
                .unwrap()
                .unwrap()
                .meta,
            updated.meta,
            "meta 半边落库"
        );
    }

    #[test]
    fn remove_from_live_unmanaged_is_idempotent_and_never_touches_file() {
        let dir = tempfile::tempdir().unwrap();
        let opencode_json = dir.path().join("opencode.json");
        std::fs::write(&opencode_json, r#"{"provider":{"o":{"npm":"x"}}}"#).unwrap();
        let f = fixture(vec![], opencode_json.clone());
        // 未托管（liveManaged=false 但有 liveKey）：第二次移除的形态。
        let unmanaged = crate::provider::testutil::provider_with_meta(
            App::OpenCode,
            "o1",
            "O",
            r#"{"options":{}}"#,
            r#"{"liveKey":"o","liveManaged":false}"#,
        );
        let before = std::fs::read_to_string(&opencode_json).unwrap();

        let updated = remove_from_live(&f.store, unmanaged, &opencode_json).unwrap();

        // 幂等：不碰文件（live 里本就没它）、meta 值不变。
        assert_eq!(std::fs::read_to_string(&opencode_json).unwrap(), before);
        assert_eq!(live_opencode::meta_live_managed(&updated.meta), Some(false));
        assert!(
            !dir.path().join("opencode.json.bak").exists(),
            "无操作不备份"
        );
    }

    #[test]
    fn remove_from_live_without_live_key_is_an_explicit_noop() {
        let dir = tempfile::tempdir().unwrap();
        let opencode_json = dir.path().join("opencode.json");
        std::fs::write(&opencode_json, r#"{"theme":"dark"}"#).unwrap();
        let f = fixture(vec![], opencode_json.clone());
        // 无 liveKey（从未写盘）：显式无操作，原样返回。
        let fresh =
            crate::provider::testutil::provider(App::OpenCode, "o1", "O", r#"{"options":{}}"#);

        let updated = remove_from_live(&f.store, fresh, &opencode_json).unwrap();

        assert_eq!(updated.meta, "{}", "无 liveKey → meta 不动");
        assert_eq!(
            std::fs::read_to_string(&opencode_json).unwrap(),
            r#"{"theme":"dark"}"#,
            "live 文件不碰"
        );
        assert!(
            f.store
                .list_providers_for(App::OpenCode)
                .unwrap()
                .is_empty(),
            "无 liveKey → 不落库（无状态可改）"
        );
    }

    // —— 小工具:统一走 activate 入口,错误信息带上下文 —— //
    fn activation(f: &Fixture, app: App, id: &str) -> AppResult<Provider> {
        activate(&f.store, &f.config, app, id, &f.paths)
    }
    fn activation_ok(f: &Fixture, app: App, id: &str) -> Provider {
        activation(f, app, id).expect("activate should succeed")
    }
}
