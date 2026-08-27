//! Provider 写盘（live）：受控合并 + 备份 + 原子写。
//!
//! 写盘分派 `write_live(app, provider, snippet)` 经 `live_adapter` 的
//! `App::write_live`（per-app 行为单一 seam，见 `live_adapter.rs`）按应用分派：
//! claude 分支在本文件，codex / gemini / grok 分支在 `live_codex` /
//! `live_gemini` / `live_grok` 模块。以下写盘语义是 claude 分支（JSON 受控
//! 合并）的精确规格，其它 app 各自沿用同一套「受控合并 / 非受控保留 / 备份 /
//! 原子写」语义：
//!
//! 写盘语义（必须精确实现）：
//! - **受控字段**（Provider 接管，切换时整块替换/合并）：`env` 块 +
//!   `includeCoAuthoredBy` / `attribution` / `effortLevel` / `enabledPlugins` /
//!   `skipWebFetchPreflight`。`env` 走整块替换（端点/key/模型映射都住在 env
//!   里），其余顶层开关按「目标存在则替换、缺失则保留 live 原值」合并。
//! - **非受控字段**（`permissions` / `hooks` / `mcpServers` /
//!   `enableAllProjectMcpServers` / `model` / `extraKnownMarketplaces` /
//!   `statusLine` 等一切其他字段）：切换时从 live **原地保留**；目标配置里的
//!   非受控字段被忽略，绝不写 live。
//! - 写盘顺序：读当前 live → 受控合并 → 内容无变化则**无操作**（不备份、
//!   不写盘、不碰 mtime，见 [`commit_live_file`]）→ 备份 `settings.json.bak`
//!   （单份覆盖）→ 原子写（临时文件 + 改名，进程中断不产生半截文件）。
//! - 清洗：写 live 前剥掉配置里的应用内部 meta 字段（`api_format` /
//!   `apiFormat` 等，类比 cc-switch `sanitize_claude_settings_for_live`）。
//! - **不做** cc-switch 的整文件覆盖 + Backfill。
//!
//! `merge_live_settings` 是纯函数（本项目最高价值的测试接缝）：输入
//! (当前 live JSON 字符串, 目标 settingsConfig 字符串, 清洗规则) → 输出合并后的
//! JSON 字符串，不碰文件系统。文件 IO（读/备份/原子写）是薄壳，直接调用它。
//! 「非受控字段保留」这个关键不变量靠它落进可测代码，而不是散文注释。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Table};

use crate::error::{AppError, AppResult};
use crate::model::App;

/// 写盘时从配置里剥掉的内部 meta 字段（类比 cc-switch
/// `sanitize_claude_settings_for_live`）：这些键只供应用自己读，不是合法的
/// settings.json 字段，绝不落 live。
pub const LIVE_INTERNAL_KEYS: &[&str] = &[
    "api_format",
    "apiFormat",
    "openrouter_compat_mode",
    "openrouterCompatMode",
];

/// 剥掉对象顶层的内部 meta 键（[`LIVE_INTERNAL_KEYS`]）——这些键只供应用
/// 自己读，不是合法的 live 配置字段，任何写盘 / 快照 / 片段路径都不带它们。
/// 单一归属（#71）：改键表只改 `LIVE_INTERNAL_KEYS` 一处，各路径共用本函数。
pub fn strip_internal_keys(obj: &mut serde_json::Map<String, serde_json::Value>) {
    for key in LIVE_INTERNAL_KEYS {
        obj.remove(*key);
    }
}

/// 受控字段：切换时整块替换/合并。除这些键之外的任何字段
/// （permissions / hooks / mcpServers / ...）都不是受控字段，切换时一律从
/// live 原地保留。
pub const CONTROLLED_FIELDS: &[&str] = &[
    "env",
    "includeCoAuthoredBy",
    "attribution",
    "effortLevel",
    "enabledPlugins",
    "skipWebFetchPreflight",
];

/// Claude Code 用户级 settings.json 路径（跨平台统一 `~/.claude/settings.json`）。
pub fn claude_settings_path() -> AppResult<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| AppError::Config("cannot resolve home dir".into()))?;
    Ok(home.join(".claude").join("settings.json"))
}

/// Merge 纯函数（测试接缝 1）：把目标 settingsConfig 的**受控字段**合并进当前
/// live 配置，非受控字段从 live 原样保留，不碰文件系统。
///
/// 语义：
/// - `env` 整块替换：目标有 `env`（哪怕是空对象）→ live 的 env 被整体覆盖；
///   目标没有 `env` → live 的 env 原样保留。
/// - 其余受控顶层开关：目标存在则替换，缺失则保留 live 原值。
/// - 非受控字段：一律从 live 保留；目标里的非受控字段被忽略（绝不写 live，
///   否则一切换就清空用户手动的 hooks / MCP / permissions）。
/// - 清洗：合并前剥掉目标里的内部字段（[`LIVE_INTERNAL_KEYS`]），合并后再
///   对结果剥一遍（live 里若残留旧应用的内部键也一并清掉）。
///
/// 边界：`live` 为空串/纯空白 → 视为 `{}`（没有现存配置可保留）；`live` 是
/// 非空非法 JSON 或非对象 → `Err`（解析不了就没法保留用户手动配置，宁可失败）；
/// `target` 为空串 → 视为 `{}`；`target` 非法 JSON、非对象、或 `env` 非对象
/// → `Err`（坏配置不能进用户 settings.json）。
pub fn merge_live_settings(live: &str, target: &str) -> AppResult<String> {
    let mut merged = parse_live_or_empty(live)?;
    let mut target_obj = parse_target_or_empty(target)?;

    // 清洗目标：剥内部 meta 字段，防止它们被当作受控字段带进 live。
    if let Some(obj) = target_obj.as_object_mut() {
        strip_internal_keys(obj);
    }

    // 目标 `env` 必须是对象：env 是受控字段，写盘时整块替换 live 的 env——
    // 非对象（手写/导入的坏配置）会被原样带进用户 settings.json。宁可报错
    // 阻止写盘，与「目标非法 JSON 报错」同一原则：配置坏了就显式失败。
    if let Some(env) = target_obj.get("env") {
        if !env.is_object() {
            return Err(AppError::Config(
                "provider settingsConfig env is not a JSON object".into(),
            ));
        }
    }

    // 受控合并：只从目标提取受控字段，其余一律忽略。
    let merged_obj = merged.as_object_mut().expect("merged is always an object");
    for key in CONTROLLED_FIELDS {
        if let Some(value) = target_obj.get(*key) {
            merged_obj.insert((*key).to_string(), value.clone());
        }
    }

    // 清洗结果：live 里残留的内部键也剥掉，保证写出去的 live 永远不含它们。
    if let Some(obj) = merged.as_object_mut() {
        strip_internal_keys(obj);
    }

    Ok(serde_json::to_string_pretty(&merged)?)
}

