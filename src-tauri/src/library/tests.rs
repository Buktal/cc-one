//! library 域测试：路径边界谓词（写路径直测 Standalone 不触网）、scan /
//! upload / delete / rename / read_text、对端遗忘的 migrate / delete。

use super::forget::migrate_folder_name;
use super::*;
use crate::config::Paths;

fn self_cfg() -> ConfigData {
    ConfigData {
        device_id: "aabbccddeeff".to_string(),
        ..Default::default()
    }
}

fn write_file(p: &Path, body: &str) {
    std::fs::write(p, body).unwrap();
}

// -------------------------------------------------------------------
// 写路径直测：self_cfg() 无 repo_url / token ⇒ Standalone，
// commit_push_library 在 !is_synced 时直接返回、不触网——写路径可以
// 对纯文件系统断言。
// -------------------------------------------------------------------

/// 造一个待上传源文件（staging 区，模拟用户拖拽进来的任意机器路径）。
fn stage_file(tmp: &Path, rel: &str, body: &str) -> String {
    let p = tmp.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    write_file(&p, body);
    p.to_string_lossy().to_string()
}

/// 造一个待上传源目录（staging 区），返回目录路径。
fn stage_dir(tmp: &Path, rel: &str, inner: &str, body: &str) -> String {
    let p = tmp.join(rel).join(inner);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    write_file(&p, body);
    tmp.join(rel).to_string_lossy().to_string()
}

fn upload_one(
    paths: &Paths,
    cfg: &ConfigData,
    source_path: String,
    target_name: &str,
    subpath: &str,
) -> AppResult<()> {
    upload(
        paths,
        cfg,
        &[UploadItem {
            source_path,
            target_name: target_name.to_string(),
        }],
        subpath,
    )
}

#[test]
fn upload_lands_in_device_subtree_and_overwrites_same_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();

    upload_one(
        &paths,
        &cfg,
        stage_file(tmp.path(), "staging/a.txt", "v1"),
        "notes.txt",
        "docs",
    )
    .unwrap();

    let landed = paths
        .library
        .join(&cfg.device_id)
        .join("docs")
        .join("notes.txt");
    assert_eq!(std::fs::read_to_string(&landed).unwrap(), "v1");

    // 同名同类型：静默覆盖（Git 历史兜底）。
    upload_one(
        &paths,
        &cfg,
        stage_file(tmp.path(), "staging/b.txt", "v2"),
        "notes.txt",
        "docs",
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&landed).unwrap(), "v2");
}

#[test]
fn upload_rejects_same_name_different_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();
    let dev = paths.library.join(&cfg.device_id);

    // 已存在目录 thing/：同名文件源被拒，目录原样保留。
    upload_one(
        &paths,
        &cfg,
        stage_dir(tmp.path(), "staging/thing", "inner.txt", "in"),
        "thing",
        "",
    )
    .unwrap();
    let err = upload_one(
        &paths,
        &cfg,
        stage_file(tmp.path(), "staging/f.txt", "x"),
        "thing",
        "",
    )
    .unwrap_err();
    assert!(err.to_string().contains("exists as a directory"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dev.join("thing/inner.txt")).unwrap(),
        "in"
    );

    // 已存在文件 solo.txt：同名目录源被拒。
    upload_one(
        &paths,
        &cfg,
        stage_file(tmp.path(), "staging/solo.txt", "s"),
        "solo.txt",
        "",
    )
    .unwrap();
    let err = upload_one(
        &paths,
        &cfg,
        stage_dir(tmp.path(), "staging/dirsrc", "inner.txt", "i"),
        "solo.txt",
        "",
    )
    .unwrap_err();
    assert!(err.to_string().contains("exists as a file"), "{err}");
    assert_eq!(std::fs::read_to_string(dev.join("solo.txt")).unwrap(), "s");
}

#[test]
fn upload_rejects_escaping_subpath() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();

    // `..` 穿越：拒绝，且 library 内外都没落下任何东西。
    let err = upload_one(
        &paths,
        &cfg,
        stage_file(tmp.path(), "staging/a.txt", "x"),
        "a.txt",
        "a/../../escaped",
    )
    .unwrap_err();
    assert!(err.to_string().contains("escapes"), "{err}");
    assert!(!tmp.path().join("escaped").exists());
    assert!(!paths.library.join(&cfg.device_id).exists());

    // 盘符前缀（Windows 绝对路径）会让 join 整体替换 base——必须拒绝。
    // 非 Windows 上 `C:/...` 只是普通文件名，不适用。
    if cfg!(windows) {
        let err = upload_one(
            &paths,
            &cfg,
            stage_file(tmp.path(), "staging/b.txt", "x"),
            "b.txt",
            "C:/cc-one-escape",
        )
        .unwrap_err();
        assert!(err.to_string().contains("escapes"), "{err}");
        assert!(!paths.library.join("cc-one-escape").exists());
    }
}

