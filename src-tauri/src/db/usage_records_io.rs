//! `usage_records` 行位形（wire protocol）的唯一归属：列清单 + 绑定编码 +
//! 行解码同居一处（架构审查Ⅳ候选⑥）。
//!
//! 列序即 wire 协议：[`USAGE_RECORDS_COLNAMES`] 定列次序，[`bind`] 按这次序
//! 把 [`UsageRecord`] 编码成绑定值，[`decode`] 按同一次序把行读回来。**三者
//! 必须同 diff 改**——加 / 删 / 换序列，一处都不能漏。位形对齐是编译期裁决：
//! [`bind`] 返回定长数组 `[SqlValue; USAGE_RECORDS_COL_COUNT]`，数组长度由列
//! 清单推导，少绑 / 多绑一列直接编不过；decode 侧由 struct 字面量的字段穷举
//! 性（`UsageRecord` 加字段必改）和 bind→SQLite→decode 往返测试钉住。INSERT
//! （`store_ingest`）、脏日重算的 SELECT、复合主键重建的 `INSERT…SELECT`
//! 投影都订购 [`USAGE_RECORDS_COLNAMES`] 这一份列清单。
//!
//! 口径边界：本模块只管原始整行的往返；桶 SUM 行的解码与聚合投影拼法归
//! [`super::aggregate_sql`]，建表 DDL 归 [`super::schema`]。

use super::*;

/// `usage_records` 全部列名，逗号拼接。次序即 wire 协议——[`bind`] /
/// [`decode`] 的位序契约，也是行级 INSERT / SELECT / 主键重建投影共用的列
/// 清单单源。原先与建表 DDL 同居 `schema`；DDL 只关心列的存在与类型，位序
/// 只被行 I/O 关心，故随 bind / decode 归入本模块。
pub(super) const USAGE_RECORDS_COLNAMES: &str = "\
    uuid, timestamp, day, model, pricing_model, source, session_id, device_id, \
    input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, \
    server_tool_use, stop_reason, service_tier, iterations, \
    input_cost_usd, output_cost_usd, cache_read_cost_usd, \
    cache_creation_cost_usd, total_cost_usd";

/// 列数：由 [`USAGE_RECORDS_COLNAMES`] 编译期推导（逗号数 + 1），作为
/// [`bind`] 返回数组的定长——清单列数与绑定值个数的对齐由编译器裁决，不靠
/// 散文约定或事后测试。
const USAGE_RECORDS_COL_COUNT: usize = count_cols(USAGE_RECORDS_COLNAMES);

/// 数一个逗号拼接的列清单里有几列。
const fn count_cols(colnames: &str) -> usize {
    let bytes = colnames.as_bytes();
    let mut n = 1;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b',' {
            n += 1;
        }
        i += 1;
    }
    n
}

/// [`UsageRecord`] → 按列次序排列的绑定值。wire 格式：字符串列存 TEXT 原文、
/// token / iterations 存 INTEGER（u32 → i64）、`server_tool_use` 存 JSON
/// TEXT、成本列存 `Decimal` 的字符串形式 TEXT。返回定长数组就是位形自检：
/// 元素个数必须等于 [`USAGE_RECORDS_COL_COUNT`]（由列清单推导），往清单里
/// 加一列而没在这里补一个绑定值是编译错误，而非静默错位。调用方以
/// `params_from_iter(bind(r))` 交给 rusqlite。
pub(super) fn bind(r: &UsageRecord) -> [SqlValue; USAGE_RECORDS_COL_COUNT] {
    [
        SqlValue::Text(r.uuid.clone()),
        SqlValue::Text(r.timestamp.clone()),
        SqlValue::Text(r.day.clone()),
        SqlValue::Text(r.model.clone()),
        SqlValue::Text(r.pricing_model.clone()),
        SqlValue::Text(r.source.clone()),
        SqlValue::Text(r.session_id.clone()),
        SqlValue::Text(r.device_id.clone()),
        SqlValue::Integer(r.tokens.input as i64),
        SqlValue::Integer(r.tokens.output as i64),
        SqlValue::Integer(r.tokens.cache_creation as i64),
        SqlValue::Integer(r.tokens.cache_read as i64),
        SqlValue::Text(serde_json::to_string(&r.server_tool_use).unwrap_or_else(|_| "{}".into())),
        SqlValue::Text(r.stop_reason.clone()),
        SqlValue::Text(r.service_tier.clone()),
        SqlValue::Integer(r.iterations as i64),
        SqlValue::Text(r.cost.input_usd.to_string()),
        SqlValue::Text(r.cost.output_usd.to_string()),
        SqlValue::Text(r.cost.cache_read_usd.to_string()),
        SqlValue::Text(r.cost.cache_creation_usd.to_string()),
        SqlValue::Text(r.cost.total_usd.to_string()),
    ]
}

