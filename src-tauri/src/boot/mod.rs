//! 启动序列编排：完整启动是一张声明式步骤清单（`boot_steps!` 宏，一行
//! 一步），次序即执行次序；每步的失败等级 [`Criticality`] 声明在清单里
//! ——加一步只改清单，改失败语义只改那一步的声明。清单的「名字 + 等级」
//! 契约视图是 `STEP_CONTRACT`（cfg(test)，供测试钉住启动语义）。
//!
//! [`run_boot`] 跑清单：BestEffort 失败聚合进 [`BootReport`] 不挡启动；
//! Fatal 失败就地终止清单并上抛 [`StepFailure`]——setup 把它交给 tauri，
//! build 处的 `expect` 崩溃退出。拆分前的语义是 config/store 裸 `expect`、
//! window/tray `?`（失败同样起不来），行为不变，诊断多了步骤名。
//!
//! 对 tauri 的依赖边界：步骤拿到的是具体 `AppHandle`（能传句柄就传句柄，
//! 不为测试引入抽象层）。可单测的是策略而非执行——「失败怎么折叠」收口在
//! 纯函数 [`fold_step`]（同 `scheduler::plan_tick` 之于调度线程），清单契约
//! 由 `tests` 表驱动钉住；跑真步骤需要活的 AppHandle，不进单测。
//!
//! 退出侧不属清单：exit flush 在 `ExitRequested` 时触发，与启动步骤生命
//! 周期不同，见 [`on_run_event`]（与启动清单同住本 module，同属一份进程
//! 生命周期编排）。scheduler 同理只有「spawn」进清单——线程本体启动后
//! 常驻。

mod scheduler;
mod tray;

pub(crate) use tray::tray_menu_for;

use std::sync::Arc;

use tauri::Manager;

use crate::commands::AppState;
use crate::config::ConfigStore;
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::{devices, events, pricing, sync};

/// 步骤失败等级——声明在清单里，[`run_boot`] 按它决定失败是否挡启动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Criticality {
    /// 失败即启动失败：终止清单、上抛 [`StepFailure`]，应用起不来。
    Fatal,
    /// 失败只聚合进 [`BootReport`]，启动继续。
    BestEffort,
}

/// 一步的失败（带步骤名归因）。
#[derive(Debug, thiserror::Error)]
#[error("boot step `{step}` failed: {message}")]
pub(crate) struct StepFailure {
    pub(crate) step: &'static str,
    pub(crate) message: String,
}

/// 启动报告：BestEffort 失败的聚合（Fatal 失败不走报告，直接上抛）。
#[derive(Debug, Default)]
pub(crate) struct BootReport {
    /// 按执行次序排列的 BestEffort 失败。
    pub(crate) failures: Vec<StepFailure>,
}

/// 一个 named step：名字 + 声明的失败等级 + 执行体。
struct BootStep {
    name: &'static str,
    criticality: Criticality,
    run: fn(&mut BootCtx) -> AppResult<()>,
}

/// 启动步骤清单：一行一步（名字、失败等级、执行体），次序即执行次序，
/// 加一步只改这里。步骤与拆分前的散装序列一一对应；`manage_state` 是
/// 步骤化后显式出来的装配管道（把 config/store 装进 tauri state，命令与
/// 后续步骤都从这里取）。
///
/// 宏从同一条目产出两份视图：[`STEP_CONTRACT`]（名字 + 等级，测试与文档
/// 引用）与 [`steps()`]（完整清单，只有 [`run_boot`] 引用）。测试刻意不碰
/// `steps()` / run fn——fn 指针会把开窗 / 托盘的执行体链进测试可执行文件，
/// 而测试 exe 不带 tauri-build 注入的 comctl32 v6 manifest（link-arg 只给
/// bins），静态导入 `TaskDialogIndirect` 等 v6 独有入口点会直接
/// `STATUS_ENTRYPOINT_NOT_FOUND`（拆分时实测踩中）。同一行声明名字、等级
/// 与执行体，契约与实现的对应关系由构造保证，不会漂移。
macro_rules! boot_steps {
    ($(($name:literal, $criticality:expr, $run:expr)),+ $(,)?) => {
        /// 启动契约：步骤名与失败等级（次序即执行次序）。测试专用视图
        /// ——生产路径的清单事实就是下面 `steps()` 本身。
        #[cfg(test)]
        pub(crate) const STEP_CONTRACT: &[(&str, Criticality)] = &[
            $(($name, $criticality)),+
        ];

        fn steps() -> &'static [BootStep] {
            &[$(BootStep { name: $name, criticality: $criticality, run: $run }),+]
        }
    };
}

