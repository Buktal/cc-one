//! `sessions` 表读路径：会话列表（分页 / 全量）、单会话行、侧栏计数，以及
//! 会话粒过滤的共享 WHERE 构建器 [`build_session_where`]。
//!
//! `build_session_where` 是「SessionFilter → SQL」的单一归属：列表、计数、
//! 维度查询（`super::store_dimensions`）与项目候选（`super::store_reads`）
//! 全部经它收窄，`excluded = 0` 的软删不可见性也由此一处生效。写路径
//! （setter / UPSERT / dirty / 收藏对账）在 `super::store_sessions_writes`；
//! 消息本体（`session_messages` 表）在 `super::store_transcript`。

use super::aggregate_sql::session_pair_join;
use super::*;

impl super::Store {
    /// List sessions for the UI, joined live with `usage_records` to compute
    /// per-session request_count / total_tokens / total_cost_usd (the usage
    /// table is the single source of token truth). Title = `custom_title` when
    /// set, else `title_orig`. `filter` is optional; `None` lists every
    /// non-excluded session (soft-deleted rows never surface).
    /// Unpaged — retained for test-only callers (the collector/sync tests);
    /// production reads go through [`Store::query_sessions_page`] so the UI
    /// only materializes one page.
    #[cfg(test)]
    pub fn query_sessions(&self, filter: Option<&SessionFilter>) -> AppResult<Vec<SessionRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_session_where(filter);
        let sql = sessions_select_sql(&clause);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), session_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// One page of the session list for the UI — same rows as
    /// [`Store::query_sessions`] but `LIMIT ? OFFSET ?` applied so a large
    /// session table renders a page instead of loading everything (mirrors the
    /// request-log table's paging). The ORDER BY adds `device_id`/`id`
    /// tiebreakers so pages never duplicate or skip rows across page turns.
    pub fn query_sessions_page(&self, query: &SessionQuery) -> AppResult<Vec<SessionRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, mut params_vec) = build_session_where(query.filter.as_ref());
        let sql = format!("{} LIMIT ? OFFSET ?", sessions_select_sql(&clause));
        params_vec.push(SqlValue::Integer(super::page_limit(query.limit)));
        params_vec.push(SqlValue::Integer(query.offset as i64));
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), session_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// One session row by its exact composite key `(id, device_id)` — the
    /// usage-side "request log → session" jump channel: the frontend resolves a
    /// usage record's `session_id` into the session row (title + identity) via
    /// this read instead of a backend join on the usage query. Same SELECT as
    /// the list (usage aggregates + project_identity truncation included), so
    /// the resolved row is identical to what the session list would show.
    /// `None` = no such session (usage record without a collected session, or
    /// one soft-deleted — deleted means nonexistent to the jump channel too,
    /// so the link degrades to the raw id like any unresolved one).
    pub fn get_session(&self, id: &str, device_id: &str) -> AppResult<Option<SessionRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let sql = sessions_select_sql("WHERE s.id = ?1 AND s.device_id = ?2 AND s.excluded = 0");
        conn.query_row(&sql, params![id, device_id], session_row)
            .optional()
            .map_err(AppError::from)
    }

    /// Sidebar + paginator counts for one grouping track under a filter: the
    /// total session count (drives the paginator and the sidebar's "All" row)
    /// plus per-bucket counts (the group rows). The track selects the group
    /// column ([`GroupTrack::Local`] → `local_group_id`,
    /// [`GroupTrack::Synced`] → `synced_group_id`); every distinct column
    /// value becomes a bucket, including the empty string (ungrouped) and
    /// stale ids whose group was deleted — the client resolves those against
    /// its known group list. Paging-independent: it describes the whole
    /// filtered set.
    pub fn count_sessions(
        &self,
        filter: Option<&SessionFilter>,
        track: GroupTrack,
    ) -> AppResult<SessionGroupCounts> {
        let col = match track {
            GroupTrack::Local => "local_group_id",
            GroupTrack::Synced => "synced_group_id",
        };
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_session_where(filter);
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM sessions s {clause}"),
            params_from_iter(params_vec.iter()),
            |r| r.get(0),
        )?;
        let sql = format!(
            "SELECT s.{col} AS gid, COUNT(*) AS n \
             FROM sessions s {clause} GROUP BY s.{col}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            Ok(SessionGroupCount {
                group_id: r.get(0)?,
                count: r.get::<_, i64>(1)? as u32,
            })
        })?;
        let groups = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)?;
        Ok(SessionGroupCounts {
            total: total as u32,
            groups,
        })
    }

    // 维度查询（query_project_stats / query_session_stats，会话粒）与 usage
    // 粒的 project/session/device 维度同在 [`super::store_dimensions`]——一个
    // 维度两种粒度一个家。
}

