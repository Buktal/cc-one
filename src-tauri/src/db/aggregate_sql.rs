//! SELECT 侧 usage 聚合的共享 SQL 片段（架构审查Ⅲ候选④）。
//!
//! WHERE 侧（「非空才约束」样板 / 项目条件 / UNKNOWN 哨兵）归
//! [`super::filter_sql`]；本模块归 SELECT 侧：四个 token 桶列名的单一清单，
//! 以及由它派生的拼法——四桶+成本 SUM 列清单、四桶总和表达式、
//! `(session_id, device_id[, model])` 聚合子查询。此前「按 (session_id,
//! device_id) 聚合」的子查询三处独立编码、四桶+成本清单约七处逐字拼写：
//! 新增第五个 token 桶或改 total 口径要同步约十条 SQL，漏一处静默少算。
//! 现在桶清单只此一份；桶增删 = 改这里的清单 + 各消费方的解码位。

/// 四个 token 桶在 `usage_records` 上的列名，按全库统一次序（input /
/// output / cache creation / cache read）。本模块所有拼法（裸投影 / SUM
/// 清单 / 总和 / 聚合子查询）都由它派生——第五个桶从这里长出来。
pub(super) const TOKEN_BUCKET_COLS: [&str; 4] = [
    "input_tokens",
    "output_tokens",
    "cache_creation_tokens",
    "cache_read_tokens",
];

