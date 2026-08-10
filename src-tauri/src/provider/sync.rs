//! Provider structure sync — the per-device `providers.json`, following the
//! Synced Group / device-registry pattern: each device writes ONLY its own
//! `repo/data/<deviceId>/providers.json` (a JSON object with one `providers`
//! array and a schema version `v`); reading merges every device's file by
//! `(app, id)`, latest `updated_at` wins (ties → first seen). This is the
//! structure half of provider sync.
//!
//! **App dimension.** Every line carries `app` (`claude` / `codex` / `gemini`);
//! the merge/dedup key is `(app, id)`, so the same vendor id in two app pools
//! stays two entries. An old file without `app` fields reads every line as
//! claude (serde default) — pre-dimension data all belongs there. A file with
//! a schema `v` NEWER than [`SYNCED_PROVIDERS_DOC_VERSION`] is skipped whole
//! (the sessions-snapshot version gate): this binary cannot attribute it, so
//! it must not mis-merge it by id.
//!
//! **API keys never enter the file.** Every provider is
//! [`Provider::redacted`] on write: the four `SECRET_ENV_KEYS` are stripped
//! from `settingsConfig`'s `env` **and** from `meta.templateValues` — the
//! frontend's record of filled `${VAR}` template variables, which is how the
//! Bedrock presets carry AK/SK (`AWS_REGION` — a region code or a `${VAR}`
//! placeholder — is not a credential and stays). Each device's keys live only
//! in its local DB; the active provider is local-only too (config.json) and
//! never touches this file.
//!
//! Sync orchestration lives in `sync::flow` (this module holds no git
//! knowledge): **push** (`push_usage`) calls [`write_own_providers`] to
//! materialize this device's file from the store — byte-stable, so an
//! unchanged store rewrites identical bytes and `commit_and_push` no-ops;
//! **pull** (`pull_and_import`) calls [`import_peer_providers`] to read
//! peers' files back into the store. Self's own directory is skipped on read —
//! self is local-authoritative, so a possibly-stale git copy of this device
//! must never overwrite fresher local rows.
//!
//! Import is latest-wins on `updated_at` and NEVER drops a local key
//! ([`merge_local_keys`]): the peer's key-stripped structure wins, but the
//! local row's secret env keys are merged back in. Since `save_provider`
//! advances `updated_at` only on structural change, the comparison is a true
//! structural freshness check — a key fill on one device can never mask a
//! peer's later edit. `sort_index` (display order) stays a local preference:
//! `import_provider` keeps the local row's value, so pulls never shuffle the
//! user's order.

use crate::config::Paths;
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::model::{Provider, SECRET_ENV_KEYS};

/// The providers.json schema version this binary reads (sessions-snapshot
/// style `v` gate). Files with a HIGHER `v` are skipped whole on read — this
/// binary cannot attribute their app fields, so merging them by id could
/// mis-attribute entries; their providers simply arrive after an upgrade.
pub const SYNCED_PROVIDERS_DOC_VERSION: u32 = 1;

/// One device's provider-file wrapper: a stable JSON object with one
/// `providers` array + schema version `v`. Files without `v` (pre-version
/// format) read as 0 — old format, still attributable (lines default to
/// claude). Missing file ⇒ empty doc.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct SyncedProvidersDoc {
    /// Schema version; absent (old format) ⇒ 0, read as an old file.
    #[serde(default)]
    v: u32,
    #[serde(default)]
    providers: Vec<Provider>,
}

/// Push-side writer: recompute THIS device's `providers.json` from the store,
/// key-stripped. Called by `sync::flow::push_usage` on every push — there is
/// no dirty flag; the write is byte-stable, so an unchanged store rewrites
/// identical bytes and `commit_and_push` stays a no-op. A provider whose
/// config cannot be parsed is skipped with a log — a provider whose secrets
/// can't be proven absent must not be published. A device that never had
/// providers gets no file at all (absent reads as empty); a leftover file is
/// cleared once the last provider is deleted.
pub fn write_own_providers(store: &Store, paths: &Paths, device_id: &str) -> AppResult<()> {
    let path = paths.providers_json_path(device_id);
    let mut providers = Vec::new();
    for p in store.list_providers()? {
        match p.redacted() {
            Ok(r) => providers.push(r),
            Err(e) => eprintln!("[cc-one] provider {} skipped from sync file: {e}", p.id),
        }
    }
    if providers.is_empty() && !path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let doc = SyncedProvidersDoc {
        v: SYNCED_PROVIDERS_DOC_VERSION,
        providers,
    };
    let json = serde_json::to_string_pretty(&doc)?;
    std::fs::write(&path, format!("{json}\n"))?;
    Ok(())
}

