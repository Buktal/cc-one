//! 筛选条件的共享 SQL 构建规则（架构审查候选④收口 WHERE 样板；架构审查Ⅲ
//! 候选⑤下沉 usage 粒装配）。
//!
//! 归属内容：
//! - [`push_nonempty_eq`] / [`push_ts_range`]：「值非空才约束」的筛选样板；
//! - [`project_condition`]：usage-粒直读表（`usage_records` /
//!   `turn_durations`）上的项目维度条件——known 身份走 `project_identity`
//!   UDF、[`UNKNOWN_PROJECT`] 哨兵走 NOT EXISTS 反转；
//! - [`push_usage_facets`] / [`FacetGates`] / [`build_where`]：usage 粒
//!   （时间 / model / source / device）条件装配 + facet 门控 + 完整 WHERE
//!   构建——`store_reads` 的直读与 `store_dimensions` 未知桶的直读共用，
//!   新增 usage 粒 facet 时两处同时获得，known/unknown 口径不再靠人眼同步
//!   （#94/#100 的事故形态）。
//!
//! 消费方：`store_reads` 的 `UsageFilter` 构建、`store_dimensions` 的维度
//! 查询与其未知桶的 usage-粒直读、`store_sessions_reads` 的 `SessionFilter`
//! 会话粒构建。会话粒特有的语义差异刻意不并入本模块（known 按 session
//! USED model 门控的 EXISTS 形式 vs 行级直等、时间列 last_active_at 与
//! timestamp 属调用方契约），只收敛真正同形的部分。

use super::*;

/// 值为 `Some` 且非空才追加 `column = ?` 并绑定参数。device / source / model
/// 等 Option<String> 筛选轴的统一判定：`None` 与空串都表示「不限」。
pub(super) fn push_nonempty_eq(
    conds: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    column: &str,
    value: &Option<String>,
) {
    if let Some(v) = value {
        if !v.is_empty() {
            conds.push(format!("{column} = ?"));
            params.push(SqlValue::Text(v.clone()));
        }
    }
}

/// 时间区间「两端各自非空才约束」：追加 `from_col >= ?` / `to_col <= ?`。
/// 列全名由调用方给出（`timestamp`、`u.timestamp`、`s.last_active_at`…）
/// ——别名与列名是查询方的契约，本模块不猜。
pub(super) fn push_ts_range(
    conds: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    column: &str,
    from_ts: &Option<String>,
    to_ts: &Option<String>,
) {
    if let Some(ts) = from_ts {
        if !ts.is_empty() {
            conds.push(format!("{column} >= ?"));
            params.push(SqlValue::Text(ts.clone()));
        }
    }
    if let Some(ts) = to_ts {
        if !ts.is_empty() {
            conds.push(format!("{column} <= ?"));
            params.push(SqlValue::Text(ts.clone()));
        }
    }
}

/// The project facet's SQL condition over `driving`, a table carrying the
/// `(session_id, device_id)` grouping pair (`usage_records`, `turn_durations`).
/// A known project identity matches via the `project_identity` SQL scalar —
/// the one Rust rule — so usage from a Claude Code worktree session matches
/// its PARENT project. The [`UNKNOWN_PROJECT`] sentinel inverts to NOT EXISTS:
/// the unknown bucket (remote usage without a pulled favorite snapshot,
/// session-less rows). `driving` is a fixed literal from the call sites, never
/// user input. Returns `(condition, param)`: the sentinel form binds no param.
/// The key probe against `sessions` is the shared composite-identity
/// predicate ([`super::aggregate_sql::session_pair_join`]) — the EXISTS forms
/// here and every dimension JOIN stay one spelling.
///
/// （架构审查候选④自 store_reads 收口至此：store_sessions_reads 会话粒
/// 未知桶的种子与 turn 侧的哨兵获取共用这一份实现。）
pub(super) fn project_condition(driving: &str, project: &str) -> (String, Option<SqlValue>) {
    let pair = super::aggregate_sql::session_pair_join(driving, "s");
    if project == UNKNOWN_PROJECT {
        (
            format!("NOT EXISTS (SELECT 1 FROM sessions s WHERE {pair})"),
            None,
        )
    } else {
        (
            format!(
                "EXISTS (SELECT 1 FROM sessions s \
                 WHERE {pair} \
                   AND project_identity(s.project_dir) = ?)"
            ),
            Some(SqlValue::Text(project.to_string())),
        )
    }
}

