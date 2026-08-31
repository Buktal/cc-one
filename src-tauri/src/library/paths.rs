//! Library root 边界不变量：「写盘只发生在 library root 内」的谓词收口。
//!
//! 两半分工：
//! - 词法半边 [`has_only_plain_components`] / [`subpath_rel`] /
//!   [`is_plain_entry_name`]：在进 `Path::join` 之前拒绝 `..` / 盘符前缀 /
//!   分隔符这类会跳出预期目录的分量；
//! - 规范化半边 [`within_library_root`]：对已存在的路径 canonicalize 后过
//!   `starts_with` library root，暴露已存在段里的符号链接。
//!
//! 全部接受前端路径参数的入口（scan / upload / device_summary /
//! forget_device_library 直呼，export / delete / rename / read_text 经根模块的
//! `resolve_rel`）都从 [`device_subdir`] 这一个函数定位。越界一律以
//! `library path escapes the root` 拒绝——话术单源在闸口，调用方不再手抄。

use std::path::{Component, Path, PathBuf};

use crate::config::Paths;
use crate::error::{AppError, AppResult};

/// 路径分量的词法合法性：每个分量都必须是普通名字。`.` / `..` / 盘符前缀
/// （Windows `C:\`、UNC）这些非 `Normal` 分量会让 `Path::join` 逃出预期目录
/// （`..` 原地穿越、绝对前缀整体替换 base），必须在进 join 之前拒绝——这是
/// 「写盘只发生在 library root 内」的词法半边；规范化半边见
/// [`within_library_root`]。域内私有：域外入口一律走 [`device_subdir`]，
/// 不自带词法校验（那会裂出第二份边界判断）。
fn has_only_plain_components(p: &Path) -> bool {
    p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Subpath normalised to a relative PathBuf (empty ⇒ device root). 非普通
/// 分量的 subpath 就是越界尝试，与越界同话术拒绝。
fn subpath_rel(subpath: &str) -> AppResult<PathBuf> {
    let trimmed = subpath.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok(PathBuf::new());
    }
    let rel = PathBuf::from(trimmed);
    if !has_only_plain_components(&rel) {
        return Err(AppError::Config(format!(
            "library path escapes the root: {subpath}"
        )));
    }
    Ok(rel)
}

/// 解析 library 内 `<deviceId>` 设备子树下的 `subpath` 目录，返回可用于 fs
/// 操作的路径。全部接受前端路径参数的入口（scan / upload / 根模块的
/// `device_summary` / [`super::forget_device_library`] 直呼，export /
/// delete / rename / read_text 经 [`super::resolve_rel`]）都从这一个函数
/// 定位，「写盘只发生在 library root 内」的不变量在这里收口。
///
/// 包含性谓词与 `resolve_rel` 相同（canonicalize + `starts_with` library
/// root），但目标允许尚不存在（upload 要创建、scan 里别的设备可能没有该
/// 目录）：此时对最深已存在的祖先 canonicalize——已存在段里的符号链接若
/// 指向 root 外会在此暴露；剩余分量已由 [`subpath_rel`] 限定为普通名字，
/// 拼回去不会越界。一路爬到 library root 本身都不存在时放行（root 由调用
/// 方按需创建，剩余分量纯词法，不可能越界）。
pub(super) fn device_subdir(paths: &Paths, device_id: &str, subpath: &str) -> AppResult<PathBuf> {
    if !has_only_plain_components(Path::new(device_id)) {
        return Err(AppError::Config(format!(
            "library path escapes the root: {device_id}"
        )));
    }
    let rel = subpath_rel(subpath)?;
    let candidate = paths.library.join(device_id).join(&rel);
    match candidate.canonicalize() {
        // 目标已存在：对它本身过规范化谓词。
        Ok(canon) => within_library_root(paths, &canon)?,
        // 目标不存在：最深已存在祖先过同一谓词。
        Err(_) => {
            let mut ancestor = candidate.as_path();
            loop {
                if let Ok(canon) = ancestor.canonicalize() {
                    within_library_root(paths, &canon)?;
                    break;
                }
                match ancestor.parent() {
                    // 未到 library root 就继续向上；到 root 为止都不存在
                    // 则停（同 `resolve_rel` 对空仓库的兜底），不再向上爬。
                    Some(parent) if ancestor != paths.library.as_path() => ancestor = parent,
                    _ => break,
                }
            }
        }
    }
    Ok(candidate)
}

/// 规范化后的路径必须落在 library root 内（root 尚不存在时退回原路径参与
/// 比较——与空仓库下的既有兜底一致）。谓词的规范化半边；词法半边见
/// [`has_only_plain_components`]。
fn within_library_root(paths: &Paths, canon: &Path) -> AppResult<()> {
    let root_canon = paths
        .library
        .canonicalize()
        .unwrap_or_else(|_| paths.library.clone());
    if !canon.starts_with(&root_canon) {
        return Err(AppError::Config("library path escapes the root".into()));
    }
    Ok(())
}

/// 条目名（upload 的目标名、rename 的新名）必须是单个普通路径分量：它会被
/// `Path::join` 成目标路径，`.` / `..` 会落出预期目录（`..` 配目录源甚至会
/// 在同类型覆盖分支里先把父树整个删掉），分隔符则借道子目录——都在拼路径
/// 前拒绝。`.gitkeep` 是目录封条保留名，upload 另行拒绝覆盖。
pub(super) fn is_plain_entry_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && has_only_plain_components(Path::new(name))
}
