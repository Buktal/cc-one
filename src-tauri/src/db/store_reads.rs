//! Dashboard read paths (stats / trend / logs / models / distinct). 维度查询
//! （project / session / device × 两种粒度）归 [`super::store_dimensions`]。

use super::filter_sql::{build_where, Facet, FacetGates};
use super::store_sessions_reads::build_session_where;
use super::*;

/// The usage columns whose distinct values a filter-dropdown can list — the
/// whole whitelist, carried by the type so a wrong column is unrepresentable
/// instead of a runtime-rejected string. `Project` is deliberately absent:
/// project candidates come from the sessions-side registry
/// ([`Store::query_distinct_projects`]), not from a `usage_records` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistinctColumn {
    Source,
    Model,
}

impl DistinctColumn {
    /// The SQL column the variant reads, plus the filter facet whose OWN
    /// constraint the candidate list must drop (the dropdown-facet rule —
    /// see the `Facet` semantics in `super::filter_sql`).
    fn parts(self) -> (&'static str, Facet) {
        match self {
            DistinctColumn::Source => ("source", Facet::Source),
            DistinctColumn::Model => ("model", Facet::Model),
        }
    }
}

impl super::Store {
    // ---------------- Reads (dashboard) ----------------

    /// Aggregate stats over a filter (BLUEPRINT 使用统计).
    pub fn query_stats(&self, filter: &UsageFilter) -> AppResult<UsageStats> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(filter, FacetGates::ALL, "usage_records");
        let sql = format!(
            "SELECT
                COUNT(*),
                {sums}
             FROM usage_records {clause}",
            sums = super::aggregate_sql::usage_sum_cols("")
        );
        let row = conn.query_row(&sql, params_from_iter(params_vec.iter()), |r| {
            Ok(UsageStats {
                request_count: r.get::<_, i64>(0)? as u32,
                input_tokens: r.get::<_, i64>(1)? as u32,
                output_tokens: r.get::<_, i64>(2)? as u32,
                cache_creation_tokens: r.get::<_, i64>(3)? as u32,
                cache_read_tokens: r.get::<_, i64>(4)? as u32,
                total_cost_usd: r.get::<_, f64>(5)?,
                ..Default::default()
            })
        })?;
        let mut s = row;
        s.total_tokens = s
            .input_tokens
            .saturating_add(s.output_tokens)
            .saturating_add(s.cache_creation_tokens)
            .saturating_add(s.cache_read_tokens);
        let tokens = TokenCounts {
            input: s.input_tokens,
            output: s.output_tokens,
            cache_creation: s.cache_creation_tokens,
            cache_read: s.cache_read_tokens,
        };
        s.cache_hit_rate = tokens.cache_hit_rate();
        // Per-turn aggregates (separate grain, from turn_durations). Time /
        // device / project facets apply — `turn_durations` carries `session_id`
        // so the project facet resolves through the sessions table exactly
        // like the usage rows above (the unknown sentinel's NOT EXISTS
        // included). Model / source do NOT apply: the turn grain has no such
        // column, and gating per-row through the session's usage would be a
        // different (unclaimed) semantic. The aggregate carries the histogram
        // (`turn_duration_buckets`, [u32; 4]) and the p95 alongside count/avg.
        let (tclause, tparams) = build_where(filter, FacetGates::TURNS, "turn_durations");
        let tsql = format!(
            "SELECT COUNT(*), COALESCE(AVG(duration_ms),0),
                COALESCE(SUM(CASE WHEN duration_ms < 10000 THEN 1 ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN duration_ms >= 10000 AND duration_ms < 30000 THEN 1 ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN duration_ms >= 30000 AND duration_ms < 60000 THEN 1 ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN duration_ms >= 60000 THEN 1 ELSE 0 END),0)
             FROM turn_durations {tclause}"
        );
        let (turn_count, avg_dur, b1, b2, b3, b4): (i64, f64, i64, i64, i64, i64) = conn
            .query_row(&tsql, params_from_iter(tparams.iter()), |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?;
        s.turn_count = turn_count as u32;
        s.avg_turn_duration_ms = avg_dur;
        s.turn_duration_buckets = [b1 as u32, b2 as u32, b3 as u32, b4 as u32];
        // p95 = smallest duration whose cumulative share reaches 95% — index
        // ceil(0.95·n) − 1 into the ascending order. A second query (LIMIT 1
        // OFFSET) instead of materializing every duration.
        s.p95_turn_duration_ms = if turn_count > 0 {
            let idx = ((95 * turn_count + 99) / 100 - 1).max(0);
            let (pclause, pparams) = build_where(filter, FacetGates::TURNS, "turn_durations");
            let psql = format!(
                "SELECT duration_ms FROM turn_durations {pclause}
                 ORDER BY duration_ms LIMIT 1 OFFSET {idx}"
            );
            let ms: Option<i64> = conn
                .query_row(&psql, params_from_iter(pparams.iter()), |r| r.get(0))
                .optional()?;
            ms.map(|v| v as f64)
        } else {
            None
        };
        Ok(s)
    }