/// 读当前 live settings.json；文件不存在 → 空串（merge 视为 `{}`）。
pub fn read_live_settings(path: &Path) -> AppResult<String> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

/// 通用备份（写盘前调用，[`commit_live_file`] 与各 live_* 编排共用）：目标
/// 存在才备份，`.bak` 单份覆盖不堆积。claude `settings.json` / codex
/// `config.toml` / grok `config.toml` / opencode `opencode.json` 同一条备份
/// 规则（备份路径由文件扩展名推导，见 [`backup_path`]；gemini 的 `.env` 是
/// 无扩展名点文件，走 dotfile 专属的 `backup_env_file`）。
pub(crate) fn backup_file(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }
    fs::copy(path, backup_path(path))?;
    Ok(())
}

/// 备份路径：`settings.json` → `settings.json.bak`，`config.toml` →
/// `config.toml.bak`（保留原名，追加 `.bak` 到扩展名之后）。
pub(crate) fn backup_path(path: &Path) -> PathBuf {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_extension(format!("{ext}.bak"))
}

/// 原子写：先把内容写入同目录的临时文件（独立名字，避免并发写冲突），再改名
/// 覆盖目标。进程在写盘中途中断只会留下临时文件，不会产生半截 live 文件。
/// claude settings.json 与 codex config.toml/auth.json 共用（单一事实来源）。
pub(crate) fn atomic_write_file(path: &Path, content: &str) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Config("live file path has no parent dir".into()))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Config("live file path has no file name".into()))?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!("{}.tmp.{nanos}", file_name.to_string_lossy()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.flush()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// 按 app 读 live 文件文本：路径映射收口在 [`App::live_paths`]（单一事实来源
/// ——写盘 / 快照 / 片段提取共用）。opencode 无单份 live 配置概念 → `None`。
/// 片段提取（commands::snippet 的 T6 提取入口）与 live 导入域共用此读取面。
/// 写盘的同等分派在 `provider::activation::resolve_paths` + [`App::write_live`]。
pub fn read_app_live_texts(app: App) -> AppResult<Option<Vec<String>>> {
    let Some(paths) = app.live_paths()? else {
        return Ok(None);
    };
    let mut texts = Vec::with_capacity(paths.len());
    for p in paths {
        texts.push(read_live_settings(&p)?);
    }
    Ok(Some(texts))
}

/// 切换写盘全流程（薄壳，按序调用）：读 live → 受控合并（含清洗）→ 无变化
/// 则无操作 → 备份 .bak → 原子写。事务（无变化判定 + 备份 + 原子写）收口在
/// [`commit_live_file`]，与 codex / grok / gemini / opencode 五个 app 共用同一
/// 份语义。
pub fn switch_live_settings(path: &Path, settings_config: &str) -> AppResult<()> {
    let live = read_live_settings(path)?;
    let merged = merge_live_settings(&live, settings_config)?;
    let unchanged = content_unchanged(&live, &merged);
    commit_live_file(path, &merged, unchanged)
}

/// 写盘事务（五个 app 共用的单一归属）：`unchanged`（内容无变化）→ 无操作
/// （不备份、不写盘、不碰 mtime）；有变化 → 备份 + 原子写。`unchanged` 的
/// 判定由调用方按应用规则给出——TOML / JSON 文档用 [`content_unchanged`]，
/// opencode 用 json5 合并前后的语义比较（保用户注释/键序），gemini 的 `.env`
/// 要求「文件存在且内容不变」（缺失时即便目标为空也要建文件）。
///
/// 「无变化不写盘」此前是 codex / grok / opencode 各自的私有实现，claude /
/// gemini 缺失（重复切换同一供应商仍重写文件 + 刷新 .bak、碰 mtime）——收口
/// 后五个 app 行为一致。
pub(crate) fn commit_live_file(path: &Path, content: &str, unchanged: bool) -> AppResult<()> {
    if unchanged {
        return Ok(());
    }
    backup_file(path).and_then(|()| atomic_write_file(path, content))
}

/// 无变化判定的通用形态：trim_end 比较（容忍 toml_edit / serde_json 重写时对
/// 结尾换行的归一化）。opencode 例外——live 是 json5（注释/键序），字符串比较
/// 永远不等，改用合并前后 `Value` 相等判定（见 `live_opencode`）。
pub(crate) fn content_unchanged(old: &str, new: &str) -> bool {
    old.trim_end() == new.trim_end()
}

/// 副文件回滚（主文件写失败时恢复先写的副文件，codex auth.json / gemini
/// `.env` 共用）：写盘前存在 → 还原原文；原本不存在 → 删除。回滚自身失败只
/// eprintln 不覆盖主错误——要报告的是主错误（主文件写失败）。
pub(crate) fn rollback_side_file(path: &Path, existing: Option<&str>, context: &str) {
    let result = match existing {
        Some(text) => atomic_write_file(path, text),
        None => fs::remove_file(path).map_err(AppError::from),
    };
    if let Err(e) = result {
        eprintln!("[cc-one] {context} also failed: {e}");
    }
}

/// 双文件写盘事务的副文件（先写方）载荷与策略。codex（auth.json）与 gemini
/// （`.env`）两家「双文件写盘」的全部差异收在这一个参数里：
/// - 是否备份：`backup: None` = 不备份（codex auth.json 是凭据/登录态）；Some =
///   写副文件前先调该备份函数（gemini `.env` 是无扩展名点文件，走 dotfile
///   专属的 [`crate::provider::live_gemini::backup_env_file`]，不用通用
///   [`backup_path`] 推导）。
/// - 缺失语义：不做独立参数——由各家的 `unchanged` 判定承载（gemini 要求
///   「文件存在且内容不变」，缺失时即便目标为空也要建；codex 无 key 根本
///   不进 [`SideWrite`]）。
/// - 载荷是否条件存在：整个 [`SideWrite`] 为 `None` 即登录态版形态——不碰
///   副文件，事务退化为主文件单文件提交。
pub(crate) struct SideWrite<'a> {
    pub path: &'a Path,
    /// 副文件目标内容（受控合并产物）。
    pub content: &'a str,
    /// 内容无变化判定（按各家规则在调用方算好）；true → 不备份不写副。
    pub unchanged: bool,
    pub backup: Option<fn(&Path) -> AppResult<()>>,
    /// 写盘前副文件的现状文本：回滚依据——Some 还原原文 / None 删除新建。
    pub existing: Option<&'a str>,
    /// 主文件写失败触发回滚时的日志上下文。
    pub context: &'a str,
}

