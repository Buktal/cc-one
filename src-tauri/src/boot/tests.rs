//! boot 模块测试（独立文件，`#[cfg(test)]` 不计行限）：启动清单契约
//! （次序 + 失败等级）表驱动钉死，失败折叠策略纯断言覆盖。
//!
//! 跑不了单测的部分：`run_boot` 的执行循环需要活的 AppHandle（步骤真开
//! 窗、真建托盘、真开库）。「BestEffort 失败聚合、Fatal 失败终止」的决策
//! 面在 [`fold_step_table`] 全覆盖——policy / executor 分工同 `plan_tick`
//! 之于调度线程循环。

use std::time::{Duration, Instant};

use super::scheduler::{plan_tick, TickAction};
use super::{fold_step, BootCtx, BootReport, BootStep, Criticality, STEP_CONTRACT};
use crate::config::ConfigData;
use crate::error::{AppError, AppResult};

/// 清单表：九个步骤的次序与失败等级是声明出来的契约——加一步、改次序、
/// 改等级都会在这里红掉，防止启动语义被静默改动。
///
/// 只引用 [`STEP_CONTRACT`]（名字 + 等级），不碰 `steps()` / run fn：fn
/// 指针会把开窗 / 托盘执行体链进测试可执行文件，而测试 exe 没有
/// tauri-build 注入的 comctl32 v6 manifest，静态导入 v6 独有入口点会在
/// 加载期 `STATUS_ENTRYPOINT_NOT_FOUND`（见 boot 清单处的宏文档）。
/// 名字 ↔ 执行体的对应由 `boot_steps!` 宏同一行声明的构造保证。
#[test]
fn boot_steps_table() {
    let expect: &[(&str, Criticality)] = &[
        ("load_config", Criticality::Fatal),
        ("open_store", Criticality::Fatal),
        ("register_self", Criticality::BestEffort),
        ("rebill", Criticality::BestEffort),
        ("manage_state", Criticality::Fatal),
        ("create_main_window", Criticality::Fatal),
        ("startup_pull", Criticality::BestEffort),
        ("build_tray", Criticality::Fatal),
        ("start_scheduler", Criticality::BestEffort),
    ];

    assert_eq!(STEP_CONTRACT.len(), expect.len(), "步骤数与清单表一致");
    for ((name, criticality), (expect_name, expect_criticality)) in STEP_CONTRACT.iter().zip(expect)
    {
        assert_eq!(name, expect_name, "步骤次序错位");
        assert_eq!(
            criticality, expect_criticality,
            "step `{}` 的失败等级",
            name
        );
    }

    // 步骤名唯一：失败日志与 BootReport 按名归因，重名即归因失真。
    let mut names: Vec<_> = STEP_CONTRACT.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), STEP_CONTRACT.len(), "步骤名唯一");
}

/// 折叠策略（fold_step 只在步骤出错时被调用，故输入恒为一个错误）：
/// BestEffort 失败聚合进报告并继续（多次失败按执行次序累积——这正是
/// run_boot 循环对它的用法）；Fatal 失败上抛、不进报告（清单就地终止）。
#[test]
fn fold_step_table() {
    // 永不执行的占位执行体：折叠策略只看 criticality 与结果。
    fn noop(_: &mut BootCtx) -> AppResult<()> {
        Ok(())
    }
    let step = |name: &'static str, criticality| BootStep {
        name,
        criticality,
        run: noop,
    };
    let err = || AppError::Internal("boom".to_string());

    // BestEffort：进报告、继续启动，归因带步骤名与消息。
    let mut report = BootReport::default();
    assert!(fold_step(&mut report, &step("rebill", Criticality::BestEffort), err()).is_ok());
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].step, "rebill");
    assert_eq!(report.failures[0].message, "internal error: boom");

    // 第二个 BestEffort 失败：按执行次序累积进同一份报告。
    assert!(fold_step(
        &mut report,
        &step("startup_pull", Criticality::BestEffort),
        err()
    )
    .is_ok());
    let names: Vec<&str> = report.failures.iter().map(|f| f.step).collect();
    assert_eq!(names, vec!["rebill", "startup_pull"]);

    // Fatal：上抛（Err，清单在此终止），且不进报告。
    let mut report = BootReport::default();
    let outcome = fold_step(&mut report, &step("build_tray", Criticality::Fatal), err());
    let failure = outcome.expect_err("Fatal 失败必须上抛");
    assert_eq!(failure.step, "build_tray");
    assert_eq!(failure.message, "internal error: boom");
    assert!(report.failures.is_empty());
}

/// Synced-mode config: repo URL + PAT present ⇒ `is_synced()` true.
fn synced_config() -> ConfigData {
    ConfigData {
        repo_url: Some("https://github.com/cc-one/test".to_string()),
        github_token: Some("github_pat_test".to_string()),
        collect_interval_secs: 30,
        push_interval_secs: 600,
        ..ConfigData::default()
    }
}

