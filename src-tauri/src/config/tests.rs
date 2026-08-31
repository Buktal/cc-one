//! config 域测试（独立测试模块文件，不计入 config.rs 行数）：ConfigData 存取
//! 语义、per-app 迁移、bootstrap 主路径（load_at 参数化直测）与 update 写盘。

use super::*;

#[test]
fn mode_requires_both_repo_url_and_token() {
    let mut c = ConfigData::default();
    assert_eq!(c.mode(), RunMode::Standalone);
    c.repo_url = Some("https://github.com/x/y".into());
    assert_eq!(c.mode(), RunMode::Standalone, "token still missing");
    c.github_token = Some("ghp_token".into());
    assert_eq!(c.mode(), RunMode::Synced);
    c.github_token = Some("   ".into());
    assert_eq!(c.mode(), RunMode::Standalone, "blank token ⇒ standalone");
}

#[test]
fn masked_token_redacts() {
    let mut c = ConfigData::default();
    assert_eq!(c.masked_token(), None);
    c.github_token = Some("short".into());
    assert_eq!(c.masked_token().as_deref(), Some("****"));
    c.github_token = Some("ghp_abcdefghijklmnop".into());
    assert_eq!(c.masked_token().as_deref(), Some("ghp_…mnop"));
}

#[test]
fn snippet_fields_default_and_roundtrip() {
    // 旧 config.json 没有片段字段 → 旧全局字段反序列化默认 false（历史
    // 语义；产品「默认开启」由 snippet_for / migrate_legacy_fields 在
    // common_config_snippets 层保证，不在此字段）。
    let c: ConfigData =
        serde_json::from_str(r#"{"device_id":"abc123def456","display_name":"V"}"#).unwrap();
    assert_eq!(c.common_config_snippet, r#"{"includeCoAuthoredBy": false}"#);
    assert!(!c.common_config_snippet_enabled);

    // 显式值经 config.json 序列化往返不丢。
    let c2 = ConfigData {
        common_config_snippet: r#"{"includeCoAuthoredBy": true, "attribution": "x"}"#.into(),
        common_config_snippet_enabled: true,
        ..Default::default()
    };
    let json = serde_json::to_string(&c2).unwrap();
    let back: ConfigData = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.common_config_snippet,
        r#"{"includeCoAuthoredBy": true, "attribution": "x"}"#
    );
    assert!(back.common_config_snippet_enabled);
}

// ---- 应用维度：per-app 激活与片段存取 ----

#[test]
fn per_app_active_provider_accessors() {
    let mut c = ConfigData::default();
    assert_eq!(c.active_provider_id_for(App::Claude), None);
    assert_eq!(c.active_provider_id_for(App::Codex), None);
    c.set_active_provider(App::Claude, "claude-1");
    c.set_active_provider(App::Codex, "codex-1");
    assert_eq!(
        c.active_provider_id_for(App::Claude).as_deref(),
        Some("claude-1")
    );
    assert_eq!(
        c.active_provider_id_for(App::Codex).as_deref(),
        Some("codex-1")
    );
    assert_eq!(c.active_provider_id_for(App::Gemini), None);
    // 覆盖同应用的旧记录。
    c.set_active_provider(App::Claude, "claude-2");
    assert_eq!(
        c.active_provider_id_for(App::Claude).as_deref(),
        Some("claude-2")
    );
    // 经 config.json 序列化往返不丢。
    let json = serde_json::to_string(&c).unwrap();
    let back: ConfigData = serde_json::from_str(&json).unwrap();
    assert_eq!(back.active_providers.len(), 2);
    assert_eq!(
        back.active_provider_id_for(App::Codex).as_deref(),
        Some("codex-1")
    );
}

#[test]
fn snippet_for_defaults_per_app() {
    let c = ConfigData::default();
    // claude 默认隐藏署名片段；codex/gemini/grok 默认空片段（留空自填）。
    // 未保存过 → 默认启用（跨供应商共享默认值开箱生效）。
    let claude = c.snippet_for(App::Claude);
    assert_eq!(claude.content, r#"{"includeCoAuthoredBy": false}"#);
    assert!(claude.enabled);
    assert_eq!(c.snippet_for(App::Codex).content, "");
    assert_eq!(c.snippet_for(App::Gemini).content, "");
    assert_eq!(c.snippet_for(App::Grok).content, "");
}

#[test]
fn snippet_set_and_roundtrip_per_app() {
    let mut c = ConfigData::default();
    c.set_snippet(
        App::Codex,
        CommonConfigSnippet {
            enabled: true,
            content: r#"{"custom": 1}"#.into(),
        },
    );
    let codex = c.snippet_for(App::Codex);
    assert!(codex.enabled);
    assert_eq!(codex.content, r#"{"custom": 1}"#);
    let json = serde_json::to_string(&c).unwrap();
    let back: ConfigData = serde_json::from_str(&json).unwrap();
    assert!(back.snippet_for(App::Codex).enabled);
    assert_eq!(back.snippet_for(App::Codex).content, r#"{"custom": 1}"#);
    // claude 键未被写入 → 仍回退默认。
    assert_eq!(
        back.snippet_for(App::Claude).content,
        r#"{"includeCoAuthoredBy": false}"#
    );
}

// ---- 应用维度迁移：存量单键归 claude ----

/// 旧 config.json（单键 active_provider_id + 全局片段）加载后迁移：
/// 激活记录归 claude 键、片段归 claude 键，旧字段被剥离，重写文件。
#[test]
fn migrate_legacy_fields_moves_single_keys_to_claude() {
    // 反序列化旧 config.json 的形状（模拟 `ConfigStore::load` 的读入）。
    let c: ConfigData = serde_json::from_str(
        r#"{"device_id":"abc123def456","display_name":"V","active_provider_id":"p1","common_config_snippet":"{\"includeCoAuthoredBy\": true}","common_config_snippet_enabled":true}"#,
    )
    .unwrap();
    let mut c = c;
    assert!(migrate_legacy_fields(&mut c), "旧字段存在 → 需要重写");
    assert_eq!(
        c.active_provider_id_for(App::Claude).as_deref(),
        Some("p1"),
        "存量激活归 claude 键"
    );
    let claude = c.snippet_for(App::Claude);
    assert!(claude.enabled);
    assert_eq!(claude.content, r#"{"includeCoAuthoredBy": true}"#);
    // 旧字段被剥离。
    assert!(c.active_provider_id.is_none());
    // 幂等：再跑一遍 → 无变化，不再标记重写。
    assert!(!migrate_legacy_fields(&mut c), "新格式幂等：无需重写");
    assert_eq!(c.active_provider_id_for(App::Claude).as_deref(), Some("p1"));
}

/// 旧 config.json 里片段从未启用（旧产品默认 false，用户没动过）→ 迁移
/// 后 claude 键默认启用（新产品语义：片段默认开启）；用户显式保存过的
/// false 在 map 里有键，迁移不会覆盖（见 migrate_legacy_fields）。
#[test]
fn migrate_flips_unset_snippet_to_enabled() {
    let c: ConfigData = serde_json::from_str(
        r#"{"device_id":"abc123def456","display_name":"V","common_config_snippet":"{\"includeCoAuthoredBy\": false}","common_config_snippet_enabled":false}"#,
    )
    .unwrap();
    let mut c = c;
    assert!(migrate_legacy_fields(&mut c));
    assert!(
        c.snippet_for(App::Claude).enabled,
        "未主动保存的片段随新产品默认开启"
    );
    // 幂等。
    assert!(!migrate_legacy_fields(&mut c));
}

/// 新旧字段并存（手改/回滚残留）→ 新字段（map）为准，旧键是 stale 的。
#[test]
fn migrate_keeps_existing_per_app_values_over_legacy() {
    let mut c = ConfigData {
        active_provider_id: Some("stale".into()),
        ..Default::default()
    };
    c.active_providers
        .insert("claude".to_string(), "current".into());
    assert!(migrate_legacy_fields(&mut c));
    assert_eq!(
        c.active_provider_id_for(App::Claude).as_deref(),
        Some("current"),
        "map 里已有的 claude 键优先，旧键不覆盖"
    );
    // 片段同理：已有 claude 键则保留，不覆盖。
    let mut c2 = ConfigData::default();
    c2.common_config_snippets.insert(
        "claude".to_string(),
        CommonConfigSnippet {
            enabled: true,
            content: "{}".into(),
        },
    );
    migrate_legacy_fields(&mut c2);
    assert_eq!(c2.snippet_for(App::Claude).content, "{}");
}

// ---- bootstrap 主路径（load_at 参数化直测）----

/// 新建默认行为：目录全建、config.json 落盘、deviceId 有效且持久化。
#[test]
fn load_at_bootstraps_fresh_root() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".config").join("cc-one");
    let store = ConfigStore::load_at(&root).unwrap();
    let data = store.get();
    assert!(is_valid_device_id(&data.device_id));
    assert_eq!(data.display_name, default_display_name(&data.device_id));
    assert_eq!(data.mode(), RunMode::Standalone);
    for dir in ["repo", "repo/config", "repo/data", "logs", "repo/library"] {
        assert!(root.join(dir).exists(), "{dir} 应已创建");
    }
    let on_disk: ConfigData =
        serde_json::from_str(&fs::read_to_string(root.join("config.json")).unwrap()).unwrap();
    assert_eq!(on_disk.device_id, data.device_id, "deviceId 应落盘持久化");
}

/// legacy 目录迁移：旧 `~/.config/vaultone` 有 config → 整树迁到新 root，
/// deviceId 原样保留（不重新生成）。
#[test]
fn load_at_migrates_legacy_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let legacy = tmp.path().join(".config").join("vaultone");
    let root = tmp.path().join(".config").join("cc-one");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(
        legacy.join("config.json"),
        r#"{"device_id":"abc123def456","display_name":"V"}"#,
    )
    .unwrap();
    let store = ConfigStore::load_at(&root).unwrap();
    assert!(!legacy.exists(), "legacy 目录应被迁移走");
    let data = store.get();
    assert_eq!(data.device_id, "abc123def456", "迁移保留原 deviceId");
    assert_eq!(data.display_name, "V");
}

/// 新 root 已存在（legacy 也在）→ 不动 legacy，新 root 优先。
#[test]
fn load_at_keeps_legacy_when_root_already_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let legacy = tmp.path().join(".config").join("vaultone");
    let root = tmp.path().join(".config").join("cc-one");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(
        legacy.join("config.json"),
        r#"{"device_id":"abc123def456","display_name":"V"}"#,
    )
    .unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("config.json"),
        r#"{"device_id":"0123456789ab","display_name":"N"}"#,
    )
    .unwrap();
    let store = ConfigStore::load_at(&root).unwrap();
    assert!(legacy.exists(), "新 root 已存在 → legacy 保留不动");
    assert_eq!(store.get().device_id, "0123456789ab");
}

