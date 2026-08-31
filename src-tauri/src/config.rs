//! Local data layout + config.
//!
//! Everything lives under `~/.config/cc-one/` (even on Windows:
//! `C:\Users\<user>\.config\cc-one\`, CodeBurn-style). The local
//! `config.json` (token / deviceId / repo URL / display-name map) never enters
//! the repo. First start defaults to Standalone.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// deviceId 原语归 devices 域（CONTEXT「Device registry」：哪些设备存在 + 设备
// 命名是 registry 的知识），bootstrap 首代从这里调取。
use crate::devices::{default_display_name, generate_device_id, is_valid_device_id};

use crate::error::{AppError, AppResult};
use crate::fs_atomic::atomic_write_file;
use crate::model::{App, CommonConfigSnippet, RunMode};

mod wire;

// 跨界偏好枚举（wire 类型）归子模块 [`wire`]；re-export 保持
// `crate::config::` 的既有引用面（调用方路径零变化）。
pub use wire::{CloseBehavior, Language, LightweightExpand, Skin};

/// Root of all cc one local data: `~/.config/cc-one`.
pub fn root_dir() -> AppResult<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| AppError::Config("cannot resolve home dir".into()))?;
    Ok(home.join(".config").join("cc-one"))
}

/// One-time migration from the pre-rename `~/.config/vaultone` directory.
///
/// When the legacy dir exists and the new root doesn't, the whole tree
/// (config.json, local DB, sync repo, library) is moved over so an installed
/// cc one keeps its data. Skipped when the new root already exists; a failed
/// rename (e.g. the legacy dir locked by a still-running old version) only
/// logs a warning — the app boots fresh rather than crash.
///
/// Legacy 路径由 root 派生（`root.parent()/vaultone`）而非再查一次 home——
/// root 恒为 `~/.config/cc-one`，旧目录与其同级；这样
/// [`ConfigStore::load_at`] 的 bootstrap 主路径可参数化直测。
fn migrate_legacy_dir(root: &Path) {
    if root.exists() {
        return;
    }
    let legacy = root.parent().unwrap_or(root).join("vaultone");
    if legacy.exists() {
        if let Err(e) = fs::rename(&legacy, root) {
            eprintln!("[cc-one] legacy config migration skipped: {e}");
        }
    }
}

/// All well-known paths under the root.
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub config_json: PathBuf,
    pub db: PathBuf,
    pub repo: PathBuf,
    pub repo_config: PathBuf,
    pub repo_data: PathBuf,
    pub logs: PathBuf,
    pub library: PathBuf,
}

impl Paths {
    /// Resolve all paths from the root (does not create anything).
    pub fn resolve(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            config_json: root.join("config.json"),
            db: root.join("cc-one.db"),
            repo: root.join("repo"),
            repo_config: root.join("repo").join("config"),
            repo_data: root.join("repo").join("data"),
            logs: root.join("logs"),
            library: root.join("repo").join("library"),
        }
    }

    /// Per-device Artifact directory: `repo/data/<deviceId>/`.
    pub fn device_data_dir(&self, device_id: &str) -> PathBuf {
        self.repo_data.join(device_id)
    }

    /// One session's derived snapshot: `repo/data/<deviceId>/sessions/<sessionId>.jsonl`.
    /// One file per session (a conversation spans days, so per-day files would
    /// shatter it). Shared across both altitudes — the push writer, the pull
    /// reader, and collect's ghost-session reconcile all reach for it — so it
    /// lives with the rest of the layout here, not inside any one caller.
    pub fn session_snapshot_path(&self, device_id: &str, session_id: &str) -> PathBuf {
        self.device_data_dir(device_id)
            .join("sessions")
            .join(format!("{session_id}.jsonl"))
    }

    /// This device's synced provider-structure list:
    /// `repo/data/<deviceId>/providers.json`. One file per device (the shared
    /// per-device-write pattern — groups.json, session snapshots): each device
    /// writes only its own file; reading merges every device's file by id,
    /// latest wins. Written key-stripped — API keys stay in the local DB and
    /// never enter this file.
    pub fn providers_json_path(&self, device_id: &str) -> PathBuf {
        self.device_data_dir(device_id).join("providers.json")
    }

    /// Local pricing config: `<root>/pricing.json`. Sits next to `config.json`,
    /// never enters the repo — pricing is a per-device local concern (each device
    /// freezes cost with its own prices at collect time). `save_pricing_to_file`
    /// writes here; `reload_pricing_from_file` reads here. DB remains the runtime
    /// truth; this file is a local export/import surface only.
    pub fn pricing_json(&self) -> PathBuf {
        self.root.join("pricing.json")
    }

    /// Cloud device-name registry: `repo/config/devices_<id>.json`, one file per
    /// device (flattened — no `devices/` subdir). Each device writes only its
    /// own file, so concurrent edits never collide (zero Git merge conflict).
    /// Carried by the normal Git sync flow.
    pub fn devices_file_path(&self, device_id: &str) -> PathBuf {
        self.repo_config.join(format!("devices_{device_id}.json"))
    }

    /// Legacy registry dir (`repo/config/devices/`) from before the flattening.
    /// Read-only fallback so a peer still on the old layout stays visible until
    /// it republishes; new writes always go to [`Self::devices_file_path`].
    pub fn legacy_devices_dir(&self) -> PathBuf {
        self.repo_config.join("devices")
    }
}

