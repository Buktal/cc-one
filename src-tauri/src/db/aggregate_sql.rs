//! usage 聚合的共享 SQL 片段 + 桶位序的编码/解码原语（架构审查Ⅲ候选④；
//! 架构审查Ⅳ候选⑤补齐三条横切面）。
//!
//! WHERE 侧（「非空才约束」样板 / 项目条件 / UNKNOWN 哨兵）归
//! [`super::filter_sql`]；本模块归 SELECT 侧 + 行解码侧：四个 token 桶列名
//! 的单一清单，以及由它派生的全部拼法——四桶+成本 SUM 列清单、四桶总和
//! 表达式、`(session_id, device_id[, model])` 聚合子查询、usage 粒维度读的
//! 驱动子查询。另两条「聚合正确性」横切面也唯一归属于此：会话复合身份
//! `(session_id, device_id)` 的 JOIN 谓词 / 键折叠（[`session_pair_join`] /
//! [`session_pair_key`]），以及桶 SUM 行的解码（[`read_bucket_sums`]）。
//! 此前「按 (session_id, device_id) 聚合」的子查询三处独立编码、四桶+成本
//! 清单约七处逐字拼写、复合键谓词五路手写、解码侧手抄 `r.get(n)` 位序：
//! 新增第五个 token 桶要同步约十条 SQL，漏一处静默少算。现在编码与解码
//! 同居本模块；桶增删 = 改这里的清单，两侧同点跟上。

use super::*;

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
/// （解码位序由这次序决定——解码原语 [`read_bucket_sums`] 同居本模块）。
/// 直读 `usage_records`（或其行投影子查询）的聚合 SELECT 统一接它；
/// `COUNT(*)` 等查询自有列由调用方排在前后。成本在 SUM 内 CAST 成
/// REAL——TEXT 存储的 Decimal 只在这一处换算。
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

/// usage 粒维度读（`query_project_usage` / `query_session_usage`）的驱动
/// 子查询：`usage_records` 经调用方的 WHERE 收窄后，投影出会话键对 +
/// `timestamp` + 四桶裸列 + 成本，以 `sel` 别名交付。「先投影行、外层再
/// 聚合」的形态让 [`super::filter_sql::build_where`] 拼出的无列前缀条件
/// 在带 JOIN 的外层查询里保持合法（子查询内无歧义列名）。列序契约：
/// `session_id, device_id, timestamp` 在前、四桶按 [`TOKEN_BUCKET_COLS`]
/// 次序、`total_cost_usd` 收尾——外层对 `sel.*` 的引用（[`usage_sum_cols`]
/// 的 `sel.` 前缀、`MAX(sel.timestamp)`）与 [`read_bucket_sums`] 的偏移都
/// 押在这个别名和列序上。`clause` 只接受 `build_where` 的产物（`"WHERE …"`
/// 或空串），不是用户输入。对 sessions 的 JOIN 方向（LEFT/INNER）是维度
/// 真差异，归调用方。
pub(super) fn usage_driver_subquery(clause: &str) -> String {
    format!(
        "(SELECT session_id, device_id, timestamp, {buckets}, total_cost_usd \
           FROM usage_records {clause}) sel",
        buckets = usage_bucket_cols("")
    )
}

/// 会话复合身份 `(session_id, device_id)` 的 JOIN 谓词——这对列是全库的
/// 隐式会话协议（usage 行、聚合子查询、`session_messages`、
/// `turn_durations` 都携带它），本模块是它唯一的代码归属。`pair_alias`
/// 侧携带键对，贴到 `sessions_alias`（`sessions` 表）的 `id` + `device_id`
/// 上——两侧列名不对称（`session_id` vs `id`），正是手抄最易错位的点。
/// 两列必须同时相等：session id 是解析器的文件名词干，跨设备会撞名，
/// 单列相等即错配。别名都是调用方的固定字面量。
pub(super) fn session_pair_join(pair_alias: &str, sessions_alias: &str) -> String {
    format!(
        "{pair_alias}.session_id = {sessions_alias}.id \
         AND {pair_alias}.device_id = {sessions_alias}.device_id"
    )
}