/// Build a WHERE clause over the `sessions` table for a [`SessionFilter`]. The
/// clause prefixes every column with `s.` so it composes with the
/// `usage_records` subquery JOIN in [`Store::query_sessions`]. Empty filter ⇒
/// the bare exclusion condition (`s.excluded = 0`): a soft-deleted session is
/// invisible to EVERY sessions-side read (list / counts / stats / project
/// buckets / project candidates) through this one builder — usage-side reads
/// deliberately keep counting its records (usage is collected history, and the
/// dashboard's bucket-sums-equal-hero calibers must not shift under a sessions
/// view action). `pub(super)` so the distinct-projects read (store_reads)
/// reuses the same sessions-side narrowing — one builder, no drifting copy.
pub(super) fn build_session_where(filter: Option<&SessionFilter>) -> (String, Vec<SqlValue>) {
    use super::filter_sql::{push_nonempty_eq, push_ts_range};
    let mut conds: Vec<String> = vec!["s.excluded = 0".into()];
    let mut params: Vec<SqlValue> = Vec::new();
    let Some(f) = filter else {
        return (format!("WHERE {}", conds.join(" AND ")), params);
    };
    push_nonempty_eq(&mut conds, &mut params, "s.device_id", &f.device_scope);
    push_nonempty_eq(&mut conds, &mut params, "s.source", &f.source);
    if let Some(fav) = f.favorited {
        conds.push(format!("s.favorited = {}", fav as i64));
    }
    if let Some(g) = &f.local_group_id {
        conds.push("s.local_group_id = ?".into());
        params.push(SqlValue::Text(g.clone()));
    }
    if let Some(g) = &f.synced_group_id {
        conds.push("s.synced_group_id = ?".into());
        params.push(SqlValue::Text(g.clone()));
    }
    if let Some(p) = &f.project {
        if !p.is_empty() {
            // Match by project IDENTITY via the `project_identity` SQL scalar
            // (the one Rust rule, registered as a UDF) — a worktree session's
            // raw launch dir collapses to its parent, so it matches the parent
            // project's filter. Same function the project aggregate groups by,
            // so filtering and bucketing can never disagree. The unknown
            // sentinel matches the EMPTY identity — the sessions-side face of
            // the unknown bucket (a session row exists but carries no launch
            // dir; the usage-side NOT EXISTS face lives in store_reads).
            if p == UNKNOWN_PROJECT {
                conds.push("project_identity(s.project_dir) = ''".into());
            } else {
                conds.push("project_identity(s.project_dir) = ?".into());
                params.push(SqlValue::Text(p.clone()));
            }
        }
    }
    // 时间区间走 sessions 粒的时间列（last_active_at）；usage-粒的 u.timestamp
    // 区间见 query_project_stats 的未知桶直读。
    push_ts_range(
        &mut conds,
        &mut params,
        "s.last_active_at",
        &f.from_ts,
        &f.to_ts,
    );
    if let Some(m) = &f.model {
        if !m.is_empty() {
            // EXISTS semantics: the session matched iff ANY usage record in
            // this session used the model. Both keys are required — a session
            // id is a parser file stem, so ids can collide across devices.
            conds.push(format!(
                "EXISTS (SELECT 1 FROM usage_records u \
                 WHERE {} AND u.model = ?)",
                session_pair_join("u", "s")
            ));
            params.push(SqlValue::Text(m.clone()));
        }
    }
    if let Some(q) = &f.search {
        let q = q.trim();
        if !q.is_empty() {
            // Substring search over the DISPLAY title (custom title wins, same
            // COALESCE as the SELECT), the project path, and every message BODY
            // (`session_messages.content`). Like the client-side filter it
            // replaces, the match is case-insensitive and literal — the pattern
            // escapes LIKE wildcards so `%`/`_` in the query never act as
            // metacharacters. The body probe is an EXISTS at session-id grain,
            // deliberately NOT scoped to `s.device_id`: the transcript a row
            // opens is `query_session_transcript`, which merges ALL devices'
            // messages for the id (deduped by uuid, self winning) — so the
            // search must see the same union, or a hit could open a transcript
            // that doesn't contain it (a peer's pulled snapshot often holds
            // messages self's local file lacks, and vice versa). The uuid-level
            // self-wins collapse only ever drops same-uuid duplicates (the same
            // source event), so the union is the merged transcript for matching
            // purposes. `idx_session_messages_sid` serves the probe.
            let pattern = like_pattern(q);
            conds.push(
                "(COALESCE(NULLIF(s.custom_title,''), s.title_orig) LIKE ? ESCAPE '\\' \
                 OR s.project_dir LIKE ? ESCAPE '\\' \
                 OR EXISTS (SELECT 1 FROM session_messages m \
                            WHERE m.session_id = s.id \
                            AND m.content LIKE ? ESCAPE '\\'))"
                    .into(),
            );
            params.push(SqlValue::Text(pattern.clone()));
            params.push(SqlValue::Text(pattern.clone()));
            params.push(SqlValue::Text(pattern));
        }
    }
    // conds is never empty — the exclusion condition seeds it above.
    let clause = format!("WHERE {}", conds.join(" AND "));
    (clause, params)
}

