//! `fs_atomic` 原语测试：成功路径（替换已存在目标、补建缺失父目录、无临时
//! 残留）与失败路径（改名必败的目标 → 报错、目标不动、临时文件被清理）。

use std::fs;
use std::path::Path;

use super::atomic_write_file;

/// 断言目录里没有本原语遗留的 `*.tmp.*` 临时文件。
fn assert_no_temp_residue(dir: &Path) {
    let leftovers: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "不得残留临时文件: {leftovers:?}");
}

/// 成功 + 已存在目标被整文件替换（Windows 上 rename 映射
/// `MOVEFILE_REPLACE_EXISTING`，本测试在 Windows 上守住「替换而非报错」的
/// 语义），且临时文件已改名，目录里没有 `.tmp.*` 残留。
#[test]
fn write_replaces_existing_target_and_leaves_no_temp_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("settings.json");
    fs::write(&path, "old").unwrap();
    atomic_write_file(&path, r#"{"env":{"A":"1"}}"#).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"env":{"A":"1"}}"#);
    assert_no_temp_residue(tmp.path());
}

/// 目标所在目录不存在 → 原语内建补建（`create_dir_all`），写入成功。
#[test]
fn write_creates_missing_parent_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nested").join("dir").join("file.json");
    atomic_write_file(&path, "v1").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "v1");
    assert_no_temp_residue(tmp.path());
}

/// 失败：目标位置被目录占住 → 改名必败（Windows 与 Unix 都不允许文件改名到
/// 目录上）→ 原语报错、目标目录原样、自建临时文件被清理（失败路径不留
/// 垃圾文件）。
#[test]
fn rename_failure_reports_error_cleans_temp_and_keeps_target() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("occupied");
    fs::create_dir(&path).unwrap();
    assert!(atomic_write_file(&path, "content").is_err());
    assert!(path.is_dir(), "目录目标不得被破坏");
    assert_no_temp_residue(tmp.path());
}