#[test]
fn upload_rejects_dot_target_names() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();

    upload_one(
        &paths,
        &cfg,
        stage_file(tmp.path(), "staging/keep.txt", "keep"),
        "keep.txt",
        "",
    )
    .unwrap();

    // `.` / `..` 作为目标名会把 join 落到父目录——`..` 配目录源甚至会
    // 在同类型覆盖分支里先删掉父树。必须在校名阶段拒绝、不动现有文件。
    for bad in [".", ".."] {
        let err = upload_one(
            &paths,
            &cfg,
            stage_dir(tmp.path(), "staging/dirsrc", "inner.txt", "i"),
            bad,
            "",
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid target name"), "{err}");
    }
    assert_eq!(
        std::fs::read_to_string(paths.library.join(&cfg.device_id).join("keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn scan_rejects_escaping_scope_and_subpath() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();
    let config = ConfigStore::for_test(paths, cfg.clone());
    let store = Store::open(Path::new(":memory:")).unwrap();

    // device_scope 就是 device_subdir 的 device_id 位，同吃这一套谓词。
    let err = scan(&store, &config, "../../outside", "").unwrap_err();
    assert!(err.to_string().contains("escapes"), "{err}");
    let err = scan(&store, &config, &cfg.device_id, "a/../../b").unwrap_err();
    assert!(err.to_string().contains("escapes"), "{err}");
}

#[test]
fn scan_lists_entries_from_uploaded_subtree() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();

    upload_one(
        &paths,
        &cfg,
        stage_file(tmp.path(), "staging/a.txt", "a"),
        "a.txt",
        "docs",
    )
    .unwrap();

    let config = ConfigStore::for_test(paths, cfg.clone());
    let store = Store::open(Path::new(":memory:")).unwrap();
    let entries = scan(&store, &config, &cfg.device_id, "docs").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a.txt");
    assert_eq!(entries[0].kind, LibraryKind::File);
    assert_eq!(entries[0].rel_path, format!("{}/docs/a.txt", cfg.device_id));
    assert!(entries[0].is_self);
}

#[test]
fn delete_entry_reseals_parent_and_rejects_escape() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();

    upload_one(
        &paths,
        &cfg,
        stage_file(tmp.path(), "staging/a.txt", "a"),
        "a.txt",
        "docs",
    )
    .unwrap();
    let target = paths.library.join(&cfg.device_id).join("docs/a.txt");
    assert!(target.exists());

    delete_entry(&paths, &cfg, &format!("{}/docs/a.txt", cfg.device_id)).unwrap();
    assert!(!target.exists());
    // 清空后的父目录用 .gitkeep 重封（git 才能同步住空目录）。
    assert!(paths
        .library
        .join(&cfg.device_id)
        .join("docs/.gitkeep")
        .exists());

    // `..` 开头的 rel_path 被同一谓词拒绝，删不到 library 外。
    let err = delete_entry(&paths, &cfg, "../../outside").unwrap_err();
    assert!(err.to_string().contains("escapes"), "{err}");
}

#[test]
fn rename_entry_rejects_existing_target() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();

    upload_one(
        &paths,
        &cfg,
        stage_file(tmp.path(), "staging/a.txt", "A"),
        "a.txt",
        "",
    )
    .unwrap();
    upload_one(
        &paths,
        &cfg,
        stage_file(tmp.path(), "staging/b.txt", "B"),
        "b.txt",
        "",
    )
    .unwrap();

    let err = rename_entry(&paths, &cfg, &format!("{}/a.txt", cfg.device_id), "b.txt").unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");

    // 冲突双方原样：没有先删后建的窗口。
    let dev = paths.library.join(&cfg.device_id);
    assert_eq!(std::fs::read_to_string(dev.join("a.txt")).unwrap(), "A");
    assert_eq!(std::fs::read_to_string(dev.join("b.txt")).unwrap(), "B");
}

#[test]
fn read_text_entry_returns_utf8_text() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let dev = "aabbccddeeff";
    let dir = paths.library.join(dev);
    std::fs::create_dir_all(&dir).unwrap();
    write_file(&dir.join("notes.md"), "# Hello\n\n正文内容");
    let text = read_text_entry(&paths, &format!("{dev}/notes.md")).unwrap();
    assert_eq!(text.as_deref(), Some("# Hello\n\n正文内容"));
}

#[test]
fn read_text_entry_returns_none_for_binary_and_oversized() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let dev = "aabbccddeeff";
    let dir = paths.library.join(dev);
    std::fs::create_dir_all(&dir).unwrap();
    // NUL byte in the head ⇒ binary.
    write_file(&dir.join("bin.dat"), "ok\0binary");
    assert_eq!(
        read_text_entry(&paths, &format!("{dev}/bin.dat")).unwrap(),
        None,
        "NUL in the first 8 KiB ⇒ not text"
    );
    // Over the 1 MiB cap ⇒ None (not an error).
    let big = "x".repeat((TEXT_READ_LIMIT + 1) as usize);
    write_file(&dir.join("big.txt"), &big);
    assert_eq!(
        read_text_entry(&paths, &format!("{dev}/big.txt")).unwrap(),
        None,
        "oversized file ⇒ not loaded"
    );
    // A directory is not text.
    assert_eq!(
        read_text_entry(&paths, dev).unwrap(),
        None,
        "directory ⇒ None"
    );
}