/// 外部损坏 config（手编 / 外部工具写坏——config.json 经 [`write_config`]
/// 原子写落盘，本应用自己的半截写盘进不到这里）：不崩溃，回退默认重新
/// bootstrap（新 deviceId 避开已有设备目录），损坏文件被重写为合法 JSON，
/// 已有设备数据目录不被触碰。这是「仅外部损坏」的最后手段，不是自有写盘
/// 的日常路径。
#[test]
fn load_at_falls_back_on_corrupt_config() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".config").join("cc-one");
    let seeded = "aabbccddeeff";
    fs::create_dir_all(root.join("repo").join("data").join(seeded)).unwrap();
    fs::write(root.join("config.json"), b"not json {").unwrap();
    let store = ConfigStore::load_at(&root).unwrap();
    let data = store.get();
    assert!(is_valid_device_id(&data.device_id));
    assert_ne!(data.device_id, seeded, "新 deviceId 避开已有设备目录");
    assert!(
        root.join("repo").join("data").join(seeded).exists(),
        "已有设备数据目录不受回退影响"
    );
    let on_disk: ConfigData =
        serde_json::from_str(&fs::read_to_string(root.join("config.json")).unwrap()).unwrap();
    assert_eq!(
        on_disk.device_id, data.device_id,
        "损坏文件被重写为合法 config"
    );
}

// ---- update 写盘（原子路径）----

/// update 重写**已存在**的 config.json：新值持久化、旧文件被整文件替换，
/// root 不留 `.tmp.*` 残留——守「config.json 走原子写而非裸 fs::write」
/// （目标已存在时的替换语义由原语自测；这里守 ConfigStore 的实际写路径）。
#[test]
fn update_replaces_existing_config_without_temp_residue() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".config").join("cc-one");
    let store = ConfigStore::load_at(&root).unwrap();
    store.update(|c| c.collect_interval_secs = 42).unwrap();
    let on_disk: ConfigData =
        serde_json::from_str(&fs::read_to_string(root.join("config.json")).unwrap()).unwrap();
    assert_eq!(on_disk.collect_interval_secs, 42);
    assert!(
        on_disk.device_id == store.get().device_id,
        "重写不得丢 deviceId"
    );
    let leftovers: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
        .collect();
    assert!(
        leftovers.is_empty(),
        "config.json 原子写不得残留临时文件: {leftovers:?}"
    );
}
