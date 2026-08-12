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

use rand::Rng;

use crate::error::{AppError, AppResult};
use crate::model::{App, CommonConfigSnippet, RunMode};

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
fn migrate_legacy_dir(root: &Path) {
    if root.exists() {
        return;
    }
    let Some(home) = dirs::home_dir() else { return };
    let legacy = home.join(".config").join("vaultone");
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

/// Window-close behavior preference. Crosses the Rust→JS boundary.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    /// Show the minimize/quit dialog each time (default).
    #[default]
    Ask,
    /// Always minimize to tray — keeps the background scheduler alive.
    Minimize,
    /// Always quit.
    Quit,
}

/// How the lightweight glance card's tucked half-icon expands.
/// Crosses the Rust→JS boundary; Rust itself doesn't act on it (a pure frontend
/// interaction), but it rides `ConfigData` so every Settings preference lives in
/// one place.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum LightweightExpand {
    /// Click the half-icon to expand (default — won't fire on a stray hover).
    #[default]
    Click,
    /// Hover the half-icon to expand.
    Hover,
}

/// Color skin for multi-skin theming (token-first). Serialized
/// snake_case; `neutral` is the default and maps to NO `data-skin` attribute on
/// `<html>` (the :root/.dark values in src/index.css ARE the Neutral palette —
/// pure greyscale chrome over a default multi-hue chart). Per-device, not synced
/// (config.json never enters the repo). The four chromatic skins each override
/// `--brand` (+ `--brand-strong`) and the button-foreground vars in index.css;
/// everything else holds. The frontend applies it; Rust only stores it.
///
/// Back-compat: the legacy snake_case names (`pixso`/`cuiwei`/`tingwu`/
/// `yanzhi`/`zizi`) are accepted as aliases, so an older config.json lands on
/// the closest new skin instead of failing to deserialize — `pixso` (the old
/// default) → `Neutral` (the new default); the rest map by hue family.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum Skin {
    #[default]
    #[serde(alias = "pixso")]
    Neutral,
    #[serde(alias = "cuiwei")]
    Sage,
    #[serde(alias = "tingwu")]
    Azure,
    #[serde(alias = "yanzhi")]
    Crimson,
    #[serde(alias = "zizi")]
    Mauve,
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

/// 通用配置片段默认内容：隐藏署名（`includeCoAuthoredBy: false`）。
fn default_common_config_snippet() -> String {
    r#"{"includeCoAuthoredBy": false}"#.to_string()
}

/// Display language. Serialized lowercase (`en`/`zh`/`ja`), matching
/// the frontend locale codes. The tray "Quit" item — the only user-facing Rust
/// string — is localized from this; all other UI text is frontend i18n.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Zh,
    Ja,
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
    /// Background collect interval in seconds. Clamped to [10, 3600]
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
        // A real deviceId is generated on first start (see `ensure_config`);
        // this default is only a fallback if config.json lacks the field.
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
    /// config.json 删了键）按应用给默认——claude 沿用隐藏署名片段，其余应用
    /// 默认空片段（它们的配置里没有署名概念，留空由用户自填）。未保存过 →
    /// **默认启用**（片段是跨供应商的共享默认值，开箱即生效；显式保存过
    /// enabled=false 的条目原样返回，尊重用户关闭）。
    pub fn snippet_for(&self, app: App) -> CommonConfigSnippet {
        self.common_config_snippets
            .get(app.as_str())
            .cloned()
            .unwrap_or_else(|| CommonConfigSnippet {
                enabled: true,
                content: if app == App::Claude {
                    default_common_config_snippet()
                } else {
                    String::new()
                },
            })
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
        let paths = Paths::resolve(&root);

        // Renamed CC One → cc one: move the old config tree over on first
        // launch so existing users keep their settings, DB, and sync repo.
        migrate_legacy_dir(&root);

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
                // Corrupt config shouldn't brick the app; log + fall back, then
                // re-bootstrap a sane deviceId below.
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

    fn write_config(paths: &Paths, data: &ConfigData) -> AppResult<()> {
        let bytes = serde_json::to_vec_pretty(data)?;
        fs::write(&paths.config_json, bytes)?;
        Ok(())
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

/// A valid deviceId is 12 lowercase hex chars (48-bit short id).
pub fn is_valid_device_id(id: &str) -> bool {
    id.len() == 12
        && id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && (!c.is_ascii_alphabetic() || c.is_ascii_lowercase()))
}

/// Generate a 12-hex deviceId (48 bits), retrying if it collides with an
/// existing device dir in `repo/data/` (collision check).
fn generate_device_id(paths: &Paths) -> String {
    let existing = list_existing_device_ids(&paths.repo_data);
    let mut rng = rand::thread_rng();
    for _ in 0..8 {
        let bytes: [u8; 6] = rng.gen();
        let id = hex_encode(&bytes);
        if !existing.iter().any(|e| e == &id) {
            return id;
        }
    }
    // Astronomically unlikely (8 × 2^-48); fall through with the last candidate.
    let bytes: [u8; 6] = rng.gen();
    hex_encode(&bytes)
}

fn list_existing_device_ids(repo_data: &Path) -> Vec<String> {
    match fs::read_dir(repo_data) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|s| is_valid_device_id(s))
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn default_display_name(device_id: &str) -> String {
    let prefix = &device_id[..6.min(device_id.len())];
    format!("Device-{prefix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_device_id_rules() {
        assert!(is_valid_device_id("0123456789ab"));
        assert!(is_valid_device_id("abcdef012345"));
        assert!(!is_valid_device_id("0123456789a")); // too short
        assert!(!is_valid_device_id("0123456789abc")); // too long
        assert!(!is_valid_device_id("abcdef01234g")); // non-hex letter
        assert!(!is_valid_device_id("ABCDEF012345")); // uppercase rejected
    }

    #[test]
    fn generated_device_id_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let id = generate_device_id(&paths);
        assert!(is_valid_device_id(&id));
    }

    #[test]
    fn generated_device_id_avoids_existing_collisions() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Pre-seed an existing device dir under repo/data/.
        fs::create_dir_all(paths.device_data_dir("aabbccddeeff")).unwrap();
        for _ in 0..16 {
            let id = generate_device_id(&paths);
            assert_ne!(
                id, "aabbccddeeff",
                "generator must avoid existing device dirs"
            );
            assert!(is_valid_device_id(&id));
        }
    }

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
}
