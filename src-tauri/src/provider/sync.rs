//! Provider structure sync — the per-device `providers.json`, following the
//! Synced Group / device-registry pattern: each device writes ONLY its own
//! `repo/data/<deviceId>/providers.json` (a JSON object with one `providers`
//! array and a schema version `v`); reading merges every device's file by
//! `(app, id)`, latest `updated_at` wins (ties → first seen). This is the
//! structure half of provider sync.
//!
//! The per-device-doc MECHANISM — tolerant read, the schema-`v` gate, the
//! byte-stable write and the latest-wins merge — lives in
//! [`crate::synced_doc`]. This module declares this domain's wire doc
//! ([`SyncedProvidersDoc`], whose field names and order serialize into the
//! file) and its merge key `(app, id)` + display sort; reads skip self
//! (`Some(self_id)` to [`crate::synced_doc::read_all_devices`]) because the
//! store is local-authoritative for this device's own rows.
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
//! [`Provider::redacted`] on write — the five key locations (settingsConfig
//! `env` / `auth`, opencode `options.apiKey` / `options.headers` auth-header
//! whitelist, `meta.templateValues`) are stripped per [`crate::provider::keys`],
//! the single source of truth for where secrets live (`AWS_REGION` — a region
//! code or a `${VAR}` placeholder — is not a credential and stays). Each
//! device's keys live only in its local DB; the active provider is local-only
//! too (config.json) and never touches this file.
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
//! local row's secret values — every location in [`crate::provider::keys`] —
//! are merged back in. Since `save_provider` advances `updated_at` only on
//! structural change, the comparison is a true structural freshness check —
//! a key fill on one device can never mask a peer's later edit. `sort_index`
//! (display order) stays a local preference: `import_provider` keeps the
//! local row's value, so pulls never shuffle the user's order.

use crate::config::Paths;
use crate::db::Store;
use crate::error::AppResult;
use crate::model::Provider;
use crate::synced_doc;

/// The providers.json schema version this binary reads (sessions-snapshot
/// style `v` gate). Files with a HIGHER `v` are skipped whole on read — this
/// binary cannot attribute their app fields, so merging them by id could
/// mis-attribute entries; their providers simply arrive after an upgrade.
///
/// v2（2026-08-11）：加 `App::Grok`。旧二进制（无 Grok 变体）读到 `app:"grok"`
/// 会经 `from_db_str` fallback 成 claude，故版本门让旧版本跳过 v2 文件，避免
/// grok 供应商被错归属到 claude 池。
///
/// v3（2026-08-11）：加 `App::OpenCode`（附加模式）+ `Provider.meta` 的
/// `live_managed` 字段。旧二进制读到 `app:"opencode"` 会 fallback 成 claude，
/// 且 `live_managed` 旧版本不认——版本门让旧版本跳过 v3 文件。
pub const SYNCED_PROVIDERS_DOC_VERSION: u32 = 3;

/// One device's provider-file wrapper: a stable JSON object with one
/// `providers` array + schema version `v`. This struct is the domain's WIRE
/// declaration — field names and order serialize into the file. Files without
/// `v` (pre-version format) read as 0 — old format, still attributable (lines
/// default to claude). Missing file ⇒ empty doc (via
/// [`synced_doc::read_json_doc`]).
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
    synced_doc::write_stable(
        &path,
        &SyncedProvidersDoc {
            v: SYNCED_PROVIDERS_DOC_VERSION,
            providers,
        },
    )
}

/// Read one device's provider file. Missing/unreadable/unparseable ⇒ empty —
/// a corrupt peer file must never abort a pull. A file whose schema `v` is
/// HIGHER than this binary's is skipped whole with a logged warning (the
/// version gate, [`synced_doc::schema_ahead_of_build`]): merging it by
/// `(app, id)` would silently mis-attribute entries this binary does not
/// understand.
fn read_device_providers(paths: &Paths, device_id: &str) -> Vec<Provider> {
    let doc: SyncedProvidersDoc =
        synced_doc::read_json_doc(&paths.providers_json_path(device_id)).unwrap_or_default();
    if synced_doc::schema_ahead_of_build(
        doc.v,
        SYNCED_PROVIDERS_DOC_VERSION,
        &format!("provider file for device {device_id}"),
    ) {
        return Vec::new();
    }
    doc.providers
}