/// `plan_tick` table: every decision the scheduler makes per wake must be
/// covered here — independent intervals, both-due ordering (collect first),
/// `is_synced` gate, and unchanged-when-not-due deadlines.
#[test]
fn plan_tick_table() {
    let t0 = Instant::now();
    let collect_after = Duration::from_secs(30);
    let push_after = Duration::from_secs(600);

    struct Case {
        name: &'static str,
        now: Instant,
        next_collect: Instant,
        next_push: Instant,
        cfg: ConfigData,
        expect: Vec<TickAction>,
        expect_collect: Instant,
        expect_push: Instant,
    }

    let cases = vec![
        // 1. Only collect deadline reached (synced) → collect advances, push unchanged.
        Case {
            name: "only collect due (synced)",
            now: t0 + Duration::from_secs(100),
            next_collect: t0 + Duration::from_secs(100),
            next_push: t0 + Duration::from_secs(700),
            cfg: synced_config(),
            expect: vec![TickAction::Collect],
            expect_collect: t0 + Duration::from_secs(100) + collect_after,
            expect_push: t0 + Duration::from_secs(700),
        },
        // 2. Only push deadline reached (synced) → sync advances, collect unchanged.
        Case {
            name: "only push due (synced)",
            now: t0 + Duration::from_secs(200),
            next_collect: t0 + Duration::from_secs(300),
            next_push: t0 + Duration::from_secs(200),
            cfg: synced_config(),
            expect: vec![TickAction::Sync],
            expect_collect: t0 + Duration::from_secs(300),
            expect_push: t0 + Duration::from_secs(200) + push_after,
        },
        // 3. BOTH due (synced) → Collect before Sync (the implementation
        //    order — a latency preference, not an invariant), both advance.
        Case {
            name: "both due (synced) — both actions, collect first",
            now: t0 + Duration::from_secs(500),
            next_collect: t0 + Duration::from_secs(100),
            next_push: t0 + Duration::from_secs(200),
            cfg: synced_config(),
            expect: vec![TickAction::Collect, TickAction::Sync],
            expect_collect: t0 + Duration::from_secs(500) + collect_after,
            expect_push: t0 + Duration::from_secs(500) + push_after,
        },
        // 4. Neither due → no action, both deadlines unchanged.
        Case {
            name: "neither due (synced)",
            now: t0 + Duration::from_secs(10),
            next_collect: t0 + Duration::from_secs(100),
            next_push: t0 + Duration::from_secs(700),
            cfg: synced_config(),
            expect: vec![],
            expect_collect: t0 + Duration::from_secs(100),
            expect_push: t0 + Duration::from_secs(700),
        },
        // 5. Push deadline reached but Standalone → Sync suppressed AND next_push
        //    is NOT advanced (the gate skips both the action and the reschedule).
        Case {
            name: "push due but standalone — sync suppressed",
            now: t0 + Duration::from_secs(300),
            next_collect: t0 + Duration::from_secs(400),
            next_push: t0 + Duration::from_secs(200),
            cfg: ConfigData::default(),
            expect: vec![],
            expect_collect: t0 + Duration::from_secs(400),
            expect_push: t0 + Duration::from_secs(200),
        },
        // 6. Both due but Standalone → Collect only; next_push unchanged.
        Case {
            name: "both due but standalone — collect only",
            now: t0 + Duration::from_secs(500),
            next_collect: t0 + Duration::from_secs(100),
            next_push: t0 + Duration::from_secs(200),
            cfg: ConfigData::default(),
            expect: vec![TickAction::Collect],
            expect_collect: t0 + Duration::from_secs(500) + collect_after,
            expect_push: t0 + Duration::from_secs(200),
        },
    ];

    for c in cases {
        let (actions, new_collect, new_push) =
            plan_tick(c.now, c.next_collect, c.next_push, &c.cfg);
        assert_eq!(actions, c.expect, "{}: actions", c.name);
        assert_eq!(new_collect, c.expect_collect, "{}: next_collect", c.name);
        assert_eq!(new_push, c.expect_push, "{}: next_push", c.name);
    }
}

/// `plan_tick` must derive its cadence from the [`ConfigData`] clamp fns —
/// the single declaration of the interval bounds — so out-of-range stored
/// values (a hand-edited config.json can carry them) never reach the
/// deadlines.
#[test]
fn plan_tick_clamps_intervals() {
    let t0 = Instant::now();
    let far_future = t0 + Duration::from_secs(99_999);

    // collect floor 5s and ceiling 3600s (push held not-due).
    let mut cfg = synced_config();
    cfg.collect_interval_secs = 1;
    let (_, nc, _) = plan_tick(t0, t0, far_future, &cfg);
    assert_eq!(nc, t0 + Duration::from_secs(5), "collect floor 5s");
    cfg.collect_interval_secs = 50_000;
    let (_, nc, _) = plan_tick(t0, t0, far_future, &cfg);
    assert_eq!(nc, t0 + Duration::from_secs(3600), "collect ceiling 3600s");

    // push floor 60s and ceiling 7200s (collect held not-due, synced so Sync fires).
    cfg.collect_interval_secs = 30;
    cfg.push_interval_secs = 1;
    let (_, _, np) = plan_tick(t0, far_future, t0, &cfg);
    assert_eq!(np, t0 + Duration::from_secs(60), "push floor 60s");
    cfg.push_interval_secs = 50_000;
    let (_, _, np) = plan_tick(t0, far_future, t0, &cfg);
    assert_eq!(np, t0 + Duration::from_secs(7200), "push ceiling 7200s");
}
