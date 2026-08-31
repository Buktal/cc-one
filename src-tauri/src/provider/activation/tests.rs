//! 激活编排测试：单激活「写盘成功才落激活态」顺序不变量的正负两半、附加模式
//! ensure-in-live / remove-from-live 两半，以及「删除供应商」的「live 撤除成功
//! 才删行」次序不变量（撤除失败行保留、未托管 / 重复删除幂等、单激活不碰
//! live）。

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
    let fresh = crate::provider::testutil::provider(App::OpenCode, "o1", "O", r#"{"options":{}}"#);

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

// ---- 删除供应商：「live 撤除成功才删行」次序不变量 -------------------------

/// 已托管形态的共享样本：opencode.json 带该 provider 的 live 条目（外加别人的
/// 条目与用户顶层字段），DB 行 meta.liveKey = o、liveManaged = true。
fn managed_live_and_row(root: &Path) -> (Fixture, PathBuf) {
    let opencode_json = root.join("opencode.json");
    std::fs::write(
        &opencode_json,
        r#"{"model":"x/y","provider":{"o":{"npm":"@ai-sdk/openai-compatible","options":{"apiKey":"sk-o"}},"k":{"npm":"x"}},"theme":"dark"}"#,
    )
    .unwrap();
    let f = fixture(vec![], opencode_json.clone());
    f.store
        .save_provider(crate::provider::testutil::provider_with_meta(
            App::OpenCode,
            "o1",
            "O",
            r#"{"options":{"apiKey":"sk-o"}}"#,
            r#"{"liveKey":"o","liveManaged":true}"#,
        ))
        .unwrap();
    (f, opencode_json)
}

#[test]
fn delete_additive_withdraws_live_then_deletes_row() {
    let dir = tempfile::tempdir().unwrap();
    let (f, opencode_json) = managed_live_and_row(dir.path());

    delete(&f, App::OpenCode, "o1").expect("删除已托管 provider 应成功");

    // 次序不变量正向半边：live 撤除 + 删行两半都完成——该键消失，其它
    // provider 条目与用户顶层字段语义保留。
    let live_doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&opencode_json).unwrap()).unwrap();
    assert!(live_doc["provider"].get("o").is_none(), "live 条目已撤");
    assert_eq!(live_doc["provider"]["k"]["npm"], "x", "其它条目保留");
    assert_eq!(live_doc["model"], "x/y", "用户顶层字段保留");
    assert!(
        f.store.get_provider(App::OpenCode, "o1").unwrap().is_none(),
        "DB 行已删"
    );
}

#[test]
fn delete_additive_failed_withdrawal_keeps_row() {
    let dir = tempfile::tempdir().unwrap();
    let opencode_json = dir.path().join("opencode.json");
    // live 文件半边必然失败（非法 JSON：撤除读不出 → Err）。
    std::fs::write(&opencode_json, "{not json").unwrap();
    let f = fixture(vec![], opencode_json.clone());
    f.store
        .save_provider(crate::provider::testutil::provider_with_meta(
            App::OpenCode,
            "o1",
            "O",
            r#"{"options":{"apiKey":"sk-o"}}"#,
            r#"{"liveKey":"o","liveManaged":true}"#,
        ))
        .unwrap();

    let err = delete(&f, App::OpenCode, "o1").expect_err("撤除失败必须让整个删除失败");

    // 次序不变量负向半边：任何 Err ⇒ 行原样保留（liveManaged 仍 true）、
    // live 不被写——「行没了、live 条目还挂着」的孤儿引用不会出现。
    assert!(
        err.to_string().contains("not valid JSON"),
        "应因 live 文件撤除失败: {err}"
    );
    let kept = f.store.get_provider(App::OpenCode, "o1").unwrap().unwrap();
    assert_eq!(
        live_opencode::meta_live_managed(&kept.meta),
        Some(true),
        "行原样保留"
    );
    assert_eq!(
        std::fs::read_to_string(&opencode_json).unwrap(),
        "{not json",
        "失败路径不写 live"
    );
}

#[test]
fn delete_additive_unmanaged_never_touches_file() {
    let dir = tempfile::tempdir().unwrap();
    let opencode_json = dir.path().join("opencode.json");
    std::fs::write(&opencode_json, r#"{"provider":{"k":{"npm":"x"}}}"#).unwrap();
    let f = fixture(vec![], opencode_json.clone());
    // 未托管（liveManaged=false 但有 liveKey）：停用过的 / 第二次删除的形态。
    f.store
        .save_provider(crate::provider::testutil::provider_with_meta(
            App::OpenCode,
            "o1",
            "O",
            r#"{"options":{}}"#,
            r#"{"liveKey":"o","liveManaged":false}"#,
        ))
        .unwrap();
    let before = std::fs::read_to_string(&opencode_json).unwrap();

    delete(&f, App::OpenCode, "o1").unwrap();

    // 幂等：撤除半边对未托管是显式无操作——不碰文件、不备份，只删行。
    assert_eq!(
        std::fs::read_to_string(&opencode_json).unwrap(),
        before,
        "未托管不碰文件"
    );
    assert!(
        !dir.path().join("opencode.json.bak").exists(),
        "无操作不备份"
    );
    assert!(f.store.get_provider(App::OpenCode, "o1").unwrap().is_none());
}

#[test]
fn delete_missing_provider_is_idempotent_and_never_touches_file() {
    let dir = tempfile::tempdir().unwrap();
    let opencode_json = dir.path().join("opencode.json");
    std::fs::write(&opencode_json, r#"{"provider":{"k":{"npm":"x"}}}"#).unwrap();
    let f = fixture(vec![], opencode_json.clone());
    let before = std::fs::read_to_string(&opencode_json).unwrap();

    delete(&f, App::OpenCode, "ghost").expect("重复删除（行已不存在）应幂等成功");

    assert_eq!(
        std::fs::read_to_string(&opencode_json).unwrap(),
        before,
        "行不存在 → 不碰文件"
    );
}

#[test]
fn delete_single_mode_only_deletes_row_and_never_touches_live() {
    let settings = tempfile::tempdir().unwrap();
    let settings_path = settings.path().join("settings.json");
    std::fs::write(&settings_path, r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-1"}}"#).unwrap();
    let f = fixture(
        vec![settings_path.clone()],
        PathBuf::from("/unused/opencode.json"),
    );
    save(
        &f.store,
        App::Claude,
        "p1",
        "A",
        r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-1"}}"#,
    );

    delete(&f, App::Claude, "p1").expect("单激活删除应成功");

    // 单激活语义：live 由「切换」受控覆盖，无残留条目概念——删除不碰 live
    // 文件，只删行。
    assert_eq!(
        std::fs::read_to_string(&settings_path).unwrap(),
        r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-1"}}"#,
        "单激活删除不改写 live"
    );
    assert!(f.store.get_provider(App::Claude, "p1").unwrap().is_none());
}

// —— 小工具:统一走 activate 入口,错误信息带上下文 —— //
fn activation(f: &Fixture, app: App, id: &str) -> AppResult<Provider> {
    activate(&f.store, &f.config, app, id, &f.paths)
}
fn activation_ok(f: &Fixture, app: App, id: &str) -> Provider {
    activation(f, app, id).expect("activate should succeed")
}
fn delete(f: &Fixture, app: App, id: &str) -> AppResult<()> {
    delete_provider(&f.store, app, id, &f.paths)
}