    /// Per-model breakdown over a filter.
    pub fn query_models(&self, filter: &UsageFilter) -> AppResult<Vec<ModelStatsRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(filter, FacetGates::ALL, "usage_records");
        // 列序：model / COUNT / 四桶总和 / 共享 SUM 清单（input..cost）。
        // output 走清单但不解码——ModelStatsRow 不携带它；占位是为让桶口径
        // 只有一份拼写（新增桶时这里与其它读同步）。
        let sql = format!(
            "SELECT model,
                COUNT(*),
                COALESCE({total},0),
                {sums}
             FROM usage_records {clause}
             GROUP BY model ORDER BY 3 DESC",
            total = super::aggregate_sql::usage_total_sum(""),
            sums = super::aggregate_sql::usage_sum_cols("")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            // 缓存命中率复用 TokenCounts 的唯一实现 (与 query_stats 一致)。
            let cache = TokenCounts {
                input: r.get::<_, i64>(3)? as u32,
                output: 0,
                cache_creation: r.get::<_, i64>(5)? as u32,
                cache_read: r.get::<_, i64>(6)? as u32,
            };
            Ok(ModelStatsRow {
                model: r.get(0)?,
                request_count: r.get::<_, i64>(1)? as u32,
                total_tokens: r.get::<_, i64>(2)? as u32,
                total_cost_usd: r.get(7)?,
                cache_hit_rate: cache.cache_hit_rate(),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Trend points over a filter (BLUEPRINT 使用趋势). `bucket` picks the
    /// granularity: `Day` groups on the UTC `day` column
    /// (cross-device deterministic); `Hour` groups on local-time hour for the
    /// single-day zoom where per-day resolution collapses to one bar. The
    /// TrendPoint `day` field carries the resolved bucket key.
    pub fn query_trend(
        &self,
        filter: &UsageFilter,
        bucket: TrendBucket,
    ) -> AppResult<Vec<TrendPoint>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(filter, FacetGates::ALL, "usage_records");
        // Hour buckets read the clock in the device's local zone so a UTC+8
        // "today" trends in hours the user recognizes; the day bucket stays on
        // the stored UTC `day` for cross-device determinism.
        let grouping: &str = match bucket {
            TrendBucket::Day => "day",
            TrendBucket::Hour => "strftime('%Y-%m-%dT%H', timestamp, 'localtime')",
        };
        let sql = format!(
            "SELECT {grouping} AS bucket,
                COUNT(*),
                {sums}
             FROM usage_records {clause}
             GROUP BY bucket ORDER BY bucket",
            sums = super::aggregate_sql::usage_sum_cols("")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            let input: i64 = r.get(2)?;
            let output: i64 = r.get(3)?;
            let cc: i64 = r.get(4)?;
            let cr: i64 = r.get(5)?;
            Ok(TrendPoint {
                day: r.get(0)?,
                request_count: r.get::<_, i64>(1)? as u32,
                input_tokens: input as u32,
                output_tokens: output as u32,
                cache_creation_tokens: cc as u32,
                cache_read_tokens: cr as u32,
                total_tokens: (input + output + cc + cr) as u32,
                total_cost_usd: r.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Distinct sources/models for a filter dropdown, narrowed by every OTHER
    /// dimension the user picked (time / device / the other facet) — never by
    /// this column itself, so picking "glm" doesn't shrink the model list to
    /// only "glm". Empty values are always excluded (legacy / unknown rows).
    pub fn query_distinct(
        &self,
        column: DistinctColumn,
        filter: &UsageFilter,
    ) -> AppResult<Vec<String>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        // The column is a fixed literal per variant (`DistinctColumn::parts`)
        // — safe to interpolate.
        let (col, own) = column.parts();
        // Facet semantics: the dropdown for one dimension ignores that
        // dimension's own filter (so any value stays pickable) but applies the
        // other facet + time + device — candidates reflect the selected window.
        let (mut clause, params_vec) =
            build_where(filter, FacetGates::dropping(own), "usage_records");
        // Always exclude empty values; splice onto the WHERE clause (or start one).
        if clause.is_empty() {
            clause = format!("WHERE {col} != ''");
        } else {
            clause = format!("{clause} AND {col} != ''");
        }
        let sql = format!("SELECT DISTINCT {col} FROM usage_records {clause} ORDER BY {col}");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            r.get::<_, String>(0)
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Distinct project candidates for the project dropdown (facet semantics:
    /// the filter's OWN project value is ignored — a picked project never
    /// shrinks its own candidate list). The known set is the sessions-side
    /// project registry — 本机全部会话 ∪ 远程收藏快照 (both live in the one
    /// `sessions` table; the only cross-device session rows are pulled
    /// favorite snapshots) — narrowed by the filter's OTHER dimensions through
    /// the sessions-side WHERE builder, whose fields map 1:1 onto
    /// `UsageFilter`'s (time reads `last_active_at`, the sessions grain). The
    /// empty identity never becomes a candidate (a session with no launch dir
    /// is not a pickable project). The unknown bucket's PRESENCE is probed on
    /// the usage side (session-less rows under the same other-dimensions
    /// window) — "unknown" is a property of usage, not of sessions.
    pub fn query_distinct_projects(&self, filter: &UsageFilter) -> AppResult<ProjectCandidates> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        // Known projects: sessions-side distinct identities, other dimensions
        // applied, own facet dropped.
        let session_filter = SessionFilter {
            from_ts: filter.from_ts.clone(),
            to_ts: filter.to_ts.clone(),
            model: filter.model.clone(),
            source: filter.source.clone(),
            device_scope: filter.device_scope.clone(),
            ..Default::default()
        };
        let (clause, params_vec) = build_session_where(Some(&session_filter));
        let sql = format!(
            "SELECT pid FROM (\
                SELECT DISTINCT project_identity(s.project_dir) AS pid FROM sessions s {clause}\
             ) WHERE pid != '' ORDER BY pid"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            r.get::<_, String>(0)
        })?;
        let projects = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)?;

        // Unknown presence: does any session-less usage row exist under the
        // same OTHER dimensions (time on usage timestamps — the unknown bucket
        // is a usage-side concept)?
        let (mut uclause, uparams) = build_where(
            filter,
            FacetGates::dropping(Facet::Project),
            "usage_records",
        );
        let unknown_cond = super::filter_sql::project_condition("usage_records", UNKNOWN_PROJECT).0;
        if uclause.is_empty() {
            uclause = format!("WHERE {unknown_cond}");
        } else {
            uclause = format!("{uclause} AND {unknown_cond}");
        }
        let has_unknown: i64 = conn.query_row(
            &format!("SELECT EXISTS (SELECT 1 FROM usage_records {uclause})"),
            params_from_iter(uparams.iter()),
            |r| r.get(0),
        )?;
        Ok(ProjectCandidates {
            projects,
            unknown: (has_unknown != 0).then(|| UNKNOWN_PROJECT.to_string()),
        })
    }

    /// Request-log rows (BLUEPRINT 请求日志; columns). Selects the full
    /// per-call field set — the row-detail panel reads from these rows, so
    /// expanding a row costs no extra round-trip. `server_tool_use` is a JSON
    /// text column; unknown/corrupt payloads fall back to zeros.
    pub fn query_logs(&self, q: &LogsQuery) -> AppResult<Vec<UsageLogRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(&q.filter, FacetGates::ALL, "usage_records");
        let limit = super::page_limit(q.limit);
        let offset = q.offset as i64;
        let sql = format!(
            "SELECT uuid, timestamp, model, pricing_model, source, session_id, device_id,
                    input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens,
                    stop_reason, service_tier, iterations, server_tool_use,
                    CAST(total_cost_usd AS REAL),
                    CAST(input_cost_usd AS REAL), CAST(output_cost_usd AS REAL),
                    CAST(cache_read_cost_usd AS REAL), CAST(cache_creation_cost_usd AS REAL)
             FROM usage_records {clause}
             ORDER BY timestamp DESC LIMIT {limit} OFFSET {offset}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            let tool_json: String = r.get(14)?;
            let server_tool_use =
                serde_json::from_str(&tool_json).unwrap_or(ServerToolUse::default());
            Ok(UsageLogRow {
                uuid: r.get(0)?,
                timestamp: r.get(1)?,
                model: r.get(2)?,
                pricing_model: r.get(3)?,
                source: r.get(4)?,
                session_id: r.get(5)?,
                device_id: r.get(6)?,
                tokens: TokenCounts {
                    input: r.get::<_, i64>(7)? as u32,
                    output: r.get::<_, i64>(8)? as u32,
                    cache_creation: r.get::<_, i64>(9)? as u32,
                    cache_read: r.get::<_, i64>(10)? as u32,
                },
                stop_reason: r.get(11)?,
                service_tier: r.get(12)?,
                iterations: r.get::<_, i64>(13)? as u32,
                server_tool_use,
                total_cost_usd: r.get(15)?,
                cost: LogCostBreakdown {
                    input_usd: r.get(16)?,
                    output_usd: r.get(17)?,
                    cache_read_usd: r.get(18)?,
                    cache_creation_usd: r.get(19)?,
                },
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Total row count (for paging display).
    pub fn count_logs(&self, filter: &UsageFilter) -> AppResult<u32> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(filter, FacetGates::ALL, "usage_records");
        let sql = format!("SELECT COUNT(*) FROM usage_records {clause}");
        let n: i64 = conn.query_row(&sql, params_from_iter(params_vec.iter()), |r| r.get(0))?;
        Ok(n as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;

    #[test]
    fn stats_and_trend_aggregate_over_records() {
        let s = mem();
        s.ingest(&[
            rec("a", "2026-07-13", "glm-5.2", "dev1", 100, 50, 1.0),
            rec("b", "2026-07-13", "glm-5.2", "dev1", 200, 100, 2.0),
            rec("c", "2026-07-14", "gpt-4o", "dev1", 300, 0, 3.0),
        ])
        .unwrap();

        let stats = s.query_stats(&UsageFilter::default()).unwrap();
        assert_eq!(stats.request_count, 3);
        assert_eq!(stats.total_tokens, 750);
        assert!((stats.total_cost_usd - 6.0).abs() < 1e-9);

        let trend = s
            .query_trend(&UsageFilter::default(), TrendBucket::Day)
            .unwrap();
        assert_eq!(trend.len(), 2);
        assert_eq!(trend[0].day, "2026-07-13");
        assert_eq!(trend[0].total_tokens, 450);
    }

    #[test]
    fn filters_by_timestamp_range_and_model() {
        let s = mem();
        s.ingest(&[
            rec("a", "2026-07-13", "glm-5.2", "d", 10, 0, 1.0),
            rec("b", "2026-07-14", "gpt-4o", "d", 20, 0, 2.0),
        ])
        .unwrap();
        // `b` lives at 2026-07-14T10:00Z; a from_ts at 2026-07-14T00:00Z
        // includes it and excludes `a` (2026-07-13T10:00Z). Range filters on
        // timestamp, never on the UTC `day` bucket (see UsageFilter).
        let from_ts = UsageFilter {
            from_ts: Some("2026-07-14T00:00:00.000Z".into()),
            ..Default::default()
        };
        assert_eq!(s.query_stats(&from_ts).unwrap().request_count, 1);
        let by_model = UsageFilter {
            model: Some("glm-5.2".into()),
            ..Default::default()
        };
        assert_eq!(s.query_stats(&by_model).unwrap().request_count, 1);
    }

    #[test]
    fn logs_ordered_desc_and_paged() {
        let s = mem();
        s.ingest(&[
            rec("a", "2026-07-13", "glm-5.2", "d", 1, 0, 1.0),
            rec("b", "2026-07-14", "glm-5.2", "d", 2, 0, 2.0),
        ])
        .unwrap();
        let q = LogsQuery {
            filter: UsageFilter::default(),
            limit: 10,
            offset: 0,
        };
        let logs = s.query_logs(&q).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].uuid, "b", "ORDER BY timestamp DESC");
        let q2 = LogsQuery {
            filter: UsageFilter::default(),
            limit: 10,
            offset: 1,
        };
        assert_eq!(s.query_logs(&q2).unwrap().len(), 1);
    }

    /// 越界 limit 走任一分页路径都被夹紧（#66：夹紧单一归属 page_limit）：
    /// 0 → 1（不空翻整页），超大 → 1000（不一次物化全表）。
    #[test]
    fn query_logs_clamps_out_of_range_limits() {
        let s = mem();
        s.ingest(&[
            rec("a", "2026-07-13", "glm-5.2", "d", 1, 0, 1.0),
            rec("b", "2026-07-14", "glm-5.2", "d", 2, 0, 2.0),
        ])
        .unwrap();
        for bad in [0, u32::MAX] {
            let logs = s
                .query_logs(&LogsQuery {
                    filter: UsageFilter::default(),
                    limit: bad,
                    offset: 0,
                })
                .unwrap();
            assert!(!logs.is_empty(), "limit={bad} 夹紧后仍返回行");
        }
    }

    #[test]
    fn models_breakdown_groups_by_model() {
        let s = mem();
        s.ingest(&[
            rec("a", "2026-07-13", "glm-5.2", "d", 100, 0, 1.0),
            rec("b", "2026-07-13", "gpt-4o", "d", 50, 0, 2.0),
        ])
        .unwrap();
        let models = s.query_models(&UsageFilter::default()).unwrap();
        assert_eq!(models.len(), 2);
        // 无缓存数据的模型命中率为 0。
        assert!(models.iter().all(|m| m.cache_hit_rate == 0.0));
    }

    #[test]
    fn models_breakdown_reports_cache_hit_rate() {
        let s = mem();
        s.ingest(&[rec("a", "2026-07-13", "glm-5.2", "d", 100, 0, 1.0), {
            let mut r = rec("b", "2026-07-13", "gpt-4o", "d", 50, 0, 2.0);
            r.tokens.cache_read = 50;
            r
        }])
        .unwrap();
        let models = s.query_models(&UsageFilter::default()).unwrap();
        let by_model = |m: &str| models.iter().find(|x| x.model == m).unwrap();
        // cache_read / (input + cache_creation + cache_read) = 50 / 100.
        assert!((by_model("gpt-4o").cache_hit_rate - 0.5).abs() < 1e-9);
        // 纯输入、无任何缓存活动 → 0。
        assert_eq!(by_model("glm-5.2").cache_hit_rate, 0.0);
    }

    #[test]
    fn distinct_models_narrow_by_window_and_ignore_own_filter() {
        let s = mem();
        s.ingest(&[
            rec("a", "2026-07-13", "glm-5.2", "d", 10, 0, 1.0),
            rec("b", "2026-07-14", "gpt-4o", "d", 20, 0, 2.0),
        ])
        .unwrap();

        // Window = day B only → only gpt-4o appears (the time range narrows it).
        let day_b = UsageFilter {
            from_ts: Some("2026-07-14T00:00:00.000Z".into()),
            to_ts: Some("2026-07-14T23:59:59.999Z".into()),
            ..Default::default()
        };
        assert_eq!(
            s.query_distinct(DistinctColumn::Model, &day_b).unwrap(),
            vec!["gpt-4o"]
        );

        // The model facet ignores its OWN filter: with model=glm picked over
        // the full range, BOTH models stay listed — a picked value never
        // shrinks its own dropdown, so the other one is always still pickable.
        let mut all_with_model = s
            .query_distinct(
                DistinctColumn::Model,
                &UsageFilter {
                    model: Some("glm-5.2".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        all_with_model.sort();
        assert_eq!(
            all_with_model,
            vec!["glm-5.2".to_string(), "gpt-4o".to_string()]
        );
    }

    #[test]
    fn distinct_facets_narrow_by_the_other_facet() {
        let s = mem();
        // glm-5.2 from gemini_cli; gpt-4o from claude_code (rec's default).
        let mut gem = rec("a", "2026-07-13", "glm-5.2", "d", 10, 0, 1.0);
        gem.source = "gemini_cli".into();
        let gpt = rec("b", "2026-07-13", "gpt-4o", "d", 20, 0, 2.0);
        s.ingest(&[gem, gpt]).unwrap();

        // Model dropdown narrowed by the OTHER facet (source=gemini_cli): only
        // the gemini model (glm-5.2) is listed.
        let by_src = UsageFilter {
            source: Some("gemini_cli".into()),
            ..Default::default()
        };
        assert_eq!(
            s.query_distinct(DistinctColumn::Model, &by_src).unwrap(),
            vec!["glm-5.2"]
        );

        // Source dropdown ignores its OWN filter: source=gemini_cli picked, yet
        // both sources remain pickable (symmetric to the model facet above).
        let mut srcs = s
            .query_distinct(
                DistinctColumn::Source,
                &UsageFilter {
                    source: Some("gemini_cli".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        srcs.sort();
        assert_eq!(
            srcs,
            vec!["claude_code".to_string(), "gemini_cli".to_string()]
        );
    }

    /// The usage-side project filter (identity 口径, aligned with the
    /// sessions side): a row matches when its session's `project_dir` maps to
    /// the project identity via the `project_identity` SQL scalar — so usage
    /// from a worktree session lands under the PARENT project in stats, logs,
    /// and counts alike, while a session-less row belongs to no project.
    #[test]
    fn usage_project_filter_buckets_worktree_usage_under_parent() {
        let s = mem();
        seed_session_project(&s, "s-main", "d", "/proj/alpha", "2026-08-02T10:00:00.000Z");
        seed_session_project(
            &s,
            "s-agent",
            "d",
            "/proj/alpha/.claude/worktrees/agent-x",
            "2026-08-03T10:00:00.000Z",
        );
        // One row per session + one session-less legacy row.
        let mut main = rec("u1", "2026-08-15", "glm-5.2", "d", 10, 0, 1.0);
        main.session_id = "s-main".into();
        let mut agent = rec("u2", "2026-08-15", "glm-5.2", "d", 20, 0, 2.0);
        agent.session_id = "s-agent".into();
        let loose = rec("u3", "2026-08-15", "glm-5.2", "d", 40, 0, 4.0);
        s.ingest(&[main, agent, loose]).unwrap();

        let alpha = UsageFilter {
            project: Some("/proj/alpha".into()),
            ..Default::default()
        };
        // Stats: both sessions' rows count (the worktree's included); the
        // session-less row matches no project.
        let stats = s.query_stats(&alpha).unwrap();
        assert_eq!(stats.request_count, 2);
        assert_eq!(stats.input_tokens, 30);
        assert!((stats.total_cost_usd - 3.0).abs() < 1e-9);
        // The log list and its count agree with stats (same WHERE builder).
        assert_eq!(s.count_logs(&alpha).unwrap(), 2);
        assert_eq!(
            s.query_logs(&LogsQuery {
                filter: alpha.clone(),
                limit: 10,
                offset: 0,
            })
            .unwrap()
            .len(),
            2
        );

        // Another project matches nothing; no constraint sees all three rows.
        let beta = UsageFilter {
            project: Some("/proj/beta".into()),
            ..Default::default()
        };
        assert_eq!(s.query_stats(&beta).unwrap().request_count, 0);
        assert_eq!(
            s.query_stats(&UsageFilter::default())
                .unwrap()
                .request_count,
            3
        );
    }

    /// The unknown-project sentinel on the usage side (#100): NOT EXISTS a
    /// session row — session-less legacy rows AND rows whose session id never
    /// arrived (remote, non-favorited) both match, while sessioned rows never
    /// do. Stats, the log list, and its count agree (same WHERE builder), and
    /// a known project and the sentinel partition the store.
    #[test]
    fn usage_project_filter_unknown_sentinel_matches_session_less_rows() {
        let s = mem();
        seed_session_project(&s, "s-main", "d", "/proj/alpha", "2026-08-02T10:00:00.000Z");
        let mut main = rec("u1", "2026-08-15", "glm-5.2", "d", 10, 0, 1.0);
        main.session_id = "s-main".into();
        // Session-less flavors: legacy (empty id) + remote (unresolvable id).
        let legacy = rec("u2", "2026-08-15", "glm-5.2", "d", 20, 0, 2.0);
        let mut remote = rec("u3", "2026-08-15", "glm-5.2", "peer", 40, 0, 4.0);
        remote.session_id = "never-pulled".into();
        s.ingest(&[main, legacy, remote]).unwrap();

        let unknown = UsageFilter {
            project: Some(UNKNOWN_PROJECT.into()),
            ..Default::default()
        };
        let stats = s.query_stats(&unknown).unwrap();
        assert_eq!(stats.request_count, 2, "both session-less rows match");
        assert_eq!(stats.input_tokens, 60);
        assert!((stats.total_cost_usd - 6.0).abs() < 1e-9);
        assert_eq!(s.count_logs(&unknown).unwrap(), 2);
        assert_eq!(
            s.query_logs(&LogsQuery {
                filter: unknown.clone(),
                limit: 10,
                offset: 0,
            })
            .unwrap()
            .len(),
            2
        );

        // A known project and the sentinel partition the store: alpha (1 row)
        // + unknown (2 rows) = the unconstrained total (3).
        let alpha = UsageFilter {
            project: Some("/proj/alpha".into()),
            ..Default::default()
        };
        assert_eq!(s.query_stats(&alpha).unwrap().request_count, 1);
        assert_eq!(
            s.query_stats(&UsageFilter::default())
                .unwrap()
                .request_count,
            3
        );
    }

    /// Distinct project candidates (#100): known projects = the sessions-side
    /// registry (local sessions ∪ peers' pulled favorite snapshots — one
    /// table), the own facet is ignored, the empty identity is never offered,
    /// and the unknown sentinel rides as data exactly when session-less usage
    /// exists in the window.
    #[test]
    fn distinct_projects_union_local_and_remote_with_window_gated_unknown() {
        let s = mem();
        seed_session_project(
            &s,
            "s-own",
            "dev",
            "/proj/alpha",
            "2026-08-10T10:00:00.000Z",
        );
        // A session with NO launch dir never becomes a candidate.
        seed_session_project(&s, "s-bare", "dev", "", "2026-08-11T10:00:00.000Z");
        // Peer's pulled favorite snapshot — the remote half of the union.
        s.import_session_snapshot(
            "peer1",
            &SessionSnapshotMeta {
                v: SESSION_SNAPSHOT_VERSION,
                id: "fav-1".into(),
                source: "claude_code".into(),
                project_dir: "/remote/beta".into(),
                title_orig: "Peer".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-12T00:00:00.000Z".into(),
                agent_type: String::new(),
                parent_session_id: String::new(),
                favorited: true,
                synced_group_id: String::new(),
            },
            &[],
        )
        .unwrap();
        // Session-less usage: recent (08-20) + old (07-01).
        let mut loose_new = rec("u-new", "2026-08-20", "glm-5.2", "d", 1, 0, 0.0);
        loose_new.session_id = String::new();
        let mut loose_old = rec("u-old", "2026-07-01", "glm-5.2", "d", 1, 0, 0.0);
        loose_old.session_id = String::new();
        s.ingest(&[loose_new, loose_old]).unwrap();

        // Unfiltered: both devices' identities, no empty string, unknown
        // present (session-less usage exists somewhere in the store).
        let c = s.query_distinct_projects(&UsageFilter::default()).unwrap();
        assert_eq!(
            c.projects,
            vec!["/proj/alpha".to_string(), "/remote/beta".to_string()]
        );
        assert_eq!(c.unknown.as_deref(), Some(UNKNOWN_PROJECT));

        // Facet semantics: the project dropdown ignores its OWN picked value —
        // picking alpha does not shrink the candidate list.
        let picked = UsageFilter {
            project: Some("/proj/alpha".into()),
            ..Default::default()
        };
        let c = s.query_distinct_projects(&picked).unwrap();
        assert_eq!(c.projects.len(), 2, "own facet ignored");

        // The window narrows: a range covering only the OLD session-less row
        // keeps unknown present; a range covering neither drops it.
        let old_window = UsageFilter {
            to_ts: Some("2026-07-02T00:00:00.000Z".into()),
            ..Default::default()
        };
        assert_eq!(
            s.query_distinct_projects(&old_window)
                .unwrap()
                .unknown
                .as_deref(),
            Some(UNKNOWN_PROJECT)
        );
        let mid_window = UsageFilter {
            from_ts: Some("2026-07-02T00:00:00.000Z".into()),
            to_ts: Some("2026-07-03T00:00:00.000Z".into()),
            ..Default::default()
        };
        assert_eq!(
            s.query_distinct_projects(&mid_window).unwrap().unknown,
            None,
            "no session-less usage in the window ⇒ option hidden"
        );

        // Device scope narrows to that device's registry: the peer's project
        // stays (its snapshot row lives here post-pull), a bare device with no
        // rows offers nothing.
        let peer_only = UsageFilter {
            device_scope: Some("peer1".into()),
            ..Default::default()
        };
        assert_eq!(
            s.query_distinct_projects(&peer_only).unwrap().projects,
            vec!["/remote/beta".to_string()]
        );
        let none = UsageFilter {
            device_scope: Some("ghost".into()),
            ..Default::default()
        };
        let c = s.query_distinct_projects(&none).unwrap();
        assert!(c.projects.is_empty());
        assert_eq!(c.unknown, None);
    }

    /// Per-turn aggregates follow the project filter (#101): turns carry
    /// `session_id`, so the turn count / average duration narrow with the
    /// project facet — known project via the session's identity, unknown
    /// sentinel via NOT EXISTS (legacy turns with an empty session id land
    /// there). Time and device facets keep applying as before.
    #[test]
    fn stats_turn_aggregates_follow_the_project_filter() {
        let s = mem();
        seed_session_project(&s, "s1", "d", "/proj/alpha", "2026-08-02T10:00:00.000Z");
        let td = |uuid: &str, sid: &str, ms: u32| TurnDuration {
            uuid: uuid.into(),
            timestamp: "2026-08-15T10:00:00Z".into(),
            day: "2026-08-15".into(),
            session_id: sid.into(),
            device_id: "d".into(),
            duration_ms: ms,
        };
        s.ingest_turn_durations(&[td("t1", "s1", 100_000), td("t2", "s1", 200_000)])
            .unwrap();
        // Legacy turns collected before the field existed ("" ⇒ unknown).
        s.ingest_turn_durations(&[td("t3", "", 50_000)]).unwrap();

        // Whole store: 3 turns, avg (100+200+50)/3 k = 116,666.67 ms.
        let all = s.query_stats(&UsageFilter::default()).unwrap();
        assert_eq!(all.turn_count, 3);

        // alpha narrows the turn set to s1's two turns.
        let alpha = UsageFilter {
            project: Some("/proj/alpha".into()),
            ..Default::default()
        };
        let stats = s.query_stats(&alpha).unwrap();
        assert_eq!(stats.turn_count, 2);
        assert!((stats.avg_turn_duration_ms - 150_000.0).abs() < 1e-9);

        // The sentinel picks up exactly the session-less legacy turn.
        let unknown = UsageFilter {
            project: Some(UNKNOWN_PROJECT.into()),
            ..Default::default()
        };
        let stats = s.query_stats(&unknown).unwrap();
        assert_eq!(stats.turn_count, 1);
        assert!((stats.avg_turn_duration_ms - 50_000.0).abs() < 1e-9);
        // alpha + unknown partition the turn set, matching the usage-side
        // semantics (same WHERE builder, driving table swapped).
        assert_eq!(stats.request_count, 0, "no usage rows seeded here");
    }

    /// The stats turn aggregate carries the duration histogram and p95 (#106):
    /// buckets split at 10s / 30s / 60s, p95 = smallest duration reaching the
    /// 95% cumulative share.
    #[test]
    fn stats_duration_buckets_and_p95() {
        let s = mem();
        let td = |uuid: &str, ms: u32| TurnDuration {
            uuid: uuid.into(),
            timestamp: "2026-08-15T10:00:00Z".into(),
            day: "2026-08-15".into(),
            session_id: String::new(),
            device_id: "d".into(),
            duration_ms: ms,
        };
        // 5s, 15s, 45s, 70s, 90s → one per bucket; avg = 45s.
        s.ingest_turn_durations(&[
            td("t1", 5_000),
            td("t2", 15_000),
            td("t3", 45_000),
            td("t4", 70_000),
            td("t5", 90_000),
        ])
        .unwrap();
        let stats = s.query_stats(&UsageFilter::default()).unwrap();
        assert_eq!(stats.turn_duration_buckets, [1, 1, 1, 2]);
        // ceil(0.95·5) − 1 = index 4 of the ascending order → the max, 90s.
        assert_eq!(stats.p95_turn_duration_ms, Some(90_000.0));
        assert!((stats.avg_turn_duration_ms - 45_000.0).abs() < 1e-9);

        // No turn rows → no p95, zero buckets.
        let from = UsageFilter {
            from_ts: Some("2027-01-01T00:00:00.000Z".into()),
            ..Default::default()
        };
        let empty = s.query_stats(&from).unwrap();
        assert_eq!(empty.p95_turn_duration_ms, None);
        assert_eq!(empty.turn_duration_buckets, [0, 0, 0, 0]);
    }

    /// Trend points carry the per-bucket request count (#106): each day's
    /// bars count the usage rows that landed in it.
    #[test]
    fn trend_carries_request_counts() {
        let s = mem();
        s.ingest(&[
            rec("a", "2026-07-13", "glm-5.2", "d", 10, 0, 1.0),
            rec("b", "2026-07-13", "glm-5.2", "d", 20, 0, 2.0),
            rec("c", "2026-07-14", "gpt-4o", "d", 30, 0, 3.0),
        ])
        .unwrap();
        let trend = s
            .query_trend(&UsageFilter::default(), TrendBucket::Day)
            .unwrap();
        assert_eq!(trend[0].request_count, 2);
        assert_eq!(trend[1].request_count, 1);
    }
}