/// Which filter facets a usage-grain WHERE applies. 三个非「全开」的面都是
/// 真实语义，不是缺省：下拉候选忽略自己那一维（选中的值不许缩小自己的候
/// 选列表）、turn 粒（`turn_durations`）没有 model / source 列、未知桶的
/// 「项目」就是桶自身（NOT EXISTS 哨兵，不再叠项目条件）。取代旧
/// `build_where` 的三个位置 bool——调用点从魔数元组变自描述。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct FacetGates {
    pub model: bool,
    pub source: bool,
    pub project: bool,
}

/// 一个可被下拉候选「忽略自身」的 facet 维度。
pub(super) enum Facet {
    Model,
    Source,
    Project,
}

impl FacetGates {
    /// 全维度生效——普通聚合读（stats / trend / logs / 维度桶）。
    pub(super) const ALL: FacetGates = FacetGates {
        model: true,
        source: true,
        project: true,
    };

    /// turn 粒：`turn_durations` 没有 model / source 列（时间 / 设备 / 项目
    /// 照常，见 `query_stats` 的口径注）。
    pub(super) const TURNS: FacetGates = FacetGates {
        model: false,
        source: false,
        project: true,
    };

    /// 下拉候选的 facet 语义：忽略 `own` 这一维，其余照常。
    pub(super) fn dropping(own: Facet) -> Self {
        let mut g = Self::ALL;
        match own {
            Facet::Model => g.model = false,
            Facet::Source => g.source = false,
            Facet::Project => g.project = false,
        }
        g
    }
}

/// usage 粒的时间 / model / source / device 四项条件，按全库统一次序追加，
/// 列名带 `prefix`（`""` / `"u."` / `"sel."` 等调用方表别名，固定字面量）。
/// 项目维不在此内——它不是可选 facet 而是维度身份：直读路径经
/// [`FacetGates::project`] 门控后接 [`project_condition`]，未知桶则以 NOT
/// EXISTS 哨兵替代（桶定义本身）。
pub(super) fn push_usage_facets(
    conds: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    prefix: &str,
    filter: &UsageFilter,
    gates: FacetGates,
) {
    push_ts_range(
        conds,
        params,
        &format!("{prefix}timestamp"),
        &filter.from_ts,
        &filter.to_ts,
    );
    if gates.model {
        push_nonempty_eq(conds, params, &format!("{prefix}model"), &filter.model);
    }
    if gates.source {
        push_nonempty_eq(conds, params, &format!("{prefix}source"), &filter.source);
    }
    push_nonempty_eq(
        conds,
        params,
        &format!("{prefix}device_id"),
        &filter.device_scope,
    );
}

