//! 筛选条件的共享 SQL 构建规则（架构审查候选④）。
//!
//! 归属内容：
//! - [`push_nonempty_eq`] / [`push_ts_range`]：「值非空才约束」的筛选样板；
//! - [`project_condition`]：usage-粒直读表（`usage_records` /
//!   `turn_durations`）上的项目维度条件——known 身份走 `project_identity`
//!   UDF、[`UNKNOWN_PROJECT`] 哨兵走 NOT EXISTS 反转。
//!
//! 消费方：`store_reads` 的 `UsageFilter` 构建、`store_transcript` 的
//! `SessionFilter` 会话粒构建与其未知桶的 usage-粒直读。此前「非空才加条
//! 件」五段样板三处手写、哨兵文本两处逐字重复，靠注释互相指认——漂移即静
//! 默错桶（#94/#100 要求 unknown 口径两侧同步）。会话粒特有的语义差异刻意
//! 不并入本模块（known 按 session USED model 门控的 EXISTS 形式 vs 行级直等、
//! 时间列 last_active_at 与 timestamp 属调用方契约），只收敛真正同形的部分。

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
///
/// （架构审查候选④自 store_reads 收口至此：store_transcript 的未知桶种子与
/// turn 侧的哨兵获取共用这一份实现。）
pub(super) fn project_condition(driving: &str, project: &str) -> (String, Option<SqlValue>) {
    if project == UNKNOWN_PROJECT {
        (
            format!(
                "NOT EXISTS (SELECT 1 FROM sessions s \
                 WHERE s.id = {driving}.session_id \
                   AND s.device_id = {driving}.device_id)"
            ),
            None,
        )
    } else {
        (
            format!(
                "EXISTS (SELECT 1 FROM sessions s \
                 WHERE s.id = {driving}.session_id \
                   AND s.device_id = {driving}.device_id \
                   AND project_identity(s.project_dir) = ?)"
            ),
            Some(SqlValue::Text(project.to_string())),
        )
    }
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
        assert!(cond.contains("s.id = usage_records.session_id"), "{cond}");
        assert_eq!(param, Some(SqlValue::Text("D:\\AI\\proj".into())));
    }

    #[test]
    fn sentinel_condition_is_not_exists_without_params() {
        // 哨兵两侧面孔之一：任意驱动别名的 NOT EXISTS 文本一致——参数化别名，
        // 两处消费方不可能再抄出第二份。
        for driving in ["usage_records", "turn_durations", "u"] {
            let (cond, param) = project_condition(driving, UNKNOWN_PROJECT);
            assert!(param.is_none());
            assert_eq!(
                cond,
                format!(
                    "NOT EXISTS (SELECT 1 FROM sessions s \
                     WHERE s.id = {driving}.session_id \
                       AND s.device_id = {driving}.device_id)"
                )
            );
        }
    }
}