/// Merge every device's providers by `(app, id)`: the newest `updated_at`
/// wins, ties → first seen (the sessions rule — the merge mechanism itself is
/// [`synced_doc::merge_latest_wins`]; the key and the sort below are this
/// domain's rules). Pure — no IO — so the dedup rule is directly
/// unit-testable. Output sorted by `(sort_index, name, id, app)` for a
/// deterministic, list-friendly order.
pub fn merge_providers_latest_wins(providers: impl IntoIterator<Item = Provider>) -> Vec<Provider> {
    let mut merged = synced_doc::merge_latest_wins(
        providers,
        |p: &Provider| (p.app.as_str().to_string(), p.id.clone()),
        |p: &Provider| p.updated_at.as_str(),
    );
    merged.sort_by(|a, b| {
        a.sort_index
            .cmp(&b.sort_index)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.app.as_str().cmp(b.app.as_str()))
    });
    merged
}

/// Read every PEER's provider file, merged by `(app, id)` (latest wins).
/// Self's own directory is skipped — self is local-authoritative (see the
/// module doc). Only valid device dirs are walked, so a stray folder never
/// shows up as a providers source.
pub fn read_all_peer_providers(paths: &Paths, self_device_id: &str) -> AppResult<Vec<Provider>> {
    let peers = synced_doc::read_all_devices(
        &crate::devices::iter_data_device_ids(paths)?,
        Some(self_device_id),
        |dev| read_device_providers(paths, dev),
    );
    Ok(merge_providers_latest_wins(peers))
}