/// Default background-collect interval in seconds (30 s — decoupled
/// from the push cadence, which has its own interval).
///
/// `u32` (not `u64`): the value crosses the Rust→JS boundary via the typed
/// specta contract, and specta forbids exporting BigInt-style types (`u64`,
/// `i64`, …) to avoid JS precision loss. `u32`'s range (≈4.29e9 s) is ample
/// for an interval clamped to [5, 3600].
fn default_collect_interval_secs() -> u32 {
    30
}

/// Default push-to-sync interval in seconds (10 min). Decoupled from
/// collect so a short collect cadence does not bloat the Git history.
fn default_push_interval_secs() -> u32 {
    600
}

/// Default delay before an invisible (minimized / hidden-to-tray) full window
/// auto-tucks into the mini bar (30 s). `0` (off) is chosen in the Settings
/// Select; the serde default keeps an upgraded config.json on 30, not off.
fn default_lightweight_auto_tuck_secs() -> u32 {
    30
}

/// 旧全局片段字段（`common_config_snippet`）的 serde 默认：旧写法只有 claude
/// 池，其默认内容 = claude 的默认片段。内容决策单源在
/// [`App::default_common_snippet`]（model 层 per-app 事实）——这里只是旧字段
/// 的存储默认垫片，不再持有片段字面量。
fn default_common_config_snippet() -> String {
    App::Claude.default_common_snippet().content
}