boot_steps![
    // 引导数据目录 + deviceId（首启写默认 config）。
    ("load_config", Criticality::Fatal, load_config),
    // 打开 Local Store（建库 + 种子 pricing）。
    ("open_store", Criticality::Fatal, open_store),
    // 本机注册进 Local Store + 发布名字 artifact（Git 同步兜底）。
    ("register_self", Criticality::BestEffort, register_self),
    // 零成本行补价（新种子 pricing 补上缺模型期间导入的行）。
    ("rebill", Criticality::BestEffort, rebill),
    ("manage_state", Criticality::Fatal, manage_state),
    // 主窗口（conf 里 create:false，平台分支在步骤内）。
    ("create_main_window", Criticality::Fatal, create_main_window),
    // Synced 时拉一次对端数据（线程内完成，成功后 emit 通知前端）。
    ("startup_pull", Criticality::BestEffort, startup_pull),
    // 托盘（拆分前 `?` 即致命——降级与否是产品决策，未动）。
    ("build_tray", Criticality::Fatal, tray::build),
    // 后台调度线程（本体常驻，这里只 spawn）。
    ("start_scheduler", Criticality::BestEffort, scheduler::start),
];

/// 跑完整清单：逐步执行，失败按声明的 [`Criticality`] 折叠。全部跑完返回
/// [`BootReport`]；Fatal 失败时 `Err`——清单就地终止，后续步骤不再执行。
pub(crate) fn run_boot(app: tauri::AppHandle) -> Result<BootReport, StepFailure> {
    let mut ctx = BootCtx {
        app,
        config: None,
        store: None,
    };
    let mut report = BootReport::default();
    for step in steps() {
        if let Err(error) = (step.run)(&mut ctx) {
            eprintln!("[cc-one] boot step `{}` failed: {error}", step.name);
            fold_step(&mut report, step, error)?;
        }
    }
    // 收尾汇总：BestEffort 失败名单一次看清（各步失败已逐步打日志）。
    if !report.failures.is_empty() {
        eprintln!(
            "[cc-one] boot finished with {} best-effort failure(s)",
            report.failures.len()
        );
    }
    Ok(report)
}

/// 失败折叠的唯一策略点：BestEffort 失败聚合进报告、启动继续（`Ok`）；
/// Fatal 失败原样上抛（`Err`，调用方终止清单）。纯决策——不打印不 IO，
/// 行为由 `tests::fold_step_table` 表驱动覆盖。
fn fold_step(report: &mut BootReport, step: &BootStep, error: AppError) -> Result<(), StepFailure> {
    let failure = StepFailure {
        step: step.name,
        message: error.to_string(),
    };
    match step.criticality {
        Criticality::BestEffort => {
            report.failures.push(failure);
            Ok(())
        }
        Criticality::Fatal => Err(failure),
    }
}

/// 步骤间上下文：tauri 句柄 + 前序 Fatal 步骤的装配产物。产物是 `Option`
/// ——由前序步骤填充；「已填充」由清单次序保证，访问器上 panic 只会因
/// 清单次序写错（与 tauri `state::<T>()` 取错状态同类，编程错误直接暴露）。
struct BootCtx {
    app: tauri::AppHandle,
    config: Option<Arc<ConfigStore>>,
    store: Option<Arc<Store>>,
}

impl BootCtx {
    fn config(&self) -> &Arc<ConfigStore> {
        self.config
            .as_ref()
            .expect("boot 清单次序保证：load_config 已执行")
    }

    fn store(&self) -> &Arc<Store> {
        self.store
            .as_ref()
            .expect("boot 清单次序保证：open_store 已执行")
    }
}

/// tauri::Error → AppError：boot 内触碰 tauri API 的步骤把错误折进自家
/// 错误通道的唯一形状。
fn tauri_err(e: tauri::Error) -> AppError {
    AppError::Internal(e.to_string())
}