/// `usage_records` 行 → [`UsageRecord`]：按 [`USAGE_RECORDS_COLNAMES`] 的位序
/// 读回，是 [`bind`] 的逆，行投影必须恰是 `SELECT {USAGE_RECORDS_COLNAMES}`
/// 的形状。容错语义与编码对称：成本 TEXT 解回 Decimal（坏值归 0）、
/// `server_tool_use` JSON 解回结构（坏值归默认）——读路径永不因一个坏列
/// 丢整行，写进去什么就读回什么。
pub(super) fn decode(r: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRecord> {
    use std::str::FromStr;
    let dec =
        |s: String| rust_decimal::Decimal::from_str(&s).unwrap_or(rust_decimal::Decimal::ZERO);
    let total = dec(r.get::<_, String>(20)?);
    Ok(UsageRecord {
        uuid: r.get(0)?,
        timestamp: r.get(1)?,
        day: r.get(2)?,
        model: r.get(3)?,
        pricing_model: r.get(4)?,
        source: r.get(5)?,
        session_id: r.get(6)?,
        device_id: r.get(7)?,
        tokens: TokenCounts {
            input: r.get::<_, i64>(8)? as u32,
            output: r.get::<_, i64>(9)? as u32,
            cache_creation: r.get::<_, i64>(10)? as u32,
            cache_read: r.get::<_, i64>(11)? as u32,
        },
        server_tool_use: serde_json::from_str(&r.get::<_, String>(12)?)
            .unwrap_or(crate::model::ServerToolUse::default()),
        stop_reason: r.get(13)?,
        service_tier: r.get(14)?,
        iterations: r.get::<_, i64>(15)? as u32,
        cost: crate::model::CostBreakdown {
            input_usd: dec(r.get::<_, String>(16)?),
            output_usd: dec(r.get::<_, String>(17)?),
            cache_read_usd: dec(r.get::<_, String>(18)?),
            cache_creation_usd: dec(r.get::<_, String>(19)?),
            total_usd: total,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CostBreakdown;

    fn sentinel() -> UsageRecord {
        UsageRecord {
            uuid: "sentinel-uuid-001".into(),
            timestamp: "2026-07-13T12:34:56Z".into(),
            day: "2026-07-13".into(),
            model: "model-sentinel".into(),
            pricing_model: "pricing-sentinel".into(),
            source: "source-sentinel".into(),
            session_id: "session-sentinel".into(),
            device_id: "dev-sentinel".into(),
            tokens: TokenCounts {
                input: 123,
                output: 456,
                cache_creation: 78,
                cache_read: 90,
            },
            server_tool_use: ServerToolUse {
                web_search: 7,
                web_fetch: 8,
            },
            stop_reason: "stop-sentinel".into(),
            service_tier: "tier-sentinel".into(),
            iterations: 42,
            cost: CostBreakdown {
                input_usd: "1.11".parse().unwrap(),
                output_usd: "2.22".parse().unwrap(),
                cache_read_usd: "3.33".parse().unwrap(),
                cache_creation_usd: "4.44".parse().unwrap(),
                total_usd: "11.10".parse().unwrap(),
            },
        }
    }

    /// 位序契约的机器裁决（编译期只裁个数，这里裁「位置 i 的值就是清单第 i
    /// 列的编码」）：逐列核对 INTEGER / TEXT 的分布，串化列抽查 wire 形状——
    /// 任何错位或格式漂移在此显形，而不是静默写歪一列。
    #[test]
    fn bind_matches_colnames_position_and_wire_format() {
        let r = sentinel();
        let bound = bind(&r);
        let names: Vec<&str> = USAGE_RECORDS_COLNAMES.split(',').map(str::trim).collect();
        assert_eq!(bound.len(), names.len(), "bind arity == colnames arity");
        for (i, name) in names.iter().enumerate() {
            let integer = matches!(
                *name,
                "input_tokens"
                    | "output_tokens"
                    | "cache_creation_tokens"
                    | "cache_read_tokens"
                    | "iterations"
            );
            if integer {
                assert!(
                    matches!(bound[i], SqlValue::Integer(_)),
                    "{name} must bind INTEGER, got {:?}",
                    bound[i]
                );
            } else {
                assert!(
                    matches!(bound[i], SqlValue::Text(_)),
                    "{name} must bind TEXT, got {:?}",
                    bound[i]
                );
            }
        }
        // 串化格式抽查：server_tool_use 是该结构的 JSON，成本是 Decimal 串。
        let tool_json = match &bound[12] {
            SqlValue::Text(s) => s,
            other => panic!("server_tool_use must bind TEXT, got {other:?}"),
        };
        assert_eq!(
            serde_json::from_str::<ServerToolUse>(tool_json).unwrap(),
            r.server_tool_use
        );
        assert_eq!(bound[20], SqlValue::Text("11.10".into()));
    }

    /// decode 是 bind 的逆：bind → 真实 SQLite → SELECT 清单 → decode，哨兵
    /// 逐字段相等。decode 的下标位序由此钉住（生产路径 ingest →
    /// usage_for_day_device 的整体往返由 store_ingest 的哨兵测试守）。
    #[test]
    fn decode_inverts_bind_through_real_sqlite() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(&schema::schema_tables_sql()).unwrap();
        let placeholders = (1..=USAGE_RECORDS_COL_COUNT)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        conn.execute(
            &format!(
                "INSERT INTO usage_records ({USAGE_RECORDS_COLNAMES}) VALUES ({placeholders})"
            ),
            params_from_iter(bind(&sentinel())),
        )
        .unwrap();
        let out: UsageRecord = conn
            .query_row(
                &format!("SELECT {USAGE_RECORDS_COLNAMES} FROM usage_records"),
                [],
                decode,
            )
            .unwrap();
        assert_eq!(out, sentinel(), "decode must invert bind field for field");
    }
}