/// Read one device's provider file. Missing/unreadable/unparseable ⇒ empty —
/// a corrupt peer file must never abort a pull. A file whose schema `v` is
/// HIGHER than this binary's is skipped whole with a logged warning (the
/// version gate): merging it by `(app, id)` would silently mis-attribute
/// entries this binary does not understand.
fn read_device_providers(paths: &Paths, device_id: &str) -> Vec<Provider> {
    let path = paths.providers_json_path(device_id);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let doc = serde_json::from_str::<SyncedProvidersDoc>(&text).unwrap_or_default();
    if doc.v > SYNCED_PROVIDERS_DOC_VERSION {
        eprintln!(
            "[cc-one] provider file for device {device_id} has schema v{} \
             (this build reads ≤ v{SYNCED_PROVIDERS_DOC_VERSION}) — skipped; upgrade to see its providers",
            doc.v
        );
        return Vec::new();
    }
    doc.providers
}

/// Merge every device's providers by `(app, id)`: the newest `updated_at`
/// wins, ties → first seen (the sessions rule). Pure — no IO — so the dedup
/// rule is directly unit-testable. Output sorted by
/// `(sort_index, name, id, app)` for a deterministic, list-friendly order.
pub fn merge_providers_latest_wins(providers: impl IntoIterator<Item = Provider>) -> Vec<Provider> {
    let mut by_key: std::collections::BTreeMap<(String, String), Provider> =
        std::collections::BTreeMap::new();
    for p in providers {
        let key = (p.app.as_str().to_string(), p.id.clone());
        let take = by_key
            .get(&key)
            .map(|e| e.updated_at < p.updated_at)
            .unwrap_or(true);
        if take {
            by_key.insert(key, p);
        }
    }
    let mut out: Vec<Provider> = by_key.into_values().collect();
    out.sort_by(|a, b| {
        a.sort_index
            .cmp(&b.sort_index)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.app.as_str().cmp(b.app.as_str()))
    });
    out
}

/// Read every PEER's provider file, merged by `(app, id)` (latest wins).
/// Self's own directory is skipped — self is local-authoritative (see the
/// module doc). Only valid device dirs are walked, so a stray folder never
/// shows up as a providers source.
pub fn read_all_peer_providers(paths: &Paths, self_device_id: &str) -> AppResult<Vec<Provider>> {
    let mut all = Vec::new();
    for dev in crate::devices::iter_data_device_ids(paths)? {
        if dev == self_device_id {
            continue;
        }
        all.extend(read_device_providers(paths, &dev));
    }
    Ok(merge_providers_latest_wins(all))
}

/// Re-apply a local row's secret env keys onto a peer's key-stripped version:
/// the pull-side key guard. The peer's structure wins, but this device's
/// locally-filled keys are merged back in — an import can update structure
/// but never leave the local key empty by overwriting it with the peer's
/// keyless copy. The same guard covers `meta.templateValues` (the frontend's
/// record of filled `${VAR}` template variables — the Bedrock presets route
/// AK/SK through those, and the sync write strips them, so the peer's copy is
/// keyless there too).
///
/// Both configs must parse (a blank/unparseable side ⇒ `Err`, and the caller
/// skips that import): a peer version we can't merge into is not imported
/// over a local row, and a local row whose key location we can't see is never
/// replaced. A local row without an `env` object (missing, or not an object)
/// contributes no keys instead of erroring: secret keys live only inside an
/// `env` object, so a missing one means there is nothing to preserve — and
/// refusing the import would freeze this row forever behind any peer edit,
/// so its structure would never receive the peer's later updates. The same
/// tolerance applies to `meta.templateValues` — a missing or non-object
/// template-values record contributes nothing and never blocks the import.
fn merge_local_keys(local: &Provider, peer: &Provider) -> AppResult<Provider> {
    let parse = |raw: &str, what: &str| -> AppResult<serde_json::Value> {
        serde_json::from_str(raw.trim())
            .map_err(|e| AppError::Config(format!("{what} settingsConfig is not valid JSON: {e}")))
    };
    let mut config = parse(&peer.settings_config, "peer provider")?;
    let config_obj = config.as_object_mut().ok_or_else(|| {
        AppError::Config("peer provider settingsConfig is not a JSON object".into())
    })?;
    if let Some(env) = config_obj.get_mut("env").and_then(|e| e.as_object_mut()) {
        let local_config = parse(&local.settings_config, "local provider")?;
        // 本机行没有 env 对象（缺失或非对象）→ 没有可回填的 key：密钥只住在
        // env 对象里，缺了就是没有可丢的 key——贡献零 key，让 peer 结构照常
        // 导入，不阻塞该行接收后续结构更新。
        if let Some(local_env) = local_config.get("env").and_then(|e| e.as_object()) {
            for key in SECRET_ENV_KEYS {
                if let Some(v) = local_env.get(*key) {
                    env.insert((*key).to_string(), v.clone());
                }
            }
        }
    }
    // 同一 guard 覆盖 meta.templateValues。peer meta 解析失败 → 无法证明其
    // 无密钥，拒绝导入（宁可不导）；本机 meta 解析失败同理——本机 key 位置
    // 不可见就不替换。
    let parse_meta = |raw: &str, what: &str| -> AppResult<serde_json::Value> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(serde_json::json!({}));
        }
        serde_json::from_str(trimmed)
            .map_err(|e| AppError::Config(format!("{what} meta is not valid JSON: {e}")))
    };
    let mut peer_meta = parse_meta(&peer.meta, "peer provider")?;
    let peer_meta_obj = peer_meta
        .as_object_mut()
        .ok_or_else(|| AppError::Config("peer provider meta is not a JSON object".into()))?;
    let mut meta_changed = false;
    let local_meta = parse_meta(&local.meta, "local provider")?;
    if let (Some(peer_values), Some(local_values)) = (
        peer_meta_obj
            .get_mut("templateValues")
            .and_then(|tv| tv.as_object_mut()),
        local_meta
            .get("templateValues")
            .and_then(|tv| tv.as_object()),
    ) {
        for key in SECRET_ENV_KEYS {
            if let Some(v) = local_values.get(*key) {
                peer_values.insert((*key).to_string(), v.clone());
                meta_changed = true;
            }
        }
        if meta_changed {
            if peer_values.is_empty() {
                peer_meta_obj.remove("templateValues");
            }
            let mut merged = peer.clone();
            merged.settings_config = serde_json::to_string_pretty(&config)?;
            merged.meta = serde_json::to_string_pretty(&peer_meta)?;
            return Ok(merged);
        }
    }
    let mut merged = peer.clone();
    merged.settings_config = serde_json::to_string_pretty(&config)?;
    Ok(merged)
}

