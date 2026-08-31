//! 启动后的后台调度：collect 与 sync 各自独立 interval，一个线程两个
//! deadline，睡到点不轮询。策略收口在纯函数 [`plan_tick`]（每 tick 决策
//! 面：独立间隔、同 tick 双到期次序、`is_synced` 门、区间 clamp），线程
//! 循环只按决策执行；决策行为由 `boot::tests` 表驱动覆盖。
//!
//! 生命周期：`start` 是 boot 清单里的 `start_scheduler` 步骤（只 spawn），
//! 线程本体启动后常驻——Settings 改动每 tick 重读 config 快照，不需重启。

use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::AppHandle;

use super::{AppResult, BootCtx};
use crate::collect::{self, SyncRoundPosture};
use crate::config::{ConfigData, ConfigStore};
use crate::db::Store;
use crate::events;

/// `start_scheduler` 步骤：spawn 常驻线程。OS 拒绝建线程时保持拆分前的
/// panic 语义；循环内的采集 / 同步失败是 best-effort 日志——步骤的
/// BestEffort 声明描述的是后者。
pub(super) fn start(ctx: &mut BootCtx) -> AppResult<()> {
    let store = ctx.store().clone();
    let config = ctx.config().clone();
    let app = ctx.app.clone();
    std::thread::spawn(move || scheduler_loop(store, config, app));
    Ok(())
}

/// One scheduler action returned by [`plan_tick`]. The list order IS the
/// execution order — when both fire in a tick, `Collect` runs first (a
/// latency preference, not a load-bearing invariant — see [`plan_tick`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TickAction {
    /// Parse Sources → Local Store. No network.
    Collect,
    /// One pull+push round (Synced only), run under the
    /// [`SyncRoundPosture::Once`] posture. The cadence is the retry —
    /// no per-tick retry here.
    Sync,
}

/// Pure per-tick decision for the background scheduler: given the current
/// time, the two deadlines, and the live config, return the actions to run
/// this tick (in order) plus the updated deadlines.
///
/// **Ordering.** When both deadlines fire in the same tick, `Collect` is
/// placed before `Sync` in the returned `Vec`. NOT a correctness invariant —
/// collect writes only the store (SQLite + dirty flags) and sync reads the
/// store, so either order is safe; collect-first merely lets a same-tick sync
/// ship the just-collected rows immediately instead of one push interval
/// later. (The old justification — collect's JSONL `writeln!` must fully flush
/// before the subsequent `git add` snapshots the files — died with the
/// JSONL-append architecture: collect is store-only, and the Artifact is
/// written exclusively by the push-side recompute.)
///
/// Pure: no IO, no global state, no clock — `now` is a parameter, so the full
/// decision surface (independent intervals, both-due ordering, `is_synced`
/// gate, interval clamping) is covered by `boot::tests::plan_tick_table`.
///
/// A deadline that does not fire is returned unchanged, so in Standalone
/// (where `Sync` never fires) `next_push` stays at its initial value —
/// preserved exactly from the inline loop.
pub(super) fn plan_tick(
    now: Instant,
    next_collect: Instant,
    next_push: Instant,
    cfg: &crate::config::ConfigData,
) -> (Vec<TickAction>, Instant, Instant) {
    // Bounds are declared once on ConfigData (same fn the settings setter
    // clamps through — the stored value can be out of range if config.json
    // was hand-edited).
    let collect_secs = ConfigData::clamp_collect_interval_secs(cfg.collect_interval_secs) as u64;
    let push_secs = ConfigData::clamp_push_interval_secs(cfg.push_interval_secs) as u64;

    let mut actions = Vec::new();
    let mut new_collect = next_collect;
    let mut new_push = next_push;
    // Collect is evaluated and pushed first: when both deadlines fire, a
    // same-tick sync can ship the just-collected rows (a latency preference —
    // the order is not load-bearing; see the fn doc).
    if now >= next_collect {
        actions.push(TickAction::Collect);
        new_collect = now + Duration::from_secs(collect_secs);
    }
    if now >= next_push && cfg.is_synced() {
        actions.push(TickAction::Sync);
        new_push = now + Duration::from_secs(push_secs);
    }
    (actions, new_collect, new_push)
}

/// Background scheduler loop. One thread, two deadlines, slept-to (not
/// polled): each tick re-reads the config snapshot and hands it to
/// [`plan_tick`], so Settings changes apply without restart.
///
/// When both deadlines fire in a tick, `Collect` runs first — the order is
/// encoded and tested in [`plan_tick`] (a latency preference, not a
/// correctness invariant: collect writes only the store, sync reads the
/// store, so either order is safe). This loop just walks the returned action
/// list in order.
///
/// Startup strategy: first collect fires immediately (next_collect = start —
/// dashboard is fresh on open); first sync is delayed one push_interval
/// (next_push = start + push_interval) so it cannot race the startup pull's
/// git-worktree ops. These are one-off initializations; [`plan_tick`] owns
/// the per-tick logic.
fn scheduler_loop(store: Arc<Store>, config: Arc<ConfigStore>, app: AppHandle) {
    let start = Instant::now();
    let mut next_collect = start;
    let first_push_secs =
        ConfigData::clamp_push_interval_secs(config.get().push_interval_secs) as u64;
    let mut next_push = start + Duration::from_secs(first_push_secs);
    loop {
        // Snapshot config once per tick (matches the original pre-sleep read
        // so live Settings changes apply next tick).
        let cfg = config.get();

        // Sleep to the nearer deadline (not polled).
        let now = Instant::now();
        let next_deadline = next_collect.min(next_push);
        if next_deadline > now {
            std::thread::sleep(next_deadline - now);
        }

        let now = Instant::now();
        let (actions, new_collect, new_push) = plan_tick(now, next_collect, next_push, &cfg);
        next_collect = new_collect;
        next_push = new_push;
        // Execute in the returned order (collect before sync when both fire
        // — [`plan_tick`]'s ordering).
        for action in actions {
            match action {
                TickAction::Collect => {
                    if let Err(e) = collect::collect_into(&store, &config) {
                        eprintln!("[cc-one] scheduled collect failed: {e}");
                    }
                    events::emit_usage_changed(&app);
                }
                TickAction::Sync => {
                    // One pull+push round under the Once posture (best-effort;
                    // the cadence is the retry — no explicit retry here). Pull
                    // lands peer devices' usage here, push sends this device's
                    // up.
                    let sr = collect::run_sync_round(&store, &config, SyncRoundPosture::Once);
                    for e in &sr.errors {
                        eprintln!("[cc-one] scheduled sync error: {e}");
                    }
                    events::emit_usage_changed(&app);
                }
            }
        }
    }
}