/// 同一复合身份的单别名键折叠（`{alias}.session_id || ':' ||
/// {alias}.device_id`）：`COUNT(DISTINCT …)` 只收单值，数「有多少个真实
/// 会话」时用它把键对折成一个串。形状与 [`session_pair_join`] 不同构——
/// 那是双别名匹配谓词、这是单别名投影——但语义同源：身份的列构成变动时
/// 两个拼法同点改。`||` 遇 NULL 得 NULL，要跳过 NULL 成员由调用方 CASE
/// 包裹（折叠只用于计数/投影，永远不做匹配）。
pub(super) fn session_pair_key(alias: &str) -> String {
    format!("{alias}.session_id || ':' || {alias}.device_id")
}

/// 桶 SUM 行的解码原语：从行内 `offset` 列起，按 [`TOKEN_BUCKET_COLS`]
/// 次序读四个 i64 列成 [`TokenCounts`]。与编码侧（[`usage_sum_cols`] /
/// [`usage_driver_subquery`]）同居本模块——桶增删或换序时两侧同点修改，
/// 调用方只表达自己的前导列数（`offset`），不再各自手抄 `r.get(n)` 位序。
/// total / 命中率等派生口径仍归 [`TokenCounts`] 本尊（`total()` /
/// `cache_hit_rate()`），此处不重复。只接受「桶 SUM 清单」行形的行——
/// 原始 `usage_records` 行（请求日志 / rebill / push 重算）的裸列投影不
/// 在此列。
pub(super) fn read_bucket_sums(
    r: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<TokenCounts> {
    Ok(TokenCounts {
        input: r.get::<_, i64>(offset)? as u32,
        output: r.get::<_, i64>(offset + 1)? as u32,
        cache_creation: r.get::<_, i64>(offset + 2)? as u32,
        cache_read: r.get::<_, i64>(offset + 3)? as u32,
    })
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

    /// 驱动子查询的形状契约：键对 + timestamp 在前、四桶按清单次序、成本
    /// 收尾、`sel` 别名收口；WHERE 原样拼在 `usage_records` 之后（空 clause
    /// 也不许吞掉别名）——外层的 `sel.*` 引用与解码偏移全押在这次序上。
    #[test]
    fn driver_subquery_keeps_the_sel_alias_and_column_order() {
        let q = usage_driver_subquery("");
        assert_eq!(
            q,
            "(SELECT session_id, device_id, timestamp, input_tokens, output_tokens, \
             cache_creation_tokens, cache_read_tokens, total_cost_usd \
             FROM usage_records ) sel"
        );
        let narrowed = usage_driver_subquery("WHERE model = ?");
        assert!(
            narrowed.ends_with("FROM usage_records WHERE model = ?) sel"),
            "{narrowed}"
        );
    }

    /// 复合身份两个拼法的形状契约：JOIN 谓词两侧列名不对称（键对侧
    /// `session_id`、sessions 侧 `id`）；键折叠是单别名投影。消费方拿到的
    /// 文本由这里唯一决定。
    #[test]
    fn session_pair_forms_pin_the_asymmetric_column_names() {
        assert_eq!(
            session_pair_join("agg", "s"),
            "agg.session_id = s.id AND agg.device_id = s.device_id"
        );
        assert_eq!(
            session_pair_join("sel", "sessions"),
            "sel.session_id = sessions.id AND sel.device_id = sessions.device_id"
        );
        assert_eq!(
            session_pair_key("sel"),
            "sel.session_id || ':' || sel.device_id"
        );
    }

    /// 解码原语的偏移契约：从 `offset` 起按桶次序读四列——与 `usage_sum_cols`
    /// 的产出位序互为镜像。
    #[test]
    fn bucket_sums_decode_four_columns_from_the_given_offset() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let tokens = conn
            .query_row("SELECT 10, 20, 30, 40, 50, 60", [], |r| {
                read_bucket_sums(r, 1)
            })
            .unwrap();
        assert_eq!(
            tokens,
            TokenCounts {
                input: 20,
                output: 30,
                cache_creation: 40,
                cache_read: 50,
            }
        );
    }
}