/// 双文件写盘事务（codex auth.json + config.toml / gemini `.env` +
/// settings.json 共用的次序不变量，关键次序落在本函数、可测，不再散在各家
/// 散文注释里）：配对无变化 → 整体无操作（不备份、不写盘、不碰 mtime）；
/// 否则**先写副文件**（有备份策略则先备份再原子写；任何一步失败即返回，
/// 主文件不碰），后提交主文件（[`commit_live_file`]：无变化跳过，否则备份 +
/// 原子写）；主文件失败时**回滚已写的副文件**并返回主错误——任何失败路径都
/// 不产生「副已换、主没换」的半截状态。
///
/// `main` 是 `(路径, 合并后内容, 内容无变化)` 三元组。per-app 差异全部在
/// [`SideWrite`] 参数（见其文档），本函数对两家逐字节同一逻辑。
pub(crate) fn commit_two_files(
    main: (&Path, &str, bool),
    side: Option<SideWrite<'_>>,
) -> AppResult<()> {
    let (main_path, main_content, main_unchanged) = main;
    // 配对无变化判定 + 整体无操作：side 原本是 None 或未变化都视作「副无操作」。
    let side = side.filter(|s| !s.unchanged);
    if main_unchanged && side.is_none() {
        return Ok(());
    }

    // 先写副：备份（若有策略）+ 原子写；失败在此返回，主文件未被触碰。
    let side_written = match side {
        Some(s) => {
            if let Some(backup) = s.backup {
                backup(s.path)?;
            }
            atomic_write_file(s.path, s.content)?;
            Some(s)
        }
        None => None,
    };

    // 提交主文件；失败 → 回滚先写的副文件，返回主错误。
    if let Err(e) = commit_live_file(main_path, main_content, main_unchanged) {
        if let Some(s) = side_written {
            rollback_side_file(s.path, s.existing, s.context);
        }
        return Err(e);
    }
    Ok(())
}

/// 拒绝写盘前的未物化模板变量：settingsConfig 里残留 `${VAR}` 占位符（保存时
/// 前端已拦截，但导入的 JSON 或手改的元数据可能绕过）会以字面量形式写进用户
/// 的 settings.json——端点/密钥位置全是占位符，等于写一份废配置。宁可切换
/// 失败，也不静默写废。空串 → 无占位符（写盘按 `{}` 处理）。
pub fn validate_no_unfilled_template_vars(settings_config: &str) -> AppResult<()> {
    let Some(name) = find_unfilled_template_var(settings_config) else {
        return Ok(());
    };
    Err(AppError::Config(format!(
        "provider settingsConfig has an unfilled template variable: ${{{name}}}"
    )))
}

/// 第一个 `${VAR}` 占位符名；无 → `None`。与前端 `derive.ts` 的
/// `TEMPLATE_VAR_RE` 同一形状（`${` + 标识符 + `}`）。
fn find_unfilled_template_var(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'{' {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if j > start && j < bytes.len() && bytes[j] == b'}' {
                return Some(String::from_utf8_lossy(&bytes[start..j]).into_owned());
            }
        }
        i += 1;
    }
    None
}

/// 解析 live 输入：空串/纯空白 → `{}`；非空但非法 JSON 或非对象 → `Err`。
/// `live_gemini` 复用同一条解析规则（现有 settings.json 缺失时视为 `{}`）。
pub(crate) fn parse_live_or_empty(live: &str) -> AppResult<serde_json::Value> {
    let trimmed = live.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    parse_object(trimmed, "live settings.json")
}

/// 解析目标输入：空串 → `{}`；非法 JSON 或非对象 → `Err`。
/// `live_gemini` 复用同一条解析规则（目标 settingsConfig 空串 = 空目标）。
pub(crate) fn parse_target_or_empty(target: &str) -> AppResult<serde_json::Value> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    parse_object(trimmed, "provider settingsConfig")
}

/// 解析 JSON 文本为对象：非法 JSON 或非对象 → `Err`。供本模块的
/// `parse_live_or_empty` / `parse_target_or_empty` 与 `snippet` 模块共用
/// （片段校验与合并走同一条解析规则）。
pub(crate) fn parse_object(raw: &str, what: &str) -> AppResult<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| AppError::Config(format!("{what} is not valid JSON: {e}")))?;
    if !v.is_object() {
        return Err(AppError::Config(format!("{what} is not a JSON object")));
    }
    Ok(v)
}

/// 解析 TOML 文本为可编辑文档：空串/纯空白 → 空文档；非法 TOML → `Err`。
/// codex / grok 的 TOML 受控合并共用（单一事实来源）——两个 live_* 模块都把
/// live / target 的 TOML 文本喂进来解析，不各自再抄一份。
pub(crate) fn parse_toml_or_empty(text: &str, what: &str) -> AppResult<DocumentMut> {
    if text.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    text.parse::<DocumentMut>()
        .map_err(|e| AppError::Config(format!("{what} is not valid TOML: {e}")))
}