/// 引导数据目录、加载（首启则生成）config。Fatal：配置起不来应用无事可做。
fn load_config(ctx: &mut BootCtx) -> AppResult<()> {
    ctx.config = Some(Arc::new(ConfigStore::load()?));
    Ok(())
}

/// 打开 Local Store。Fatal：库打不开应用无事可做。
fn open_store(ctx: &mut BootCtx) -> AppResult<()> {
    let db = ctx.config().paths().db;
    ctx.store = Some(Arc::new(Store::open(&db)?));
    Ok(())
}

/// 本机注册进 Local Store 并发布名字 artifact（覆盖首启与旧版本升级）；
/// 正常 Git 同步兜底，失败不挡启动。
fn register_self(ctx: &mut BootCtx) -> AppResult<()> {
    devices::register_self(ctx.store(), ctx.config())
}

/// 启动期零成本补价：新种子 pricing 会给「模型缺失期间导入」的行留下
/// 零成本行，这里重算一次。pricing book 读不出来时退回种子书（降级可用，
/// 只记日志不算失败）。
fn rebill(ctx: &mut BootCtx) -> AppResult<()> {
    let store = ctx.store();
    let book = store.load_pricing_book().unwrap_or_else(|e| {
        eprintln!("[cc-one] boot rebill skipped: {e}");
        pricing::seed_book()
    });
    store.rebill_zero_cost(&book)?;
    Ok(())
}

/// 把 config/store 装进 tauri state：命令层的 `State<AppState>` 与退出侧
/// flush 都从这里取。Fatal——state 没装上应用等于没装配。
fn manage_state(ctx: &mut BootCtx) -> AppResult<()> {
    let state = AppState {
        store: ctx.store().clone(),
        config: ctx.config().clone(),
    };
    if !ctx.app.manage(state) {
        return Err(AppError::Internal("AppState already managed".to_string()));
    }
    Ok(())
}

/// 创建主窗口。conf 里 `create:false`，平台分支在步骤内：macOS 保留系统
/// 标题栏（Overlay），Windows/Linux 去装饰用自绘标题栏。刻意不分平台 conf
/// 文件——json-patch 合并会整替换 `windows` 数组、丢基础窗口几何。
fn create_main_window(ctx: &mut BootCtx) -> AppResult<()> {
    let conf = ctx
        .app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .expect("tauri.conf.json must define the main window");
    let builder =
        tauri::webview::WebviewWindowBuilder::from_config(&ctx.app, conf).map_err(tauri_err)?;
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let builder = builder.decorations(false);
    builder.build().map_err(tauri_err)?;
    Ok(())
}

/// 启动期对端数据拉取（Synced only，覆盖设备切换场景）：线程内跑完，
/// 成功后 emit `usage_changed` 通知前端失效刷新——不依赖 scheduler 第一跳
/// 的巧合；失败只记日志不挡启动。步骤本身只负责 spawn，立即返回。
fn startup_pull(ctx: &mut BootCtx) -> AppResult<()> {
    let store = ctx.store().clone();
    let config = ctx.config().clone();
    let app = ctx.app.clone();
    std::thread::spawn(move || {
        let cfg = config.get();
        if !cfg.is_synced() {
            return;
        }
        let paths = config.paths();
        match sync::pull_and_import(&store, &paths, &cfg) {
            Ok(n) => {
                eprintln!("[cc-one] startup pull imported {n} item(s)");
                events::emit_usage_changed(&app);
            }
            Err(e) => eprintln!("[cc-one] startup pull failed: {e}"),
        }
    });
    Ok(())
}

/// 退出侧编排（不在启动清单里）：`ExitRequested` 时把未推送的 Artifact
/// 推上去，覆盖 close-A / open-B 的设备切换。collect 现在只写 store 不写
/// Artifact，所以 flush 必须先重算再推。Synced only，best-effort。
pub(crate) fn on_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    if let tauri::RunEvent::ExitRequested { .. } = event {
        let state = app.state::<AppState>();
        let cfg = state.config.get();
        let paths = state.config.paths();
        sync::push_usage_best_effort(&state.store, &paths, &cfg);
    }
}

#[cfg(test)]
mod tests;