/// The local `config.json` content. Never uploaded to the repo.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfigData {
    pub device_id: String,
    /// Friendly name for *this* device (display name, not a key).
    pub display_name: String,
    /// Sync repo URL; `None` ⇒ Standalone.
    pub repo_url: Option<String>,
    /// Fine-grained PAT; kept only in local config + Rust memory.
    #[serde(default)]
    pub github_token: Option<String>,
    /// `deviceId → friendly name` for other devices seen in the repo.
    #[serde(default)]
    pub device_names: BTreeMap<String, String>,
    /// Optional: GitHub handle resolved from the token (for display only).
    #[serde(default)]
    pub github_user: Option<String>,
    /// Window-close behavior. `Ask` ⇒ show the minimize/quit dialog.
    #[serde(default)]
    pub close_behavior: CloseBehavior,
    /// Background collect interval in seconds. Clamped to [5, 3600]
    /// at use; serialized verbatim so the UI shows what the user typed.
    #[serde(default = "default_collect_interval_secs")]
    pub collect_interval_secs: u32,
    /// Push-to-sync interval in seconds. Synced only; clamped to
    /// [60, 7200] at use. Decoupled from collect so the Git push cadence stays
    /// independent of the (shorter) collect cadence.
    #[serde(default = "default_push_interval_secs")]
    pub push_interval_secs: u32,
    /// Display language. Default English; per-device, not synced
    /// (config.json never enters the repo).
    #[serde(default)]
    pub language: Language,
    /// How the lightweight half-icon expands. Frontend-only behavior;
    /// Rust doesn't read it, but it lives here so all Settings prefs are unified.
    #[serde(default)]
    pub lightweight_expand: LightweightExpand,
    /// Delay (seconds) before an invisible full window auto-tucks into the
    /// mini bar; `0` = off. Frontend-only — Rust stores it for unity.
    #[serde(default = "default_lightweight_auto_tuck_secs")]
    pub lightweight_auto_tuck_secs: u32,
    /// Color skin (multi-skin theming). Frontend-only effect; Rust doesn't act
    /// on it, but it rides ConfigData so every Settings preference is unified.
    #[serde(default)]
    pub skin: Skin,
    /// 当前激活的供应商 id，**按应用各一份**（`app → provider id`）：
    /// claude / codex / gemini 各自独立，切换时记录对应应用的键。存本机
    /// config.json，重启后保持、不进 git。某应用未激活时没有它的键。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub active_providers: BTreeMap<String, String>,
    /// 应用维度之前的单键激活记录（旧写法，只有 claude 池）。保留仅为
    /// [`ConfigStore::load`] 迁移到 [`Self::active_providers`] 的 claude 键；
    /// 迁移后不再读取、下次写盘时被剥掉（`skip_serializing_if`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider_id: Option<String>,
    /// 通用配置片段，**按应用各一份**（`app → 片段`）：claude / codex /
    /// gemini 各自独立。存本机 config.json——与 `active_providers` 同属本机
    /// 配置，不进 git、不随同步仓库走。见 [`Self::snippet_for`]。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub common_config_snippets: BTreeMap<String, CommonConfigSnippet>,
    /// 应用维度之前的全局单条片段（旧写法，只有 claude 池）：settings.json
    /// 片段原文 + 启用开关。保留仅为 [`ConfigStore::load`] 迁移到
    /// [`Self::common_config_snippets`] 的 claude 键；迁移后不再读取。
    #[serde(default = "default_common_config_snippet")]
    pub common_config_snippet: String,
    /// 应用维度之前的全局单条片段启用开关（见 [`Self::common_config_snippet`]）。
    #[serde(default)]
    pub common_config_snippet_enabled: bool,
}

impl Default for ConfigData {
    fn default() -> Self {
        // A real deviceId is generated on first start (see [`ConfigStore::load`]
        // bootstrap); this default is only a fallback if config.json lacks the
        // field.
        Self {
            device_id: String::new(),
            display_name: "CC One".to_string(),
            repo_url: None,
            github_token: None,
            device_names: BTreeMap::new(),
            github_user: None,
            close_behavior: CloseBehavior::Ask,
            collect_interval_secs: default_collect_interval_secs(),
            push_interval_secs: default_push_interval_secs(),
            language: Language::En,
            lightweight_expand: LightweightExpand::Click,
            lightweight_auto_tuck_secs: default_lightweight_auto_tuck_secs(),
            skin: Skin::Neutral,
            active_providers: BTreeMap::new(),
            active_provider_id: None,
            common_config_snippets: BTreeMap::new(),
            common_config_snippet: default_common_config_snippet(),
            // 旧全局字段的默认保持 false（历史语义；该字段已被
            // common_config_snippets 取代——片段默认开启由 snippet_for /
            // migrate_legacy_fields 在 map 层保证，这里不影响任何行为）。
            common_config_snippet_enabled: false,
        }
    }
}

impl ConfigData {
    /// Synced iff a repo URL *and* a token are configured.
    pub fn mode(&self) -> RunMode {
        match self.repo_url.as_deref().zip(self.github_token.as_deref()) {
            Some((url, token)) if !url.trim().is_empty() && !token.trim().is_empty() => {
                RunMode::Synced
            }
            _ => RunMode::Standalone,
        }
    }

    pub fn is_synced(&self) -> bool {
        self.mode() == RunMode::Synced
    }

    /// 某应用当前激活的供应商 id；未激活 → `None`。
    pub fn active_provider_id_for(&self, app: App) -> Option<String> {
        self.active_providers.get(app.as_str()).cloned()
    }