/// Wrap a user search query in `%…%` with LIKE metacharacters (`%`, `_`, `\`)
/// escaped — the SQL mirror of the old client-side substring filter, so a
/// literal `%` or `_` in the query matches itself instead of acting as a
/// wildcard. The ESCAPE char is `\` (SQLite's default), quoted with `ESCAPE
/// '\'` in the SQL above.
fn like_pattern(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 2);
    out.push('%');
    for c in q.chars() {
        if c == '%' || c == '_' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// The shared session-list SELECT (rows + live usage aggregate + optional
/// WHERE), ending in a stable time-desc ORDER BY. `device_id`/`id`
/// tiebreakers make the ordering total, so offset paging never duplicates or
/// skips a row across page turns. Callers append `LIMIT ? OFFSET ?` when
/// paging (or leave the clause empty for the full unpaged read). The usage
/// aggregate is the one [`super::aggregate_sql::usage_agg_subquery`];
/// `total_tokens` sums its four bucket columns at read time (the shared total
/// caliber over aggregated columns).
fn sessions_select_sql(clause: &str) -> String {
    format!(
        "SELECT s.id, s.device_id, s.source, s.project_dir,
                COALESCE(NULLIF(s.custom_title,''), s.title_orig) AS title,
                s.favorited, s.local_group_id, s.synced_group_id,
                s.started_at, s.last_active_at, s.agent_type, s.parent_session_id,
                COALESCE(agg.request_count, 0),
                {total_of} AS total_tokens,
                COALESCE(agg.total_cost_usd, 0.0)
         FROM sessions s
         LEFT JOIN ({agg}) agg ON {pair}
         {clause}
         ORDER BY s.last_active_at DESC, s.device_id, s.id",
        agg = super::aggregate_sql::usage_agg_subquery(false),
        pair = session_pair_join("agg", "s"),
        total_of = super::aggregate_sql::usage_total_of_cols("agg.")
    )
}

/// Decode a `sessions` row in the shared SELECT's column order (14 columns —
/// the positional mapping lives in one place for both the paged and unpaged
/// reads). `project_dir` crosses as the PROJECT IDENTITY
/// ([`crate::model::project_identity`]): a Claude Code worktree suffix
/// (`.claude\worktrees\…`) collapses to its parent project here, at the one
/// decode seam every session-list read goes through — so worktree sessions
/// (subagents et al.) surface under their parent project in every consumer of
/// the list. The stored row keeps the raw launch dir; only the read is
/// truncated.
fn session_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    let project_dir: String = r.get(3)?;
    Ok(SessionRow {
        id: r.get(0)?,
        device_id: r.get(1)?,
        source: r.get(2)?,
        project_dir: project_identity(&project_dir).to_string(),
        title: r.get(4)?,
        favorited: r.get::<_, i64>(5)? != 0,
        local_group_id: r.get(6)?,
        synced_group_id: r.get(7)?,
        started_at: r.get(8)?,
        last_active_at: r.get(9)?,
        agent_type: r.get(10)?,
        parent_session_id: r.get(11)?,
        request_count: r.get::<_, i64>(12)? as u32,
        total_tokens: r.get::<_, i64>(13)? as u32,
        total_cost_usd: r.get(14)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;
    use crate::model::SessionMessageRole;

    // ---------------------------------------------------------- filters ----

    #[test]
    fn query_sessions_time_range_filters_last_active_at() {
        let s = mem();
        seed_session(&s, "old", "dev", "2026-08-01T10:00:00.000Z");
        seed_session(&s, "mid", "dev", "2026-08-15T10:00:00.000Z");
        seed_session(&s, "new", "dev", "2026-08-31T10:00:00.000Z");

        // from_ts narrows to sessions at or after Aug 10.
        let from = SessionFilter {
            from_ts: Some("2026-08-10T00:00:00.000Z".into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions(Some(&from))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["new", "mid"], "from_ts excludes early sessions");

        // to_ts narrows to sessions at or before Aug 20.
        let to = SessionFilter {
            to_ts: Some("2026-08-20T23:59:59.999Z".into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions(Some(&to))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["mid", "old"], "to_ts excludes late sessions");

        // both bounds → only "mid".
        let both = SessionFilter {
            from_ts: Some("2026-08-10T00:00:00.000Z".into()),
            to_ts: Some("2026-08-20T23:59:59.999Z".into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions(Some(&both))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["mid"], "from_ts + to_ts intersect to one session");
    }

    #[test]
    fn query_sessions_model_filter_uses_exists_semantics() {
        let s = mem();
        // s1 uses model A + B; s2 uses only B.
        seed_session_with_record(&s, "s1", "dev", "model-a");
        seed_session_with_record(&s, "s1", "dev", "model-b");
        seed_session_with_record(&s, "s2", "dev", "model-b");

        let ids = |model: &str| -> Vec<String> {
            let f = SessionFilter {
                model: Some(model.into()),
                ..Default::default()
            };
            s.query_sessions(Some(&f))
                .unwrap()
                .into_iter()
                .map(|r| r.id)
                .collect()
        };
        assert_eq!(ids("model-a"), ["s1"], "A matches only s1");
        let both: std::collections::BTreeSet<String> = ids("model-b").into_iter().collect();
        assert_eq!(
            both,
            std::collections::BTreeSet::from(["s1".to_string(), "s2".to_string()]),
            "B matches both (same last_active_at ⇒ order is unspecified)"
        );
        assert!(
            ids("no-such-model").is_empty(),
            "a model nobody used matches nothing"
        );
    }

    #[test]
    fn query_sessions_model_filter_is_device_isolated() {
        let s = mem();
        // Same session id on two devices; the model record exists only on dev1.
        seed_session_with_record(&s, "same", "dev1", "model-x");
        seed_session(&s, "same", "dev2", "2026-08-15T10:00:00.000Z");

        let f = SessionFilter {
            device_scope: Some("dev2".into()),
            model: Some("model-x".into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions(Some(&f))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert!(
            ids.is_empty(),
            "dev2's row must not match dev1's usage record (session ids can collide across devices)"
        );
    }

    // ---------------------------------------------------------- paging ------

    /// Paged reads return consecutive time-desc slices with no overlap or gap
    /// (the ORDER BY tiebreakers make the ordering total) and the page sizes
    /// agree with the count query's total under the same filter.
    #[test]
    fn query_sessions_page_is_consecutive_and_agrees_with_count() {
        let s = mem();
        seed_session(&s, "d", "dev", "2026-08-04T10:00:00.000Z");
        seed_session(&s, "a", "dev", "2026-08-01T10:00:00.000Z");
        seed_session(&s, "c", "dev", "2026-08-03T10:00:00.000Z");
        seed_session(&s, "e", "dev", "2026-08-05T10:00:00.000Z");
        seed_session(&s, "b", "dev", "2026-08-02T10:00:00.000Z");

        let page1 = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 2,
                offset: 0,
            })
            .unwrap();
        let page2 = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 2,
                offset: 2,
            })
            .unwrap();
        let page3 = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 2,
                offset: 4,
            })
            .unwrap();
        let ids =
            |rows: Vec<SessionRow>| -> Vec<String> { rows.into_iter().map(|r| r.id).collect() };
        assert_eq!(ids(page1), ["e", "d"], "page 1 = newest two");
        assert_eq!(ids(page2), ["c", "b"], "page 2 = next two");
        assert_eq!(ids(page3), ["a"], "page 3 = the tail");
        // Offsets past the end return an empty page, never an error.
        let past = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 2,
                offset: 99,
            })
            .unwrap();
        assert!(past.is_empty());

        let counts = s.count_sessions(None, GroupTrack::Local).unwrap();
        assert_eq!(counts.total, 5, "count total matches the paged set");
        let all = s.query_sessions(None).unwrap();
        assert_eq!(all.len(), 5, "unpaged read still returns everything");
    }

    /// 越界 limit 被夹紧（#66：会话分页原漏夹紧、直接透传 query.limit）：
    /// 0 → 1（不空翻整页），超大 → 1000（不一次物化全表）。
    #[test]
    fn query_sessions_page_clamps_out_of_range_limits() {
        let s = mem();
        seed_session(&s, "a", "dev", "2026-08-01T10:00:00.000Z");
        seed_session(&s, "b", "dev", "2026-08-02T10:00:00.000Z");
        // 0 → 夹到 1（返回一行，不空翻整页也不报错）；超大 → 夹到 1000（全返回）。
        let zero = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 0,
                offset: 0,
            })
            .unwrap();
        assert_eq!(zero.len(), 1, "limit=0 夹到 1");
        let huge = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: u32::MAX,
                offset: 0,
            })
            .unwrap();
        assert_eq!(huge.len(), 2, "limit 超大夹到 1000，全部返回");
    }

    /// Search is backend-side (LIKE) so a paged result searches the whole set,
    /// not just the loaded page. Matches the display title (custom title wins)
    /// and the project path, case-insensitively.
    #[test]
    fn query_sessions_page_search_matches_title_project_and_custom_title() {
        let s = mem();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "s1".into(),
                source: "claude_code".into(),
                project_dir: "/home/u/parser".into(),
                title_orig: "Refactor tokenizer".into(),
                ..sys_session("s1", "2026-08-01T10:00:00.000Z")
            },
        )
        .unwrap();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "s2".into(),
                project_dir: "/home/u/www".into(),
                title_orig: "Unrelated".into(),
                ..sys_session("s2", "2026-08-02T10:00:00.000Z")
            },
        )
        .unwrap();

        let ids = |q: &str| -> Vec<String> {
            let filter = SessionFilter {
                search: Some(q.into()),
                ..Default::default()
            };
            s.query_sessions_page(&SessionQuery {
                filter: Some(filter),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
        };
        assert_eq!(ids("refactor"), ["s1"], "title_orig matches");
        assert_eq!(ids("parser"), ["s1"], "project_dir matches");
        assert!(ids("HELLO").is_empty(), "no match for unrelated text");
        // Custom title replaces the display title — search then sees it, not
        // the title_orig behind it (same COALESCE as the SELECT).
        s.set_session_custom_title("dev", "s1", Some("Casework"))
            .unwrap();
        assert_eq!(
            ids("casework"),
            ["s1"],
            "custom title becomes the searchable display title"
        );
        assert!(
            ids("refactor").is_empty(),
            "title_orig behind a custom title is not searched"
        );
        assert!(ids("zzz").is_empty(), "no match");
        // Search composes with the tab filter (device scope).
        let scoped = SessionFilter {
            device_scope: Some("other-dev".into()),
            search: Some("refactor".into()),
            ..Default::default()
        };
        assert!(s
            .query_sessions_page(&SessionQuery {
                filter: Some(scoped),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .is_empty());
    }

    /// LIKE wildcards in the search query are escaped — a literal `%` or `_`
    /// matches itself, mirroring the old client-side substring filter.
    #[test]
    fn query_sessions_page_search_escapes_like_wildcards() {
        let s = mem();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "pct".into(),
                title_orig: "100% done".into(),
                ..sys_session("pct", "2026-08-01T10:00:00.000Z")
            },
        )
        .unwrap();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "plain".into(),
                title_orig: "One hundred".into(),
                ..sys_session("plain", "2026-08-02T10:00:00.000Z")
            },
        )
        .unwrap();
        let ids = |q: &str| -> Vec<String> {
            let filter = SessionFilter {
                search: Some(q.into()),
                ..Default::default()
            };
            s.query_sessions_page(&SessionQuery {
                filter: Some(filter),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
        };
        assert_eq!(ids("%"), ["pct"], "a lone % matches the literal % row only");
        assert_eq!(ids("00%"), ["pct"], "% is not a wildcard in the query");
        assert_eq!(
            ids("One hundred"),
            ["plain"],
            "plain query unaffected by escaping"
        );
    }

    // ---------------------------------------------------- list DTO ---------

    /// Parent link roundtrip (#90): a subagent row's `parent_session_id`
    /// persists and crosses to the list DTO, keyed to the same device.
    #[test]
    fn session_list_carries_parent_link() {
        let s = mem();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "main-1".into(),
                agent_type: String::new(),
                parent_session_id: String::new(),
                ..sys_session("main-1", "2026-08-02T10:00:00.000Z")
            },
        )
        .unwrap();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "agent-x".into(),
                agent_type: "Explore".into(),
                parent_session_id: "main-1".into(),
                ..sys_session("agent-x", "2026-08-01T10:00:00.000Z")
            },
        )
        .unwrap();
        let rows = s.query_sessions(None).unwrap();
        let by_id = |id: &str| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(by_id("agent-x").agent_type, "Explore");
        assert_eq!(by_id("agent-x").parent_session_id, "main-1");
        assert_eq!(by_id("main-1").parent_session_id, "");
    }

    /// Worktree sessions surface under their parent project on the session
    /// read path: a row storing a `.claude\worktrees\…` project_dir (the
    /// launch dir Claude Code gives subagent/parallel sessions, issue #84)
    /// comes back from `query_sessions_page` truncated to the parent — every
    /// consumer of the session list reasons about the parent project, while
    /// the stored row keeps the raw launch dir (the snapshot meta read below
    /// pins that the raw value is NOT rewritten).
    #[test]
    fn query_sessions_page_collapses_worktree_project_to_parent() {
        let s = mem();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "agent-a10c476b".into(),
                source: "claude_code".into(),
                project_dir: "D:\\Project\\O_CC_One\\.claude\\worktrees\\agent-a10c476b".into(),
                title_orig: "核实 cc-switch 供应商".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-02T00:00:00.000Z".into(),
                agent_type: "Explore".into(),
                parent_session_id: String::new(),
            },
        )
        .unwrap();
        let rows = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 50,
                offset: 0,
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project_dir, "D:\\Project\\O_CC_One");
        assert_eq!(rows[0].agent_type, "Explore", "subagent tag crosses as-is");
        // The stored row (and thus the git snapshot meta) keeps the RAW launch
        // dir — truncation is a read-side rule, not a rewrite.
        let meta = s
            .get_session_snapshot_meta("dev", "agent-a10c476b")
            .unwrap()
            .unwrap();
        assert_eq!(
            meta.project_dir,
            "D:\\Project\\O_CC_One\\.claude\\worktrees\\agent-a10c476b"
        );
    }

    /// The jump read resolves by the FULL composite key: the same session id
    /// can exist under two devices (a session collected on both), and usage
    /// aggregates must come from that device's records only. A key that
    /// matches no row resolves to `None` (session-less historical usage).
    #[test]
    fn get_session_resolves_by_composite_key_with_usage_aggregate() {
        let s = mem();
        for (dev, title) in [("dev-a", "本机采集的会话"), ("dev-b", "peer 同 id 会话")] {
            s.upsert_session(
                dev,
                &SessionSystemData {
                    id: "sid-1".into(),
                    source: "claude_code".into(),
                    project_dir: "D:\\Project\\O_CC_One".into(),
                    title_orig: title.into(),
                    started_at: "2026-08-01T00:00:00.000Z".into(),
                    last_active_at: "2026-08-02T00:00:00.000Z".into(),
                    agent_type: String::new(),
                    parent_session_id: String::new(),
                },
            )
            .unwrap();
        }
        // rec 侧 helper 固定 device "dev"，这里需要 dev-a —— 就地组记录。
        let mut r = rec("u1", "2026-08-15", "glm-5.2", "dev-a", 100, 50, 0.25);
        r.session_id = "sid-1".into();
        s.ingest_marking_dirty(&[r]).unwrap();
        let a = s.get_session("sid-1", "dev-a").unwrap().unwrap();
        assert_eq!(a.device_id, "dev-a");
        assert_eq!(a.title, "本机采集的会话");
        assert_eq!(a.request_count, 1, "usage aggregate joins on device too");
        assert_eq!(a.total_tokens, 150);
        assert_eq!(a.total_cost_usd, 0.25);
        let b = s.get_session("sid-1", "dev-b").unwrap().unwrap();
        assert_eq!(b.title, "peer 同 id 会话");
        assert_eq!(b.request_count, 0, "peer row has no usage of its own");
        assert!(s.get_session("sid-1", "dev-x").unwrap().is_none());
        assert!(s.get_session("sid-2", "dev-a").unwrap().is_none());
    }

    // ------------------------------------------------------- search --------

    /// Cross-session full-text search: the `search` filter also matches message
    /// BODIES, on the production paths (paged list + sidebar counts share
    /// `build_session_where`). Case-insensitive and literal — a `%` in the query
    /// matches a literal `%` in a body, never acts as a wildcard.
    #[test]
    fn session_filter_search_matches_message_bodies() {
        let s = mem();
        seed_session(&s, "s1", "dev", "2026-08-01T10:00:00.000Z");
        seed_session(&s, "s2", "dev", "2026-08-02T10:00:00.000Z");
        let mut hit = msg("u1", "s1", SessionMessageRole::User, "2026-08-01T10:00:00Z");
        hit.content = "the tokamak calibration notes".into();
        let mut pct = msg("u2", "s2", SessionMessageRole::User, "2026-08-02T10:00:00Z");
        pct.content = "shipment 100% done".into();
        s.ingest_session_messages_marking_dirty("dev", &[hit, pct])
            .unwrap();

        let ids = |q: &str| -> Vec<String> {
            s.query_sessions_page(&SessionQuery {
                filter: Some(SessionFilter {
                    search: Some(q.into()),
                    ..Default::default()
                }),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
        };
        assert_eq!(
            ids("tokamak"),
            ["s1"],
            "body-only hit surfaces the session (title/project miss)"
        );
        assert_eq!(ids("TOKAMAK"), ["s1"], "body match is case-insensitive");
        assert_eq!(
            ids("00%"),
            ["s2"],
            "literal % in a body match, not a wildcard"
        );
        assert!(ids("glorb").is_empty(), "no body/title/project match");
        // Sidebar counts go through the same clause — they must agree with the
        // paged list, or the paginator would contradict the rows it counts.
        let counts = s
            .count_sessions(
                Some(&SessionFilter {
                    search: Some("tokamak".into()),
                    ..Default::default()
                }),
                GroupTrack::Local,
            )
            .unwrap();
        assert_eq!(counts.total, 1, "counts see the body hit too");
    }

    /// The body probe reuses the transcript MERGE semantics: a message that
    /// exists only under a PEER's device id (a pulled snapshot row) still
    /// matches the session, because the transcript the row opens
    /// (`query_session_transcript`) merges all devices' messages for the id.
    /// A device-scoped probe would miss it and show a hit-less list while the
    /// opened transcript contains the match. Pinned end-to-end here: the
    /// Local-tab shape (device_scope = self) matches, and the merged transcript
    /// actually holds the peer-only message.
    #[test]
    fn session_filter_search_sees_peer_device_message_bodies() {
        let s = mem();
        // Self collected the session; its own slice does NOT contain the term.
        seed_session(&s, "s1", "dev", "2026-08-01T10:00:00.000Z");
        let mut own = msg("u1", "s1", SessionMessageRole::User, "2026-08-01T10:00:00Z");
        own.content = "own-device chatter".into();
        s.ingest_session_messages_marking_dirty("dev", &[own])
            .unwrap();
        // A peer's favorited snapshot carries an extra message self never saw —
        // imported through the production pull path, under the PEER's device id.
        let mut extra = msg(
            "p1",
            "s1",
            SessionMessageRole::Assistant,
            "2026-08-01T11:00:00Z",
        );
        extra.content = "the zeppelin docking checklist".into();
        s.import_session_snapshot(
            "peer1",
            &SessionSnapshotMeta {
                v: SESSION_SNAPSHOT_VERSION,
                id: "s1".into(),
                source: "claude_code".into(),
                project_dir: "/proj".into(),
                title_orig: "Title".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-01T12:00:00.000Z".into(),
                agent_type: String::new(),
                parent_session_id: String::new(),
                favorited: true,
                synced_group_id: String::new(),
            },
            &[extra],
        )
        .unwrap();

        let filter = SessionFilter {
            device_scope: Some("dev".into()),
            search: Some("zeppelin".into()),
            ..Default::default()
        };
        let rows = s
            .query_sessions_page(&SessionQuery {
                filter: Some(filter),
                limit: 50,
                offset: 0,
            })
            .unwrap();
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["s1"],
            "self's row matches via the peer-only body (Local-tab shape)"
        );
        // The consistency premise this pins: opening the session really does
        // show the peer-only message in the merged transcript.
        let merged = s.query_session_transcript("s1", "dev").unwrap();
        assert!(
            merged.iter().any(|m| m.content.contains("zeppelin")),
            "merged transcript holds the peer message the search matched"
        );
    }

    /// Sidebar counts: total under the filter + one bucket per distinct group
    /// column value (empty string = ungrouped), per track.
    #[test]
    fn count_sessions_totals_and_group_buckets_per_track() {
        let s = mem();
        for (id, last) in [
            ("a", "2026-08-01T10:00:00.000Z"),
            ("b", "2026-08-02T10:00:00.000Z"),
            ("c", "2026-08-03T10:00:00.000Z"),
            ("d", "2026-08-04T10:00:00.000Z"),
        ] {
            seed_session(&s, id, "dev", last);
        }
        s.set_session_local_group("dev", "a", Some("lg1")).unwrap();
        s.set_session_local_group("dev", "b", Some("lg1")).unwrap();
        s.set_session_local_group("dev", "c", Some("lg2")).unwrap();
        s.set_session_synced_group("dev", "a", Some("sg1")).unwrap();

        let local = s.count_sessions(None, GroupTrack::Local).unwrap();
        assert_eq!(local.total, 4, "total ignores the track");
        let buckets: std::collections::BTreeMap<String, u32> = local
            .groups
            .iter()
            .map(|g| (g.group_id.clone(), g.count))
            .collect();
        assert_eq!(buckets["lg1"], 2, "two sessions in lg1");
        assert_eq!(buckets["lg2"], 1, "one session in lg2");
        assert_eq!(buckets[""], 1, "the ungrouped bucket is the empty id");

        let synced = s.count_sessions(None, GroupTrack::Synced).unwrap();
        let synced_buckets: std::collections::BTreeMap<String, u32> = synced
            .groups
            .iter()
            .map(|g| (g.group_id.clone(), g.count))
            .collect();
        assert_eq!(synced_buckets["sg1"], 1);
        assert_eq!(synced_buckets[""], 3);

        // Filtered counts narrow with the filter (source scope).
        let src_filter = SessionFilter {
            source: Some("codex_cli".into()),
            ..Default::default()
        };
        let empty = s
            .count_sessions(Some(&src_filter), GroupTrack::Local)
            .unwrap();
        assert_eq!(empty.total, 0);
        assert!(empty.groups.is_empty());
    }
}
