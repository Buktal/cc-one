//! The pull gate's mechanism tests — signature shape and the skip/pass
//! decision, once for every domain that shares the gate. Domain-level gate
//! behavior (what a skipped dir does or doesn't do to the store) is pinned
//! per domain in `sync::domains`' tests.

use super::*;
use crate::config::Paths;
use std::collections::HashMap;
use std::sync::Mutex;

/// Predicate standing in for the sessions domain: every regular file (the
/// tolerant reader consumes them all).
fn all_files(_: &Path) -> bool {
    true
}

/// Predicate standing in for the usage domain: `.jsonl` files only.
fn jsonl_files(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()) == Some("jsonl")
}

fn tmp_paths() -> (tempfile::TempDir, Paths) {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    (tmp, paths)
}

/// The signature tracks file NAME, LENGTH (and mtime) for exactly the files
/// the predicate accepts: an absent or non-dir path is the empty signature,
/// subdirectories are never tracked, and a non-matching file stays invisible
/// to a narrower predicate. Changes are driven through LENGTH, never a
/// presumed mtime refresh — the same-mtime-same-length rewrite race is
/// documented on the module (coarse by design; the store's primary keys
/// backstop whatever slips through).
#[test]
fn dir_sig_tracks_predicate_matching_file_names_and_lengths() {
    let (_tmp, paths) = tmp_paths();
    let dir = paths.device_data_dir("aabbccddeeff");

    // Absent dir ⇒ empty signature; so is a path that exists but is no dir.
    assert_eq!(dir_sig(&dir, all_files).unwrap(), DirSig::default());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("stray-file"), "x").unwrap();
    assert_eq!(dir_sig(&paths.db, all_files).unwrap(), DirSig::default());

    std::fs::write(dir.join("usage-2026-07-13.jsonl"), "{\"u\":1}\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "not an artifact").unwrap();
    std::fs::create_dir_all(dir.join("usage-2026-07-14.jsonl")).unwrap(); // a dir, not a file

    let jsonl = dir_sig(&dir, jsonl_files).unwrap();
    let all = dir_sig(&dir, all_files).unwrap();
    let names: Vec<&str> = jsonl.file_names().collect();
    assert_eq!(
        names,
        ["usage-2026-07-13.jsonl"],
        "predicate filters by name shape"
    );
    assert_eq!(
        all.file_names().count(),
        3,
        "every regular file, the subdirectory never tracked: {}",
        all.file_names().collect::<Vec<_>>().join(",")
    );

    // A content-changing rewrite (a peer's new row) changes the signature;
    // an untouched dir re-signs identically (the gate's skip condition).
    let s1 = dir_sig(&dir, jsonl_files).unwrap();
    std::fs::write(dir.join("usage-2026-07-13.jsonl"), "{\"u\":1}\n{\"u\":2}\n").unwrap();
    assert_ne!(
        s1,
        dir_sig(&dir, jsonl_files).unwrap(),
        "length change ⇒ sig change"
    );
    assert_eq!(
        dir_sig(&dir, jsonl_files).unwrap(),
        dir_sig(&dir, jsonl_files).unwrap(),
        "unchanged dir ⇒ stable signature"
    );
}

/// The gate protocol, end to end: a cold cache reads, the vouched read makes
/// the next check a hit, any dir mutation (rewrite, vanish, appear) re-opens
/// it, and re-vouching closes it again. This is the one test of the
/// skip/pass decision both import domains rely on.
#[test]
fn gate_skips_vouched_dirs_and_reopens_on_any_change() {
    let (_tmp, paths) = tmp_paths();
    let dir = paths.device_data_dir("aabbccddeeff");
    let file = dir.join("usage-2026-07-13.jsonl");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&file, "{\"u\":1}\n").unwrap();

    let cache = Mutex::new(HashMap::new());

    // Cold cache ⇒ miss (first pull reads).
    let sig1 = dir_sig(&dir, all_files).unwrap();
    assert!(!DirGate::new(cache.lock().unwrap()).unchanged(&dir, &sig1));
    DirGate::new(cache.lock().unwrap()).observe_all(vec![(dir.clone(), sig1.clone())]);

    // Vouched ⇒ hit: an unchanged dir is skipped without a read.
    let sig2 = dir_sig(&dir, all_files).unwrap();
    assert_eq!(sig1, sig2);
    assert!(DirGate::new(cache.lock().unwrap()).unchanged(&dir, &sig2));

    // Rewrite with a length change ⇒ the gate re-opens (peer shipped a row).
    std::fs::write(&file, "{\"u\":1}\n{\"u\":2}\n").unwrap();
    let sig3 = dir_sig(&dir, all_files).unwrap();
    assert_ne!(sig2, sig3);
    assert!(!DirGate::new(cache.lock().unwrap()).unchanged(&dir, &sig3));
    DirGate::new(cache.lock().unwrap()).observe_all(vec![(dir.clone(), sig3)]);

    // Dir vanishes (peer deleted its data) ⇒ empty sig ≠ vouched ⇒ re-opens;
    // vouching the empty sig is what makes a LATER-appearing dir read again.
    std::fs::remove_file(&file).unwrap();
    let empty = dir_sig(&dir, all_files).unwrap();
    assert_eq!(empty, DirSig::default());
    assert!(!DirGate::new(cache.lock().unwrap()).unchanged(&dir, &empty));
    DirGate::new(cache.lock().unwrap()).observe_all(vec![(dir.clone(), empty)]);

    // Dir re-appears (checkout restores the peer) ⇒ non-empty ≠ vouched
    // empty ⇒ re-opens.
    std::fs::write(&file, "{\"u\":1}\n").unwrap();
    let back = dir_sig(&dir, all_files).unwrap();
    assert!(!DirGate::new(cache.lock().unwrap()).unchanged(&dir, &back));
}
