//! Atomic file write primitive (temp + rename) — the single home for
//! "overwrite a file without ever leaving a half-written target".
//!
//! A bare `fs::write` truncates in place: a process interrupted mid-write
//! leaves a torn file. Here the payload lands in a same-directory temp file
//! first and a rename flips it into place, so the target only ever exists as
//! the old or the new complete content. Consumers today: config.json
//! ([`crate::config::ConfigStore`] — the file carries the deviceId / PAT /
//! activation state, where a torn write would fork the device identity) and
//! the provider live-file write transactions (`provider::live`). New file
//! overwrites with integrity requirements go through here, not `fs::write`.

use std::fs;
use std::io::Write;
use std::path::Path;

use crate::error::{AppError, AppResult};

/// 原子写：先把内容写入同目录的临时文件（独立名字，避免并发写冲突），再改名
/// 覆盖目标。进程在写盘中途中断只会留下临时文件，不会产生半截目标文件。
///
/// Windows 语义：`std::fs::rename` 在 Windows 映射为 `MoveFileExW` +
/// `MOVEFILE_REPLACE_EXISTING`，目标已存在时是「替换」而非报错——无需（也不
/// 应）先 `remove_file`，先删会留出一个目标短暂缺失的窗口。测试
/// `write_replaces_existing_target_and_leaves_no_temp_file` 在 Windows 上守住
/// 该语义。
///
/// 保证边界：这里 flush 的是写缓冲（不做 fsync）——防进程中断（崩溃/被杀）
/// 不产生半截文件；不承诺断电时字节已落盘。
///
/// 失败卫生：改名失败时尽力删掉自建的临时文件，失败路径不留垃圾文件（清理
/// 自身失败不掩盖改名错误）。
pub(crate) fn atomic_write_file(path: &Path, content: &str) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Config("atomic write path has no parent dir".into()))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Config("atomic write path has no file name".into()))?;
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
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