/// 递归表级补缺失：`source` 的键 `target` 没有 → 插入；双方都是 table → 递归
/// 合并子键；其余（target 已有非 table、或类型不一致）→ target 赢，跳过。codex
/// / grok 的写盘层片段补缺失共用（单一事实来源）——保证片段的 `[mcp_servers.a]`
/// 与 live 的 `[mcp_servers.b]` 并存、同子键 live 已有则不覆盖（见 ADR-0010）。
pub(crate) fn fill_missing_table(target: &mut Table, source: &Table) {
    for (key, item) in source.iter() {
        let existing_is_table = target.get(key).is_some_and(|e| e.is_table());
        if existing_is_table && item.is_table() {
            let t = target
                .get_mut(key)
                .and_then(|e| e.as_table_mut())
                .expect("checked table");
            let s = item.as_table().expect("checked table");
            fill_missing_table(t, s);
        } else if target.get(key).is_none() {
            target.insert(key, item.clone());
        }
    }
}

/// 受控轴三态合并原语（机制唯一归属，codex / gemini 两载体共用）：`controlled`
/// 清单内的键按「目标携带 → 整体替换进 live；目标缺失 → 从 live 撤除」合并，
/// 清单外的键此循环一概不碰——「跳过」即非清单键原地保留。受控轴「新供应商
/// 赢」的落点：缺失不撤的话旧供应商的身份键残留、切换静默失效（ADR-0010）。
///
/// 两家的受控区形状不同：codex 清单即受控区（替换域 = 撤除域 = 清单，循环即
/// 全部合并逻辑）；gemini 受控区 = settings.json 顶层整体（供应商声明的一切
/// 替换，「声明即接管」，由 gemini 侧先整体替换），本原语只承担其撤除域——
/// 身份键清单（[`crate::provider::live_gemini::GEMINI_IDENTITY_FIELDS`]）。
///
/// 载体差异（toml_edit 的 `Table` 键级编辑保留注释与格式 vs `serde_json::Map`）
/// 用双形态收纳：三态语义只有本契约一份，两个载体各一段机械循环，任何一边改
/// 动语义必须同步另一边（两边的现场测试互为回归）。
pub(crate) fn merge_controlled_fields_toml(live: &mut Table, target: &Table, controlled: &[&str]) {
    for &key in controlled {
        match target.get(key) {
            Some(item) => {
                live.insert(key, item.clone());
            }
            None => {
                live.remove(key);
            }
        }
    }
}

/// JSON 载体的 [`merge_controlled_fields_toml`]（三态契约见彼处）。
pub(crate) fn merge_controlled_fields_json(
    live: &mut serde_json::Map<String, serde_json::Value>,
    target: &serde_json::Map<String, serde_json::Value>,
    controlled: &[&str],
) {
    for &key in controlled {
        match target.get(key) {
            Some(value) => {
                live.insert(key.to_string(), value.clone());
            }
            None => {
                live.remove(key);
            }
        }
    }
}

/// 从已剥内部 meta 键的 settingsConfig 对象提取 `config` TOML 字符串（codex /
/// grok 共用，两家的 settingsConfig 形状在此字段上同构）：缺失 → 空串（登录
/// 态版 / 无受控内容）；非字符串 → `Err`（坏配置不能进用户 config.toml）。
pub(crate) fn config_toml_field(obj: &serde_json::Value) -> AppResult<String> {
    match obj.get("config") {
        None => Ok(String::new()),
        Some(v) => v.as_str().map(str::to_string).ok_or_else(|| {
            AppError::Config("provider settingsConfig config must be a TOML string".into())
        }),
    }
}

/// TOML 片段校验共用骨架（codex / grok，合并层分派见 ADR-0010）：片段须是
/// 合法 TOML 且不含受控身份键——`identity_hit` 返回命中身份键的描述（用于
/// 报错指明具体键，#55：键名细节只出现在校验报错里；`None` = 未命中）。
/// set 命令的提前拦截与 [`merge_toml_snippet`] 的兜底拒绝走同一条规则。
pub(crate) fn validate_toml_snippet(
    snippet: &str,
    label: &str,
    identity_hit: impl Fn(&DocumentMut) -> Option<String>,
) -> AppResult<()> {
    let doc = parse_toml_or_empty(snippet, &format!("{label} snippet"))?;
    reject_snippet_identity(&doc, label, &identity_hit)
}

/// TOML 片段补缺失共用骨架（codex / grok）：在 merge 结果上
/// [`fill_missing_table`] 补片段键（live 已有则保留、递归进子表），身份键
/// 拒绝与 [`validate_toml_snippet`] 同一条规则（防绕过 set 的路径）。
pub(crate) fn merge_toml_snippet(
    merged: &str,
    snippet: &str,
    label: &str,
    identity_hit: impl Fn(&DocumentMut) -> Option<String>,
) -> AppResult<String> {
    let mut doc = parse_toml_or_empty(merged, "merged config.toml")?;
    let snippet_doc = parse_toml_or_empty(snippet, &format!("{label} snippet"))?;
    reject_snippet_identity(&snippet_doc, label, &identity_hit)?;
    fill_missing_table(doc.as_table_mut(), snippet_doc.as_table());
    Ok(doc.to_string())
}

/// 片段携带受控身份键 → `Err`（报错含 `identity_hit` 给出的具体键）。
fn reject_snippet_identity(
    doc: &DocumentMut,
    label: &str,
    identity_hit: &impl Fn(&DocumentMut) -> Option<String>,
) -> AppResult<()> {
    if let Some(hit) = identity_hit(doc) {
        return Err(AppError::Config(format!(
            "{label} 通用片段不得包含受控身份键 {hit}（身份键归供应商管理）"
        )));
    }
    Ok(())
}