/// Build a `WHERE` clause + bound params for a `UsageFilter` (timestamp range,
/// model, source, device scope, project) over `driving` — the table the query
/// reads, which must carry the filter's columns (`timestamp`, `device_id`, and
/// the `(session_id, device_id)` pair for the project facet). The range filters
/// on `timestamp` (UTC), not `day` — see `UsageFilter` for why. `gates` picks
/// which facets apply (see [`FacetGates`]）。Returns `("WHERE ...", vec![...])`
/// or `("", [])`.
///
/// （架构审查Ⅲ候选⑤自 store_reads 下沉至此：直读与 store_dimensions 的
/// 未知桶共用同一份装配，本模块成为 usage 粒 WHERE 的单一归属。）
pub(super) fn build_where(
    filter: &UsageFilter,
    gates: FacetGates,
    driving: &str,
) -> (String, Vec<SqlValue>) {
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();
    push_usage_facets(&mut conds, &mut params, "", filter, gates);
    if gates.project {
        if let Some(p) = &filter.project {
            if !p.is_empty() {
                let (cond, param) = project_condition(driving, p);
                conds.push(cond);
                if let Some(v) = param {
                    params.push(v);
                }
            }
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

    fn opt(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn nonempty_eq_skips_none_and_empty_but_binds_real_values() {
        let mut conds: Vec<String> = vec![];
        let mut params: Vec<SqlValue> = vec![];
        push_nonempty_eq(&mut conds, &mut params, "model", &None);
        push_nonempty_eq(&mut conds, &mut params, "model", &opt(""));
        assert!(conds.is_empty() && params.is_empty(), "不限轴零条件");
        push_nonempty_eq(&mut conds, &mut params, "model", &opt("glm-5.2"));
        assert_eq!(conds, vec!["model = ?".to_string()]);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn ts_range_binds_each_end_independently() {
        let mut conds: Vec<String> = vec![];
        let mut params: Vec<SqlValue> = vec![];
        push_ts_range(&mut conds, &mut params, "u.timestamp", &None, &None);
        assert!(conds.is_empty());
        push_ts_range(
            &mut conds,
            &mut params,
            "u.timestamp",
            &opt("2026-08-01T00:00:00Z"),
            &opt("2026-08-27T00:00:00Z"),
        );
        assert_eq!(
            conds,
            vec![
                "u.timestamp >= ?".to_string(),
                "u.timestamp <= ?".to_string(),
            ]
        );
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn known_project_condition_uses_identity_udf_with_param() {
        let (cond, param) = project_condition("usage_records", "D:\\AI\\proj");
        assert!(
            cond.contains("project_identity(s.project_dir) = ?"),
            "{cond}"
        );
        assert!(cond.contains("usage_records.session_id = s.id"), "{cond}");
        assert_eq!(param, Some(SqlValue::Text("D:\\AI\\proj".into())));
    }

    #[test]
    fn sentinel_condition_is_not_exists_without_params() {
        // 哨兵两侧面孔之一：任意驱动别名的 NOT EXISTS 文本一致——参数化别名，
        // 两处消费方不可能再抄出第二份。键对探针文本由
        // aggregate_sql::session_pair_join 唯一决定。
        for driving in ["usage_records", "turn_durations", "u"] {
            let (cond, param) = project_condition(driving, UNKNOWN_PROJECT);
            assert!(param.is_none());
            assert_eq!(
                cond,
                format!(
                    "NOT EXISTS (SELECT 1 FROM sessions s \
                     WHERE {driving}.session_id = s.id \
                       AND {driving}.device_id = s.device_id)"
                )
            );
        }
    }

    /// usage 粒装配的次序与前缀契约：时间→model→source→device，门控只摘除
    /// 对应条件，前缀逐列生效——直读与未知桶共用后，两处的条件文本由同一
    /// 个函数决定。
    #[test]
    fn usage_facets_order_prefix_and_gates() {
        let filter = UsageFilter {
            from_ts: opt("2026-08-01T00:00:00Z"),
            model: opt("glm-5.2"),
            source: opt("claude_code"),
            device_scope: opt("dev1"),
            ..Default::default()
        };
        let mut conds: Vec<String> = vec![];
        let mut params: Vec<SqlValue> = vec![];
        push_usage_facets(&mut conds, &mut params, "", &filter, FacetGates::ALL);
        assert_eq!(
            conds,
            vec![
                "timestamp >= ?".to_string(),
                "model = ?".to_string(),
                "source = ?".to_string(),
                "device_id = ?".to_string(),
            ]
        );
        assert_eq!(params.len(), 4);

        // turn 粒门控摘掉 model / source，时间与设备照常；前缀逐列生效。
        let mut conds: Vec<String> = vec![];
        let mut params: Vec<SqlValue> = vec![];
        push_usage_facets(&mut conds, &mut params, "u.", &filter, FacetGates::TURNS);
        assert_eq!(
            conds,
            vec![
                "u.timestamp >= ?".to_string(),
                "u.device_id = ?".to_string()
            ]
        );

        // dropping(Facet::Project) = 全开 minus 项目——未知桶与项目下拉的
        // 未知探测共用同一扇门。
        assert_eq!(
            FacetGates::dropping(Facet::Project),
            FacetGates {
                model: true,
                source: true,
                project: false
            }
        );
    }

    /// build_where 的项目门控：关闭时哨兵不进条件（未知桶 / 项目下拉的探测
    /// 路径——桶的「项目」由自身定义），开启时 NOT EXISTS 落在四项之后。
    #[test]
    fn build_where_gates_the_project_facet() {
        let sentinel = UsageFilter {
            project: opt(UNKNOWN_PROJECT),
            model: opt("glm-5.2"),
            ..Default::default()
        };
        let (clause, params) = build_where(
            &sentinel,
            FacetGates::dropping(Facet::Project),
            "usage_records",
        );
        assert!(!clause.contains("NOT EXISTS"), "{clause}");
        assert_eq!(params.len(), 1, "只有 model 一个参数");

        let (clause, params) = build_where(&sentinel, FacetGates::ALL, "usage_records");
        assert!(clause.contains("NOT EXISTS"), "{clause}");
        assert_eq!(params.len(), 1, "哨兵形式不绑参数");
    }
}
