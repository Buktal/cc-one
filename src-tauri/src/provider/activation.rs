//! 供应商激活编排（架构审查候选③）：「切换供应商」这条全后端最深的业务流
//! 此前住在 `switch_provider_cmd` 的命令闭包里——不可测（「写盘成功才落激
//! 活态」的顺序不变量只存在于行序），且附加模式分支反向借用 commands 兄弟
//! 模块的 `ensure_opencode_in_live`，providers ↔ live_import 双向互依。下沉
//! 到 provider 域后：命令层只剩 spawn_blocking 薄壳 + emit；依赖注入与
//! [`App::write_live`] 的 paths 参数同一形状，编排可用内存库 +
//! `ConfigStore::for_test` + 临时 live 目录直测（见本模块测试组）。
//!
//! 附加模式「加入 live」与「移出 live」是同一编排的两半，都收在本模块
//! （[`ensure_opencode_in_live`] / [`remove_from_live`]）——撤除半边曾散在
//! commands 层两处（停用路径带 meta 半边、删除路径只撤文件），现在对称归位。
//! 第四条组合「删除供应商」（附加模式 = 撤除半边 + 删行、单激活 = 只删行，
//! [`delete_provider`]）同样收在本模块——mode 分派与路径布局知识不进命令层，
//! 命令层对四条组合一律只剩薄壳。
//!
//! 片段合并层的 per-app 分派（ADR-0010，含 settings_config 层的合并域）收口
//! 在 [`crate::provider::live_adapter`]；本模块是它们的组合次序权威。

use std::path::{Path, PathBuf};

use crate::config::ConfigStore;
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::model::{App, Provider};
use crate::provider::live_adapter::SnippetLayer;
use crate::provider::{live, live_opencode};

#[cfg(test)]
mod tests;

/// 激活动作涉及的 live 文件路径集。生产由 [`resolve_paths`] 从真实 home 解析；
/// 测试注入临时目录。形状与 [`App::write_live`] 的 paths 参数一致（主配置在
/// `[0]`、副文件在 `[1]`）。
pub(crate) struct ActivatePaths {
    /// 单激活应用的 live 文件路径集（[`App::live_paths`]：claude/grok 一份、
    /// codex/gemini 两份）。附加模式的写盘不经 [`App::write_live`]，不用此字段
    /// （live_paths 对附加模式也返回 opencode.json，供反向导入读，不进写盘）。
    pub(crate) single: Vec<PathBuf>,
    /// opencode.json 路径（附加模式的写入目标）；单激活忽略。
    pub(crate) opencode_config: PathBuf,
}

/// 解析真实的 live 路径集（生产入口一次解析、编排全程持有）。
pub(crate) fn resolve_paths(app: App) -> AppResult<ActivatePaths> {
    Ok(ActivatePaths {
        single: app.live_paths()?,
        opencode_config: live_opencode::opencode_config_path()?,
    })
}

/// 激活编排（supplier 侧唯一业务入口，命令层薄壳直接调用）：单激活 =
/// 「切换」——按应用的 ADR-0010 策略分派片段合并层后受控写盘，**写盘成功才**
/// `set_active_provider`（重启后激活态指向没写成的配置 = 废状态，故反序绝
/// 不允许）；附加模式（OpenCode）= ensure-in-live——写进 opencode.json + 设
/// `meta.liveManaged = true`，**不取消其它 provider、不碰 active_providers**
/// （附加模式无唯一激活）。两条路都返回入库后的 provider。
pub(crate) fn activate(
    store: &Store,
    config: &ConfigStore,
    app: App,
    id: &str,
    paths: &ActivatePaths,
) -> AppResult<Provider> {
    let provider = store
        .get_provider(app, id)?
        .ok_or_else(|| AppError::Config(format!("provider not found in {app:?} pool: {id}")))?;
    if app.is_additive_mode() {
        return ensure_opencode_in_live(store, provider, &paths.opencode_config);
    }
    // 单激活：片段按 provider 归属的应用读取（claude 池读 claude 片段）。读
    // guard 随语句结束释放。
    let snippet_record = config.get().snippet_for(app);
    let write_provider = match app.snippet_layer() {
        // settings_config 层（claude/gemini）：片段按该 app 的合并域（受控区
        // 形状，随层声明）并入供应商配置，再随受控写盘落地。claude 的
        // settings.json 是字面量 JSON：${VAR} 占位符会原样写进 live = 废配置，
        // 切换前拦下（gemini 的 .env 由 dotenv 展开 ${VAR} 是合法引用，不拦
        // ——见 App::validates_template_vars）。
        SnippetLayer::SettingsConfig(domain) => {
            let settings_config = crate::provider::snippet::apply_snippet(
                &provider.settings_config,
                &snippet_record.content,
                snippet_record.enabled,
                domain,
            )?;
            if app.validates_template_vars() {
                crate::provider::live::validate_no_unfilled_template_vars(&settings_config)?;
            }
            Provider {
                settings_config,
                ..provider.clone()
            }
        }
        // 写盘层（codex/grok）与无片段（opencode，先于此处返回）：供应商配置
        // 原样进写盘，片段随 write_snippet 走写盘层补缺失。
        SnippetLayer::WriteLayer | SnippetLayer::NoSnippet => provider.clone(),
    };
    // 写盘层片段（codex/grok）：启用 → 片段内容，否则空串（switch_*_live 空
    // 串即无操作）。settings_config 层应用一律空串（其片段已在上面并入供应商
    // 配置）。
    let write_snippet = match app.snippet_layer() {
        SnippetLayer::WriteLayer if snippet_record.enabled => snippet_record.content.clone(),
        _ => String::new(),
    };
    app.write_live(&paths.single, &write_provider, &write_snippet)?;
    config.update(|c| c.set_active_provider(app, id))?;
    Ok(provider)
}

