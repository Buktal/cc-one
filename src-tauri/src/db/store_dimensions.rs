//! 维度查询：project / session / device 三个维度 × usage / session 两种粒度
//! ，一个维度两种粒度同屏对照（架构审查Ⅲ候选⑤自 store_reads /
//! store_transcript 收拢至此——归属理由见两个来源模块各自「维度查询搬家」
//! 的提交；本模块是维度读路径的单一家园）。
//!
//! - usage 粒（`UsageFilter` 直读 `usage_records`，桶和与 `query_stats` 的
//!   hero 总量精确相等——同一条 [`super::filter_sql::build_where`]）：
//!   [`Store::query_project_usage`] / [`Store::query_session_usage`] /
//!   [`Store::query_device_usage`]；
//! - session 粒（`SessionFilter` 走 sessions 表 + 聚合子查询 LEFT JOIN）：
//!   [`Store::query_project_stats`] / [`Store::query_session_stats`]——聚合
//!   源是 [`super::aggregate_sql::usage_agg_subquery`] 的同一份子查询，两粒
//!   同口径（三读相等测试守住）。
//!
//! 未知项目桶（[`UNKNOWN_PROJECT`] 哨兵）：[`Store::query_project_stats`] 的
//! 合成行，其 usage 粒条件与 build_where 共用
//! [`super::filter_sql::push_usage_facets`]——known/unknown 两桶的收窄口径
//! 不再两份实现各改各的（#94/#100 的事故形态）。

use super::filter_sql::{build_where, project_condition, push_usage_facets, Facet, FacetGates};
use super::store_transcript::build_session_where;
use super::*;