#[test]
fn read_text_entry_rejects_escape_and_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    // `..` escapes the library root.
    std::fs::create_dir_all(paths.library.join("aabbccddeeff")).unwrap();
    let err = read_text_entry(&paths, "../secret").unwrap_err();
    assert!(
        err.to_string().contains("escapes") || err.to_string().contains("not found"),
        "path escape rejected: {err}"
    );
    // Missing file errors (resolve_rel canonicalize fails).
    assert!(read_text_entry(&paths, "aabbccddeeff/nope.txt").is_err());
}

#[test]
fn migrate_moves_subtree_into_self() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();
    let peer = "112233445566";

    let peer_dir = paths.library.join(peer);
    std::fs::create_dir_all(peer_dir.join("sub")).unwrap();
    write_file(&peer_dir.join("note.txt"), "hi");
    write_file(&peer_dir.join("sub").join("inner.txt"), "yo");
    write_file(&peer_dir.join(".gitkeep"), "");

    forget_device_library(&paths, &cfg, peer, LibraryForgetAction::Migrate, "MacBook").unwrap();

    // Peer subtree gone; contents now under self/from-MacBook/.
    assert!(!peer_dir.exists());
    let moved = paths.library.join(&cfg.device_id).join("from-MacBook");
    assert!(moved.is_dir());
    assert_eq!(
        std::fs::read_to_string(moved.join("note.txt")).unwrap(),
        "hi"
    );
    assert!(moved.join("sub").is_dir());
    assert_eq!(
        std::fs::read_to_string(moved.join("sub").join("inner.txt")).unwrap(),
        "yo"
    );
}

#[test]
fn migrate_collision_appends_suffix() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();
    let peer = "112233445566";

    // Pre-existing migrate target must NOT be overwritten.
    let existing = paths.library.join(&cfg.device_id).join("from-MacBook");
    std::fs::create_dir_all(&existing).unwrap();
    write_file(&existing.join("keeper.txt"), "keep");

    let peer_dir = paths.library.join(peer);
    std::fs::create_dir_all(&peer_dir).unwrap();
    write_file(&peer_dir.join("note.txt"), "hi");

    forget_device_library(&paths, &cfg, peer, LibraryForgetAction::Migrate, "MacBook").unwrap();

    assert_eq!(
        std::fs::read_to_string(existing.join("keeper.txt")).unwrap(),
        "keep"
    );
    let moved = paths.library.join(&cfg.device_id).join("from-MacBook-2");
    assert_eq!(
        std::fs::read_to_string(moved.join("note.txt")).unwrap(),
        "hi"
    );
}

#[test]
fn delete_removes_subtree() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();
    let peer = "112233445566";

    let peer_dir = paths.library.join(peer);
    std::fs::create_dir_all(peer_dir.join("sub")).unwrap();
    write_file(&peer_dir.join("note.txt"), "hi");

    forget_device_library(&paths, &cfg, peer, LibraryForgetAction::Delete, "MacBook").unwrap();

    assert!(!peer_dir.exists());
}

#[test]
fn forget_is_noop_when_peer_has_no_library() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let cfg = self_cfg();
    // No library/<peer>/ exists for either action.
    forget_device_library(
        &paths,
        &cfg,
        "112233445566",
        LibraryForgetAction::Delete,
        "MacBook",
    )
    .unwrap();
    forget_device_library(
        &paths,
        &cfg,
        "112233445566",
        LibraryForgetAction::Migrate,
        "MacBook",
    )
    .unwrap();
    // Migrate must not spuriously create the self root on a no-op.
    assert!(!paths.library.join(&cfg.device_id).exists());
}

#[test]
fn count_subtree_excludes_gitkeep() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let peer_dir = paths.library.join("112233445566");
    std::fs::create_dir_all(peer_dir.join("d1")).unwrap();
    write_file(&peer_dir.join("a.txt"), "a");
    write_file(&peer_dir.join(".gitkeep"), "");
    write_file(&peer_dir.join("d1").join("b.txt"), "b");

    let s = count_subtree(&peer_dir);
    assert_eq!(s.files, 2.0); // a.txt + d1/b.txt (.gitkeep excluded)
    assert_eq!(s.dirs, 1.0); // d1
}

#[test]
fn count_subtree_missing_dir_is_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let s = count_subtree(&paths.library.join("nope"));
    assert_eq!(s.files, 0.0);
    assert_eq!(s.dirs, 0.0);
}

#[test]
fn migrate_folder_name_sanitises() {
    assert_eq!(
        migrate_folder_name("MacBook", "112233445566"),
        "from-MacBook"
    );
    assert_eq!(migrate_folder_name("a/b\\c", "112233445566"), "from-a_b_c");
    assert_eq!(migrate_folder_name("", "112233445566"), "from-112233445566");
    assert_eq!(
        migrate_folder_name("   ", "112233445566"),
        "from-112233445566"
    );
}