/// 解析供应商 settingsConfig JSON 文本为「剥过内部 meta 键的对象」：空串/
/// 纯空白 → `None`（登录态版）；非对象 → `Err`。剥 [`LIVE_INTERNAL_KEYS`]——这些
/// 键只供应用自己读，不是任何写盘文件（auth.json / config.toml）的合法字段。
/// codex / grok 两个 live_* 分支共用同一条「解析 + 清洗」前缀，各自只写后段
/// 的字段提取。
pub(crate) fn parse_and_strip_settings(
    settings_config: &str,
) -> AppResult<Option<serde_json::Value>> {
    let trimmed = settings_config.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let mut obj = parse_object(trimmed, "provider settingsConfig")?;
    if let Some(o) = obj.as_object_mut() {
        strip_internal_keys(o);
    }
    Ok(Some(obj))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 构造一份带非受控字段的 live 配置（模拟用户手动配了 hooks / MCP /
    /// permissions / model 的 settings.json）。
    fn live_with_uncontrolled(env: &str) -> String {
        format!(
            r#"{{
  "env": {env},
  "permissions": {{"allow": ["Bash"]}},
  "hooks": {{"PreToolUse": [{{"matcher": "Bash"}}]}},
  "mcpServers": {{"filesystem": {{"command": "npx"}}}},
  "enableAllProjectMcpServers": true,
  "model": "claude-sonnet-4-5",
  "extraKnownMarketplaces": ["marketplace.a"],
  "statusLine": {{"type": "command", "command": "echo hi"}}
}}"#
        )
    }

    /// 解析合并结果并返回对象引用。
    fn parsed(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn controlled_env_replaces_live_wholesale() {
        let live = live_with_uncontrolled(r#"{"ANTHROPIC_MODEL": "old", "KEEP_ME": "1"}"#);
        let target = r#"{
            "env": {"ANTHROPIC_BASE_URL": "https://new.dev", "ANTHROPIC_AUTH_TOKEN": "sk-new"},
            "includeCoAuthoredBy": false
        }"#;
        let out = parsed(&merge_live_settings(&live, target).unwrap());
        // env 整块替换：live 的旧 env（含 KEEP_ME）全被覆盖。
        assert_eq!(
            out["env"],
            serde_json::json!({
                "ANTHROPIC_BASE_URL": "https://new.dev",
                "ANTHROPIC_AUTH_TOKEN": "sk-new"
            })
        );
        assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(false));
    }

    #[test]
    fn uncontrolled_fields_kept_verbatim_from_live() {
        let live = live_with_uncontrolled(r#"{"ANTHROPIC_MODEL": "old"}"#);
        let target = r#"{"env": {"ANTHROPIC_MODEL": "new"}}"#;
        let out = parsed(&merge_live_settings(&live, target).unwrap());
        // 非受控字段从 live 原样保留。
        assert_eq!(out["permissions"], serde_json::json!({"allow": ["Bash"]}));
        assert_eq!(
            out["hooks"],
            serde_json::json!({"PreToolUse": [{"matcher": "Bash"}]})
        );
        assert_eq!(
            out["mcpServers"],
            serde_json::json!({"filesystem": {"command": "npx"}})
        );
        assert_eq!(out["enableAllProjectMcpServers"], serde_json::json!(true));
        assert_eq!(out["model"], serde_json::json!("claude-sonnet-4-5"));
        assert_eq!(
            out["extraKnownMarketplaces"],
            serde_json::json!(["marketplace.a"])
        );
        assert_eq!(
            out["statusLine"],
            serde_json::json!({"type": "command", "command": "echo hi"})
        );
        // 受控字段 env 已替换。
        assert_eq!(out["env"], serde_json::json!({"ANTHROPIC_MODEL": "new"}));
    }

    #[test]
    fn target_uncontrolled_fields_are_ignored_not_written() {
        let live = live_with_uncontrolled(r#"{"ANTHROPIC_MODEL": "old"}"#);
        // 目标也带了 hooks / permissions / model——非受控，绝不能覆盖 live 的。
        let target = r#"{
            "env": {"ANTHROPIC_MODEL": "new"},
            "hooks": {"PostToolUse": [{"matcher": "*"}]},
            "permissions": {"deny": ["Bash"]},
            "model": "claude-opus-4-5"
        }"#;
        let out = parsed(&merge_live_settings(&live, target).unwrap());
        assert_eq!(
            out["hooks"],
            serde_json::json!({"PreToolUse": [{"matcher": "Bash"}]}),
            "live 的 hooks 保留，目标的 hooks 被忽略"
        );
        assert_eq!(
            out["permissions"],
            serde_json::json!({"allow": ["Bash"]}),
            "live 的 permissions 保留"
        );
        assert_eq!(out["model"], serde_json::json!("claude-sonnet-4-5"));
    }

    #[test]
    fn missing_env_in_target_keeps_live_env() {
        let live = live_with_uncontrolled(r#"{"ANTHROPIC_BASE_URL": "https://live.dev"}"#);
        // 目标没有 env（只有受控开关）——live 的 env 原样保留。
        let target = r#"{"includeCoAuthoredBy": true}"#;
        let out = parsed(&merge_live_settings(&live, target).unwrap());
        assert_eq!(
            out["env"],
            serde_json::json!({"ANTHROPIC_BASE_URL": "https://live.dev"}),
            "目标缺失 env 时不得清空 live 的 env"
        );
        assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(true));
    }

    #[test]
    fn explicit_empty_env_replaces_live_env() {
        let live = live_with_uncontrolled(r#"{"ANTHROPIC_BASE_URL": "https://live.dev"}"#);
        // 目标显式写了空 env =「该供应商不想要任何 env」→ 整块替换成空。
        let target = r#"{"env": {}}"#;
        let out = parsed(&merge_live_settings(&live, target).unwrap());
        assert_eq!(out["env"], serde_json::json!({}));
        // 非受控字段仍保留。
        assert_eq!(out["permissions"], serde_json::json!({"allow": ["Bash"]}));
    }

    #[test]
    fn empty_live_merges_to_sanitized_target() {
        let out = parsed(
            &merge_live_settings(
                "",
                r#"{"env": {"ANTHROPIC_MODEL": "m"}, "includeCoAuthoredBy": false}"#,
            )
            .unwrap(),
        );
        assert_eq!(out["env"], serde_json::json!({"ANTHROPIC_MODEL": "m"}));
        assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(false));
    }

    #[test]
    fn invalid_live_json_is_an_error() {
        let r = merge_live_settings("{not json", r#"{"env":{}}"#);
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "live 非法 JSON 必须失败"
        );
    }

    #[test]
    fn non_object_live_is_an_error() {
        let r = merge_live_settings(r#"[1,2,3]"#, r#"{"env":{}}"#);
        assert!(matches!(r, Err(AppError::Config(_))), "live 非对象必须失败");
    }

    #[test]
    fn invalid_target_json_is_an_error() {
        let r = merge_live_settings("{}", "{nope");
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "目标非法 JSON 必须失败"
        );
    }

    #[test]
    fn non_object_target_is_an_error() {
        let r = merge_live_settings("{}", r#""just a string""#);
        assert!(matches!(r, Err(AppError::Config(_))));
    }

    #[test]
    fn non_object_target_env_is_an_error() {
        // 目标 env 非对象（手写/导入的坏配置）——若放行会被整块写进用户的
        // settings.json，必须报错阻止写盘。
        for bad in [r#"{"env": "garbage"}"#, r#"{"env": ["A=1"]}"#] {
            let r = merge_live_settings("{}", bad);
            assert!(
                matches!(r, Err(AppError::Config(_))),
                "目标 env 非对象必须失败: {bad}"
            );
        }
    }

    #[test]
    fn sanitize_strips_internal_keys_from_target_and_live() {
        // 目标带着应用内部 meta 字段（cc-switch 遗留的写法）——必须被剥掉。
        let target = r#"{
            "api_format": "anthropic",
            "apiFormat": "anthropic",
            "openrouter_compat_mode": true,
            "openrouterCompatMode": true,
            "env": {"ANTHROPIC_MODEL": "m"}
        }"#;
        let out = merge_live_settings("{}", target).unwrap();
        let v = parsed(&out);
        assert!(v.get("api_format").is_none(), "api_format 必须被剥");
        assert!(v.get("apiFormat").is_none(), "apiFormat 必须被剥");
        assert!(v.get("openrouter_compat_mode").is_none());
        assert!(v.get("openrouterCompatMode").is_none());
        assert_eq!(v["env"], serde_json::json!({"ANTHROPIC_MODEL": "m"}));

        // live 里残留的内部键同样被清掉（写出去的 live 永远不含内部字段）。
        let live = r#"{"api_format": "anthropic", "permissions": {"allow": ["Bash"]}}"#;
        let out2 = merge_live_settings(live, r#"{"env":{}}"#).unwrap();
        let v2 = parsed(&out2);
        assert!(v2.get("api_format").is_none());
        assert_eq!(v2["permissions"], serde_json::json!({"allow": ["Bash"]}));
    }

    #[test]
    fn backup_creates_bak_when_live_exists_and_skips_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        // live 不存在 → 跳过备份。
        backup_file(&path).unwrap();
        assert!(!path.with_extension("json.bak").exists());

        fs::write(&path, r#"{"env":{}}"#).unwrap();
        backup_file(&path).unwrap();
        let bak = path.with_extension("json.bak");
        assert!(bak.exists(), "live 存在时必须生成 .bak");
        assert_eq!(fs::read_to_string(&bak).unwrap(), r#"{"env":{}}"#);

        // 单份覆盖：再次备份，旧 .bak 被新内容覆盖，不会堆积多份。
        fs::write(&path, r#"{"env":{"A":"2"}}"#).unwrap();
        backup_file(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&bak).unwrap(),
            r#"{"env":{"A":"2"}}"#,
            ".bak 单份覆盖，不追加不堆积"
        );
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_and_replaces_target() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        fs::write(&path, "old").unwrap();
        atomic_write_file(&path, r#"{"env":{"A":"1"}}"#).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"env":{"A":"1"}}"#);
        // 临时文件已改名，目录里没有残留 .tmp.*。
        let leftovers: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains("settings.json.tmp.")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "原子写后不得残留临时文件: {leftovers:?}"
        );
    }

    #[test]
    fn switch_live_settings_runs_full_flow_and_preserves_uncontrolled() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        fs::write(
            &path,
            live_with_uncontrolled(r#"{"ANTHROPIC_MODEL": "old"}"#),
        )
        .unwrap();

        switch_live_settings(&path, r#"{"env":{"ANTHROPIC_MODEL":"new"}}"#).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        let v = parsed(&written);
        assert_eq!(v["env"], serde_json::json!({"ANTHROPIC_MODEL": "new"}));
        assert_eq!(
            v["permissions"],
            serde_json::json!({"allow": ["Bash"]}),
            "非受控字段经完整流程后仍保留"
        );
        // 备份内容 = 写盘前的 live。
        let bak = fs::read_to_string(path.with_extension("json.bak")).unwrap();
        let bak_v = parsed(&bak);
        assert_eq!(
            bak_v["env"],
            serde_json::json!({"ANTHROPIC_MODEL": "old"}),
            ".bak 是写盘前的 live 快照"
        );
    }

    #[test]
    fn switch_live_settings_when_live_missing_creates_file_no_bak() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        switch_live_settings(&path, r#"{"env":{"ANTHROPIC_MODEL":"m"}}"#).unwrap();
        assert!(path.exists());
        let v = parsed(&fs::read_to_string(&path).unwrap());
        assert_eq!(v["env"], serde_json::json!({"ANTHROPIC_MODEL": "m"}));
        assert!(
            !path.with_extension("json.bak").exists(),
            "live 原本不存在 → 无备份"
        );
    }

    /// 重复切换同一供应商 → 无操作（不重写文件、不刷新 .bak、不碰 mtime）。
    /// 与 codex / grok / opencode 的既有语义对齐（此前 claude 缺失，重复切换
    /// 仍重写 + 刷新备份）。
    #[test]
    fn switch_live_settings_no_change_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        let target = r#"{"env":{"ANTHROPIC_MODEL":"m"}}"#;
        switch_live_settings(&path, target).unwrap();
        let before = fs::read_to_string(&path).unwrap();
        let mtime_before = fs::metadata(&path).unwrap().modified().unwrap();

        // 同一目标再切一次：内容不变、不新建 .bak、mtime 不动（睡眠跨过文件
        // 系统的时间戳粒度，mtime 相同才证明没写盘）。
        std::thread::sleep(std::time::Duration::from_millis(20));
        switch_live_settings(&path, target).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert!(
            !path.with_extension("json.bak").exists(),
            "内容无变化不得触发备份"
        );
        let mtime_after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "无变化不得重写文件（mtime 不得变化）"
        );
    }

    #[test]
    fn claude_settings_path_points_at_home() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            claude_settings_path().unwrap(),
            home.join(".claude").join("settings.json")
        );
    }

    #[test]
    fn validate_no_unfilled_template_vars_rejects_placeholders() {
        // 未物化的占位符 → 拒绝写盘。
        let bad = r#"{"env":{"ANTHROPIC_BASE_URL":"https://bedrock-runtime.${AWS_REGION}.amazonaws.com"}}"#;
        let err = validate_no_unfilled_template_vars(bad).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("AWS_REGION"), "报错要指出是哪个变量: {msg}");
        // 物化后 → 通过；空串 → 通过。
        assert!(validate_no_unfilled_template_vars(
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://bedrock-runtime.us-east-1.amazonaws.com"}}"#
        )
        .is_ok());
        assert!(validate_no_unfilled_template_vars("  ").is_ok());
    }

    // ---- 双文件写盘事务 commit_two_files ----

    use crate::provider::live_gemini::backup_env_file as dotfile_backup;

    const OLD_MAIN: &str = "old-main\n";
    const NEW_MAIN: &str = "new-main\n";
    const OLD_SIDE: &str = "old-side\n";

    /// 事务里副文件这侧的计划：不进事务（载荷缺席 = 登录态形态）/ 内容未变 /
    /// 要写新内容（带不带备份策略）。
    enum SidePlan {
        Skip,
        Unchanged,
        Changed { content: String, backup: bool },
    }

    fn side_plan_backup(plan: &SidePlan) -> Option<fn(&Path) -> AppResult<()>> {
        match plan {
            // 直接用 gemini 的生产备份函数（dotfile 语义），测试跑生产路径。
            SidePlan::Changed { backup: true, .. } => Some(dotfile_backup),
            _ => None,
        }
    }

    /// 组合矩阵：配对无变化判定、只写该写的一侧、备份随写盘出现、缺失语义——
    /// codex（auth+config）与 gemini（.env+settings）共用的全部次序不变量在此
    /// 一次性守住，两家各自的现场测试成为这套矩阵的参数化回归。
    #[test]
    fn commit_two_files_pairwise_matrix() {
        let cases: Vec<(&str, bool, SidePlan)> = vec![
            (
                "主副都无变化（副载荷缺席）→ 整体无操作",
                false,
                SidePlan::Skip,
            ),
            (
                "主无变化 + 副内容未变 → 整体无操作",
                false,
                SidePlan::Unchanged,
            ),
            ("仅主变化 → 只写主 + 主 .bak，不碰副", true, SidePlan::Skip),
            (
                "主变化 + 副未变 → 只写主，副字节不动",
                true,
                SidePlan::Unchanged,
            ),
            (
                "仅副变化 → 只写副 + 副 .bak，主不动",
                false,
                SidePlan::Changed {
                    content: "new-side\n".into(),
                    backup: true,
                },
            ),
            (
                "双侧变化 → 都写，两侧备份都在",
                true,
                SidePlan::Changed {
                    content: "new-side\n".into(),
                    backup: true,
                },
            ),
            (
                "双侧变化 + 副不备份（codex auth.json 形态）→ 都写且无副 .bak",
                true,
                SidePlan::Changed {
                    content: "new-side\n".into(),
                    backup: false,
                },
            ),
        ];

        for (desc, main_changed, plan) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let main_path = tmp.path().join("settings.json");
            let side_path = tmp.path().join("side.file");
            let side_bak = tmp.path().join("side.file.bak");
            fs::write(&main_path, OLD_MAIN).unwrap();
            fs::write(&side_path, OLD_SIDE).unwrap();

            let side_content: String = match &plan {
                SidePlan::Changed { content, .. } => content.clone(),
                _ => OLD_SIDE.to_string(),
            };
            let side_unchanged = !matches!(plan, SidePlan::Changed { .. });
            let side = if matches!(plan, SidePlan::Skip) {
                None
            } else {
                Some(SideWrite {
                    path: &side_path,
                    content: &side_content,
                    unchanged: side_unchanged,
                    backup: side_plan_backup(&plan),
                    existing: Some(OLD_SIDE),
                    context: "test",
                })
            };

            commit_two_files((&main_path, NEW_MAIN, !main_changed), side).unwrap();

            let side_written = matches!(plan, SidePlan::Changed { .. });
            // 内容与备份足证是否走写路径（原子写必改内容）：无操作 = 主字节不变
            // 且无任何 .bak。
            if main_changed {
                assert_eq!(fs::read_to_string(&main_path).unwrap(), NEW_MAIN, "{desc}");
                assert_eq!(
                    fs::read_to_string(main_path.with_extension("json.bak")).unwrap(),
                    OLD_MAIN,
                    "{desc}: 主 .bak 是写前快照"
                );
            } else {
                assert_eq!(fs::read_to_string(&main_path).unwrap(), OLD_MAIN, "{desc}");
                assert!(
                    !main_path.with_extension("json.bak").exists(),
                    "{desc}: 主未写不得备份"
                );
            }
            if side_written {
                assert_eq!(
                    fs::read_to_string(&side_path).unwrap(),
                    side_content,
                    "{desc}"
                );
                let wants_bak = side_plan_backup(&plan).is_some();
                assert_eq!(side_bak.exists(), wants_bak, "{desc}: 副 .bak 随备份策略");
                if wants_bak {
                    assert_eq!(fs::read_to_string(&side_bak).unwrap(), OLD_SIDE, "{desc}");
                }
            } else {
                assert_eq!(
                    fs::read_to_string(&side_path).unwrap(),
                    OLD_SIDE,
                    "{desc}: 副不应被写"
                );
                assert!(!side_bak.exists(), "{desc}");
            }
        }
    }

    /// 副写失败（目标位置占成目录 → 原子写必败）→ 报错且主文件不碰、不触发主
    /// 备份（codex「auth 失败不得先写 config」的原语层版本）。
    #[test]
    fn commit_two_files_side_failure_leaves_main_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let main_path = tmp.path().join("settings.json");
        let side_path = tmp.path().join("auth.json");
        fs::write(&main_path, OLD_MAIN).unwrap();
        fs::create_dir(&side_path).unwrap();

        let r = commit_two_files(
            (&main_path, NEW_MAIN, false),
            Some(SideWrite {
                path: &side_path,
                content: "new-side\n",
                unchanged: false,
                backup: None,
                existing: None,
                context: "test",
            }),
        );
        assert!(r.is_err(), "副写失败必须报错");
        assert_eq!(
            fs::read_to_string(&main_path).unwrap(),
            OLD_MAIN,
            "主文件不得先于副文件被写"
        );
        assert!(
            !main_path.with_extension("json.bak").exists(),
            "副失败不得触发主备份"
        );
    }

    /// 主写失败（主 .bak 占成目录 → 备份一步败）→ 先写的副回滚到写盘前；
    /// 副原本不存在（回滚删新建）与存在（回滚还原原文）两种形态都验，且与副
    /// 是否带备份策略无关。
    #[test]
    fn commit_two_files_main_failure_rolls_back_side() {
        for (desc, side_existed) in [
            ("副原本存在 → 还原原文", true),
            ("副原本不存在 → 删除新建", false),
        ] {
            for backup in [true, false] {
                let tmp = tempfile::tempdir().unwrap();
                let main_path = tmp.path().join("config.toml");
                let side_path = tmp.path().join(".env");
                fs::write(&main_path, "old = 1\n").unwrap();
                if side_existed {
                    fs::write(&side_path, OLD_SIDE).unwrap();
                }
                fs::create_dir(main_path.with_extension("toml.bak")).unwrap();

                let r = commit_two_files(
                    (&main_path, "new = 1\n", false),
                    Some(SideWrite {
                        path: &side_path,
                        content: "NEW=1\n",
                        unchanged: false,
                        backup: if backup { Some(dotfile_backup) } else { None },
                        existing: side_existed.then_some(OLD_SIDE),
                        context: "test rollback",
                    }),
                );
                assert!(r.is_err(), "{desc}");

                if side_existed {
                    assert_eq!(fs::read_to_string(&side_path).unwrap(), OLD_SIDE, "{desc}");
                } else {
                    assert!(!side_path.exists(), "{desc}");
                }
                assert_eq!(
                    fs::read_to_string(&main_path).unwrap(),
                    "old = 1\n",
                    "{desc}: 主不留半截内容"
                );
            }
        }
    }

    /// 缺失语义：副原本不存在 + 未变化=false → 照常创建（gemini 登录态建空
    /// `.env` 形态）、无备份；主原本不存在 → 主创建且无主备份；副载荷缺席
    /// （side=None，codex 登录态形态）→ 不碰副文件，事务退化为主文件单文件提交。
    #[test]
    fn commit_two_files_missing_file_semantics() {
        let tmp = tempfile::tempdir().unwrap();
        let main_path = tmp.path().join("settings.json");
        let side_path = tmp.path().join(".env");
        fs::write(&main_path, OLD_MAIN).unwrap();
        commit_two_files(
            (&main_path, NEW_MAIN, false),
            Some(SideWrite {
                path: &side_path,
                content: "K=V",
                unchanged: false,
                backup: Some(dotfile_backup),
                existing: None,
                context: "test",
            }),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&side_path).unwrap(), "K=V");
        assert!(
            !tmp.path().join(".env.bak").exists(),
            "副原本不存在 → 无备份"
        );

        let tmp2 = tempfile::tempdir().unwrap();
        let main2 = tmp2.path().join("settings.json");
        let side2 = tmp2.path().join("auth.json");
        commit_two_files((&main2, "{}\n", false), None).unwrap();
        assert_eq!(fs::read_to_string(&main2).unwrap(), "{}\n");
        assert!(!side2.exists(), "载荷缺席不得创建副文件");
        assert!(
            !main2.with_extension("json.bak").exists(),
            "主原本不存在 → 无备份"
        );
    }

    // ---- 受控轴三态合并原语（codex / gemini 共用，双载体同契约）----

    /// TOML 载体：清单内携带 → 替换、缺失 → 撤除；清单外（含目标声明了的
    /// 非受控键）一概不碰——「跳过」由循环边界保证。
    #[test]
    fn controlled_fields_toml_replaces_withdraws_and_skips() {
        let mut live: DocumentMut =
            "model = \"old\"\napproval_policy   =   \"on\"\n[mcp_servers.fs]\ncommand = \"npx\"\n"
                .parse()
                .unwrap();
        let target: DocumentMut = "model = \"new\"\n\n[mcp_servers.fs]\ncommand = \"python\"\n"
            .parse()
            .unwrap();
        merge_controlled_fields_toml(
            live.as_table_mut(),
            target.as_table(),
            &["model", "model_provider"],
        );
        let out = live.to_string();
        assert!(out.contains("model = \"new\""), "携带 → 替换: {out}");
        assert!(
            !out.contains("model_provider"),
            "缺失 → 撤除，不残留旧值: {out}"
        );
        assert!(
            out.contains("approval_policy   =   \"on\""),
            "清单外跳过（格式逐字节保留）: {out}"
        );
        assert!(
            out.contains("command = \"npx\""),
            "目标声明的清单外键被忽略，live 原样: {out}"
        );
    }

    /// JSON 载体：与 TOML 载体同一三态契约（携带 → 替换 / 缺失 → 撤除 /
    /// 清单外跳过）。
    #[test]
    fn controlled_fields_json_replaces_withdraws_and_skips() {
        let mut live = serde_json::json!({
            "model": "old",
            "selectedTheme": "auto",
            "mcpServers": {"fs": {"command": "npx"}}
        });
        let target =
            serde_json::json!({ "model": "new", "mcpServers": {"fs": {"command": "python"}} });
        merge_controlled_fields_json(
            live.as_object_mut().unwrap(),
            target.as_object().unwrap(),
            &["model", "model_provider"],
        );
        assert_eq!(live["model"], serde_json::json!("new"), "携带 → 替换");
        assert!(live.get("model_provider").is_none(), "缺失 → 撤除");
        assert_eq!(
            live["selectedTheme"],
            serde_json::json!("auto"),
            "清单外跳过"
        );
        assert_eq!(
            live["mcpServers"]["fs"]["command"],
            serde_json::json!("npx"),
            "目标声明的清单外键被忽略，live 原样"
        );
    }
}
