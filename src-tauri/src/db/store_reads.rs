//! Dashboard read paths (stats / trend / logs / models / distinct).

use super::*;

impl super::Store {
    // ---------------- Reads (dashboard) ----------------

    /// Aggregate stats over a filter (BLUEPRINT 使用统计).
    pub fn query_stats(&self, filter: &UsageFilter) -> AppResult<UsageStats> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(filter, true);
        let sql = format!(
            "SELECT
                COUNT(*),
                COALESCE(SUM(input_tokens),0),
                COALESCE(SUM(output_tokens),0),
                COALESCE(SUM(cache_creation_tokens),0),
                COALESCE(SUM(cache_read_tokens),0),
                COALESCE(SUM(CAST(total_cost_usd AS REAL)),0)
             FROM usage_records {clause}"
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
        // Per-turn aggregates (separate grain, from turn_durations).
        let (tclause, tparams) = build_where(filter, false);
        let tsql =
            format!("SELECT COUNT(*), COALESCE(AVG(duration_ms),0) FROM turn_durations {tclause}");
        let (turn_count, avg_dur): (i64, f64) =
            conn.query_row(&tsql, params_from_iter(tparams.iter()), |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        s.turn_count = turn_count as u32;
        s.avg_turn_duration_ms = avg_dur;
        Ok(s)
    }

    /// Per-model breakdown over a filter.
    pub fn query_models(&self, filter: &UsageFilter) -> AppResult<Vec<ModelStatsRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(filter, true);
        let sql = format!(
            "SELECT model,
                COUNT(*),
                COALESCE(SUM(input_tokens+output_tokens+cache_creation_tokens+cache_read_tokens),0),
                COALESCE(SUM(CAST(total_cost_usd AS REAL)),0),
                COALESCE(SUM(input_tokens),0),
                COALESCE(SUM(cache_creation_tokens),0),
                COALESCE(SUM(cache_read_tokens),0)
             FROM usage_records {clause}
             GROUP BY model ORDER BY 4 DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            // 缓存命中率复用 TokenCounts 的唯一实现 (与 query_stats 一致)。
            let cache = TokenCounts {
                input: r.get::<_, i64>(4)? as u32,
                output: 0,
                cache_creation: r.get::<_, i64>(5)? as u32,
                cache_read: r.get::<_, i64>(6)? as u32,
            };
            Ok(ModelStatsRow {
                model: r.get(0)?,
                request_count: r.get::<_, i64>(1)? as u32,
                total_tokens: r.get::<_, i64>(2)? as u32,
                total_cost_usd: r.get(3)?,
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
        let (clause, params_vec) = build_where(filter, true);
        // Hour buckets read the clock in the device's local zone so a UTC+8
        // "today" trends in hours the user recognizes; the day bucket stays on
        // the stored UTC `day` for cross-device determinism.
        let grouping: &str = match bucket {
            TrendBucket::Day => "day",
            TrendBucket::Hour => "strftime('%Y-%m-%dT%H', timestamp, 'localtime')",
        };
        let sql = format!(
            "SELECT {grouping} AS bucket,
                COALESCE(SUM(input_tokens),0),
                COALESCE(SUM(output_tokens),0),
                COALESCE(SUM(cache_creation_tokens),0),
                COALESCE(SUM(cache_read_tokens),0),
                COALESCE(SUM(CAST(total_cost_usd AS REAL)),0)
             FROM usage_records {clause}
             GROUP BY bucket ORDER BY bucket"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            let input: i64 = r.get(1)?;
            let output: i64 = r.get(2)?;
            let cc: i64 = r.get(3)?;
            let cr: i64 = r.get(4)?;
            Ok(TrendPoint {
                day: r.get(0)?,
                input_tokens: input as u32,
                output_tokens: output as u32,
                cache_creation_tokens: cc as u32,
                cache_read_tokens: cr as u32,
                total_tokens: (input + output + cc + cr) as u32,
                total_cost_usd: r.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Distinct sources/models present (for filter dropdowns).
    pub fn query_distinct(&self, column: &str) -> AppResult<Vec<String>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        // column is a fixed whitelist below, not user input — safe to interpolate.
        let col = match column {
            "source" => "source",
            "model" => "model",
            _ => return Err(AppError::Db("bad distinct column".into())),
        };
        let sql =
            format!("SELECT DISTINCT {col} FROM usage_records WHERE {col} != '' ORDER BY {col}");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Request-log rows (BLUEPRINT 请求日志; columns).
    pub fn query_logs(&self, q: &LogsQuery) -> AppResult<Vec<UsageLogRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(&q.filter, true);
        let limit = q.limit.clamp(1, 1000) as i64;
        let offset = q.offset as i64;
        let sql = format!(
            "SELECT uuid, timestamp, model, source, device_id,
                    input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens,
                    stop_reason, CAST(total_cost_usd AS REAL)
             FROM usage_records {clause}
             ORDER BY timestamp DESC LIMIT {limit} OFFSET {offset}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            Ok(UsageLogRow {
                uuid: r.get(0)?,
                timestamp: r.get(1)?,
                model: r.get(2)?,
                source: r.get(3)?,
                device_id: r.get(4)?,
                tokens: TokenCounts {
                    input: r.get::<_, i64>(5)? as u32,
                    output: r.get::<_, i64>(6)? as u32,
                    cache_creation: r.get::<_, i64>(7)? as u32,
                    cache_read: r.get::<_, i64>(8)? as u32,
                },
                stop_reason: r.get(9)?,
                total_cost_usd: r.get(10)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Total row count (for paging display).
    pub fn count_logs(&self, filter: &UsageFilter) -> AppResult<u32> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_where(filter, true);
        let sql = format!("SELECT COUNT(*) FROM usage_records {clause}");
        let n: i64 = conn.query_row(&sql, params_from_iter(params_vec.iter()), |r| r.get(0))?;
        Ok(n as u32)
    }
}

/// Build a `WHERE` clause + bound params for a `UsageFilter` (timestamp range,
/// model, source, device scope). The range filters on `timestamp` (UTC), not
/// `day` — see `UsageFilter` for why. Returns `("WHERE ...", vec![...])` or
/// `("", [])`.
fn build_where(filter: &UsageFilter, include_model_source: bool) -> (String, Vec<SqlValue>) {
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();
    if let Some(ts) = &filter.from_ts {
        if !ts.is_empty() {
            conds.push("timestamp >= ?".into());
            params.push(SqlValue::Text(ts.clone()));
        }
    }
    if let Some(ts) = &filter.to_ts {
        if !ts.is_empty() {
            conds.push("timestamp <= ?".into());
            params.push(SqlValue::Text(ts.clone()));
        }
    }
    if include_model_source {
        if let Some(m) = &filter.model {
            if !m.is_empty() {
                conds.push("model = ?".into());
                params.push(SqlValue::Text(m.clone()));
            }
        }
        if let Some(s) = &filter.source {
            if !s.is_empty() {
                conds.push("source = ?".into());
                params.push(SqlValue::Text(s.clone()));
            }
        }
    }
    if let Some(d) = &filter.device_scope {
        if !d.is_empty() {
            conds.push("device_id = ?".into());
            params.push(SqlValue::Text(d.clone()));
        }
    }
    let clause = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    (clause, params)
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
        s.ingest(&[
            rec("a", "2026-07-13", "glm-5.2", "d", 100, 0, 1.0),
            {
                let mut r = rec("b", "2026-07-13", "gpt-4o", "d", 50, 0, 2.0);
                r.tokens.cache_read = 50;
                r
            },
        ])
        .unwrap();
        let models = s.query_models(&UsageFilter::default()).unwrap();
        let by_model = |m: &str| models.iter().find(|x| x.model == m).unwrap();
        // cache_read / (input + cache_creation + cache_read) = 50 / 100.
        assert!((by_model("gpt-4o").cache_hit_rate - 0.5).abs() < 1e-9);
        // 纯输入、无任何缓存活动 → 0。
        assert_eq!(by_model("glm-5.2").cache_hit_rate, 0.0);
    }
}