    /// 记录某应用的激活供应商 id（切换写盘后调用）。
    pub fn set_active_provider(&mut self, app: App, id: &str) {
        self.active_providers
            .insert(app.as_str().to_string(), id.to_string());
    }

    /// 某应用的通用配置片段：已有条目原样返回；缺省（新应用池 / 手改
    /// config.json 删了键）回退 [`App::default_common_snippet`] 的 per-app
    /// 默认（claude 隐藏署名片段、其余空片段；未保存过 → 默认启用）。本方法
    /// 只管「存 map、缺键给默认」的存取语义，不含 per-app 内容决策——那归
    /// model 层的 App（第 6 个应用带默认片段时改 App，不改这里）。
    pub fn snippet_for(&self, app: App) -> CommonConfigSnippet {
        self.common_config_snippets
            .get(app.as_str())
            .cloned()
            .unwrap_or_else(|| app.default_common_snippet())
    }

    /// 写入某应用的通用配置片段（set 命令；内容合法性由调用方校验）。
    pub fn set_snippet(&mut self, app: App, snippet: CommonConfigSnippet) {
        self.common_config_snippets
            .insert(app.as_str().to_string(), snippet);
    }

    /// Mask the token for any non-storage surface (logs / UI echoes).
    pub fn masked_token(&self) -> Option<String> {
        self.github_token.as_ref().map(|t| {
            let len = t.chars().count();
            if len <= 8 {
                "****".to_string()
            } else {
                let head: String = t.chars().take(4).collect();
                let tail: String = t.chars().skip(len.saturating_sub(4)).collect();
                format!("{head}…{tail}")
            }
        })
    }
}

/// Thread-safe holder for the loaded config + paths, shared via Tauri state.
#[derive(Debug)]
pub struct ConfigStore {
    paths: Paths,
    data: Mutex<ConfigData>,
}

impl ConfigStore {
    /// Load (or bootstrap on first run) config + ensure the full directory
    /// layout exists. Idempotent.
    pub fn load() -> AppResult<Self> {
        let root = root_dir()?;
        Self::load_at(&root)
    }

    /// Bootstrap 主路径（root 参数化，测试直测）：解析路径 → 迁移 legacy 目录
    /// → 建全目录 → 读 config（损坏回退默认）→ 必要时重写。`load()` 解析真实
    /// home 后委托这里。
    fn load_at(root: &Path) -> AppResult<Self> {
        let paths = Paths::resolve(root);

        // Renamed CC One → cc one: move the old config tree over on first
        // launch so existing users keep their settings, DB, and sync repo.
        migrate_legacy_dir(root);

        // Full directory layout up front.
        for dir in [
            &paths.root,
            &paths.repo,
            &paths.repo_config,
            &paths.repo_data,
            &paths.logs,
            &paths.library,
        ] {
            fs::create_dir_all(dir)?;
        }

        let data = match fs::read(&paths.config_json) {
            Ok(bytes) => serde_json::from_slice::<ConfigData>(&bytes).unwrap_or_else(|e| {
                // 兜底语义：config.json 经 [`atomic_write_file`] 落盘，本应用
                // 自己的半截写盘进不到这里——「读不进」只可能来自应用之外的
                // 损坏（手编 / 外部工具写坏 / 磁盘问题）。兜底保留（外部损坏
                // 不能挡启动），但代价是重新 bootstrap deviceId——设备身份
                // 分叉、旧设备子树留在 repo 里回不去。原子写已把这个后果从
                // 「自有写盘的日常尾部」压缩成「仅外部损坏的最后手段」。
                eprintln!("[cc-one] config.json unreadable, re-bootstrapping: {e}");
                ConfigData::default()
            }),
            Err(_) => ConfigData::default(),
        };

        let mut data = data;
        let mut dirty = false;

        // deviceId first-generation: persistent 12-hex, collision-checked.
        if data.device_id.is_empty() || !is_valid_device_id(&data.device_id) {
            data.device_id = generate_device_id(&paths);
            if data.display_name.trim().is_empty() || data.display_name == "CC One" {
                data.display_name = default_display_name(&data.device_id);
            }
            dirty = true;
        }

        // 应用维度迁移（存量 config.json 只有 claude 池的旧写法）——纯函数，
        // 见 [`migrate_legacy_fields`]；返回「是否需要重写文件」。
        if migrate_legacy_fields(&mut data) {
            dirty = true;
        }

        if dirty {
            Self::write_config(&paths, &data)?;
        }

        Ok(Self {
            paths,
            data: Mutex::new(data),
        })
    }