/// 带列前缀的四桶裸投影（`sel.input_tokens, sel.output_tokens, …`）。
/// 「先投影行、外层再聚合」的查询（`query_project_usage` /
/// `query_session_usage` 的驱动子查询）以此拼列，桶清单在这种形态下也只有
/// 一份拼写。`prefix` 是调用方的表别名，固定字面量，不是用户输入。
pub(super) fn usage_bucket_cols(prefix: &str) -> String {
    TOKEN_BUCKET_COLS
        .iter()
        .map(|c| format!("{prefix}{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// 带列前缀的四桶 + 成本 SUM 列清单，次序 = 桶次序 + `total_cost_usd` 收尾
/// （解码位序由这次序决定）。直读 `usage_records`（或其行投影子查询）的
/// 聚合 SELECT 统一接它；`COUNT(*)` 等查询自有列由调用方排在前后。成本在
/// SUM 内 CAST 成 REAL——TEXT 存储的 Decimal 只在这一处换算。
pub(super) fn usage_sum_cols(prefix: &str) -> String {
    let mut cols: Vec<String> = TOKEN_BUCKET_COLS
        .iter()
        .map(|c| format!("COALESCE(SUM({prefix}{c}),0)"))
        .collect();
    cols.push(format!(
        "COALESCE(SUM(CAST({prefix}total_cost_usd AS REAL)),0)"
    ));
    cols.join(", ")
}

/// 四桶总和的单个 SUM 表达式（`SUM(<p>input_tokens + …)`），作用在原始行
/// 上。不带 COALESCE：GROUP BY 组内行集非空、四列 NOT NULL，SUM 恒非 NULL，
/// 空桶的归零由调用方包 COALESCE。
pub(super) fn usage_total_sum(prefix: &str) -> String {
    let summands: Vec<String> = TOKEN_BUCKET_COLS
        .iter()
        .map(|c| format!("{prefix}{c}"))
        .collect();
    format!("SUM({})", summands.join(" + "))
}

/// 四桶逐列 COALESCE 后相加的表达式，作用在已聚合的桶列上（如聚合子查询
/// 的 `agg.input_tokens`）——LEFT JOIN 未命中时逐桶归 0，命中时与
/// [`usage_total_sum`] 同值（整数加法，和的分解= 分解的和）。同一 total
/// 口径在两种行源上的形态：SUM 在原始行上、本函数在聚合列上。
pub(super) fn usage_total_of_cols(prefix: &str) -> String {
    TOKEN_BUCKET_COLS
        .iter()
        .map(|c| format!("COALESCE({prefix}{c},0)"))
        .collect::<Vec<_>>()
        .join(" + ")
}

/// `usage_records` 按 `(session_id, device_id)`（`with_model` 时再加
/// `model`）聚合的规范子查询：键列、`COUNT(*) AS request_count`、四桶
/// `SUM(col) AS col`、`SUM(CAST(… AS REAL)) AS total_cost_usd`。会话侧读
/// 路径 LEFT JOIN 的聚合源只此一份——分组键差异（±model）是参数，桶口径
/// 不再各自拼写。组内 SUM 恒非 NULL（见 [`usage_total_sum`]），外层 JOIN
/// 未命中的归零由调用方 COALESCE。
pub(super) fn usage_agg_subquery(with_model: bool) -> String {
    let key = if with_model { ", model" } else { "" };
    let sums: Vec<String> = TOKEN_BUCKET_COLS
        .iter()
        .map(|c| format!("SUM({c}) AS {c}"))
        .collect();
    format!(
        "SELECT session_id, device_id{key}, COUNT(*) AS request_count, \
         {sums}, SUM(CAST(total_cost_usd AS REAL)) AS total_cost_usd \
         FROM usage_records GROUP BY session_id, device_id{key}",
        sums = sums.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SUM 清单的次序契约：桶次序 + 成本收尾，前缀逐列生效——解码位序由它
    /// 决定，消费方不许本地重排。
    #[test]
    fn sum_cols_follow_bucket_order_with_prefix_on_every_column() {
        assert_eq!(
            usage_sum_cols(""),
            "COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0), \
             COALESCE(SUM(cache_creation_tokens),0), COALESCE(SUM(cache_read_tokens),0), \
             COALESCE(SUM(CAST(total_cost_usd AS REAL)),0)"
        );
        assert_eq!(
            usage_sum_cols("sel."),
            "COALESCE(SUM(sel.input_tokens),0), COALESCE(SUM(sel.output_tokens),0), \
             COALESCE(SUM(sel.cache_creation_tokens),0), COALESCE(SUM(sel.cache_read_tokens),0), \
             COALESCE(SUM(CAST(sel.total_cost_usd AS REAL)),0)"
        );
    }

    /// 总和与投影两个派生拼法覆盖且仅覆盖桶清单——新增桶时它们自动跟上。
    #[test]
    fn total_and_projection_derivations_cover_the_bucket_list() {
        for prefix in ["", "u.", "agg."] {
            let buckets: Vec<String> = TOKEN_BUCKET_COLS
                .iter()
                .map(|c| format!("{prefix}{c}"))
                .collect();
            assert_eq!(usage_bucket_cols(prefix), buckets.join(", "));
            assert_eq!(
                usage_total_sum(prefix),
                format!("SUM({})", buckets.join(" + "))
            );
            let coalesced: Vec<String> =
                buckets.iter().map(|c| format!("COALESCE({c},0)")).collect();
            assert_eq!(usage_total_of_cols(prefix), coalesced.join(" + "));
        }
    }

    /// 聚合子查询的形状契约：键列在前（model 可选）、COUNT 与四桶别名同名、
    /// GROUP BY 键与 SELECT 键一致——外层 JOIN 按 `agg.<列名>` 读，别名漂移
    /// 即断。
    #[test]
    fn agg_subquery_keys_and_groupby_stay_aligned() {
        let q = usage_agg_subquery(false);
        assert!(q.contains("SELECT session_id, device_id, COUNT(*) AS request_count"));
        assert!(!q.contains("model"), "无 model 键时不出现 model：{q}");
        assert!(q.contains("FROM usage_records GROUP BY session_id, device_id"));
        assert!(q.contains("SUM(input_tokens) AS input_tokens"));
        assert!(q.contains("SUM(CAST(total_cost_usd AS REAL)) AS total_cost_usd"));

        let qm = usage_agg_subquery(true);
        assert!(qm.contains("SELECT session_id, device_id, model, COUNT(*)"));
        assert!(qm.ends_with("GROUP BY session_id, device_id, model"));
    }
}