/// 附加模式核心动作（OpenCode）:把 provider ensure-in-live——写进 opencode.json
/// 同时设 `meta.liveManaged = true` 并落库。key 由 `live_opencode::derive_live_key`
/// 派生（优先沿用 meta.liveKey，改名不重算；首次按 name slugify，空 → 回落
/// id）。路径由调用方给定（生产 [`resolve_paths`]，测试注入临时目录）。
pub(crate) fn ensure_opencode_in_live(
    store: &Store,
    provider: Provider,
    path: &Path,
) -> AppResult<Provider> {
    let live_text = live::read_live_settings(path)?;
    let key =
        live_opencode::derive_live_key(&provider.name, &provider.id, &provider.meta, &live_text);
    live_opencode::set_opencode_provider(path, &key, &provider.settings_config)?;
    let updated = Provider {
        meta: live_opencode::with_meta_live_state(&provider.meta, &key, true)?,
        ..provider
    };
    store.save_provider(updated)
}

/// 附加模式移除半边（[`ensure_opencode_in_live`] 的对称物，停用与删除路径
/// 共用同一入口）：已托管（`meta.liveManaged = true`）→ 从 live 配置移除该键；
/// 随后 `meta.liveManaged = false` 落库。**liveKey 保留**——key 稳定才不弄断
/// 用户顶层 `model: "<key>/<model>"` 引用，再加回来时沿用原 key。无 liveKey
/// （从未写盘）→ 显式无操作、原样返回；重复移除幂等（未托管不碰文件，meta
/// 值不变 → `save_provider` 判无结构变化、不刷新 `updated_at`）。路径由调用
/// 方给定（生产 [`resolve_paths`]，测试注入临时目录）。
pub(crate) fn remove_from_live(
    store: &Store,
    provider: Provider,
    path: &Path,
) -> AppResult<Provider> {
    let Some(key) = live_opencode::meta_live_key(&provider.meta) else {
        return Ok(provider);
    };
    if live_opencode::meta_live_managed(&provider.meta) == Some(true) {
        live_opencode::remove_opencode_provider(path, &key)?;
    }
    let updated = Provider {
        meta: live_opencode::with_meta_live_state(&provider.meta, &key, false)?,
        ..provider
    };
    store.save_provider(updated)
}

/// 删除供应商编排（第四条组合：停用半边 + 删行，命令层薄壳直接调用）。
/// 附加模式先走对称的移除半边（[`remove_from_live`]：已托管才撤 live 条目 +
/// `meta.liveManaged = false` 落库），**live 撤除成功才删 DB 行**——撤除半边
/// 成功后行（managed=false）与 live 文件（条目已撤）已经一致，删行只是收尾；
/// 撤除失败（live 读不出 / 写不进）→ 整个删除以 Err 收场、行原样保留，绝不
/// 产生「行没了、live 条目还挂着」的孤儿引用，重试同一编排即收敛（撤除幂等）。
/// 单激活直接删行、不碰 live 文件——其 live 由「切换」受控覆盖，没有「残留
/// 条目」概念，撤无可撤。provider 不存在 → 跳过撤除半边、删行照做（DELETE
/// 幂等，重复删除无副作用）。路径由调用方给定（生产 [`resolve_paths`]，测试
/// 注入临时目录）。
pub(crate) fn delete_provider(
    store: &Store,
    app: App,
    id: &str,
    paths: &ActivatePaths,
) -> AppResult<()> {
    if app.is_additive_mode() {
        if let Some(provider) = store.get_provider(app, id)? {
            remove_from_live(store, provider, &paths.opencode_config)?;
        }
    }
    store.delete_provider(app, id)
}