/// Re-apply a local row's secret values onto a peer's key-stripped version:
/// the pull-side key guard. The peer's structure wins, but this device's
/// locally-filled credentials are merged back in — an import can update
/// structure but never leave a local credential empty by overwriting it with
/// the peer's keyless copy. The key locations and their strip/restore
/// semantics are defined in [`crate::provider::keys`] (single source of
/// truth); this function is a thin shell that restores both surfaces.
/// (It used to restore only `env` / `templateValues` — a pull that imported a
/// peer's codex / opencode structure silently zeroed the local `auth` key and
/// `options.apiKey` / whitelist headers.)
///
/// `Err` ⇒ the caller skips the import: a peer version we can't merge into
/// is not imported over a local row, and a local row whose key locations we
/// can't see is never replaced.
fn merge_local_keys(local: &Provider, peer: &Provider) -> AppResult<Provider> {
    let mut merged = peer.clone();
    merged.settings_config = crate::provider::keys::restore_settings_config(
        &local.settings_config,
        &peer.settings_config,
    )?;
    merged.meta = crate::provider::keys::restore_meta(&local.meta, &peer.meta)?;
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
///
/// Returns the number of entries actually written into the store (inserts +
/// structure updates; locally-fresher and unmergeable skips excluded) — the
/// providers domain's `imported` count for the sync report.
pub fn import_peer_providers(store: &Store, paths: &Paths, self_device_id: &str) -> AppResult<u32> {
    let mut imported = 0u32;
    for peer in read_all_peer_providers(paths, self_device_id)? {
        let peer_id = peer.id.clone();
        let local = store.get_provider(peer.app, &peer_id)?;
        let import = match &local {
            None => Ok(Some(peer)),
            Some(l) if l.updated_at >= peer.updated_at => Ok(None),
            Some(l) => merge_local_keys(l, &peer).map(Some),
        };
        match import {
            Ok(Some(p)) => {
                store.import_provider(&p)?;
                imported += 1;
            }
            Ok(None) => {}
            Err(e) => eprintln!("[cc-one] provider {peer_id} skipped from import: {e}"),
        }
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::db::testutil::mem;
    use crate::model::App;
    use crate::provider::keys::SECRET_ENV_KEYS;
    use crate::provider::testutil;
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
            updated_at: updated_at.into(),
            ..testutil::provider_with_meta(App::Claude, id, name, settings_config, meta)
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

    /// Golden wire bytes: the exact file `write_own_providers` lands, pinned
    /// line-for-line so the shared byte-stable write
    /// ([`crate::synced_doc::stable_bytes`]) can never drift the providers
    /// wire format (pretty JSON + exactly one trailing newline — an unchanged
    /// store must rewrite identical bytes for `commit_and_push` to no-op).
    #[test]
    fn write_own_providers_lands_pinned_wire_bytes() {
        let s = mem();
        // website_url 覆盖为非空值：golden bytes 锁的是 wire 格式（字段序 /
        // 转义形状），非空 URL 让格式断言覆盖到真实值的转义。
        s.import_provider(&Provider {
            website_url: "https://example.com".into(),
            ..provider(
                "aaaaaaaa",
                "Kimi",
                r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com"}}"#,
                "2026-08-01T00:00:00.000Z",
            )
        })
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        write_own_providers(&s, &paths, "aabbccddeeff").unwrap();

        let text = std::fs::read_to_string(paths.providers_json_path("aabbccddeeff")).unwrap();
        let expected = [
            "{",
            "  \"v\": 3,",
            "  \"providers\": [",
            "    {",
            "      \"id\": \"aaaaaaaa\",",
            "      \"name\": \"Kimi\",",
            "      \"websiteUrl\": \"https://example.com\",",
            "      \"category\": \"custom\",",
            "      \"app\": \"claude\",",
            "      \"icon\": \"\",",
            "      \"iconColor\": \"\",",
            "      \"sortIndex\": 0,",
            "      \"notes\": \"\",",
            "      \"settingsConfig\": \"{\\\"env\\\":{\\\"ANTHROPIC_BASE_URL\\\":\\\"https://api.kimi.com\\\"}}\",",
            "      \"meta\": \"{}\",",
            "      \"updatedAt\": \"2026-08-01T00:00:00.000Z\"",
            "    }",
            "  ]",
            "}",
        ];
        assert_eq!(
            text.lines().collect::<Vec<&str>>(),
            expected,
            "providers wire bytes drifted"
        );
        assert!(text.ends_with("}\n"), "exactly one trailing newline");
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

    /// codex 形状：pull 导入 peer 结构后，local 的 `auth.OPENAI_API_KEY`
    /// 必须回填——曾经只回填 env / templateValues 两处，codex 的 auth 被
    /// peer 的无密副本静默清空（真实缺陷的防回归）。
    #[test]
    fn import_peer_providers_merges_local_codex_auth_key_back() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let local = Provider {
            app: App::Codex,
            ..provider(
                "aaaaaaaa",
                "Codex-Provider",
                r#"{"auth":{"OPENAI_API_KEY":"sk-codex-local"},"config":"model = \"gpt-5.6\""}"#,
                "2026-08-01T00:00:00.000Z",
            )
        };
        s.import_provider(&local).unwrap();
        // Peer 文件：更新的结构（新模型），auth 无密（同步写剥掉了
        // OPENAI_API_KEY，auth 对象本身保留）。
        write_file(
            &paths,
            "bbccddee0011",
            &[Provider {
                app: App::Codex,
                ..provider(
                    "aaaaaaaa",
                    "Codex-Provider",
                    r#"{"auth":{},"config":"model = \"gpt-6\""}"#,
                    "2026-08-02T00:00:00.000Z",
                )
            }],
        );

        import_peer_providers(&s, &paths, "aabbccddeeff").unwrap();

        let row = s.get_provider(App::Codex, "aaaaaaaa").unwrap().unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&row.settings_config).unwrap();
        assert_eq!(
            cfg["config"],
            serde_json::json!("model = \"gpt-6\""),
            "peer 结构导入"
        );
        assert_eq!(
            cfg["auth"]["OPENAI_API_KEY"],
            serde_json::json!("sk-codex-local"),
            "local codex auth key merged back"
        );
    }

    /// opencode 形状：pull 导入 peer 结构后，local 的 `options.apiKey` 与
    /// `options.headers` 白名单条目必须回填；元数据头（非凭据）以 peer 为准。
    #[test]
    fn import_peer_providers_merges_local_opencode_keys_back() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let local = Provider {
            app: App::OpenCode,
            ..provider(
                "aaaaaaaa",
                "DeepSeek",
                r#"{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://old.dev","apiKey":"sk-local","headers":{"Authorization":"Bearer local-tok","Helicone-Auth":"meta"}}}"#,
                "2026-08-01T00:00:00.000Z",
            )
        };
        s.import_provider(&local).unwrap();
        // Peer 文件：更新的 baseURL；apiKey 被剥、Authorization 白名单头被剥，
        // 元数据头 Helicone-Auth 保留（同步写的真实投影形状）。
        write_file(
            &paths,
            "bbccddee0011",
            &[Provider {
                app: App::OpenCode,
                ..provider(
                    "aaaaaaaa",
                    "DeepSeek",
                    r#"{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://api.deepseek.com","headers":{"Helicone-Auth":"meta"}}}"#,
                    "2026-08-02T00:00:00.000Z",
                )
            }],
        );

        import_peer_providers(&s, &paths, "aabbccddeeff").unwrap();

        let row = s.get_provider(App::OpenCode, "aaaaaaaa").unwrap().unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&row.settings_config).unwrap();
        assert_eq!(
            cfg["options"]["baseURL"], "https://api.deepseek.com",
            "peer 结构导入"
        );
        assert_eq!(
            cfg["options"]["apiKey"], "sk-local",
            "local apiKey merged back"
        );
        assert_eq!(
            cfg["options"]["headers"]["Authorization"], "Bearer local-tok",
            "local whitelist header merged back"
        );
        assert_eq!(
            cfg["options"]["headers"]["Helicone-Auth"], "meta",
            "peer 的元数据头保留"
        );
    }

    /// peer 的 meta 在同步写时把全密钥的 templateValues 整体移除后，pull 仍
    /// 要把 local 的 AK/SK 建回来——restore 重建被 strip 移除的空记录。
    #[test]
    fn import_peer_providers_recreates_all_secret_template_values() {
        let s = mem();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let local = provider_with_meta(
            "aaaaaaaa",
            "Bedrock",
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://bedrock-runtime.${AWS_REGION}.amazonaws.com","AWS_REGION":"us-east-1"}}"#,
            r#"{"templateValues":{"AWS_ACCESS_KEY_ID":"AKIA123","AWS_SECRET_ACCESS_KEY":"top-secret"}}"#,
            "2026-08-01T00:00:00.000Z",
        );
        s.import_provider(&local).unwrap();
        // Peer 文件：更新的 region；templateValues 只有密钥 → 同步写整体移除
        // 了该记录，meta 变空。
        write_file(
            &paths,
            "bbccddee0011",
            &[provider_with_meta(
                "aaaaaaaa",
                "Bedrock",
                r#"{"env":{"ANTHROPIC_BASE_URL":"https://bedrock-runtime.${AWS_REGION}.amazonaws.com","AWS_REGION":"eu-west-1"}}"#,
                r#"{}"#,
                "2026-08-02T00:00:00.000Z",
            )],
        );

        import_peer_providers(&s, &paths, "aabbccddeeff").unwrap();

        let row = s.get_provider(App::Claude, "aaaaaaaa").unwrap().unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&row.settings_config).unwrap();
        assert_eq!(cfg["env"]["AWS_REGION"], "eu-west-1", "peer 结构导入");
        let meta: serde_json::Value = serde_json::from_str(&row.meta).unwrap();
        assert_eq!(
            meta["templateValues"]["AWS_ACCESS_KEY_ID"], "AKIA123",
            "全密钥 templateValues 被整体移除后重建，AK 回来"
        );
        assert_eq!(
            meta["templateValues"]["AWS_SECRET_ACCESS_KEY"], "top-secret",
            "SK 回来"
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

    /// v3 文件含 OpenCode provider（`app:"opencode"` + meta.liveKey/liveManaged）
    /// → 当前二进制（v3）正常读：provider 反序列化为 `App::OpenCode`（不 fallback
    /// Claude），meta 的 liveKey/liveManaged 原样保留。锁住 serde rename
    /// "opencode" + 附加模式 meta 字段在同步路径的 round-trip。
    #[test]
    fn v3_file_with_opencode_provider_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        write_raw(
            &paths,
            "bbccddee0011",
            r#"{"v":3,"providers":[{"id":"p1","name":"GLM","websiteUrl":"https://open.bigmodel.cn","category":"custom","app":"opencode","icon":"","iconColor":"","sortIndex":0,"notes":"","settingsConfig":"{\"npm\":\"@ai-sdk/openai-compatible\",\"options\":{\"baseURL\":\"https://open.bigmodel.cn\"}}","meta":"{\"liveKey\":\"glm\",\"liveManaged\":true}","updatedAt":"2026-08-11T00:00:00.000Z"}]}"#,
        );
        let all = read_all_peer_providers(&paths, "aabbccddeeff").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(
            all[0].app,
            App::OpenCode,
            "app 反序列化为 OpenCode，不 fallback Claude"
        );
        assert_eq!(all[0].name, "GLM");
        let meta: serde_json::Value = serde_json::from_str(&all[0].meta).unwrap();
        assert_eq!(meta["liveKey"], "glm");
        assert_eq!(meta["liveManaged"], true);
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