    /// Snapshot the current config.
    pub fn get(&self) -> ConfigData {
        self.data.lock().expect("config mutex poisoned").clone()
    }

    /// Mutate the in-memory config under the lock, persist, and return a copy.
    pub fn update<F>(&self, mutate: F) -> AppResult<ConfigData>
    where
        F: FnOnce(&mut ConfigData),
    {
        let mut data = self.data.lock().expect("config mutex poisoned");
        mutate(&mut data);
        Self::write_config(&self.paths, &data)?;
        Ok(data.clone())
    }

    /// Read-only path accessors.
    pub fn paths(&self) -> Paths {
        self.paths.clone()
    }

    /// Persist config.json. Atomic (temp + rename, [`atomic_write_file`]):
    /// this file carries the deviceId (device identity), the PAT and the
    /// activation state, and the read side's corrupt fallback regenerates the
    /// deviceId — a torn write from a process interruption would fork the
    /// device identity (the old device subtree stays in the repo, unreachable).
    /// Atomic write removes that failure mode instead of leaning on the
    /// fallback (see `load_at`).
    fn write_config(paths: &Paths, data: &ConfigData) -> AppResult<()> {
        let json = serde_json::to_string_pretty(data)?;
        atomic_write_file(&paths.config_json, &json)
    }
}

/// 应用维度迁移（纯函数，`ConfigStore::load` 在生产路径调用）：把存量
/// config.json 的旧写法迁到按应用存的字段——
/// - 单键 `active_provider_id` → `active_providers["claude"]`（旧写法只有
///   claude 池；map 里已有 claude 键时以 map 为准，旧键是 stale 的）；
/// - 全局片段 `common_config_snippet` / `common_config_snippet_enabled` →
///   `common_config_snippets["claude"]`（已有 claude 键则不覆盖）。
///
/// 迁移即剥离旧字段（旧字段 `take()` / 后续写盘不再序列化）。返回「数据是否
/// 变化、需要重写 config.json」——幂等：新格式（没有旧字段）返回 `false`。
fn migrate_legacy_fields(data: &mut ConfigData) -> bool {
    let mut dirty = false;
    if let Some(id) = data.active_provider_id.take() {
        data.active_providers
            .entry("claude".to_string())
            .or_insert(id);
        dirty = true;
    }
    if !data.common_config_snippets.contains_key("claude") {
        // 未保存过片段（map 无键）→ 插入时恒为启用：片段默认开启是产品
        // 语义（跨供应商共享默认值开箱生效），旧全局字段的默认 false 只是
        // 旧版本的产品默认，不是用户主动选择——显式保存过 enabled=false
        // 的条目在 map 里有键，这里不会覆盖它。
        data.common_config_snippets.insert(
            "claude".to_string(),
            CommonConfigSnippet {
                enabled: true,
                content: data.common_config_snippet.clone(),
            },
        );
        dirty = true;
    }
    dirty
}

/// Test-only constructor: back a `ConfigStore` with `paths` and `data` without
/// bootstrapping the home layout (`load` does that). The caller ensures the
/// directory layout exists. Mutations persist to `paths.config_json`, so tests
/// exercise the real `update`/`get`/`paths` code path. Used by `devices`
/// lifecycle tests (register/rename/forget orchestrators take `&ConfigStore`).
#[cfg(test)]
impl ConfigStore {
    pub(crate) fn for_test(paths: Paths, data: ConfigData) -> Self {
        Self {
            paths,
            data: Mutex::new(data),
        }
    }
}

#[cfg(test)]
mod tests;