impl super::Store {
    /// The project dimension: sessions rolled up by project identity, joined
    /// live with their `usage_records` aggregates (requests / token
    /// four-buckets / cost — the usage table stays the single source of token
    /// truth, nothing is stored at project grain). One bucket per
    /// `project_identity(s.project_dir)` value — the SQL scalar backed by the
    /// one Rust rule — so Claude Code worktree sessions and their usage land
    /// under the PARENT project. `MAX(last_active_at)` feeds the
    /// recent-activity metric and orders the buckets (most recent first, `pid`
    /// tiebreaker for determinism). Sessions with NO usage still form their
    /// bucket's session count (LEFT JOIN + COALESCE): the dimension describes
    /// where sessions ran, not only where usage landed. The filter applies
    /// BEFORE grouping, so a time range narrows which sessions feed the
    /// buckets at all.
    ///
    /// One SYNTHETIC row carries the [`UNKNOWN_PROJECT`] sentinel: the
    /// aggregate over session-less usage — remote usage whose favorite
    /// snapshot was never pulled (the only cross-device session rows are
    /// favorites), plus session-less legacy rows. Without it, that usage
    /// silently vanished from every project view. `session_count` is 0 by
    /// definition (no session rows exist); `last_active_at` is the MAX usage
    /// timestamp so the bucket sorts by real recency. Session-attribute
    /// constraints the bucket can never satisfy (favorited-only, a group, a
    /// search) suppress the row entirely ([`unknown_bucket_suppressed`]);
    /// `favorited = Some(false)` keeps it (session-less is definitionally not
    /// favorited).
    pub fn query_project_stats(
        &self,
        filter: Option<&SessionFilter>,
    ) -> AppResult<Vec<ProjectStatsRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_session_where(filter);
        let sql = format!(
            "SELECT project_identity(s.project_dir) AS pid,
                    COUNT(*) AS session_count,
                    COALESCE(SUM(agg.request_count), 0) AS request_count,
                    COALESCE(SUM(agg.input_tokens), 0) AS input_tokens,
                    COALESCE(SUM(agg.output_tokens), 0) AS output_tokens,
                    COALESCE(SUM(agg.cache_creation_tokens), 0) AS cache_creation_tokens,
                    COALESCE(SUM(agg.cache_read_tokens), 0) AS cache_read_tokens,
                    COALESCE(SUM(agg.total_cost_usd), 0.0) AS total_cost_usd,
                    MAX(s.last_active_at) AS last_active_at
             FROM sessions s
             LEFT JOIN ({agg}) agg ON agg.session_id = s.id AND agg.device_id = s.device_id
             {clause}
             GROUP BY pid
             ORDER BY last_active_at DESC, pid",
            agg = super::aggregate_sql::usage_agg_subquery(false)
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            let tokens = TokenCounts {
                input: r.get::<_, i64>(3)? as u32,
                output: r.get::<_, i64>(4)? as u32,
                cache_creation: r.get::<_, i64>(5)? as u32,
                cache_read: r.get::<_, i64>(6)? as u32,
            };
            Ok(ProjectStatsRow {
                project_dir: r.get(0)?,
                session_count: r.get::<_, i64>(1)? as u32,
                request_count: r.get::<_, i64>(2)? as u32,
                // Both derived metrics reuse TokenCounts' single
                // implementations — the same ones the dashboard's stats and
                // per-model rows use (output is not in the hit-rate
                // denominator; the formula ignores it).
                total_tokens: tokens.total(),
                input_tokens: tokens.input,
                output_tokens: tokens.output,
                cache_creation_tokens: tokens.cache_creation,
                cache_read_tokens: tokens.cache_read,
                cache_hit_rate: tokens.cache_hit_rate(),
                total_cost_usd: r.get(7)?,
                last_active_at: r.get(8)?,
            })
        })?;
        let mut out = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)?;

        // ---- the synthetic unknown row (see method doc) ----
        if !filter.is_some_and(unknown_bucket_suppressed) {
            // Apply the filter's OTHER dimensions at usage grain — the SAME
            // assembly every direct usage read goes through
            // (push_usage_facets), the unknown sentinel leading. Time runs on
            // `u.timestamp` (no session row exists to read `last_active_at`
            // from); device / source / model map to their usage columns. Note
            // the model asymmetry: known buckets gate "session USED the model"
            // then sum its FULL usage, while the unknown bucket can only match
            // per-row (`u.model = ?`) — it has no session to gate on. The
            // project facet is not among them: the bucket IS the NOT EXISTS
            // form, not a filter over it.
            let (unknown_cond, _) = project_condition("u", UNKNOWN_PROJECT);
            let mut conds: Vec<String> = vec![unknown_cond];
            let mut uparams: Vec<SqlValue> = Vec::new();
            if let Some(f) = filter {
                push_usage_facets(
                    &mut conds,
                    &mut uparams,
                    "u.",
                    &usage_grain(f),
                    FacetGates::dropping(Facet::Project),
                );
            }
            let usql = format!(
                "SELECT COUNT(*), {sums}, COALESCE(MAX(u.timestamp),'')
                 FROM usage_records u WHERE {}",
                conds.join(" AND "),
                sums = super::aggregate_sql::usage_sum_cols("u.")
            );
            let (request_count, input, output, cc, cr, cost, last): (
                i64,
                i64,
                i64,
                i64,
                i64,
                f64,
                String,
            ) = conn.query_row(&usql, params_from_iter(uparams.iter()), |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?;
            if request_count > 0 {
                let tokens = TokenCounts {
                    input: input as u32,
                    output: output as u32,
                    cache_creation: cc as u32,
                    cache_read: cr as u32,
                };
                out.push(ProjectStatsRow {
                    project_dir: UNKNOWN_PROJECT.to_string(),
                    session_count: 0,
                    request_count: request_count as u32,
                    total_tokens: tokens.total(),
                    input_tokens: tokens.input,
                    output_tokens: tokens.output,
                    cache_creation_tokens: tokens.cache_creation,
                    cache_read_tokens: tokens.cache_read,
                    cache_hit_rate: tokens.cache_hit_rate(),
                    total_cost_usd: cost,
                    last_active_at: last,
                });
                // Keep the bucket ordering contract (recency desc, key asc)
                // over the appended row too.
                out.sort_by(|a, b| {
                    b.last_active_at
                        .cmp(&a.last_active_at)
                        .then_with(|| a.project_dir.cmp(&b.project_dir))
                });
            }
        }
        Ok(out)
    }

    /// The stats dimension at SESSION grain: every session (unpaged, list
    /// order) with its usage four-buckets / hit rate / cost, its
    /// `session_messages` row count, and its per-model token split. The
    /// sessions workbench consumes this one read for everything the paged
    /// list cannot answer — the left tree's node aggregates, the right rail's
    /// per-session and per-project cards, and the duration buckets. Same
    /// sources and rules as `query_project_stats` (live `usage_records`
    /// aggregates via a LEFT JOIN so usage-less sessions still appear;
    /// `project_identity` truncation at the decode seam) — only the grain
    /// differs, so the two dimensions can never disagree on a session's
    /// numbers (pinned by the three-reads-agree test below). The SQL emits
    /// one row per (session, model); the fold below merges them into one
    /// `SessionStatsRow` per session.
    pub fn query_session_stats(
        &self,
        filter: Option<&SessionFilter>,
    ) -> AppResult<Vec<SessionStatsRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_session_where(filter);
        let sql = format!(
            "SELECT s.id, s.device_id, s.source, s.project_dir,
                    COALESCE(NULLIF(s.custom_title,''), s.title_orig) AS title,
                    s.favorited, s.local_group_id, s.synced_group_id,
                    s.started_at, s.last_active_at, s.agent_type,
                    COALESCE(u.request_count, 0),
                    COALESCE(m.message_count, 0),
                    COALESCE(u.input_tokens, 0),
                    COALESCE(u.output_tokens, 0),
                    COALESCE(u.cache_creation_tokens, 0),
                    COALESCE(u.cache_read_tokens, 0),
                    COALESCE(u.total_cost_usd, 0.0),
                    u.model
             FROM sessions s
             LEFT JOIN ({uagg}) u ON u.session_id = s.id AND u.device_id = s.device_id
             LEFT JOIN (
                SELECT session_id, device_id, COUNT(*) AS message_count
                FROM session_messages GROUP BY session_id, device_id
             ) m ON m.session_id = s.id AND m.device_id = s.device_id
             {clause}
             ORDER BY s.last_active_at DESC, s.device_id, s.id, u.model",
            uagg = super::aggregate_sql::usage_agg_subquery(true)
        );
        let mut stmt = conn.prepare(&sql)?;
        let raw = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            // Columns 13-16 are COALESCE'd to 0 for the usage-less LEFT JOIN
            // row, so a plain read works; only u.model (18) is nullable.
            let input = r.get::<_, i64>(13)?;
            let output = r.get::<_, i64>(14)?;
            let cache_creation = r.get::<_, i64>(15)?;
            let cache_read = r.get::<_, i64>(16)?;
            let model: Option<String> = r.get(18)?;
            let project_dir: String = r.get(3)?;
            let row = SessionStatsRow {
                id: r.get(0)?,
                device_id: r.get(1)?,
                source: r.get(2)?,
                // Same decode-seam truncation as `session_row`: the identity
                // the list shows, so the tree buckets built on these rows
                // match the project aggregate.
                project_dir: project_identity(&project_dir).to_string(),
                title: r.get(4)?,
                favorited: r.get::<_, i64>(5)? != 0,
                local_group_id: r.get(6)?,
                synced_group_id: r.get(7)?,
                started_at: r.get(8)?,
                last_active_at: r.get(9)?,
                agent_type: r.get(10)?,
                request_count: r.get::<_, i64>(11)? as u32,
                message_count: r.get::<_, i64>(12)? as u32,
                input_tokens: input as u32,
                output_tokens: output as u32,
                cache_creation_tokens: cache_creation as u32,
                cache_read_tokens: cache_read as u32,
                cache_hit_rate: 0.0,
                total_cost_usd: r.get(17)?,
                models: Vec::new(),
            };
            let slice = SessionModelTokens {
                model: model.unwrap_or_default(),
                tokens: (input + output + cache_creation + cache_read) as u32,
            };
            Ok((row, slice))
        })?;
        let mut per_session: Vec<(SessionStatsRow, SessionModelTokens)> = raw
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)?;

        // Fold consecutive (session, model) rows into one row per session —
        // the ORDER BY keeps a session's model rows adjacent, and the fold's
        // key check makes an ordering regression surface as duplicate rows
        // instead of silently merged sessions. Bucket sums, request count and
        // cost accumulate across the model rows; message_count is identical
        // on every row of the session (the join is session-grain).
        let mut out: Vec<SessionStatsRow> = Vec::with_capacity(per_session.len());
        for (row, slice) in per_session.drain(..) {
            match out.last_mut() {
                Some(prev) if prev.id == row.id && prev.device_id == row.device_id => {
                    prev.request_count += row.request_count;
                    prev.message_count = row.message_count;
                    prev.input_tokens += row.input_tokens;
                    prev.output_tokens += row.output_tokens;
                    prev.cache_creation_tokens += row.cache_creation_tokens;
                    prev.cache_read_tokens += row.cache_read_tokens;
                    prev.total_cost_usd += row.total_cost_usd;
                    prev.models.push(slice);
                }
                _ => {
                    let mut row = row;
                    row.models.push(slice);
                    out.push(row);
                }
            }
        }
        for row in &mut out {
            let tokens = TokenCounts {
                input: row.input_tokens,
                output: row.output_tokens,
                cache_creation: row.cache_creation_tokens,
                cache_read: row.cache_read_tokens,
            };
            row.cache_hit_rate = tokens.cache_hit_rate();
            // Drop the usage-less phantom slice (empty model, zero tokens) so
            // a session without usage renders "no model data", not a blank row.
            row.models
                .retain(|m| !(m.model.is_empty() && m.tokens == 0));
            row.models.sort_by_key(|m| std::cmp::Reverse(m.tokens));
        }
        Ok(out)
    }

    /// The project dimension at USAGE grain (#106 dashboard): `usage_records`
    /// grouped by the owning session's `project_identity`, so the bucket sums
    /// equal `query_stats`'s totals under the same filter EXACTLY (time bounds
    /// run on usage timestamps — the hero's caliber; the sessions page's
    /// `query_project_stats` instead selects sessions by `last_active_at`, a
    /// sessions-grain caliber). Every usage row lands in exactly one bucket:
    /// rows with a session row map by the identity rule (the one
    /// `project_identity` UDF — same rule the filter applies), while
    /// session-less rows AND rows whose session carries no launch dir both
    /// fall to the synthetic [`UNKNOWN_PROJECT`] bucket — attribution missing
    /// either way, which is what「未知项目」means to the user. Note the
    /// project FILTER's sentinel stays the stricter NOT-EXISTS form, so
    /// picking the unknown bucket narrows to its session-less share only.
    pub fn query_project_usage(&self, filter: &UsageFilter) -> AppResult<Vec<ProjectUsageRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(filter, FacetGates::ALL, "usage_records");
        // The driving subquery keeps build_where's UNQUALIFIED column names
        // valid (no join ambiguity); the LEFT JOIN lives outside it. The
        // identity expression COALESCEs the NULL of a session-less row (the
        // UDF takes TEXT, not NULL) and folds the empty identity in with the
        // sentinel. `COUNT(DISTINCT a, b)` skips the NULL CASE arm, so only
        // buckets with a real session row count sessions. The projection is
        // the bare bucket columns (one list); the SUMs read them through the
        // shared prefix-parameterized list.
        let sql = format!(
            "SELECT COALESCE(NULLIF(project_identity(COALESCE(s.project_dir, '')), ''), '{UNKNOWN_PROJECT}') AS pid,
                    COUNT(*),
                    COUNT(DISTINCT CASE WHEN s.id IS NOT NULL
                         THEN sel.session_id || ':' || sel.device_id END),
                    {sums},
                    COALESCE({total},0) AS total_tokens,
                    MAX(sel.timestamp)
             FROM (
                SELECT session_id, device_id, timestamp, {buckets}, total_cost_usd
                FROM usage_records {clause}
             ) sel
             LEFT JOIN sessions s ON s.id = sel.session_id AND s.device_id = sel.device_id
             GROUP BY pid
             ORDER BY total_tokens DESC, pid",
            sums = super::aggregate_sql::usage_sum_cols("sel."),
            total = super::aggregate_sql::usage_total_sum("sel."),
            buckets = super::aggregate_sql::usage_bucket_cols("")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            let tokens = TokenCounts {
                input: r.get::<_, i64>(3)? as u32,
                output: r.get::<_, i64>(4)? as u32,
                cache_creation: r.get::<_, i64>(5)? as u32,
                cache_read: r.get::<_, i64>(6)? as u32,
            };
            let project: String = r.get(0)?;
            Ok(ProjectUsageRow {
                is_unknown: project == UNKNOWN_PROJECT,
                project,
                request_count: r.get::<_, i64>(1)? as u32,
                session_count: r.get::<_, i64>(2)? as u32,
                input_tokens: tokens.input,
                output_tokens: tokens.output,
                cache_creation_tokens: tokens.cache_creation,
                cache_read_tokens: tokens.cache_read,
                cache_hit_rate: tokens.cache_hit_rate(),
                total_cost_usd: r.get(7)?,
                total_tokens: r.get::<_, i64>(8)? as u32,
                last_active_at: r.get(9)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// The session dimension at usage grain (#106): `usage_records` grouped by
    /// `(session_id, device_id)`, INNER-joined to the sessions table — only
    /// sessions that EXIST in the store (本机采集 ∪ 拉回的远程收藏快照)
    /// appear; session-less usage is the project dimension's unknown bucket,
    /// not a phantom session here. `turn_count` merges a second GROUP BY over
    /// `turn_durations` under the facets that apply to the turn grain
    /// (time / device / project — model / source have no turn column, the
    /// same caliber note as `query_stats`), so the turn distribution keeps ONE
    /// caliber with the stats card instead of an approximation.
    pub fn query_session_usage(&self, filter: &UsageFilter) -> AppResult<Vec<SessionUsageRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(filter, FacetGates::ALL, "usage_records");
        let sql = format!(
            "SELECT sel.session_id, sel.device_id,
                    COALESCE(NULLIF(COALESCE(NULLIF(s.custom_title,''), s.title_orig),''), sel.session_id) AS title,
                    COALESCE(s.agent_type, ''),
                    s.started_at,
                    MAX(sel.timestamp), COUNT(*),
                    {sums}
             FROM (
                SELECT session_id, device_id, timestamp, {buckets}, total_cost_usd
                FROM usage_records {clause}
             ) sel
             JOIN sessions s ON s.id = sel.session_id AND s.device_id = sel.device_id
             GROUP BY sel.session_id, sel.device_id
             ORDER BY COALESCE({total},0) DESC, sel.session_id",
            sums = super::aggregate_sql::usage_sum_cols("sel."),
            buckets = super::aggregate_sql::usage_bucket_cols(""),
            total = super::aggregate_sql::usage_total_sum("sel.")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            Ok(SessionUsageRow {
                session_id: r.get(0)?,
                device_id: r.get(1)?,
                title: r.get(2)?,
                agent_type: r.get(3)?,
                started_at: r.get(4)?,
                last_active_at: r.get(5)?,
                request_count: r.get::<_, i64>(6)? as u32,
                input_tokens: r.get::<_, i64>(7)? as u32,
                output_tokens: r.get::<_, i64>(8)? as u32,
                cache_creation_tokens: r.get::<_, i64>(9)? as u32,
                cache_read_tokens: r.get::<_, i64>(10)? as u32,
                total_tokens: (r.get::<_, i64>(7)?
                    + r.get::<_, i64>(8)?
                    + r.get::<_, i64>(9)?
                    + r.get::<_, i64>(10)?) as u32,
                total_cost_usd: r.get(11)?,
                turn_count: 0,
            })
        })?;
        let mut out = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)?;

        // Per-session turn counts, merged by the same composite key. A
        // session may own turns but no usage in the window (or vice versa) —
        // the merge only touches rows both sides know.
        let (tclause, tparams) = build_where(filter, FacetGates::TURNS, "turn_durations");
        let tsql = format!(
            "SELECT session_id, device_id, COUNT(*) FROM turn_durations {tclause}
             GROUP BY session_id, device_id"
        );
        let mut tstmt = conn.prepare(&tsql)?;
        let turns = tstmt.query_map(params_from_iter(tparams.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? as u32,
            ))
        })?;
        let by_key: std::collections::HashMap<(String, String), u32> = turns
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)?
            .into_iter()
            .map(|(sid, dev, n)| ((sid, dev), n))
            .collect();
        for row in &mut out {
            if let Some(n) = by_key.get(&(row.session_id.clone(), row.device_id.clone())) {
                row.turn_count = *n;
            }
        }
        Ok(out)
    }

    /// The device dimension at USAGE grain (#107 dashboard): `usage_records`
    /// grouped by `device_id`, following `query_models`' pattern — every
    /// `UsageFilter` facet applies through the one WHERE builder (project
    /// included), so the bucket sums equal `query_stats`'s totals under the
    /// same filter EXACTLY. Only devices with usage in the window appear
    /// (GROUP BY omits empty buckets — a silent peer is invisible by design,
    /// the prototype's「未计入」). Device naming / "this machine" identity
    /// live in the registry; the frontend joins `list_devices` for them.
    pub fn query_device_usage(&self, filter: &UsageFilter) -> AppResult<Vec<DeviceUsageRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(filter, FacetGates::ALL, "usage_records");
        let sql = format!(
            "SELECT device_id,
                    COUNT(*),
                    {sums},
                    COALESCE({total},0) AS total_tokens,
                    MAX(timestamp)
             FROM usage_records {clause}
             GROUP BY device_id
             ORDER BY total_tokens DESC, device_id",
            sums = super::aggregate_sql::usage_sum_cols(""),
            total = super::aggregate_sql::usage_total_sum("")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            let tokens = TokenCounts {
                input: r.get::<_, i64>(2)? as u32,
                output: r.get::<_, i64>(3)? as u32,
                cache_creation: r.get::<_, i64>(4)? as u32,
                cache_read: r.get::<_, i64>(5)? as u32,
            };
            Ok(DeviceUsageRow {
                device_id: r.get(0)?,
                request_count: r.get::<_, i64>(1)? as u32,
                input_tokens: tokens.input,
                output_tokens: tokens.output,
                cache_creation_tokens: tokens.cache_creation,
                cache_read_tokens: tokens.cache_read,
                cache_hit_rate: tokens.cache_hit_rate(),
                total_cost_usd: r.get(6)?,
                total_tokens: r.get::<_, i64>(7)? as u32,
                last_active_at: r.get(8)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }
}

/// Session-attribute constraints session-less usage can never satisfy — a
/// favorited-only view, a group, or a search all read session columns the
/// unknown bucket doesn't have, so the synthetic row is suppressed entirely
/// rather than always-empty. `favorited = Some(false)` keeps it (session-less
/// is definitionally not favorited); a blank search is no constraint.
/// 纯函数直测（见 tests），此前埋在 query_project_stats 的闭包里零覆盖。
fn unknown_bucket_suppressed(f: &SessionFilter) -> bool {
    f.favorited == Some(true)
        || f.local_group_id.is_some()
        || f.synced_group_id.is_some()
        || f.search.as_deref().is_some_and(|s| !s.trim().is_empty())
}

/// The usage-grain view of a `SessionFilter` — the five fields the two filter
/// shapes share (time / device / model / source). The unknown bucket's direct
/// read goes through the SAME usage-grain assembly as every other usage read
/// ([`push_usage_facets`]), which takes the `UsageFilter` shape; this explicit
/// mapping is the one seam. 字段穷举而非 `..Default::default()`：给
/// `UsageFilter` 新增字段会让这个字面量编译失败——漏接未知桶在这里被编译
/// 器拦下，而不是静默漂移。`project` 恒 `None`：桶的「项目」就是其 NOT
/// EXISTS 定义，不是一层筛选；known 桶的项目语义归 build_session_where。
fn usage_grain(f: &SessionFilter) -> UsageFilter {
    UsageFilter {
        from_ts: f.from_ts.clone(),
        to_ts: f.to_ts.clone(),
        model: f.model.clone(),
        source: f.source.clone(),
        device_scope: f.device_scope.clone(),
        project: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;

    /// Seed one usage record bound to `sid` with explicit token buckets +
    /// cost (the project aggregate's inputs).
    fn bound_rec(s: &Store, uuid: &str, sid: &str, tokens: TokenCounts, cost: f64) {
        let mut r = rec(
            uuid,
            "2026-08-15",
            "glm-5.2",
            "dev",
            tokens.input,
            tokens.output,
            cost,
        );
        r.session_id = sid.into();
        r.tokens = tokens;
        s.ingest_marking_dirty(&[r]).unwrap();
    }

    /// The project dimension's core rollup: per project — session count,
    /// request count, token four-buckets, cost, and MAX(last_active_at) — with
    /// sessions that have NO usage still counting toward their bucket's
    /// session count (LEFT JOIN + COALESCE), and buckets ordered most-recent
    /// first. Cache-hit rate reuses `TokenCounts::cache_hit_rate` (the same
    /// single formula the dashboard and per-model rows read).
    #[test]
    fn project_stats_rolls_up_sessions_usage_and_recency_per_project() {
        let s = mem();
        seed_session_project(&s, "a1", "dev", "/proj/alpha", "2026-08-10T10:00:00.000Z");
        seed_session_project(&s, "a2", "dev", "/proj/alpha", "2026-08-12T10:00:00.000Z");
        seed_session_project(&s, "b1", "dev", "/proj/beta", "2026-08-11T10:00:00.000Z");
        bound_rec(
            &s,
            "u1",
            "a1",
            TokenCounts {
                input: 100,
                output: 20,
                cache_read: 70,
                cache_creation: 0,
            },
            1.0,
        );
        bound_rec(
            &s,
            "u2",
            "a2",
            TokenCounts {
                input: 50,
                output: 0,
                cache_read: 50,
                cache_creation: 0,
            },
            2.0,
        );

        let rows = s.query_project_stats(None).unwrap();
        assert_eq!(rows.len(), 2, "one bucket per project identity");
        // Most recent first: alpha (08-12) before beta (08-11).
        assert_eq!(rows[0].project_dir, "/proj/alpha");
        assert_eq!(rows[0].session_count, 2);
        assert_eq!(rows[0].request_count, 2);
        assert_eq!(rows[0].input_tokens, 150);
        assert_eq!(rows[0].output_tokens, 20);
        assert_eq!(rows[0].cache_read_tokens, 120);
        assert_eq!(rows[0].total_tokens, 290);
        assert!((rows[0].total_cost_usd - 3.0).abs() < 1e-9);
        // cache_read / (input + cache_creation + cache_read) = 120 / 270
        // (the cacheable pool includes cache_read itself).
        assert!((rows[0].cache_hit_rate - 120.0 / 270.0).abs() < 1e-9);
        assert_eq!(rows[0].last_active_at, "2026-08-12T10:00:00.000Z");

        // Beta has one session but zero usage: the bucket survives with
        // zeroed aggregates (the dimension describes where sessions ran).
        assert_eq!(rows[1].project_dir, "/proj/beta");
        assert_eq!(rows[1].session_count, 1);
        assert_eq!(rows[1].request_count, 0);
        assert_eq!(rows[1].total_tokens, 0);
        assert_eq!(rows[1].cache_hit_rate, 0.0);
        assert_eq!(rows[1].total_cost_usd, 0.0);
    }

    /// Worktree sessions aggregate under their PARENT project (issue #84's
    /// rule, applied by the `project_identity` SQL scalar at GROUP BY): the
    /// parent bucket absorbs the worktree session, its usage, and its newer
    /// last_active_at — while an unrelated project stays its own bucket.
    #[test]
    fn project_stats_collapses_worktree_sessions_into_parent() {
        let s = mem();
        seed_session_project(
            &s,
            "s-main",
            "dev",
            "D:\\Project\\O_CC_One",
            "2026-08-02T10:00:00.000Z",
        );
        seed_session_project(
            &s,
            "s-agent",
            "dev",
            "D:\\Project\\O_CC_One\\.claude\\worktrees\\agent-a10c476b",
            "2026-08-03T10:00:00.000Z",
        );
        seed_session_project(
            &s,
            "s-other",
            "dev",
            "D:\\Project\\Other",
            "2026-08-04T10:00:00.000Z",
        );
        bound_rec(
            &s,
            "u1",
            "s-main",
            TokenCounts {
                input: 10,
                output: 0,
                cache_read: 0,
                cache_creation: 0,
            },
            0.5,
        );
        bound_rec(
            &s,
            "u2",
            "s-agent",
            TokenCounts {
                input: 20,
                output: 0,
                cache_read: 0,
                cache_creation: 0,
            },
            1.5,
        );

        let rows = s.query_project_stats(None).unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r.project_dir.as_str())
                .collect::<Vec<_>>(),
            ["D:\\Project\\Other", "D:\\Project\\O_CC_One"],
            "two buckets: the worktree never forms its own"
        );
        let parent = rows.last().unwrap();
        assert_eq!(parent.session_count, 2, "main + worktree session");
        assert_eq!(parent.request_count, 2, "both sessions' usage landed");
        assert!((parent.total_cost_usd - 2.0).abs() < 1e-9);
        assert_eq!(
            parent.last_active_at, "2026-08-03T10:00:00.000Z",
            "MAX over the bucket incl. the worktree session"
        );
    }

    /// The unknown-project bucket (#100): usage with NO session row forms one
    /// synthetic [`UNKNOWN_PROJECT`] row instead of vanishing. Local legacy
    /// rows (empty session_id) and unresolvable session ids land there alike;
    /// a known project's bucket is untouched; `session_count` is 0 by
    /// definition and `last_active_at` is the MAX usage timestamp, so the
    /// bucket sorts by real recency.
    #[test]
    fn project_stats_appends_unknown_bucket_for_session_less_usage() {
        let s = mem();
        seed_session_project(&s, "a1", "dev", "/proj/alpha", "2026-08-10T10:00:00.000Z");
        bound_rec(
            &s,
            "u1",
            "a1",
            TokenCounts {
                input: 100,
                output: 0,
                cache_read: 0,
                cache_creation: 0,
            },
            1.0,
        );
        // Two flavors of session-less usage: a legacy row (empty session_id)
        // and a row whose session id resolves to no sessions row (the remote
        // shape — a peer's session that was never favorited, so no snapshot
        // was pulled).
        let mut legacy = rec("u2", "2026-08-15", "glm-5.2", "dev", 10, 0, 0.5);
        legacy.session_id = String::new();
        let mut remote = rec("u3", "2026-08-16", "glm-5.2", "peer", 20, 10, 1.5);
        remote.session_id = "never-pulled".into();
        s.ingest(&[legacy, remote]).unwrap();

        let rows = s.query_project_stats(None).unwrap();
        assert_eq!(rows.len(), 2, "alpha bucket + the synthetic unknown row");
        let unknown = rows
            .iter()
            .find(|r| r.project_dir == UNKNOWN_PROJECT)
            .expect("synthetic unknown row present");
        assert_eq!(
            unknown.session_count, 0,
            "no session rows exist by definition"
        );
        assert_eq!(unknown.request_count, 2);
        assert_eq!(unknown.input_tokens, 30);
        assert_eq!(unknown.output_tokens, 10);
        assert_eq!(unknown.total_tokens, 40);
        assert_eq!(unknown.total_cost_usd, 2.0);
        // cache_read / (input + cache_creation + cache_read) = 0 / 30 = 0.
        assert_eq!(unknown.cache_hit_rate, 0.0);
        assert_eq!(unknown.last_active_at, "2026-08-16T10:00:00.000Z");
        // Recency ordering: the unknown bucket (08-16) sorts before alpha
        // (last_active 08-10).
        assert_eq!(rows[0].project_dir, UNKNOWN_PROJECT);

        let alpha = rows
            .iter()
            .find(|r| r.project_dir == "/proj/alpha")
            .unwrap();
        assert_eq!(alpha.request_count, 1, "alpha's bucket untouched");

        // A favorited-only filter can never be satisfied by session-less
        // usage — the unknown row is suppressed, known buckets remain.
        let fav = SessionFilter {
            favorited: Some(true),
            ..Default::default()
        };
        let rows = s.query_project_stats(Some(&fav)).unwrap();
        assert!(
            rows.iter().all(|r| r.project_dir != UNKNOWN_PROJECT),
            "favorited-only suppresses the unknown bucket"
        );

        // A time window that excludes the session-less rows drops the bucket
        // entirely (filter applies before aggregation).
        let early = SessionFilter {
            to_ts: Some("2026-08-15T00:00:00.000Z".into()),
            ..Default::default()
        };
        let rows = s.query_project_stats(Some(&early)).unwrap();
        assert!(
            rows.iter().all(|r| r.project_dir != UNKNOWN_PROJECT),
            "window without session-less usage yields no unknown row"
        );
    }

    /// The cross-device shape the bucket exists for (#94): a peer's FAVORITED
    /// session arrives as a pulled snapshot → its usage lands under the
    /// snapshot's project; the same peer's NON-favorited session has no
    /// snapshot → its usage lands in the unknown bucket, not nowhere.
    #[test]
    fn project_stats_unknown_bucket_covers_remote_nonfavorited_usage() {
        let s = mem();
        let peer = "peerdev01";
        // Pulled favorite snapshot: session row for the peer, project /remote.
        s.import_session_snapshot(
            peer,
            &SessionSnapshotMeta {
                v: SESSION_SNAPSHOT_VERSION,
                id: "fav-1".into(),
                source: "claude_code".into(),
                project_dir: "/remote".into(),
                title_orig: "Favorited".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-12T10:00:00.000Z".into(),
                agent_type: String::new(),
                parent_session_id: String::new(),
                favorited: true,
                synced_group_id: String::new(),
            },
            &[],
        )
        .unwrap();
        let mut fav_usage = rec("ru1", "2026-08-15", "glm-5.2", peer, 100, 0, 1.0);
        fav_usage.session_id = "fav-1".into();
        let mut plain_usage = rec("ru2", "2026-08-15", "glm-5.2", peer, 40, 0, 4.0);
        plain_usage.session_id = "plain-1".into();
        s.ingest(&[fav_usage, plain_usage]).unwrap();

        let rows = s.query_project_stats(None).unwrap();
        let remote = rows.iter().find(|r| r.project_dir == "/remote").unwrap();
        assert_eq!(
            remote.request_count, 1,
            "favorited snapshot's usage bucketed"
        );
        let unknown = rows
            .iter()
            .find(|r| r.project_dir == UNKNOWN_PROJECT)
            .expect("non-favorited remote usage did not vanish");
        assert_eq!(unknown.request_count, 1);
        assert_eq!(unknown.input_tokens, 40);
    }

    /// The model / device / source facets narrow the unknown bucket with the
    /// SAME caliber as the known buckets（架构审查Ⅲ候选⑤的守卫测试，此前零
    /// 覆盖）：未知桶的直读与 build_where 共用 push_usage_facets，两侧口径
    /// 由同一函数决定——此测试把「共用」钉成行为。
    #[test]
    fn project_stats_unknown_bucket_facets_narrow_with_the_known_caliber() {
        let s = mem();
        seed_session_project(&s, "s1", "dev", "/proj/alpha", "2026-08-10T10:00:00.000Z");
        // known 侧：dev 的会话用 glm-5.2 / claude_code。
        let mut known = rec("u0", "2026-08-15", "glm-5.2", "dev", 10, 0, 1.0);
        known.session_id = "s1".into();
        // unknown 侧三行 session-less：dev+glm / peer+gpt-4o / dev+gpt-4o 且
        // source=gemini_cli——三行分别只被一个 facet 选中。
        let mut loose_glm = rec("l1", "2026-08-15", "glm-5.2", "dev", 20, 0, 2.0);
        loose_glm.session_id = "never-a".into();
        let mut loose_peer = rec("l2", "2026-08-15", "gpt-4o", "peer", 40, 0, 4.0);
        loose_peer.session_id = "never-b".into();
        let mut loose_gem = rec("l3", "2026-08-15", "gpt-4o", "dev", 80, 0, 8.0);
        loose_gem.source = "gemini_cli".into();
        loose_gem.session_id = "never-c".into();
        s.ingest(&[known, loose_glm, loose_peer, loose_gem])
            .unwrap();

        let bucket = |rows: &[ProjectStatsRow], pid: &str| {
            rows.iter()
                .find(|r| r.project_dir == pid)
                .cloned()
                .unwrap_or_else(|| panic!("bucket {pid} missing in {rows:?}"))
        };

        // 无筛选：unknown 桶收齐三行。
        let rows = s.query_project_stats(None).unwrap();
        assert_eq!(bucket(&rows, UNKNOWN_PROJECT).request_count, 3);

        // model facet：known 桶收 s1 的 glm 用量，unknown 桶只剩 glm 行。
        let rows = s
            .query_project_stats(Some(&SessionFilter {
                model: Some("glm-5.2".into()),
                ..Default::default()
            }))
            .unwrap();
        assert_eq!(bucket(&rows, "/proj/alpha").request_count, 1);
        let unknown = bucket(&rows, UNKNOWN_PROJECT);
        assert_eq!(unknown.request_count, 1, "只有 loose_glm 是 glm");
        assert_eq!(unknown.input_tokens, 20);

        // device facet：peer 上一无会话二无 known 用量——known 桶整体消失，
        // unknown 桶只剩 peer 的 session-less 行。
        let rows = s
            .query_project_stats(Some(&SessionFilter {
                device_scope: Some("peer".into()),
                ..Default::default()
            }))
            .unwrap();
        assert!(
            rows.iter().all(|r| r.project_dir == UNKNOWN_PROJECT),
            "alpha 的会话在 dev 上，被 device 门排除——只剩未知桶"
        );
        let unknown = bucket(&rows, UNKNOWN_PROJECT);
        assert_eq!(unknown.request_count, 1);
        assert_eq!(unknown.input_tokens, 40);

        // source facet：s1 的用量是 claude_code，会话行随之被排除；unknown
        // 桶只剩 gemini 行。
        let rows = s
            .query_project_stats(Some(&SessionFilter {
                source: Some("gemini_cli".into()),
                ..Default::default()
            }))
            .unwrap();
        assert!(
            rows.iter().all(|r| r.project_dir == UNKNOWN_PROJECT),
            "known 会话行全在 claude_code 名下"
        );
        let unknown = bucket(&rows, UNKNOWN_PROJECT);
        assert_eq!(unknown.request_count, 1);
        assert_eq!(unknown.input_tokens, 80);
    }

    /// 未知桶压制判定直测（架构审查Ⅲ候选⑤）：四个会话属性约束任一命中即
    /// 压制；`favorited = Some(false)` 与空白 search 不算约束。
    #[test]
    fn unknown_bucket_suppression_is_decided_by_session_only_constraints() {
        assert!(
            !unknown_bucket_suppressed(&SessionFilter::default()),
            "无约束不压制"
        );
        assert!(
            !unknown_bucket_suppressed(&SessionFilter {
                favorited: Some(false),
                ..Default::default()
            }),
            "favorited=false 保留未知桶（session-less 定义上未收藏）"
        );
        assert!(
            !unknown_bucket_suppressed(&SessionFilter {
                search: Some("   ".into()),
                ..Default::default()
            }),
            "空白 search 不算约束"
        );
        for suppressed in [
            SessionFilter {
                favorited: Some(true),
                ..Default::default()
            },
            SessionFilter {
                local_group_id: Some("lg".into()),
                ..Default::default()
            },
            SessionFilter {
                synced_group_id: Some("sg".into()),
                ..Default::default()
            },
            SessionFilter {
                search: Some("tokamak".into()),
                ..Default::default()
            },
        ] {
            assert!(unknown_bucket_suppressed(&suppressed));
        }
    }

    /// SessionFilter → UsageFilter 的接缝直测：五个共享轴逐字段搬运，
    /// `project` 恒 `None`（桶的项目身份 = NOT EXISTS，不是筛选）。
    #[test]
    fn usage_grain_maps_the_shared_axes_and_drops_project() {
        let f = SessionFilter {
            from_ts: Some("2026-08-01T00:00:00Z".into()),
            to_ts: Some("2026-08-27T00:00:00Z".into()),
            model: Some("glm-5.2".into()),
            source: Some("claude_code".into()),
            device_scope: Some("dev".into()),
            project: Some(UNKNOWN_PROJECT.into()),
            ..Default::default()
        };
        let u = usage_grain(&f);
        assert_eq!(u.from_ts, f.from_ts);
        assert_eq!(u.to_ts, f.to_ts);
        assert_eq!(u.model, f.model);
        assert_eq!(u.source, f.source);
        assert_eq!(u.device_scope, f.device_scope);
        assert!(u.project.is_none(), "项目不映射——桶自身即项目定义");
    }

    /// The SessionFilter project dimension: matching runs through
    /// `project_identity`, so filtering to the parent project returns BOTH the
    /// parent's own sessions and its worktree sessions — in the paged list,
    /// the sidebar counts, and the project aggregate alike. A project nobody
    /// ran in matches nothing.
    #[test]
    fn session_filter_project_matches_worktree_sessions_to_parent() {
        let s = mem();
        seed_session_project(
            &s,
            "s-main",
            "dev",
            "/proj/alpha",
            "2026-08-02T10:00:00.000Z",
        );
        seed_session_project(
            &s,
            "s-agent",
            "dev",
            "/proj/alpha/.claude/worktrees/agent-x",
            "2026-08-03T10:00:00.000Z",
        );
        seed_session_project(
            &s,
            "s-other",
            "dev",
            "/proj/beta",
            "2026-08-04T10:00:00.000Z",
        );

        let f = SessionFilter {
            project: Some("/proj/alpha".into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions_page(&SessionQuery {
                filter: Some(f.clone()),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["s-agent", "s-main"], "worktree matches the parent");

        let counts = s.count_sessions(Some(&f), "local").unwrap();
        assert_eq!(counts.total, 2);

        let buckets = s.query_project_stats(Some(&f)).unwrap();
        assert_eq!(
            buckets.len(),
            1,
            "the filter narrows the aggregate's buckets"
        );
        assert_eq!(buckets[0].project_dir, "/proj/alpha");
        assert_eq!(buckets[0].session_count, 2);

        // A project with no sessions matches nothing anywhere.
        let none = SessionFilter {
            project: Some("/proj/gone".into()),
            ..Default::default()
        };
        assert!(s
            .query_sessions_page(&SessionQuery {
                filter: Some(none.clone()),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .is_empty());
        assert_eq!(s.count_sessions(Some(&none), "local").unwrap().total, 0);
        assert!(s.query_project_stats(Some(&none)).unwrap().is_empty());
    }

    /// The sessions-side unknown sentinel: it matches sessions whose project
    /// identity is EMPTY (a session row exists but carries no launch dir) —
    /// the sessions face of the unknown bucket, mirroring the usage-side NOT
    /// EXISTS face. Worktree/sessioned projects never match it.
    #[test]
    fn session_filter_unknown_sentinel_matches_project_less_sessions() {
        let s = mem();
        seed_session_project(&s, "s1", "dev", "/proj/alpha", "2026-08-10T10:00:00.000Z");
        seed_session_project(&s, "s2", "dev", "", "2026-08-11T10:00:00.000Z");

        let f = SessionFilter {
            project: Some(UNKNOWN_PROJECT.into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions_page(&SessionQuery {
                filter: Some(f.clone()),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["s2"], "only the project-less session matches");

        // The paged list, the sidebar counts, and the project aggregate all
        // share the same clause — the aggregate narrows to the "" bucket.
        assert_eq!(s.count_sessions(Some(&f), "local").unwrap().total, 1);
        let buckets = s.query_project_stats(Some(&f)).unwrap();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].project_dir, "");
    }

    /// The stats dimension at session grain: one row per session (folded from
    /// its per-(session, model) SQL rows) carrying the session's identity, its
    /// usage four-buckets / hit rate / cost, its message count, and its
    /// per-model token split most-tokens-first. A session with NO usage still
    /// appears with zeroed aggregates and no phantom model slice.
    #[test]
    fn session_stats_folds_model_rows_per_session() {
        let s = mem();
        seed_session_project(&s, "a1", "dev", "/proj/alpha", "2026-08-10T10:00:00.000Z");
        seed_session_project(&s, "b1", "dev", "/proj/beta", "2026-08-11T10:00:00.000Z");
        // Two usage records on a1 across two models: the fold must sum the
        // buckets/cost/requests AND keep one model slice per model.
        bound_rec(
            &s,
            "u1",
            "a1",
            TokenCounts {
                input: 100,
                output: 20,
                cache_read: 70,
                cache_creation: 10,
            },
            1.0,
        );
        let mut r = rec("u2", "2026-08-15", "glm-5.2-air", "dev", 30, 0, 2.0);
        r.session_id = "a1".into();
        r.tokens = TokenCounts {
            input: 30,
            output: 0,
            cache_read: 60,
            cache_creation: 0,
        };
        s.ingest_marking_dirty(&[r]).unwrap();
        // Two transcript messages for a1 — the message count follows
        // session_messages, not usage_records.
        s.ingest_session_messages_marking_dirty(
            "dev",
            &[
                msg("m1", "a1", SessionMessageRole::User, "2026-07-13T10:00:00Z"),
                msg(
                    "m2",
                    "a1",
                    SessionMessageRole::Assistant,
                    "2026-07-13T10:00:01Z",
                ),
            ],
        )
        .unwrap();

        let rows = s.query_session_stats(None).unwrap();
        assert_eq!(rows.len(), 2, "one row per session, never per model");
        // List order: most recent first (b1 08-11 before a1 08-10).
        assert_eq!(rows[0].id, "b1");
        assert_eq!(rows[0].request_count, 0);
        assert_eq!(rows[0].message_count, 0);
        assert_eq!(rows[0].total_cost_usd, 0.0);
        assert_eq!(rows[0].cache_hit_rate, 0.0);
        assert!(
            rows[0].models.is_empty(),
            "usage-less session renders no phantom model slice"
        );

        let a1 = &rows[1];
        assert_eq!(a1.id, "a1");
        assert_eq!(a1.request_count, 2);
        assert_eq!(a1.message_count, 2);
        assert_eq!(a1.input_tokens, 130);
        assert_eq!(a1.output_tokens, 20);
        assert_eq!(a1.cache_creation_tokens, 10);
        assert_eq!(a1.cache_read_tokens, 130);
        assert!((a1.total_cost_usd - 3.0).abs() < 1e-9);
        // Same single formula the project grain reads:
        // cache_read / (input + cache_creation + cache_read) = 130/270.
        assert!((a1.cache_hit_rate - 130.0 / 270.0).abs() < 1e-9);
        assert_eq!(
            a1.models
                .iter()
                .map(|m| (m.model.as_str(), m.tokens))
                .collect::<Vec<_>>(),
            [("glm-5.2", 200), ("glm-5.2-air", 90)],
            "per-model slices, most-tokens-first, bucket sums intact"
        );
    }

    /// The session grain applies the same project-identity truncation as the
    /// list decode seam and groups nothing — but its `project_dir` output must
    /// match what the list shows, so a worktree session stats row carries the
    /// PARENT project (the tree buckets the frontend builds on top stay
    /// consistent with the project aggregate).
    #[test]
    fn session_stats_collapses_worktree_project_to_parent() {
        let s = mem();
        seed_session_project(
            &s,
            "s-agent",
            "dev",
            "D:\\Project\\O_CC_One\\.claude\\worktrees\\agent-a10c476b",
            "2026-08-03T10:00:00.000Z",
        );
        let rows = s.query_session_stats(None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project_dir, "D:\\Project\\O_CC_One");
    }

    /// The filter applies BEFORE grouping: a time range narrows which sessions
    /// feed the buckets, so a session excluded by the window drops out of both
    /// the session count and its usage out of the token/cost aggregates.
    #[test]
    fn project_stats_time_filter_narrows_buckets_before_grouping() {
        let s = mem();
        seed_session_project(
            &s,
            "a-old",
            "dev",
            "/proj/alpha",
            "2026-08-01T10:00:00.000Z",
        );
        seed_session_project(
            &s,
            "a-new",
            "dev",
            "/proj/alpha",
            "2026-08-10T10:00:00.000Z",
        );
        seed_session_project(&s, "b-mid", "dev", "/proj/beta", "2026-08-05T10:00:00.000Z");
        bound_rec(
            &s,
            "u1",
            "a-old",
            TokenCounts {
                input: 100,
                output: 0,
                cache_read: 0,
                cache_creation: 0,
            },
            1.0,
        );
        bound_rec(
            &s,
            "u2",
            "a-new",
            TokenCounts {
                input: 10,
                output: 0,
                cache_read: 0,
                cache_creation: 0,
            },
            0.5,
        );

        let from = SessionFilter {
            from_ts: Some("2026-08-04T00:00:00.000Z".into()),
            ..Default::default()
        };
        let rows = s.query_project_stats(Some(&from)).unwrap();
        assert_eq!(rows.len(), 2, "alpha keeps its newer session");
        let alpha = rows
            .iter()
            .find(|r| r.project_dir == "/proj/alpha")
            .unwrap();
        assert_eq!(alpha.session_count, 1, "a-old excluded by the window");
        assert_eq!(alpha.request_count, 1, "its usage dropped with it");
        assert!((alpha.total_cost_usd - 0.5).abs() < 1e-9);
    }

    /// 口径一致性（架构审查Ⅲ候选④）：同一 seeded 数据经三条读路径聚合相等
    /// ——会话列表行（query_sessions_page）、会话粒统计（query_session_stats）、
    /// 项目桶（query_project_stats）共享同一份聚合子查询与桶清单。
    /// `query_session_stats` 文档宣称 "the two dimensions can never disagree …
    /// only the grain differs"——这里把那句散文变成断言。
    #[test]
    fn sessions_page_stats_and_project_buckets_aggregate_identically() {
        let s = mem();
        seed_session_project(&s, "s1", "dev", "/proj/alpha", "2026-08-10T10:00:00.000Z");
        seed_session_project(&s, "s2", "dev", "/proj/alpha", "2026-08-12T10:00:00.000Z");
        seed_session_project(&s, "b1", "dev", "/proj/beta", "2026-08-11T10:00:00.000Z");
        let bound = |uuid: &str, sid: &str, model: &str, t: TokenCounts, cost: f64| {
            let mut r = rec(uuid, "2026-08-15", model, "dev", t.input, t.output, cost);
            r.session_id = sid.into();
            r.tokens = t;
            s.ingest_marking_dirty(&[r]).unwrap();
        };
        // s1 跨两模型、s2 / b1 各一模型，桶值互异——任何读路径间的错配都会
        // 显形，而不是恰好对上。
        bound(
            "u1",
            "s1",
            "glm-5.2",
            TokenCounts {
                input: 100,
                output: 20,
                cache_creation: 10,
                cache_read: 70,
            },
            1.0,
        );
        bound(
            "u2",
            "s1",
            "glm-5.2-air",
            TokenCounts {
                input: 30,
                output: 0,
                cache_creation: 60,
                cache_read: 0,
            },
            2.0,
        );
        bound(
            "u3",
            "s2",
            "glm-5.2",
            TokenCounts {
                input: 50,
                output: 5,
                cache_creation: 0,
                cache_read: 50,
            },
            0.5,
        );
        bound(
            "u4",
            "b1",
            "glm-5.2",
            TokenCounts {
                input: 7,
                output: 8,
                cache_creation: 9,
                cache_read: 10,
            },
            0.25,
        );

        let page: std::collections::HashMap<String, SessionRow> = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| (r.id.clone(), r))
            .collect();
        let stats: std::collections::HashMap<String, SessionStatsRow> = s
            .query_session_stats(None)
            .unwrap()
            .into_iter()
            .map(|r| (r.id.clone(), r))
            .collect();
        let buckets: std::collections::HashMap<String, ProjectStatsRow> = s
            .query_project_stats(None)
            .unwrap()
            .into_iter()
            .map(|r| (r.project_dir.clone(), r))
            .collect();

        // 会话粒两条路径逐会话相等：请求数 / 总 token / 成本。
        for sid in ["s1", "s2", "b1"] {
            let (p, st) = (&page[sid], &stats[sid]);
            assert_eq!(p.request_count, st.request_count, "{sid}: request_count");
            assert_eq!(
                p.total_tokens,
                st.input_tokens
                    + st.output_tokens
                    + st.cache_creation_tokens
                    + st.cache_read_tokens,
                "{sid}: total_tokens"
            );
            assert!(
                (p.total_cost_usd - st.total_cost_usd).abs() < 1e-9,
                "{sid}: total_cost_usd"
            );
        }
        assert_eq!(page["s1"].request_count, 2, "s1 跨两模型各一条");

        // 项目桶 = 成员会话之和（alpha 两会话、beta 一会话）。
        let alpha = &buckets["/proj/alpha"];
        assert_eq!(alpha.session_count, 2);
        assert_eq!(
            alpha.request_count,
            page["s1"].request_count + page["s2"].request_count
        );
        let sum_of = |f: fn(&SessionStatsRow) -> u32| -> u32 {
            ["s1", "s2"].iter().map(|sid| f(&stats[*sid])).sum()
        };
        assert_eq!(alpha.input_tokens, sum_of(|r| r.input_tokens));
        assert_eq!(alpha.output_tokens, sum_of(|r| r.output_tokens));
        assert_eq!(
            alpha.cache_creation_tokens,
            sum_of(|r| r.cache_creation_tokens)
        );
        assert_eq!(alpha.cache_read_tokens, sum_of(|r| r.cache_read_tokens));
        assert!(
            (alpha.total_cost_usd - (stats["s1"].total_cost_usd + stats["s2"].total_cost_usd))
                .abs()
                < 1e-9
        );
        let beta = &buckets["/proj/beta"];
        assert_eq!(beta.session_count, 1);
        assert_eq!(beta.request_count, page["b1"].request_count);
        assert_eq!(beta.input_tokens, stats["b1"].input_tokens);
    }

    /// Project buckets at usage grain (#106): sums equal the hero totals
    /// exactly, sessions map by the identity rule, and the sentinel merges
    /// session-less rows WITH rows whose session has no launch dir (both are
    /// attribution-missing). The filter's own sentinel stays the stricter
    /// NOT-EXISTS form.
    #[test]
    fn project_usage_buckets_match_stats_and_merge_unknown() {
        let s = mem();
        seed_session_project(&s, "s1", "d", "/proj/alpha", "2026-08-02T10:00:00.000Z");
        // No launch dir: its usage is attribution-missing too.
        seed_session_project(&s, "s2", "d", "", "2026-08-02T10:00:00.000Z");
        let bound = |uuid: &str, sid: &str, input: u32| {
            let mut r = rec(uuid, "2026-08-15", "glm-5.2", "d", input, 0, 1.0);
            r.session_id = sid.into();
            r
        };
        s.ingest(&[
            bound("a", "s1", 100),
            bound("b", "s2", 40),
            rec("loose", "2026-08-15", "glm-5.2", "d", 10, 0, 1.0),
        ])
        .unwrap();

        let rows = s.query_project_usage(&UsageFilter::default()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].project, "/proj/alpha");
        assert!(!rows[0].is_unknown);
        assert_eq!(rows[0].input_tokens, 100);
        assert_eq!(rows[0].session_count, 1);
        assert_eq!(rows[0].request_count, 1);
        // The sentinel bucket: s2's empty-identity share + the session-less
        // row — merged, 2 requests, but only ONE countable session (s2).
        assert_eq!(rows[1].project, UNKNOWN_PROJECT);
        assert!(rows[1].is_unknown);
        assert_eq!(rows[1].input_tokens, 50);
        assert_eq!(rows[1].request_count, 2);
        assert_eq!(rows[1].session_count, 1);
        // Bucket sums equal the stats totals exactly (the hero's caliber).
        let stats = s.query_stats(&UsageFilter::default()).unwrap();
        let sum: u32 = rows.iter().map(|r| r.input_tokens).sum();
        assert_eq!(sum, stats.input_tokens);

        // The filter's sentinel is stricter (NOT EXISTS): it narrows to the
        // session-less share only — 10 tokens, not the merged 50.
        let unknown = UsageFilter {
            project: Some(UNKNOWN_PROJECT.into()),
            ..Default::default()
        };
        let narrowed = s.query_project_usage(&unknown).unwrap();
        assert_eq!(narrowed.len(), 1);
        assert_eq!(narrowed[0].input_tokens, 10);
        assert_eq!(narrowed[0].session_count, 0);
    }

    /// Session buckets at usage grain (#106): one row per store-known session
    /// with usage, tokens-desc; session-less usage never appears (it belongs
    /// to the project dimension's unknown bucket); per-session turn counts
    /// merge under the turn grain's applicable facets.
    #[test]
    fn session_usage_buckets_and_turn_merge() {
        let s = mem();
        seed_session_project(&s, "s1", "d", "/proj/alpha", "2026-08-02T10:00:00.000Z");
        seed_session_project(&s, "s2", "d", "/proj/beta", "2026-08-02T10:00:00.000Z");
        let bound = |uuid: &str, sid: &str, input: u32| {
            let mut r = rec(uuid, "2026-08-15", "glm-5.2", "d", input, 0, 1.0);
            r.session_id = sid.into();
            r
        };
        s.ingest(&[
            bound("a", "s1", 100),
            bound("b", "s1", 50),
            bound("c", "s2", 70),
            rec("loose", "2026-08-15", "glm-5.2", "d", 10, 0, 1.0),
        ])
        .unwrap();
        let td = |uuid: &str, sid: &str, ms: u32| TurnDuration {
            uuid: uuid.into(),
            timestamp: "2026-08-15T10:00:00Z".into(),
            day: "2026-08-15".into(),
            session_id: sid.into(),
            device_id: "d".into(),
            duration_ms: ms,
        };
        s.ingest_turn_durations(&[
            td("t1", "s1", 10_000),
            td("t2", "s1", 20_000),
            td("t3", "", 5_000),
        ])
        .unwrap();

        let rows = s.query_session_usage(&UsageFilter::default()).unwrap();
        // s1 (150 tokens) before s2 (70); the session-less row never appears.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].session_id, "s1");
        assert_eq!(rows[0].input_tokens, 150);
        assert_eq!(rows[0].request_count, 2);
        assert_eq!(rows[0].turn_count, 2);
        assert_eq!(rows[1].session_id, "s2");
        assert_eq!(rows[1].input_tokens, 70);
        assert_eq!(rows[1].turn_count, 0);
        // Session rows carry the session's display fields.
        assert_eq!(rows[0].title, "Title");
        assert_eq!(rows[0].started_at, "2026-08-01T00:00:00.000Z");
        assert_eq!(rows[0].last_active_at, "2026-08-15T10:00:00.000Z");

        // Project facet narrows the buckets like every other usage read.
        let alpha = UsageFilter {
            project: Some("/proj/alpha".into()),
            ..Default::default()
        };
        let narrowed = s.query_session_usage(&alpha).unwrap();
        assert_eq!(narrowed.len(), 1);
        assert_eq!(narrowed[0].session_id, "s1");
    }

    /// Device buckets at usage grain (#107): GROUP BY device_id over the one
    /// WHERE builder — tokens-desc order, bucket sums equal the stats totals
    /// (the hero's caliber), cache hit rate from the one TokenCounts rule, and
    /// every facet narrows them (project through the session join, device
    /// scope, time). Devices with no usage in the window never appear.
    #[test]
    fn device_usage_buckets_match_stats_and_follow_filters() {
        let s = mem();
        seed_session_project(&s, "s1", "d1", "/proj/alpha", "2026-08-02T10:00:00.000Z");
        seed_session_project(&s, "s2", "d2", "/proj/beta", "2026-08-02T10:00:00.000Z");
        let bound = |uuid: &str, sid: &str, dev: &str, day: &str, input: u32, cache_read: u32| {
            let mut r = rec(uuid, day, "glm-5.2", dev, input, 0, 1.0);
            r.session_id = sid.into();
            r.tokens.cache_read = cache_read;
            r
        };
        // d1: 100 fresh + 50 cached reads on 08-15; d2: 300 on 08-14 — d2
        // outranks d1 by tokens; the cache hit rate exercises the shared
        // TokenCounts rule (50 / 150 for d1).
        s.ingest(&[
            bound("a", "s1", "d1", "2026-08-15", 100, 50),
            bound("b", "s2", "d2", "2026-08-14", 300, 0),
        ])
        .unwrap();

        let rows = s.query_device_usage(&UsageFilter::default()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].device_id, "d2", "tokens-desc order");
        assert_eq!(rows[0].total_tokens, 300);
        assert_eq!(rows[0].request_count, 1);
        assert_eq!(rows[0].last_active_at, "2026-08-14T10:00:00.000Z");
        assert_eq!(rows[1].device_id, "d1");
        assert_eq!(rows[1].total_tokens, 150);
        assert!((rows[1].cache_hit_rate - (50.0 / 150.0)).abs() < 1e-9);
        assert_eq!(rows[1].last_active_at, "2026-08-15T10:00:00.000Z");
        // Bucket sums equal the stats totals exactly (the hero's caliber).
        let stats = s.query_stats(&UsageFilter::default()).unwrap();
        let sum: u32 = rows.iter().map(|r| r.total_tokens).sum();
        assert_eq!(sum, stats.total_tokens);

        // Project facet narrows through the session join (alpha = d1's rows).
        let alpha = UsageFilter {
            project: Some("/proj/alpha".into()),
            ..Default::default()
        };
        let narrowed = s.query_device_usage(&alpha).unwrap();
        assert_eq!(narrowed.len(), 1);
        assert_eq!(narrowed[0].device_id, "d1");

        // Device scope + time facets narrow the same way.
        let by_device = UsageFilter {
            device_scope: Some("d2".into()),
            ..Default::default()
        };
        assert_eq!(
            s.query_device_usage(&by_device)
                .unwrap()
                .iter()
                .map(|r| r.device_id.as_str())
                .collect::<Vec<_>>(),
            vec!["d2"]
        );
        let later = UsageFilter {
            from_ts: Some("2026-08-15T00:00:00.000Z".into()),
            ..Default::default()
        };
        let rows = s.query_device_usage(&later).unwrap();
        assert_eq!(rows.len(), 1, "d2 has no usage on/after 08-15");
        assert_eq!(rows[0].device_id, "d1");
    }
}