/// Pull-side import of peers' provider structure into the local store,
/// latest-wins by `updated_at` (per `(app, id)` entry) and always keeping
/// this device's keys:
/// - no local row ⇒ insert the peer version as-is (keys absent — the user
///   fills them locally);
/// - local row at least as fresh as the peer (tie counts) ⇒ skip — a pull
///   never overwrites a newer local row;
/// - peer strictly newer ⇒ import its structure with the local row's secret
///   keys merged back in ([`merge_local_keys`]).
///
/// A single bad provider logs and is skipped — it must not abort the whole
/// pull.
pub fn import_peer_providers(store: &Store, paths: &Paths, self_device_id: &str) -> AppResult<()> {
    for peer in read_all_peer_providers(paths, self_device_id)? {
        let peer_id = peer.id.clone();
        let local = store.get_provider(peer.app, &peer_id)?;
        let import = match &local {
            None => Ok(Some(peer)),
            Some(l) if l.updated_at >= peer.updated_at => Ok(None),
            Some(l) => merge_local_keys(l, &peer).map(Some),
        };
        match import {
            Ok(Some(p)) => store.import_provider(&p)?,
            Ok(None) => {}
            Err(e) => eprintln!("[cc-one] provider {peer_id} skipped from import: {e}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::db::testutil::mem;
    use crate::model::{App, ProviderCategory};
    use std::path::PathBuf;

    fn provider(id: &str, name: &str, settings_config: &str, updated_at: &str) -> Provider {
        provider_with_meta(id, name, settings_config, r#"{}"#, updated_at)
    }

    fn provider_with_meta(
        id: &str,
        name: &str,
        settings_config: &str,
        meta: &str,
        updated_at: &str,
    ) -> Provider {
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
            updated_at: updated_at.into(),
        }
    }

    /// Hand-write one device's providers.json (the read side's input).
    fn write_file(paths: &Paths, device_id: &str, providers: &[Provider]) {
        let path = paths.providers_json_path(device_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let doc = SyncedProvidersDoc {
            v: SYNCED_PROVIDERS_DOC_VERSION,
            providers: providers.to_vec(),
        };
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    }

    fn keyed_provider(
        id: &str,
        name: &str,
        token: &str,
        endpoint: &str,
        updated_at: &str,
    ) -> Provider {
        provider(
            id,
            name,
            &format!(
                r#"{{"env":{{"ANTHROPIC_BASE_URL":"{endpoint}","ANTHROPIC_AUTH_TOKEN":"{token}"}}}}"#
            ),
            updated_at,
        )
    }

    #[test]
    fn write_own_providers_writes_redacted_file() {
        let s = mem();
        let p = s
            .save_provider(keyed_provider(
                "aaaaaaaa",
                "Kimi",
                "sk-secret-token",
                "https://api.kimi.com",
                "2026-08-01T00:00:00.000Z",
            ))
            .unwrap();
        s.save_provider(provider(
            "bbbbbbbb",
            "Plain",
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://plain.dev"}}"#,
            "2026-08-02T00:00:00.000Z",
        ))
        .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        write_own_providers(&s, &paths, "aabbccddeeff").unwrap();

        let text = std::fs::read_to_string(paths.providers_json_path("aabbccddeeff")).unwrap();
        // The key value and every secret key name are absent from the file.
        assert!(!text.contains("sk-secret-token"));
        for key in SECRET_ENV_KEYS {
            assert!(!text.contains(key), "{key} must not appear in the file");
        }
        // Structure (name, endpoint) survived.
        let doc: SyncedProvidersDoc = serde_json::from_str(&text).unwrap();
        assert_eq!(doc.providers.len(), 2);
        let kimi = doc.providers.iter().find(|x| x.id == p.id).unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&kimi.settings_config).unwrap();
        assert_eq!(cfg["env"]["ANTHROPIC_BASE_URL"], "https://api.kimi.com");
        assert!(cfg["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn write_own_providers_strips_secret_template_values_from_meta() {
        let s = mem();
        s.save_provider(provider_with_meta(
            "aaaaaaaa",
            "Bedrock",
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://bedrock-runtime.${AWS_REGION}.amazonaws.com","AWS_REGION":"us-east-1"}}"#,
            r#"{"templateValues":{"AWS_REGION":"us-east-1","AWS_ACCESS_KEY_ID":"AKIA123","AWS_SECRET_ACCESS_KEY":"top-secret"}}"#,
            "2026-08-01T00:00:00.000Z",
        ))
        .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        write_own_providers(&s, &paths, "aabbccddeeff").unwrap();

        let text = std::fs::read_to_string(paths.providers_json_path("aabbccddeeff")).unwrap();
        // The secret key NAMES never appear in the file — from env or meta.
        for key in SECRET_ENV_KEYS {
            assert!(!text.contains(key), "{key} must not appear in the file");
        }
        assert!(!text.contains("top-secret"));
        assert!(!text.contains("AKIA123"));
        // Non-secret template values survive.
        let doc: SyncedProvidersDoc = serde_json::from_str(&text).unwrap();
        let meta: serde_json::Value = serde_json::from_str(&doc.providers[0].meta).unwrap();
        assert_eq!(meta["templateValues"]["AWS_REGION"], "us-east-1");
    }

    #[test]
    fn write_own_providers_skips_provider_with_unparseable_config() {
        let s = mem();
        s.save_provider(provider(
            "cccccccc",
            "Broken",
            "{oops",
            "2026-08-01T00:00:00.000Z",
        ))
        .unwrap();
        s.save_provider(provider(
            "dddddddd",
            "Fine",
            r#"{"env":{}}"#,
            "2026-08-01T00:00:00.000Z",
        ))
        .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Must not error — the broken provider is skipped, the good one lands.
        write_own_providers(&s, &paths, "aabbccddeeff").unwrap();
        let doc: SyncedProvidersDoc = serde_json::from_str(
            &std::fs::read_to_string(paths.providers_json_path("aabbccddeeff")).unwrap(),
        )
        .unwrap();
        assert_eq!(doc.providers.len(), 1);
        assert_eq!(doc.providers[0].id, "dddddddd");
    }

    #[test]
    fn write_own_providers_writes_nothing_without_providers_and_clears_leftovers() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Never had providers ⇒ no file at all.
        write_own_providers(&s, &paths, "aabbccddeeff").unwrap();
        assert!(!paths.providers_json_path("aabbccddeeff").exists());

        // Had a provider, then deleted it ⇒ the leftover file is cleared.
        let p = s
            .save_provider(provider(
                "eeeeeeee",
                "Gone",
                r#"{"env":{}}"#,
                "2026-08-01T00:00:00.000Z",
            ))
            .unwrap();
        write_own_providers(&s, &paths, "aabbccddeeff").unwrap();
        assert!(paths.providers_json_path("aabbccddeeff").exists());
        s.delete_provider(App::Claude, &p.id).unwrap();
        write_own_providers(&s, &paths, "aabbccddeeff").unwrap();
        let doc: SyncedProvidersDoc = serde_json::from_str(
            &std::fs::read_to_string(paths.providers_json_path("aabbccddeeff")).unwrap(),
        )
        .unwrap();
        assert!(doc.providers.is_empty(), "leftover file cleared");
    }

    #[test]
    fn write_own_providers_is_byte_stable() {
        let s = mem();
        s.save_provider(keyed_provider(
            "aaaaaaaa",
            "Kimi",
            "sk-secret",
            "https://api.kimi.com",
            "2026-08-01T00:00:00.000Z",
        ))
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        write_own_providers(&s, &paths, "aabbccddeeff").unwrap();
        let first = std::fs::read_to_string(paths.providers_json_path("aabbccddeeff")).unwrap();
        write_own_providers(&s, &paths, "aabbccddeeff").unwrap();
        let second = std::fs::read_to_string(paths.providers_json_path("aabbccddeeff")).unwrap();
        assert_eq!(
            first, second,
            "unchanged store ⇒ identical bytes (no git churn)"
        );
    }

    #[test]
    fn merge_providers_latest_wins_dedupes_by_id_newest_wins_ties_first_seen() {
        let old = provider("p1", "Old", r#"{"env":{}}"#, "2026-08-01T00:00:00.000Z");
        let new = provider("p1", "New", r#"{"env":{}}"#, "2026-08-02T00:00:00.000Z");
        let other = provider("p2", "Other", r#"{"env":{}}"#, "2026-08-01T00:00:00.000Z");
        let merged = merge_providers_latest_wins([old.clone(), new.clone(), other.clone()]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "p1");
        assert_eq!(merged[0].name, "New", "newest updated_at wins");
        // Ties → first seen keeps the first copy.
        let tie = merge_providers_latest_wins([old.clone(), old.clone()]);
        assert_eq!(tie.len(), 1);
        assert_eq!(tie[0].name, "Old");
        assert!(merge_providers_latest_wins(std::iter::empty::<Provider>()).is_empty());
    }

    #[test]
    fn read_all_peer_providers_merges_by_id_latest_wins_and_skips_self() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Self's dir carries a provider that would win — it must be skipped.
        write_file(
            &paths,
            "aabbccddeeff",
            &[provider(
                "p-self",
                "Self",
                r#"{"env":{}}"#,
                "2026-08-09T00:00:00.000Z",
            )],
        );
        // Peer B: p1 old + p2.
        write_file(
            &paths,
            "bbccddee0011",
            &[
                provider("p1", "Old", r#"{"env":{}}"#, "2026-08-01T00:00:00.000Z"),
                provider("p2", "Other", r#"{"env":{}}"#, "2026-08-01T00:00:00.000Z"),
            ],
        );
        // Peer C: p1 newer.
        write_file(
            &paths,
            "001122334455",
            &[provider(
                "p1",
                "New",
                r#"{"env":{}}"#,
                "2026-08-02T00:00:00.000Z",
            )],
        );

        let all = read_all_peer_providers(&paths, "aabbccddeeff").unwrap();
        let by_id: std::collections::HashMap<String, String> =
            all.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
        assert_eq!(
            by_id.get("p1").map(String::as_str),
            Some("New"),
            "latest wins"
        );
        assert!(by_id.contains_key("p2"));
        assert!(
            !by_id.contains_key("p-self"),
            "self's own file is skipped on read"
        );
    }

    #[test]
    fn read_all_peer_providers_ignores_non_device_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        write_file(
            &paths,
            "not-a-device",
            &[provider(
                "p1",
                "Stray",
                r#"{"env":{}}"#,
                "2026-08-01T00:00:00.000Z",
            )],
        );
        let all = read_all_peer_providers(&paths, "aabbccddeeff").unwrap();
        assert!(all.is_empty(), "stray folder is not a providers source");
    }

    #[test]
    fn read_all_peer_providers_tolerates_broken_files() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let broken = paths.providers_json_path("bbccddee0011");
        std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
        std::fs::write(&broken, "{not json").unwrap();
        write_file(
            &paths,
            "001122334455",
            &[provider(
                "p1",
                "Fine",
                r#"{"env":{}}"#,
                "2026-08-01T00:00:00.000Z",
            )],
        );
        let all = read_all_peer_providers(&paths, "aabbccddeeff").unwrap();
        assert_eq!(all.len(), 1, "broken file skipped, healthy one read");
        assert_eq!(all[0].id, "p1");
    }

    #[test]
    fn import_peer_providers_merges_local_keys_and_never_overwrites_them() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Local row: old endpoint + a filled key, author timestamp t1.
        // Seeded via import_provider so the timestamp is the explicit t1
        // (save_provider would stamp "now", which is newer than the peer).
        let local = keyed_provider(
            "aaaaaaaa",
            "Kimi",
            "sk-local-key",
            "https://old.dev",
            "2026-08-01T00:00:00.000Z",
        );
        s.import_provider(&local).unwrap();
        // Peer file: NEWER structure (new endpoint), key-stripped.
        write_file(
            &paths,
            "bbccddee0011",
            &[
                provider(
                    "aaaaaaaa",
                    "Kimi",
                    r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com"}}"#,
                    "2026-08-02T00:00:00.000Z",
                ),
                provider(
                    "bbbbbbbb",
                    "Brand New",
                    r#"{"env":{"ANTHROPIC_BASE_URL":"https://new.dev"}}"#,
                    "2026-08-02T00:00:00.000Z",
                ),
            ],
        );

        import_peer_providers(&s, &paths, "aabbccddeeff").unwrap();

        let kimi = s.get_provider(App::Claude, "aaaaaaaa").unwrap().unwrap();
        assert_eq!(
            kimi.updated_at, "2026-08-02T00:00:00.000Z",
            "peer freshness"
        );
        let cfg: serde_json::Value = serde_json::from_str(&kimi.settings_config).unwrap();
        assert_eq!(
            cfg["env"]["ANTHROPIC_BASE_URL"], "https://api.kimi.com",
            "peer structure imported"
        );
        assert_eq!(
            cfg["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-local-key",
            "local key merged back, never overwritten"
        );
        // A provider the peer has and we don't is inserted as-is (keys absent).
        let fresh = s.get_provider(App::Claude, "bbbbbbbb").unwrap().unwrap();
        assert_eq!(fresh.name, "Brand New");
        assert_eq!(fresh.updated_at, "2026-08-02T00:00:00.000Z");
        assert!(!fresh.settings_config.contains("ANTHROPIC_AUTH_TOKEN"));
        // The key-filled row's sort_index stayed local (import preserves it).
        assert_eq!(kimi.sort_index, local.sort_index);
    }

    #[test]
    fn import_peer_providers_merges_local_template_value_keys_back() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Local row: old endpoint + filled AK/SK as template values in meta.
        let local = provider_with_meta(
            "aaaaaaaa",
            "Bedrock",
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://bedrock-runtime.${AWS_REGION}.amazonaws.com","AWS_REGION":"us-east-1"}}"#,
            r#"{"templateValues":{"AWS_REGION":"us-east-1","AWS_ACCESS_KEY_ID":"AKIA123","AWS_SECRET_ACCESS_KEY":"top-secret"}}"#,
            "2026-08-01T00:00:00.000Z",
        );
        s.import_provider(&local).unwrap();
        // Peer file: NEWER structure (new region), meta.templateValues keyless
        // (the sync write strips secrets from it).
        write_file(
            &paths,
            "bbccddee0011",
            &[provider_with_meta(
                "aaaaaaaa",
                "Bedrock",
                r#"{"env":{"ANTHROPIC_BASE_URL":"https://bedrock-runtime.${AWS_REGION}.amazonaws.com","AWS_REGION":"eu-west-1"}}"#,
                r#"{"templateValues":{"AWS_REGION":"eu-west-1"}}"#,
                "2026-08-02T00:00:00.000Z",
            )],
        );

        import_peer_providers(&s, &paths, "aabbccddeeff").unwrap();

        let row = s.get_provider(App::Claude, "aaaaaaaa").unwrap().unwrap();
        // Peer structure imported (new region)…
        let cfg: serde_json::Value = serde_json::from_str(&row.settings_config).unwrap();
        assert_eq!(cfg["env"]["AWS_REGION"], "eu-west-1");
        // …but the local AK/SK template values merged back into meta.
        let meta: serde_json::Value = serde_json::from_str(&row.meta).unwrap();
        assert_eq!(meta["templateValues"]["AWS_REGION"], "eu-west-1");
        assert_eq!(meta["templateValues"]["AWS_ACCESS_KEY_ID"], "AKIA123");
        assert_eq!(
            meta["templateValues"]["AWS_SECRET_ACCESS_KEY"], "top-secret",
            "local secret template values never overwritten"
        );
    }

    #[test]
    fn import_peer_providers_skips_stale_peer_versions() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Local row is NEWER (t2) — the peer's older structure must not land.
        s.import_provider(&keyed_provider(
            "aaaaaaaa",
            "Kimi",
            "sk-local-key",
            "https://new.dev",
            "2026-08-02T00:00:00.000Z",
        ))
        .unwrap();
        write_file(
            &paths,
            "bbccddee0011",
            &[provider(
                "aaaaaaaa",
                "Kimi",
                r#"{"env":{"ANTHROPIC_BASE_URL":"https://old.dev"}}"#,
                "2026-08-01T00:00:00.000Z",
            )],
        );

        import_peer_providers(&s, &paths, "aabbccddeeff").unwrap();

        let row = s.get_provider(App::Claude, "aaaaaaaa").unwrap().unwrap();
        assert_eq!(
            row.updated_at, "2026-08-02T00:00:00.000Z",
            "local freshness kept"
        );
        assert!(
            row.settings_config.contains("https://new.dev"),
            "stale peer structure not imported"
        );
        assert!(row.settings_config.contains("sk-local-key"));
    }

    #[test]
    fn import_peer_providers_keeps_local_row_when_peer_config_unparseable() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        s.import_provider(&keyed_provider(
            "aaaaaaaa",
            "Kimi",
            "sk-local-key",
            "https://new.dev",
            "2026-08-01T00:00:00.000Z",
        ))
        .unwrap();
        // Peer claims a NEWER structure but its config can't be merged into —
        // importing it would risk dropping the local key, so it is skipped.
        write_file(
            &paths,
            "bbccddee0011",
            &[provider(
                "aaaaaaaa",
                "Kimi",
                "{oops",
                "2026-08-03T00:00:00.000Z",
            )],
        );

        import_peer_providers(&s, &paths, "aabbccddeeff").unwrap();

        let row = s.get_provider(App::Claude, "aaaaaaaa").unwrap().unwrap();
        assert_eq!(row.updated_at, "2026-08-01T00:00:00.000Z");
        assert!(
            row.settings_config.contains("sk-local-key"),
            "local row untouched"
        );
    }

    /// 本机行没有 env 块（或 env 非对象）不阻塞导入：密钥只住在 env 对象里，
    /// 缺了就没有可丢的 key——贡献零 key，peer 的新结构照常落地，该行不会
    /// 永远收不到 peer 的结构更新。
    #[test]
    fn import_peer_providers_local_without_env_still_imports_peer_structure() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Local row has NO env block (older structure, t1).
        s.import_provider(&provider(
            "aaaaaaaa",
            "Kimi",
            r#"{"includeCoAuthoredBy":false}"#,
            "2026-08-01T00:00:00.000Z",
        ))
        .unwrap();
        // Peer: NEWER structure with an env block.
        write_file(
            &paths,
            "bbccddee0011",
            &[provider(
                "aaaaaaaa",
                "Kimi",
                r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com"}}"#,
                "2026-08-02T00:00:00.000Z",
            )],
        );

        import_peer_providers(&s, &paths, "aabbccddeeff").unwrap();

        let row = s.get_provider(App::Claude, "aaaaaaaa").unwrap().unwrap();
        assert_eq!(
            row.updated_at, "2026-08-02T00:00:00.000Z",
            "peer structure imported despite local row having no env"
        );
        let cfg: serde_json::Value = serde_json::from_str(&row.settings_config).unwrap();
        assert_eq!(
            cfg["env"]["ANTHROPIC_BASE_URL"], "https://api.kimi.com",
            "peer env imported, zero keys merged"
        );
        assert!(!row.settings_config.contains("ANTHROPIC_AUTH_TOKEN"));
    }

    #[test]
    fn import_peer_providers_local_garbage_env_contributes_zero_keys() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Local env is not an object (garbage) — same zero-keys rule as missing.
        s.import_provider(&provider(
            "aaaaaaaa",
            "Kimi",
            r#"{"env":"garbage"}"#,
            "2026-08-01T00:00:00.000Z",
        ))
        .unwrap();
        write_file(
            &paths,
            "bbccddee0011",
            &[provider(
                "aaaaaaaa",
                "Kimi",
                r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com"}}"#,
                "2026-08-02T00:00:00.000Z",
            )],
        );

        import_peer_providers(&s, &paths, "aabbccddeeff").unwrap();

        let row = s.get_provider(App::Claude, "aaaaaaaa").unwrap().unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&row.settings_config).unwrap();
        assert_eq!(
            cfg["env"]["ANTHROPIC_BASE_URL"], "https://api.kimi.com",
            "peer structure imported"
        );
        assert!(
            cfg["env"].get("ANTHROPIC_AUTH_TOKEN").is_none(),
            "本地 env 非对象 → 没有 key 可回填"
        );
    }

    #[test]
    fn import_peer_providers_skips_own_file_and_missing_dirs() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Only SELF has a providers.json — nothing must be imported.
        write_file(
            &paths,
            "aabbccddeeff",
            &[provider(
                "p1",
                "Self",
                r#"{"env":{}}"#,
                "2026-08-01T00:00:00.000Z",
            )],
        );
        import_peer_providers(&s, &paths, "aabbccddeeff").unwrap();
        assert!(s.get_provider(App::Claude, "p1").unwrap().is_none());
        // No device dirs at all ⇒ no-op.
        let empty_tmp = tempfile::tempdir().unwrap();
        import_peer_providers(&s, &Paths::resolve(empty_tmp.path()), "aabbccddeeff").unwrap();
    }

    #[test]
    fn providers_json_path_is_under_device_data_dir() {
        let paths = Paths::resolve(std::path::Path::new("/root"));
        assert_eq!(
            paths.providers_json_path("aabbccddeeff"),
            PathBuf::from("/root/repo/data/aabbccddeeff/providers.json")
        );
    }

    // ---- 应用维度：去重键 (app, id)、版本门、旧文件读为 claude ----

    /// A provider pinned to a specific app pool.
    fn provider_for(app: App, id: &str, name: &str, updated_at: &str) -> Provider {
        Provider {
            app,
            ..provider(id, name, r#"{"env":{}}"#, updated_at)
        }
    }

    /// Hand-write a raw providers.json body (version / line shape under test).
    fn write_raw(paths: &Paths, device_id: &str, body: &str) {
        let path = paths.providers_json_path(device_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
    }

    /// The same id in two app pools stays two entries: dedup key is (app, id),
    /// not id alone — the Claude pool's entry never wins over the Codex pool's.
    #[test]
    fn merge_providers_latest_wins_dedupes_by_app_and_id() {
        let claude_new = provider_for(
            App::Claude,
            "p1",
            "Claude-Latest",
            "2026-08-02T00:00:00.000Z",
        );
        let codex_new = provider_for(App::Codex, "p1", "Codex-Latest", "2026-08-02T00:00:00.000Z");
        let claude_old = provider_for(App::Claude, "p1", "Claude-Old", "2026-08-01T00:00:00.000Z");
        let merged = merge_providers_latest_wins([
            claude_old.clone(),
            claude_new.clone(),
            codex_new.clone(),
        ]);
        assert_eq!(merged.len(), 2, "same id across apps is two entries");
        let by_key: std::collections::HashMap<(String, String), String> = merged
            .iter()
            .map(|p| ((p.app.as_str().to_string(), p.id.clone()), p.name.clone()))
            .collect();
        assert_eq!(
            by_key
                .get(&("claude".into(), "p1".into()))
                .map(String::as_str),
            Some("Claude-Latest")
        );
        assert_eq!(
            by_key
                .get(&("codex".into(), "p1".into()))
                .map(String::as_str),
            Some("Codex-Latest")
        );
        // 同 (app, id) 的平局 → 先见先得，与原来按 id 的规则一致。
        let tie = merge_providers_latest_wins([claude_old.clone(), claude_old]);
        assert_eq!(tie.len(), 1);
    }

    /// 旧格式文件（无 v 字段、行无 app 字段）整体读为 claude——存量数据
    /// 全部归入 Claude 池，原样可用。
    #[test]
    fn old_format_file_without_app_reads_every_line_as_claude() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // 手工写应用维度之前的文件形状：{"providers": [...]}，行没有 app。
        write_raw(
            &paths,
            "bbccddee0011",
            r#"{"providers":[{"id":"p1","name":"Kimi","websiteUrl":"https://x.dev","category":"custom","icon":"","iconColor":"","sortIndex":0,"notes":"","settingsConfig":"{}","meta":"{}","updatedAt":"2026-08-01T00:00:00.000Z"}]}"#,
        );
        let all = read_all_peer_providers(&paths, "aabbccddeeff").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].app, App::Claude, "old line defaults to claude");
        assert_eq!(all[0].name, "Kimi");
    }

    /// 版本门：文件 `v` 高于本版本 → 整个文件跳过（老版本不能按 (app, id)
    /// 合并它无法归属的行）；`v` 等于或低于当前版本正常读。
    #[test]
    fn higher_schema_version_file_is_skipped_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        write_raw(
            &paths,
            "bbccddee0011",
            r#"{"v":99,"providers":[{"id":"p1","name":"Future","websiteUrl":"https://x.dev","category":"custom","app":"gemini","icon":"","iconColor":"","sortIndex":0,"notes":"","settingsConfig":"{}","meta":"{}","updatedAt":"2026-08-01T00:00:00.000Z"}]}"#,
        );
        assert!(
            read_all_peer_providers(&paths, "aabbccddeeff")
                .unwrap()
                .is_empty(),
            "newer-schema file must be skipped, not mis-merged"
        );
    }

    /// 写出文件带 schema 版本号 v（与 sessions 快照同款版本门）。
    #[test]
    fn write_own_providers_writes_schema_version() {
        let s = mem();
        s.save_provider(provider(
            "aaaaaaaa",
            "Kimi",
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com"}}"#,
            "2026-08-01T00:00:00.000Z",
        ))
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        write_own_providers(&s, &paths, "aabbccddeeff").unwrap();
        let text = std::fs::read_to_string(paths.providers_json_path("aabbccddeeff")).unwrap();
        let doc: SyncedProvidersDoc = serde_json::from_str(&text).unwrap();
        assert_eq!(doc.v, SYNCED_PROVIDERS_DOC_VERSION);
        assert_eq!(doc.providers[0].app, App::Claude, "每行带 app");
    }
}
