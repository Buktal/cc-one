//! Provider (供应商) local-store CRUD. `save_provider` / `delete_provider` /
//! `get_provider` / `reorder_providers` are the command-layer surface;
//! `import_provider` is the pull-side sync import (author timestamp +
//! local sort_index preserved — see its doc).

use super::*;
use crate::model::{App, Provider, ProviderCategory};

impl super::Store {
    /// All providers, in user order (`sort_index`, name as the deterministic
    /// tie-break so rows created before reorder support keep a stable order —
    /// same rule as `list_local_groups`).
    pub fn list_providers(&self) -> AppResult<Vec<Provider>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, website_url, category, icon, icon_color, sort_index, \
             notes, settings_config, meta, updated_at FROM provider \
             ORDER BY sort_index, name",
        )?;
        let rows = stmt.query_map([], row_to_provider)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Insert-or-replace one provider. An empty `id` means "create": a fresh
    /// id is generated and the row is appended at the END of the current order
    /// (max `sort_index` + 1). A non-empty `id` edits the existing row and
    /// keeps its `sort_index` (saving must never move the user's order).
    /// Returns the persisted row — the caller gets the assigned id / position
    /// without a second read. `updated_at` is refreshed on save only when the
    /// syncable structure changed: a key-only edit (the one local-only field)
    /// must not advance the freshness timestamp, or a key fill on device B
    /// would make B's row look structurally newer than a peer's real edit and
    /// the next pull would silently reverse the peer's change. The structural
    /// comparison is `Provider::structure_equals` (key-stripped, pure) — the
    /// invariant lives in code, not prose.
    pub fn save_provider(&self, provider: Provider) -> AppResult<Provider> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (id, sort_index, updated_at) = if provider.id.is_empty() {
            let id = crate::model::generate_provider_id();
            let sort_index: i64 = conn.query_row(
                "SELECT COALESCE(MAX(sort_index), -1) + 1 FROM provider",
                [],
                |r| r.get(0),
            )?;
            (id, sort_index, crate::time::now_iso())
        } else {
            // Editing: keep the row's CURRENT sort_index — the value on disk,
            // never the caller's (saving must not move the user's order, and an
            // outdated caller must not corrupt it). A missing row (deleted since
            // the caller read it) falls back to appending at the end, so the
            // upsert "revives" it into a sane position instead of whatever the
            // caller carried.
            let existing: Option<Provider> = conn
                .query_row(
                    "SELECT id, name, website_url, category, icon, icon_color, \
                     sort_index, notes, settings_config, meta, updated_at \
                     FROM provider WHERE id = ?1",
                    params![provider.id],
                    row_to_provider,
                )
                .optional()?;
            let sort_index = match &existing {
                Some(e) => e.sort_index as i64,
                None => conn.query_row(
                    "SELECT COALESCE(MAX(sort_index), -1) + 1 FROM provider",
                    [],
                    |r| r.get(0),
                )?,
            };
            let updated_at = match &existing {
                Some(e) if e.structure_equals(&provider) => e.updated_at.clone(),
                _ => crate::time::now_iso(),
            };
            (provider.id.clone(), sort_index, updated_at)
        };
        conn.execute(
            "INSERT INTO provider \
             (id, name, website_url, category, icon, icon_color, sort_index, notes, \
              settings_config, meta, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name, website_url = excluded.website_url, \
               category = excluded.category, icon = excluded.icon, \
               icon_color = excluded.icon_color, notes = excluded.notes, \
               settings_config = excluded.settings_config, meta = excluded.meta, \
               updated_at = excluded.updated_at",
            params![
                id,
                provider.name,
                provider.website_url,
                provider.category.as_str(),
                provider.icon,
                provider.icon_color,
                sort_index,
                provider.notes,
                provider.settings_config,
                provider.meta,
                updated_at
            ],
        )?;
        Ok(Provider {
            id,
            sort_index: sort_index as u32,
            updated_at,
            ..provider
        })
    }

    /// Pull-side import of a peer's provider (synced): upsert preserving the
    /// AUTHOR's `updated_at` — sync freshness is the author's, not this
    /// device's import time — and the LOCAL row's `sort_index` (display order
    /// stays a local preference; a peer's file never shuffles it). A missing
    /// row appends at the end like `save_provider`. Never refreshes
    /// `updated_at` (unlike `save_provider`): an import is not an edit. The
    /// caller (`provider::sync`) owns the latest-wins decision and the
    /// key-merge — this method only lands the row.
    pub fn import_provider(&self, provider: &Provider) -> AppResult<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let sort_index: i64 = match conn
            .query_row(
                "SELECT sort_index FROM provider WHERE id = ?1",
                params![provider.id],
                |r| r.get(0),
            )
            .optional()?
        {
            Some(i) => i,
            None => conn.query_row(
                "SELECT COALESCE(MAX(sort_index), -1) + 1 FROM provider",
                [],
                |r| r.get(0),
            )?,
        };
        conn.execute(
            "INSERT INTO provider \
             (id, name, website_url, category, icon, icon_color, sort_index, notes, \
              settings_config, meta, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name, website_url = excluded.website_url, \
               category = excluded.category, icon = excluded.icon, \
               icon_color = excluded.icon_color, notes = excluded.notes, \
               settings_config = excluded.settings_config, meta = excluded.meta, \
               updated_at = excluded.updated_at",
            params![
                provider.id,
                provider.name,
                provider.website_url,
                provider.category.as_str(),
                provider.icon,
                provider.icon_color,
                sort_index,
                provider.notes,
                provider.settings_config,
                provider.meta,
                provider.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_provider(&self, id: &str) -> AppResult<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        conn.execute("DELETE FROM provider WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 按 id 取一个 provider；不存在 → `None`。供「切换」与「当前使用」光卡
    /// 用——避免拉全表再过滤。
    pub fn get_provider(&self, id: &str) -> AppResult<Option<Provider>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, website_url, category, icon, icon_color, sort_index, \
             notes, settings_config, meta, updated_at FROM provider WHERE id = ?1",
        )?;
        let row = stmt.query_row(params![id], row_to_provider).optional()?;
        Ok(row)
    }

    /// Apply a full new display order: each id's `sort_index` becomes its
    /// index in `ordered_ids`. Unknown ids are ignored and absent ids keep
    /// their old position — same tolerant semantics as
    /// `reorder_local_groups` (a stale caller must not fail the whole drop).
    /// One transaction so the new order lands atomically.
    pub fn reorder_providers(&self, ordered_ids: &[String]) -> AppResult<()> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        for (i, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE provider SET sort_index = ?2 WHERE id = ?1",
                params![id, i as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn row_to_provider(r: &rusqlite::Row) -> rusqlite::Result<Provider> {
    Ok(Provider {
        // TEMP-APP-SHIM: #32 合并后移除，以 #32 实现为准——#32 的 DDL 加 app
        // 列后从此处读取；当前表无该列，全部存量行归 claude。
        app: App::Claude,
        id: r.get(0)?,
        name: r.get(1)?,
        website_url: r.get(2)?,
        category: ProviderCategory::from_db_str(&r.get::<_, String>(3)?),
        icon: r.get(4)?,
        icon_color: r.get(5)?,
        sort_index: r.get::<_, i64>(6)? as u32,
        notes: r.get(7)?,
        settings_config: r.get(8)?,
        meta: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::mem;

    /// Build a minimal provider literal (most fields empty) with an explicit
    /// sort_index. `save`'s id-generation path is what's under test, so the
    /// helper takes no id.
    fn provider(name: &str, category: ProviderCategory) -> Provider {
        Provider {
            app: App::Claude,
            id: String::new(),
            name: name.into(),
            website_url: "https://example.com".into(),
            category,
            icon: String::new(),
            icon_color: String::new(),
            sort_index: 0,
            notes: String::new(),
            settings_config: r#"{"env":{}}"#.into(),
            meta: r#"{}"#.into(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn save_assigns_id_and_appends_to_end() {
        let s = mem();
        let a = s
            .save_provider(provider("Alpha", ProviderCategory::Custom))
            .unwrap();
        let b = s
            .save_provider(provider("Beta", ProviderCategory::Custom))
            .unwrap();
        // Fresh ids, appended order (0 then 1).
        assert!(!a.id.is_empty());
        assert_eq!(a.sort_index, 0);
        assert_eq!(b.sort_index, 1);
        let ids: Vec<String> = s
            .list_providers()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, [a.id, b.id]);
    }

    #[test]
    fn save_edits_keep_position_and_refresh_updated_at() {
        let s = mem();
        let created = s
            .save_provider(provider("First", ProviderCategory::Custom))
            .unwrap();
        // Reorder so First is last, then edit it.
        s.reorder_providers(std::slice::from_ref(&created.id))
            .unwrap();
        let mut edited = created.clone();
        edited.name = "Renamed".into();
        edited.sort_index = 99; // caller must not be able to move via save
        let saved = s.save_provider(edited).unwrap();
        let rows = s.list_providers().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Renamed");
        assert_eq!(
            rows[0].sort_index, created.sort_index,
            "sort_index preserved"
        );
        // The return value reports the DB position, not the caller's stale 99 —
        // a caller that carried an outdated sort_index must not be lied to.
        assert_eq!(
            saved.sort_index, rows[0].sort_index,
            "returned sort_index matches the persisted row"
        );
        assert_eq!(saved.updated_at, rows[0].updated_at);
        assert!(!saved.updated_at.is_empty());
    }

    #[test]
    fn save_editing_deleted_id_appends_to_end_not_stale_slot() {
        let s = mem();
        let a = s
            .save_provider(provider("Alpha", ProviderCategory::Custom))
            .unwrap();
        let b = s
            .save_provider(provider("Beta", ProviderCategory::Custom))
            .unwrap();
        s.delete_provider(&a.id).unwrap();
        // The caller still holds the deleted row (id + a stale sort_index 0);
        // the upsert revives it as a fresh append (max + 1) rather than at the
        // old slot, so the order stays sane.
        let revived = s.save_provider(a).unwrap();
        assert_eq!(revived.sort_index, 2, "appended after Beta");
        let ids: Vec<String> = s
            .list_providers()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, [b.id, revived.id]);
    }

    #[test]
    fn delete_removes_row() {
        let s = mem();
        let p = s
            .save_provider(provider("Gone", ProviderCategory::Custom))
            .unwrap();
        s.delete_provider(&p.id).unwrap();
        assert!(s.list_providers().unwrap().is_empty());
    }

    #[test]
    fn get_provider_finds_by_id_and_returns_none_for_missing() {
        let s = mem();
        let p = s
            .save_provider(provider("Alpha", ProviderCategory::Custom))
            .unwrap();
        let found = s.get_provider(&p.id).unwrap().expect("row must exist");
        assert_eq!(found.id, p.id);
        assert_eq!(found.name, "Alpha");
        assert_eq!(found.settings_config, r#"{"env":{}}"#);
        assert!(s.get_provider("no-such-id").unwrap().is_none());
    }

    #[test]
    fn reorder_rewrites_sort_index_in_list_order() {
        let s = mem();
        let mut ids = Vec::new();
        for name in ["a", "b", "c"] {
            let p = s
                .save_provider(provider(name, ProviderCategory::Custom))
                .unwrap();
            ids.push(p.id);
        }
        ids.reverse();
        s.reorder_providers(&ids).unwrap();
        let got: Vec<String> = s
            .list_providers()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(got, ids);
    }

    #[test]
    fn reorder_tolerates_stale_or_unknown_ids() {
        let s = mem();
        let mut ids = Vec::new();
        for name in ["a", "b", "c"] {
            let p = s
                .save_provider(provider(name, ProviderCategory::Custom))
                .unwrap();
            ids.push(p.id);
        }
        // "c" was deleted between fetch and drop — the reorder still lands,
        // and an injected unknown id is ignored.
        s.reorder_providers(&[ids[1].clone(), "zz".into(), ids[0].clone()])
            .unwrap();
        let got: Vec<String> = s
            .list_providers()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        // b index 0, a index 2; c keeps its old position 2 and ties with a →
        // name order puts it after "a" (deterministic fallback).
        assert_eq!(got, [ids[1].clone(), ids[0].clone(), ids[2].clone()]);
    }

    #[test]
    fn category_roundtrips_through_db() {
        let s = mem();
        let mut p = provider("Bedrock", ProviderCategory::CloudProvider);
        p.category = ProviderCategory::CnOfficial;
        let saved = s.save_provider(p).unwrap();
        let rows = s.list_providers().unwrap();
        assert_eq!(rows[0].category, ProviderCategory::CnOfficial);
        assert_eq!(rows[0].category.as_str(), "cn_official");
        assert_eq!(saved.category, ProviderCategory::CnOfficial);
    }

    /// Like [`provider`] but with an explicit settings_config.
    fn provider_with_config(name: &str, settings_config: &str) -> Provider {
        Provider {
            settings_config: settings_config.into(),
            ..provider(name, ProviderCategory::Custom)
        }
    }

    /// The sync-freshness invariant: `updated_at` advances on structural
    /// change only. A key-only edit must not make the row look newer than a
    /// peer's real edit — otherwise a key fill would let a stale local copy
    /// win the latest-wins merge and silently reverse the peer's change.
    #[test]
    fn key_only_edit_keeps_updated_at_structure_edit_refreshes() {
        let s = mem();
        let created = s
            .save_provider(provider_with_config(
                "Kimi",
                r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com","ANTHROPIC_AUTH_TOKEN":"sk-old"}}"#,
            ))
            .unwrap();
        let first_updated_at = created.updated_at.clone();

        // A key-only edit must NOT advance the freshness timestamp.
        let mut keyed = created.clone();
        keyed.settings_config =
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com","ANTHROPIC_AUTH_TOKEN":"sk-new"}}"#
                .into();
        let saved = s.save_provider(keyed).unwrap();
        assert_eq!(
            saved.updated_at, first_updated_at,
            "key-only edit keeps updated_at"
        );
        // The key itself IS persisted — keys live in the local DB, only the
        // freshness timestamp is untouched.
        let row = s.get_provider(&created.id).unwrap().unwrap();
        assert!(row.settings_config.contains("sk-new"));

        // A structural edit (endpoint) refreshes it.
        let mut moved = created.clone();
        moved.settings_config =
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://other.dev","ANTHROPIC_AUTH_TOKEN":"sk-new"}}"#
                .into();
        let saved2 = s.save_provider(moved).unwrap();
        assert_ne!(
            saved2.updated_at, first_updated_at,
            "structural edit bumps updated_at"
        );
    }

    /// `import_provider` preserves the AUTHOR's updated_at (sync freshness is
    /// the author's, not the import time) and the LOCAL sort_index (a peer's
    /// file must not shuffle this device's display order).
    #[test]
    fn import_provider_preserves_author_timestamp_and_local_sort_index() {
        let s = mem();
        let created = s.save_provider(provider_with_config("Kimi", "{}")).unwrap();
        s.save_provider(provider_with_config("Beta", "{}")).unwrap();
        s.reorder_providers(std::slice::from_ref(&created.id))
            .unwrap();
        let before = s.get_provider(&created.id).unwrap().unwrap();
        assert_eq!(before.sort_index, 0, "local order put Kimi first");
        let before_updated = before.updated_at.clone();

        let peer = Provider {
            app: App::Claude,
            id: created.id.clone(),
            name: "Kimi Pro".into(),
            website_url: "https://x.dev".into(),
            category: ProviderCategory::Custom,
            icon: String::new(),
            icon_color: String::new(),
            sort_index: 99, // the peer's order — must not land here
            notes: String::new(),
            settings_config: r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com"}}"#.into(),
            meta: r#"{}"#.into(),
            updated_at: "2026-08-01T00:00:00.000Z".into(),
        };
        s.import_provider(&peer).unwrap();

        let row = s.get_provider(&created.id).unwrap().unwrap();
        assert_eq!(row.name, "Kimi Pro");
        assert_eq!(
            row.updated_at, "2026-08-01T00:00:00.000Z",
            "author timestamp preserved"
        );
        assert_eq!(row.sort_index, 0, "local sort_index kept");
        assert_ne!(row.updated_at, before_updated);
    }

    #[test]
    fn import_provider_appends_new_rows_at_end() {
        let s = mem();
        s.save_provider(provider_with_config("Alpha", "{}"))
            .unwrap();
        s.save_provider(provider_with_config("Beta", "{}")).unwrap();

        let peer = Provider {
            app: App::Claude,
            id: "newpeer01".into(),
            name: "New Peer".into(),
            website_url: "https://x.dev".into(),
            category: ProviderCategory::Custom,
            icon: String::new(),
            icon_color: String::new(),
            sort_index: 0,
            notes: String::new(),
            settings_config: r#"{"env":{}}"#.into(),
            meta: r#"{}"#.into(),
            updated_at: "2026-08-01T00:00:00.000Z".into(),
        };
        s.import_provider(&peer).unwrap();

        let row = s.get_provider("newpeer01").unwrap().unwrap();
        assert_eq!(row.sort_index, 2, "new import appends after existing rows");
        assert_eq!(row.updated_at, "2026-08-01T00:00:00.000Z");
    }
}
