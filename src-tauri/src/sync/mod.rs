//! GitHub-repo sync over libgit2, split into three layers:
//!   - [`git`] — pure libgit2 primitives (credential callback, open/clone,
//!     pull, rebase, commit, push, status queries). Knows nothing about the
//!     Local Store or dirty flags.
//!   - [`domains`] — the per-syncable-domain pairs (usage / sessions /
//!     providers / devices): each domain's push-side materialize + pull-side
//!     import, and the shared atomic dirty-flag clear. Knows nothing about git.
//!   - [`flow`] — the high-level pull → import → commit → push pipeline that
//!     composes the git primitives with the domain pairs.
//!
//! Synced-mode only: the high-level entry (`ensure_repo`) refuses to run unless
//! a repo URL *and* a PAT are configured, so Standalone mode never touches a
//! remote. Auth is an in-process git2 credential callback — the fine-grained PAT
//! lives only in Rust memory; it never appears in the URL, a credential helper,
//! or an env var. Public API is re-exported below so command-layer callers keep
//! using `crate::sync::*` unchanged.

mod domains;
mod flow;
mod git;

// The remote probe (Settings「测试连接」) is an independent feature with its own
// types (`VerifyReport`) and error model (a failed probe is `ok: false`, never
// an `AppError`); see `remote_probe`. Re-exported here so the command layer
// keeps using `crate::sync::verify_remote` / `crate::sync::VerifyReport`, which
// leaves the tauri-specta binding for `verify_sync_repo` unchanged.
mod remote_probe;
pub use remote_probe::{verify_remote, VerifyReport};

// ---- Re-exports: the crate's public sync API ----
// Only items the command layer consumes via `crate::sync::*` are `pub use`
// here; sync-internal primitives the tests touch are cfg(test) imports, so
// non-test builds stay free of unused-import warnings (a `pub use` of a symbol
// no outer module references is itself flagged unused).

// Low-level git primitive used outside sync.
pub use git::reset_local_git;
// High-level flow entries used outside sync.
pub use flow::{commit_and_push_best_effort, pull_and_import, push_usage, push_usage_best_effort};
// (verify_remote / VerifyReport are re-exported above, next to `mod remote_probe`.)

// `seed_remote` is reached by `sync::remote_probe::tests` as
// `crate::sync::seed_remote`; cfg(test) — it only exists under tests.
#[cfg(test)]
pub(crate) use git::seed_remote;

// Test-only imports — the sync tests exercise the internal git primitives and
// the fallible (non-best-effort) flow entries directly. cfg(test) keeps
// non-test builds from warning about unused imports.
#[cfg(test)]
use crate::config::ConfigData;
#[cfg(test)]
use crate::error::AppError;
#[cfg(test)]
use crate::sessions::snapshot_policy::presence_mismatches;
#[cfg(test)]
use flow::commit_and_push;
#[cfg(test)]
use git::{
    commit_all, ensure_repo, has_changes, is_ahead_of_origin, open_or_clone, pull, push,
    rebase_and_push, require_synced, PullOutcome,
};
#[cfg(test)]
use git2::{Repository, ResetType};

#[cfg(test)]
mod tests {
    use super::*;

    /// A Synced-mode config (values are trimmed by `require_synced`).
    fn synced_cfg(repo_url: &str, github_token: &str) -> ConfigData {
        ConfigData {
            repo_url: Some(repo_url.into()),
            github_token: Some(github_token.into()),
            ..Default::default()
        }
    }

    #[test]
    fn require_synced_guard() {
        // Standalone ⇒ refused.
        assert!(matches!(
            require_synced(&ConfigData::default()).unwrap_err(),
            AppError::Sync(_)
        ));

        // Synced ⇒ returns trimmed url + token.
        let (u, t) =
            require_synced(&synced_cfg("  https://github.com/x/y  ", "  ghp_t  ")).unwrap();
        assert_eq!(u, "https://github.com/x/y");
        assert_eq!(t, "ghp_t");

        // Token present but blank ⇒ Standalone.
        assert!(matches!(
            require_synced(&synced_cfg("  https://github.com/x/y  ", "   ")).unwrap_err(),
            AppError::Sync(_)
        ));
    }

    #[test]
    fn clone_sees_seeded_content_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();

        let dest = tmp.path().join("device-b");
        let repo = open_or_clone(&url, &dest, "").unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("README"))
                .unwrap()
                .trim_end(),
            "cc-one sync seed"
        );
        drop(repo);

        // Second call reopens the existing repo (does not re-clone).
        let _repo2 = open_or_clone(&url, &dest, "").unwrap();
    }

    #[test]
    fn ensure_repo_clones_when_synced_then_opens() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        // Local file:// transport needs no auth; the token is unused but keeps
        // the config in Synced mode so the guard passes.
        let cfg = synced_cfg(&remote.to_string_lossy(), "local-no-auth");

        let dir = tmp.path().join("dev");
        let _r1 = ensure_repo(&cfg, &dir).unwrap(); // clones
        assert!(dir.join(".git").exists());
        assert!(dir.join("README").exists());
        let _r2 = ensure_repo(&cfg, &dir).unwrap(); // opens (idempotent)
    }

    #[test]
    fn ensure_repo_refuses_standalone() {
        let cfg = ConfigData::default(); // Standalone
        let tmp = tempfile::tempdir().unwrap();
        // Repository doesn't impl Debug, so match on the Result directly.
        assert!(matches!(
            ensure_repo(&cfg, tmp.path()),
            Err(AppError::Sync(_))
        ));
    }

    /// End-to-end Standalone→Synced against an EMPTY remote: `local` already holds
    /// collected artifacts and no `.git`. open_or_clone bootstraps in place, the
    /// first commit+push creates the branch and ships the local data upstream.
    #[test]
    fn open_or_clone_then_push_ships_local_data_into_empty_remote() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        Repository::init_bare(&remote).unwrap(); // empty — unborn
                                                 // GitHub's empty repos default to `main`; mirror that so the bare HEAD
                                                 // lines up with the branch we push (libgit2's init_bare defaults `master`).
        std::fs::write(remote.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let url = remote.to_string_lossy().to_string();

        let local = tmp.path().join("device");
        let artifact = local
            .join("data")
            .join("localdev")
            .join("usage-2026-07-22.jsonl");
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, "{\"uuid\":\"local-1\"}\n").unwrap();

        // Non-empty `local`, no `.git` ⇒ init_with_remote (unborn HEAD).
        let repo = open_or_clone(&url, &local, "").unwrap();
        commit_all(&repo, "first sync", "DevA", "a@devices.cc-one").unwrap();
        push(&repo, "").unwrap();

        // The first push creates our pinned default branch `main`, not master.
        let bare = Repository::open_bare(&remote).unwrap();
        assert!(
            bare.refname_to_id("refs/heads/main").is_ok(),
            "first push must create `main`, not libgit2's default `master`"
        );

        // A fresh clone now sees the local artifact on the remote.
        let check = tmp.path().join("check");
        let _r2 = open_or_clone(&url, &check, "").unwrap();
        assert!(check.join("data/localdev/usage-2026-07-22.jsonl").exists());
    }

    /// `clear_sync_repo` drops `.git` so a re-bind starts clean, but must leave
    /// the per-device usage artifacts (`data/`) intact — Standalone keeps
    /// writing there and they are not git state.
    #[test]
    fn reset_local_git_removes_dot_git_but_keeps_data() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join("data/dev")).unwrap();
        std::fs::write(repo.join("data/dev/usage.jsonl"), "{}\n").unwrap();
        Repository::init(&repo).unwrap();
        assert!(repo.join(".git").exists());

        reset_local_git(&repo);

        assert!(!repo.join(".git").exists(), "clear must drop .git");
        assert!(
            repo.join("data/dev/usage.jsonl").exists(),
            "usage artifacts must survive a clear"
        );
    }

    /// Unbind (`reset_local_git`) then re-bind the SAME repo: locally-collected
    /// `data/` survives, the remote history is re-fetched, and a fresh
    /// collect+push round-trips. Proves clearing `.git` on unbind loses nothing
    /// when the user re-binds the very same repo.
    #[test]
    fn rebind_same_repo_after_reset_keeps_data_and_resyncs() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();

        // Bind, collect own data, push.
        let local = tmp.path().join("device");
        let repo = open_or_clone(&url, &local, "").unwrap();
        let own = local.join("data/localdev");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("usage-2026-07-22.jsonl"), "{\"uuid\":\"x\"}\n").unwrap();
        commit_all(&repo, "collect", "Dev", "d@devices.cc-one").unwrap();
        push(&repo, "").unwrap();
        drop(repo);

        // Unbind: `.git` dropped, `data/` kept.
        reset_local_git(&local);
        assert!(!local.join(".git").exists());
        assert!(own.join("usage-2026-07-22.jsonl").exists());

        // Re-bind the same repo: re-init + fetch + SAFE checkout.
        let repo2 = open_or_clone(&url, &local, "").unwrap();
        assert!(
            own.join("usage-2026-07-22.jsonl").exists(),
            "own data survives rebind"
        );
        assert!(local.join("README").exists(), "remote content re-lands");

        // A new collect after rebind commits + pushes cleanly.
        std::fs::write(own.join("usage-2026-07-23.jsonl"), "{\"uuid\":\"y\"}\n").unwrap();
        commit_all(&repo2, "collect 2", "Dev", "d@devices.cc-one").unwrap();
        push(&repo2, "").unwrap();

        // A fresh device sees both days.
        let check = tmp.path().join("check");
        let _r3 = open_or_clone(&url, &check, "").unwrap();
        assert!(check.join("data/localdev/usage-2026-07-22.jsonl").exists());
        assert!(check.join("data/localdev/usage-2026-07-23.jsonl").exists());
    }

    #[test]
    fn two_devices_sync_via_push_and_pull() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();

        // Device A clones, writes its per-device usage artifact, commits, pushes.
        let dir_a = tmp.path().join("a");
        let repo_a = open_or_clone(&url, &dir_a, "").unwrap();
        let a_data = dir_a.join("data/dev_a");
        std::fs::create_dir_all(&a_data).unwrap();
        std::fs::write(a_data.join("usage-2026-07-16.jsonl"), "{\"uuid\":\"u1\"}\n").unwrap();
        commit_all(&repo_a, "device A usage", "DevA", "a@devices.cc-one").unwrap();
        push(&repo_a, "").unwrap();

        // Device B clones and immediately sees A's artifact.
        let dir_b = tmp.path().join("b");
        let repo_b = open_or_clone(&url, &dir_b, "").unwrap();
        assert!(dir_b.join("data/dev_a/usage-2026-07-16.jsonl").exists());

        // A pushes a second day; B pulls and sees it (fast-forward).
        std::fs::write(a_data.join("usage-2026-07-17.jsonl"), "{\"uuid\":\"u2\"}\n").unwrap();
        commit_all(&repo_a, "device A day 2", "DevA", "a@devices.cc-one").unwrap();
        push(&repo_a, "").unwrap();
        pull(&repo_b, "").unwrap();
        assert!(dir_b.join("data/dev_a/usage-2026-07-17.jsonl").exists());

        // B's local artifact (its own device subtree) survives the pull untouched.
        let b_data = dir_b.join("data/dev_b");
        std::fs::create_dir_all(&b_data).unwrap();
        std::fs::write(b_data.join("usage-2026-07-16.jsonl"), "{\"uuid\":\"b1\"}\n").unwrap();
        pull(&repo_b, "").unwrap();
        assert_eq!(
            std::fs::read_to_string(b_data.join("usage-2026-07-16.jsonl")).unwrap(),
            "{\"uuid\":\"b1\"}\n",
            "B's own untracked artifact must survive a fast-forward pull"
        );
    }

    // ---- S2b high-level flow tests ----

    fn raw_usage(uuid: &str) -> crate::source_parser::RawUsage {
        use crate::model::{ServerToolUse, TokenCounts};
        crate::source_parser::RawUsage {
            uuid: uuid.into(),
            timestamp: "2026-07-13T16:55:22.467Z".into(),
            model: "glm-5.2".into(),
            source: "claude_code".into(),
            session_id: String::new(),
            tokens: TokenCounts {
                input: 1000,
                output: 500,
                cache_creation: 0,
                cache_read: 0,
            },
            server_tool_use: ServerToolUse::default(),
            stop_reason: "end_turn".into(),
            service_tier: "standard".into(),
            iterations: 0,
        }
    }

    #[test]
    fn pull_and_import_brings_remote_artifacts_into_store() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();

        // Device A: clone, write a usage artifact, commit, push.
        let paths_a = crate::config::Paths::resolve(&tmp.path().join("a"));
        let repo_a = open_or_clone(&url, &paths_a.repo, "").unwrap();
        let book = crate::pricing::seed_book();
        let rec = crate::collect::ingest::recordify(&raw_usage("import-1"), "aabbccddeeff", &book);
        crate::collect::artifact::append_jsonl(&paths_a, "aabbccddeeff", &[rec]).unwrap();
        commit_all(&repo_a, "A usage", "DevA", "a@devices.cc-one").unwrap();
        push(&repo_a, "").unwrap();

        // Device B: pull_and_import into a fresh in-memory store.
        let paths_b = crate::config::Paths::resolve(&tmp.path().join("b"));
        let cfg_b = synced_cfg(&url, "tok");
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        let n = pull_and_import(&store, &paths_b, &cfg_b).unwrap();
        assert_eq!(n, 1, "one new record imported from A");
        let stats = store
            .query_stats(&crate::model::UsageFilter::default())
            .unwrap();
        assert_eq!(stats.request_count, 1);

        // Re-pulling is a no-op (uuid already in the store).
        let n2 = pull_and_import(&store, &paths_b, &cfg_b).unwrap();
        assert_eq!(n2, 0, "re-pull dedups via the store's primary key");
    }

    /// Regression: `pull` used to be fast-forward-only and errored on divergent
    /// histories ("pull would diverge on 'main'; refusing to auto-merge") — the
    /// exact state a lost push race leaves a device in, with no way out. pull now
    /// rebases local-only commits onto the remote tip and pushes, so BOTH
    /// devices' data survive on the remote (a soft/reset-only fix would replay
    /// the local tree verbatim and silently clobber the peer's data).
    #[test]
    fn pull_rebases_diverged_local_commits_onto_remote_tip() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();

        // A: baseline under its own data dir + push (remote = A1).
        let paths_a = crate::config::Paths::resolve(&tmp.path().join("a"));
        let repo_a = open_or_clone(&url, &paths_a.repo, "").unwrap();
        let a_file = paths_a
            .repo
            .join("data/aaaaaaaaaaaa/usage-2026-07-30.jsonl");
        std::fs::create_dir_all(a_file.parent().unwrap()).unwrap();
        std::fs::write(&a_file, "a-1\n").unwrap();
        commit_all(&repo_a, "A1", "A", "a@devices.cc-one").unwrap();
        push(&repo_a, "").unwrap();

        // Peer B pushes under its OWN data dir (remote = B1 on A1).
        let paths_b = crate::config::Paths::resolve(&tmp.path().join("b"));
        let repo_b = open_or_clone(&url, &paths_b.repo, "").unwrap();
        let b_file = paths_b
            .repo
            .join("data/bbbbbbbbbbbb/usage-2026-07-30.jsonl");
        std::fs::create_dir_all(b_file.parent().unwrap()).unwrap();
        std::fs::write(&b_file, "b-1\n").unwrap();
        commit_all(&repo_b, "B1", "B", "b@devices.cc-one").unwrap();
        push(&repo_b, "").unwrap();
        drop(repo_b);

        // A commits a second local-only change WITHOUT pushing ⇒ diverge.
        std::fs::write(&a_file, "a-1\na-2\n").unwrap();
        commit_all(&repo_a, "A2", "A", "a@devices.cc-one").unwrap();

        // pull surfaces the diverge (does NOT rebase/push itself); rebase_and_push
        // self-heals — rebases A2 onto B1 and pushes — as an explicit step.
        let outcome = pull(&repo_a, "").unwrap();
        let upstream = match outcome {
            PullOutcome::Diverged(u) => u,
            _ => panic!("expected PullOutcome::Diverged after A's local-only commit"),
        };
        rebase_and_push(&repo_a, &upstream, "", "A", "a@devices.cc-one").unwrap();

        // A fresh clone sees BOTH devices' data — A's local-only a-2 change
        // landed on top of B1 without clobbering B (a soft/reset-only fix would
        // replay A's tree verbatim and B's data would vanish from the remote).
        let paths_c = crate::config::Paths::resolve(&tmp.path().join("c"));
        let _repo_c = open_or_clone(&url, &paths_c.repo, "").unwrap();
        let a_text = std::fs::read_to_string(
            paths_c
                .repo
                .join("data/aaaaaaaaaaaa/usage-2026-07-30.jsonl"),
        )
        .unwrap();
        assert!(
            a_text.contains("a-2"),
            "A's local-only change reached the remote: {a_text}"
        );
        let b_text = std::fs::read_to_string(
            paths_c
                .repo
                .join("data/bbbbbbbbbbbb/usage-2026-07-30.jsonl"),
        )
        .unwrap();
        assert!(
            b_text.contains("b-1"),
            "B's data survived the rebase: {b_text}"
        );
    }

    /// Regression: when a device's local commit duplicates a patch a peer
    /// already pushed (e.g. the same device-cleanup run on two machines),
    /// rebase reports the local copy as "already applied". pull must drop it
    /// and keep rebasing instead of aborting the whole sync — the stuck
    /// diverge 1.5.0 hit on Ubuntu when a device-cleanup landed on the remote
    /// first.
    #[test]
    fn pull_rebase_skips_local_commit_whose_patch_is_already_upstream() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let rel_a = "data/aaaaaaaaaaaa/usage-2026-07-30.jsonl";
        let rel_b = "data/bbbbbbbbbbbb/usage-2026-07-30.jsonl";

        // A writes one file under its data dir and pushes (remote = A1).
        let paths_a = crate::config::Paths::resolve(&tmp.path().join("a"));
        let repo_a = open_or_clone(&url, &paths_a.repo, "").unwrap();
        let a_file = paths_a.repo.join(rel_a);
        std::fs::create_dir_all(a_file.parent().unwrap()).unwrap();
        std::fs::write(&a_file, "a-1\n").unwrap();
        commit_all(&repo_a, "A1", "A", "a@devices.cc-one").unwrap();
        push(&repo_a, "").unwrap();
        drop(repo_a);

        // B clones (sees A1) then rewinds to the seed base — simulating B
        // never pulling A1 and independently making the SAME change A1 did.
        let paths_b = crate::config::Paths::resolve(&tmp.path().join("b"));
        let repo_b = open_or_clone(&url, &paths_b.repo, "").unwrap();
        let head_b = repo_b.head().unwrap().peel_to_commit().unwrap();
        let base = head_b.parents().next().unwrap();
        repo_b
            .reset(
                &repo_b.find_object(base.id(), None).unwrap(),
                ResetType::Hard,
                None,
            )
            .unwrap();
        // B_dup: an identical patch to A1 (same file, same contents).
        let a_file_b = paths_b.repo.join(rel_a);
        std::fs::create_dir_all(a_file_b.parent().unwrap()).unwrap();
        std::fs::write(&a_file_b, "a-1\n").unwrap();
        commit_all(&repo_b, "B dup of A1", "B", "b@devices.cc-one").unwrap();
        // B_unique: B's own data, not yet on the remote.
        let b_file = paths_b.repo.join(rel_b);
        std::fs::create_dir_all(b_file.parent().unwrap()).unwrap();
        std::fs::write(&b_file, "b-1\n").unwrap();
        commit_all(&repo_b, "B unique", "B", "b@devices.cc-one").unwrap();

        // pull surfaces the diverge (does NOT rebase/push itself); rebase_and_push
        // self-heals — drops B_dup (patch == A1, already upstream), rebases
        // B_unique onto A1, and pushes — as an explicit step.
        let outcome = pull(&repo_b, "").unwrap();
        let upstream = match outcome {
            PullOutcome::Diverged(u) => u,
            _ => panic!("expected PullOutcome::Diverged after B's local-only commits"),
        };
        rebase_and_push(&repo_b, &upstream, "", "B", "b@devices.cc-one").unwrap();

        // A fresh clone sees A's file (from A1) AND B's unique file; B_dup was
        // skipped rather than turning the rebase into a conflict.
        let paths_c = crate::config::Paths::resolve(&tmp.path().join("c"));
        let _repo_c = open_or_clone(&url, &paths_c.repo, "").unwrap();
        let a_text = std::fs::read_to_string(paths_c.repo.join(rel_a)).unwrap();
        assert!(a_text.contains("a-1"), "A's data still on remote: {a_text}");
        let b_text = std::fs::read_to_string(paths_c.repo.join(rel_b)).unwrap();
        assert!(
            b_text.contains("b-1"),
            "B's unique data reached the remote: {b_text}"
        );
    }

    /// New semantic (ticket 02): a row collect writes to the store — but NOT to a
    /// file, and NOT yet pushed — survives a pull that force-checks-out the
    /// worktree, because it lives in the store (pull only ADDS to the store). The
    /// next push recomputes the dirty day from the store and ships it to git.
    /// Replaces the old own-data-snapshot test: collect no longer appends files
    /// between pushes, so there is no uncommitted file-append to protect.
    #[test]
    fn unpushed_collect_survives_pull_and_reaches_git_on_next_push() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let dev_a = "aaaaaaaaaaaa";

        // A clones + collects one row into its store (dirty). No file, no push.
        let paths_a = crate::config::Paths::resolve(&tmp.path().join("a"));
        let _repo_a = open_or_clone(&url, &paths_a.repo, "").unwrap();
        let cfg_a = ConfigData {
            repo_url: Some(url.clone()),
            github_token: Some("tok".into()),
            device_id: dev_a.into(),
            ..Default::default()
        };
        let store_a = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        let book = crate::pricing::seed_book();
        let rec = crate::collect::ingest::recordify(&raw_usage("a-1"), dev_a, &book);
        store_a
            .ingest_marking_dirty(std::slice::from_ref(&rec))
            .unwrap();
        let day_file = paths_a
            .device_data_dir(dev_a)
            .join("usage-2026-07-13.jsonl");
        assert!(!day_file.exists(), "collect wrote the store, not a file");

        // Peer B advances the remote tip so A's pull fast-forwards + force-checks-out
        // the worktree. A's unpushed row is safe in the store.
        let paths_b = crate::config::Paths::resolve(&tmp.path().join("b"));
        let repo_b = open_or_clone(&url, &paths_b.repo, "").unwrap();
        let b_file = paths_b
            .device_data_dir("bbbbbbbbbbbb")
            .join("usage-2026-07-30.jsonl");
        std::fs::create_dir_all(b_file.parent().unwrap()).unwrap();
        std::fs::write(&b_file, "{\"uuid\":\"b-1\"}\n").unwrap();
        commit_all(&repo_b, "B new data", "B", "b@devices.cc-one").unwrap();
        push(&repo_b, "").unwrap();

        // A pulls (imports B; force-checkout rewrites the worktree — A's row is
        // untouched in the store, and still no file for it).
        pull_and_import(&store_a, &paths_a, &cfg_a).unwrap();
        assert!(
            !day_file.exists(),
            "pull does not write A's file either; the row lives in the store"
        );

        // A pushes: recompute the dirty day from the store ⇒ file ⇒ commit ⇒ push,
        // and the dirty day is cleared on success.
        let pushed = push_usage(&store_a, &paths_a, &cfg_a).unwrap();
        assert!(pushed, "A had its collected day to recompute + push");
        assert!(
            store_a.dirty_days().unwrap().is_empty(),
            "successful push clears the dirty day"
        );
        assert!(day_file.exists(), "push materialized A's day file");

        // A fresh clone + pull sees A's row on the remote.
        let paths_c = crate::config::Paths::resolve(&tmp.path().join("c"));
        let _repo_c = open_or_clone(&url, &paths_c.repo, "").unwrap();
        let store_c = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        pull_and_import(&store_c, &paths_c, &synced_cfg(&url, "tok")).unwrap();
        let stats_a = store_c
            .query_stats(&crate::model::UsageFilter {
                device_scope: Some(dev_a.into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            stats_a.request_count, 1,
            "A's unpushed row reached the remote after the next push"
        );
    }

    #[test]
    fn commit_and_push_is_noop_when_worktree_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let paths = crate::config::Paths::resolve(&tmp.path().join("dev"));
        let cfg = synced_cfg(&url, "tok");
        // Clone ⇒ clean worktree ⇒ nothing to push.
        let _repo = open_or_clone(&url, &paths.repo, "").unwrap();
        let pushed = commit_and_push(&paths, &cfg, "cc-one: usage sync").unwrap();
        assert!(!pushed, "clean worktree ⇒ no commit/push");
    }

    /// Regression (review): a push that failed after its commit landed leaves a
    /// clean worktree whose branch is ahead of origin — the retry must still
    /// push. The pre-review `has_changes`-only gate no-op'd the retry, stranding
    /// the commit until an unrelated change re-dirtied the worktree.
    #[test]
    fn commit_and_push_retries_ahead_of_origin_with_clean_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let paths = crate::config::Paths::resolve(&tmp.path().join("dev"));
        let cfg = synced_cfg(&url, "tok");
        let repo = open_or_clone(&url, &paths.repo, "").unwrap();

        // A pushed commit: ships one day file to the remote.
        let f1 = paths
            .device_data_dir("aaaaaaaaaaaa")
            .join("usage-2026-07-13.jsonl");
        std::fs::create_dir_all(f1.parent().unwrap()).unwrap();
        std::fs::write(&f1, "{\"uuid\":\"a-1\"}\n").unwrap();
        assert!(commit_and_push(&paths, &cfg, "cc-one: usage sync").unwrap());

        // Simulate the failed-push residue: a second commit that the remote
        // never got — worktree clean, local branch ahead.
        let f2 = paths
            .device_data_dir("aaaaaaaaaaaa")
            .join("usage-2026-07-14.jsonl");
        std::fs::write(&f2, "{\"uuid\":\"a-2\"}\n").unwrap();
        commit_all(
            &repo,
            "usage sync (push failed)",
            "cc one",
            "a@devices.cc-one",
        )
        .unwrap();
        assert!(
            !has_changes(&repo).unwrap(),
            "worktree clean after the commit"
        );
        assert!(
            is_ahead_of_origin(&repo).unwrap(),
            "local branch ahead of origin"
        );

        // Retry: push must happen despite the clean worktree.
        assert!(commit_and_push(&paths, &cfg, "cc-one: usage sync").unwrap());
        assert!(
            !is_ahead_of_origin(&repo).unwrap(),
            "the stranded commit shipped"
        );
    }

    /// Regression (review): push_usage recovers the same stranded-commit state
    /// end to end — the retry ships the leftover commit, the recomputed day is
    /// already on git, and the dirty day is cleared.
    #[test]
    fn push_usage_recovers_stranded_commit_and_clears_days() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let paths = crate::config::Paths::resolve(&tmp.path().join("dev"));
        let cfg = ConfigData {
            repo_url: Some(url.clone()),
            github_token: Some("tok".into()),
            device_id: "aaaaaaaaaaaa".into(),
            ..Default::default()
        };
        let repo = open_or_clone(&url, &paths.repo, "").unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();

        // Collect one row into the store (dirty, no file).
        let book = crate::pricing::seed_book();
        let rec = crate::collect::ingest::recordify(&raw_usage("a-1"), "aaaaaaaaaaaa", &book);
        store
            .ingest_marking_dirty(std::slice::from_ref(&rec))
            .unwrap();

        // Stranded-commit state: the day file is committed but the push never
        // landed (worktree clean, branch ahead).
        let day_file = paths
            .device_data_dir("aaaaaaaaaaaa")
            .join("usage-2026-07-13.jsonl");
        std::fs::create_dir_all(day_file.parent().unwrap()).unwrap();
        std::fs::write(&day_file, "{\"uuid\":\"a-1\"}\n").unwrap();
        commit_all(
            &repo,
            "usage sync (push failed)",
            "cc one",
            "a@devices.cc-one",
        )
        .unwrap();
        assert!(!has_changes(&repo).unwrap());

        // push_usage: recompute is byte-identical (no worktree churn), but the
        // retry must still ship the stranded commit and clear the day.
        assert!(push_usage(&store, &paths, &cfg).unwrap());
        assert!(
            store.dirty_days().unwrap().is_empty(),
            "push landed ⇒ dirty day cleared"
        );
        assert!(
            !is_ahead_of_origin(&repo).unwrap(),
            "stranded commit shipped"
        );
    }

    /// push_usage with no dirty days and a clean worktree is a no-op: it does not
    /// push, does not error, and (trivially) clears nothing.
    #[test]
    fn push_usage_is_noop_with_nothing_dirty() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let paths = crate::config::Paths::resolve(&tmp.path().join("dev"));
        let cfg = synced_cfg(&url, "tok");
        let _repo = open_or_clone(&url, &paths.repo, "").unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        let pushed = push_usage(&store, &paths, &cfg).unwrap();
        assert!(!pushed, "no dirty days + clean worktree ⇒ no push");
        assert!(store.dirty_days().unwrap().is_empty());
    }

    /// Session snapshots round-trip across devices and un-favorite propagates:
    /// A favorites + pushes a snapshot; B pulls it in (meta + favorited +
    /// message); A un-favorites + pushes (the file vanishes from git); B pulls
    /// again and the un-favorite propagates (favorited clears, shared messages
    /// drop). Exercises the whole 3b-2/3 loop end to end.
    #[test]
    fn session_snapshots_roundtrip_and_unfavorite_propagates() {
        use crate::model::{SessionMessage, SessionMessageRole, SessionSystemData};

        fn dev_cfg(url: &str, dev: &str) -> ConfigData {
            let mut cfg = synced_cfg(url, "tok");
            cfg.device_id = dev.to_string();
            cfg
        }
        fn sys(id: &str, agent_type: &str) -> SessionSystemData {
            SessionSystemData {
                id: id.into(),
                source: "claude_code".into(),
                project_dir: "/p".into(),
                title_orig: format!("Title {id}"),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-02T00:00:00.000Z".into(),
                agent_type: agent_type.into(),
                // A subagent row carries its parent link — the test asserts the
                // link rides the snapshot meta line like agent_type does.
                parent_session_id: if agent_type.is_empty() {
                    String::new()
                } else {
                    "main-1".to_string()
                },
            }
        }
        fn msg(uuid: &str, sid: &str) -> SessionMessage {
            SessionMessage {
                uuid: uuid.into(),
                session_id: sid.into(),
                role: SessionMessageRole::User,
                ts: "2026-08-01T10:00:00.000Z".into(),
                model: None,
                name: None,
                content: format!("body {uuid}"),
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let dev_a = "aabbccddeeff";
        let dev_b = "bbccddee0011";

        // Device A: collect a favorited session + message, then push its snapshot.
        let paths_a = crate::config::Paths::resolve(&tmp.path().join("a"));
        let cfg_a = dev_cfg(&url, dev_a);
        let _repo_a = open_or_clone(&url, &paths_a.repo, "").unwrap();
        let store_a = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        crate::collect::ingest::ingest_sessions(
            &store_a,
            dev_a,
            &[sys("sx", "Explore")],
            &[msg("u1", "sx")],
        )
        .unwrap();
        store_a.set_session_favorited(dev_a, "sx", true).unwrap();
        assert!(
            push_usage(&store_a, &paths_a, &cfg_a).unwrap(),
            "A pushed the snapshot"
        );
        assert!(
            paths_a.session_snapshot_path(dev_a, "sx").exists(),
            "snapshot written"
        );

        // Device B: pull → imports A's session (meta + favorited + message). B's
        // own snapshot dir stays empty (B favorited nothing, so it pushes nothing).
        let paths_b = crate::config::Paths::resolve(&tmp.path().join("b"));
        let cfg_b = dev_cfg(&url, dev_b);
        let store_b = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        pull_and_import(&store_b, &paths_b, &cfg_b).unwrap();
        let b_sx = store_b
            .query_sessions(None)
            .unwrap()
            .into_iter()
            .find(|r| r.id == "sx")
            .expect("B sees A's session");
        assert_eq!(b_sx.device_id, dev_a);
        assert!(b_sx.favorited, "favorited rode the snapshot meta line");
        assert_eq!(b_sx.title, "Title sx");
        assert_eq!(
            b_sx.agent_type, "Explore",
            "agent_type rode the snapshot meta line (pull must not zero it)"
        );
        assert_eq!(
            b_sx.parent_session_id, "main-1",
            "the subagent parent link rode the snapshot meta line too"
        );
        assert_eq!(
            store_b.query_session_messages(dev_a, "sx").unwrap().len(),
            1,
            "message imported"
        );

        // A un-favorites + pushes → the snapshot file vanishes from git.
        store_a.set_session_favorited(dev_a, "sx", false).unwrap();
        assert!(push_usage(&store_a, &paths_a, &cfg_a).unwrap());
        assert!(
            !paths_a.session_snapshot_path(dev_a, "sx").exists(),
            "A removed the snapshot on un-favorite"
        );

        // B pulls again → un-favorite propagates: favorited clears, shared
        // messages drop (the cross-device un-favorite path).
        pull_and_import(&store_b, &paths_b, &cfg_b).unwrap();
        let b_sx2 = store_b
            .query_sessions(None)
            .unwrap()
            .into_iter()
            .find(|r| r.id == "sx")
            .expect("meta row kept");
        assert!(!b_sx2.favorited, "un-favorite propagated to B");
        assert!(
            store_b
                .query_session_messages(dev_a, "sx")
                .unwrap()
                .is_empty(),
            "shared messages dropped on un-favorite"
        );
    }

    /// Fast (no-git) check of the pull un-favorite composition: a peer's
    /// favorited sessions whose snapshot file is absent this pull are exactly
    /// what `presence_mismatches` flags, and `bulk_unfavorite_sessions` clears
    /// those (favorited flag + shared messages) while leaving the still-filed
    /// ones alone. Mirrors `domains::sessions_import`'s per-peer loop without a
    /// real git round-trip.
    #[test]
    fn pull_unfavorite_matches_presence_mismatches_without_git() {
        use crate::model::{SessionMessage, SessionMessageRole, SessionSystemData};

        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        let peer = "peerdevice01";

        fn sys(id: &str) -> SessionSystemData {
            SessionSystemData {
                id: id.into(),
                source: "claude_code".into(),
                project_dir: "/p".into(),
                title_orig: id.into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-02T00:00:00.000Z".into(),
                agent_type: String::new(),
                parent_session_id: String::new(),
            }
        }
        fn msg(uuid: &str, sid: &str) -> SessionMessage {
            SessionMessage {
                uuid: uuid.into(),
                session_id: sid.into(),
                role: SessionMessageRole::User,
                ts: "2026-08-01T10:00:00.000Z".into(),
                model: None,
                name: None,
                content: format!("body {uuid}"),
            }
        }

        // Three favorited sessions; each carries one shared message.
        for sid in ["s1", "s2", "s3"] {
            store.upsert_session(peer, &sys(sid)).unwrap();
            store.set_session_favorited(peer, sid, true).unwrap();
            store
                .ingest_session_messages_marking_dirty(
                    peer,
                    std::slice::from_ref(&msg(&format!("u-{sid}"), sid)),
                )
                .unwrap();
        }

        // Only s1 and s2 still have a snapshot file this pull → s3 was un-favorited.
        let still_present: std::collections::BTreeSet<String> =
            ["s1".to_string(), "s2".to_string()].into_iter().collect();
        let peer_favorited: std::collections::BTreeSet<String> = store
            .favorited_session_ids(peer)
            .unwrap()
            .into_iter()
            .collect();
        let to_unfavorite =
            presence_mismatches(&still_present, &peer_favorited).favorites_without_files;
        assert_eq!(to_unfavorite, vec!["s3".to_string()]);

        store
            .bulk_unfavorite_sessions(peer, &to_unfavorite)
            .unwrap();
        assert_eq!(
            store.favorited_session_ids(peer).unwrap(),
            vec!["s1".to_string(), "s2".to_string()],
            "s3 un-favorited; s1/s2 kept"
        );
        assert!(
            store.query_session_messages(peer, "s3").unwrap().is_empty(),
            "s3 shared messages dropped"
        );
        assert_eq!(
            store.query_session_messages(peer, "s1").unwrap().len(),
            1,
            "untouched session keeps its message"
        );
    }

    #[test]
    fn sync_roundtrips_usage_across_devices() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();

        // Device A: write an artifact, then pull (no-op) + commit+push.
        let paths_a = crate::config::Paths::resolve(&tmp.path().join("a"));
        let cfg_a = synced_cfg(&url, "tok");
        let _repo_a = open_or_clone(&url, &paths_a.repo, "").unwrap();
        let book = crate::pricing::seed_book();
        let rec = crate::collect::ingest::recordify(&raw_usage("round-1"), "aabbccddeeff", &book);
        crate::collect::artifact::append_jsonl(&paths_a, "aabbccddeeff", &[rec]).unwrap();
        let store_a = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        let imported_a = pull_and_import(&store_a, &paths_a, &cfg_a).unwrap();
        let pushed_a = commit_and_push(&paths_a, &cfg_a, "cc-one: usage sync").unwrap();
        assert!(pushed_a, "A had a local change to push");
        assert_eq!(imported_a, 1, "A imports its own artifact into its store");

        // Device B: pull A's artifact into B's fresh store.
        let paths_b = crate::config::Paths::resolve(&tmp.path().join("b"));
        let cfg_b = synced_cfg(&url, "tok");
        let store_b = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        let imported_b = pull_and_import(&store_b, &paths_b, &cfg_b).unwrap();
        let pushed_b = commit_and_push(&paths_b, &cfg_b, "cc-one: usage sync").unwrap();
        assert_eq!(imported_b, 1, "B imported A's record");
        assert!(!pushed_b, "B has no local change beyond what it pulled");
        let stats = store_b
            .query_stats(&crate::model::UsageFilter::default())
            .unwrap();
        assert_eq!(stats.request_count, 1);
    }

    /// A device's drag-reordered group order rides the groups.json artifact:
    /// device A creates three groups, reorders them, and device B sees the new
    /// order after a pull — the core promise of the synced track.
    #[test]
    fn synced_groups_reorder_propagates_across_devices() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let dev_a = "aabbccddeeff";
        let dev_b = "bbccddee0011";

        // Device A: bind, create three groups, then reorder them.
        let paths_a = crate::config::Paths::resolve(&tmp.path().join("a"));
        let mut cfg_a = synced_cfg(&url, "tok");
        cfg_a.device_id = dev_a.to_string();
        let _repo_a = open_or_clone(&url, &paths_a.repo, "").unwrap();
        let g1 = crate::sessions::create_synced_group_owned(&paths_a, &cfg_a, "One").unwrap();
        let g2 = crate::sessions::create_synced_group_owned(&paths_a, &cfg_a, "Two").unwrap();
        let g3 = crate::sessions::create_synced_group_owned(&paths_a, &cfg_a, "Three").unwrap();
        // Order: g3, g1, g2.
        crate::sessions::reorder_synced_groups_owned(
            &paths_a,
            &cfg_a,
            &[g3.id.clone(), g1.id.clone(), g2.id.clone()],
        )
        .unwrap();

        // Device B: pull → the reordered groups.json lands in the worktree.
        let paths_b = crate::config::Paths::resolve(&tmp.path().join("b"));
        let mut cfg_b = synced_cfg(&url, "tok");
        cfg_b.device_id = dev_b.to_string();
        let store_b = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        pull_and_import(&store_b, &paths_b, &cfg_b).unwrap();

        let ids: Vec<String> = crate::sessions::read_all_synced_groups(&paths_b)
            .into_iter()
            .map(|g| g.id)
            .collect();
        assert_eq!(ids, [g3.id, g1.id, g2.id], "B sees A's drag order");
    }

    /// Provider sync carries STRUCTURE across devices and never a key: A
    /// creates a provider with a key and pushes; B pulls and sees the
    /// structure (name / endpoint) with an empty key, fills its own key and
    /// pushes; A pulls and keeps ITS key (an import never overwrites a local
    /// key); A renames the provider and pushes; B pulls and gets the new name
    /// with B's key still filled. Neither device's providers.json ever
    /// contains a key value or key name.
    #[test]
    fn provider_structure_syncs_across_devices_but_keys_stay_local() {
        use crate::model::{App, Provider, ProviderCategory};

        fn dev_cfg(url: &str, dev: &str) -> ConfigData {
            let mut cfg = synced_cfg(url, "tok");
            cfg.device_id = dev.to_string();
            cfg
        }
        fn provider(token: &str, endpoint: &str) -> Provider {
            Provider {
                id: "abcdef01".into(),
                name: "Kimi".into(),
                website_url: "https://platform.kimi.com".into(),
                category: ProviderCategory::Custom,
                app: App::Claude,
                icon: String::new(),
                icon_color: String::new(),
                sort_index: 0,
                notes: String::new(),
                settings_config: format!(
                    r#"{{"env":{{"ANTHROPIC_BASE_URL":"{endpoint}","ANTHROPIC_AUTH_TOKEN":"{token}"}}}}"#
                ),
                meta: r#"{}"#.into(),
                updated_at: String::new(),
            }
        }
        fn env_of(p: &Provider) -> serde_json::Value {
            let v: serde_json::Value = serde_json::from_str(&p.settings_config).unwrap();
            v["env"].clone()
        }

        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let dev_a = "aabbccddeeff";
        let dev_b = "bbccddee0011";

        // Device A: create a provider with a key, then push.
        let paths_a = crate::config::Paths::resolve(&tmp.path().join("a"));
        let cfg_a = dev_cfg(&url, dev_a);
        let _repo_a = open_or_clone(&url, &paths_a.repo, "").unwrap();
        let store_a = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        store_a
            .save_provider(provider("sk-a-secret", "https://api.kimi.com"))
            .unwrap();
        assert!(push_usage(&store_a, &paths_a, &cfg_a).unwrap());
        let a_file = std::fs::read_to_string(paths_a.providers_json_path(dev_a)).unwrap();
        assert!(a_file.contains("Kimi"));
        assert!(!a_file.contains("sk-a-secret"), "key never enters the file");

        // Device B: pull → the provider arrives with structure but NO key.
        let paths_b = crate::config::Paths::resolve(&tmp.path().join("b"));
        let cfg_b = dev_cfg(&url, dev_b);
        let store_b = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        pull_and_import(&store_b, &paths_b, &cfg_b).unwrap();
        let b_p = store_b
            .get_provider(App::Claude, "abcdef01")
            .unwrap()
            .expect("B sees A's provider");
        assert_eq!(b_p.name, "Kimi");
        let env_b = env_of(&b_p);
        assert_eq!(env_b["ANTHROPIC_BASE_URL"], "https://api.kimi.com");
        assert!(
            env_b.get("ANTHROPIC_AUTH_TOKEN").is_none(),
            "key left empty on import"
        );

        // B fills its own key (structure unchanged) and pushes.
        let mut b_edit = b_p;
        b_edit.settings_config = r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com","ANTHROPIC_AUTH_TOKEN":"sk-b-secret"}}"#.into();
        store_b.save_provider(b_edit).unwrap();
        assert!(push_usage(&store_b, &paths_b, &cfg_b).unwrap());
        let b_file = std::fs::read_to_string(paths_b.providers_json_path(dev_b)).unwrap();
        assert!(!b_file.contains("sk-b-secret"));

        // A pulls again: A's own key survived (never overwritten by B's
        // keyless copy — B's copy is not even newer, same author timestamp).
        pull_and_import(&store_a, &paths_a, &cfg_a).unwrap();
        let a_p = store_a
            .get_provider(App::Claude, "abcdef01")
            .unwrap()
            .expect("A still has its provider");
        assert!(
            a_p.settings_config.contains("sk-a-secret"),
            "A's key kept after pulling B's keyless copy"
        );

        // A edits the structure (name); B pulls and gets the new name WITH
        // its own key still filled.
        let mut a_edit = a_p;
        a_edit.name = "Kimi Pro".into();
        store_a.save_provider(a_edit).unwrap();
        assert!(push_usage(&store_a, &paths_a, &cfg_a).unwrap());
        pull_and_import(&store_b, &paths_b, &cfg_b).unwrap();
        let b_p2 = store_b
            .get_provider(App::Claude, "abcdef01")
            .unwrap()
            .expect("B still has its provider");
        assert_eq!(b_p2.name, "Kimi Pro", "A's structural edit reached B");
        let env_b2 = env_of(&b_p2);
        assert_eq!(env_b2["ANTHROPIC_BASE_URL"], "https://api.kimi.com");
        assert_eq!(
            env_b2["ANTHROPIC_AUTH_TOKEN"], "sk-b-secret",
            "B's key survived A's edit"
        );
    }

    /// The dirty-flag pairing invariant, flow level: a PUSH FAILURE leaves the
    /// store untouched — both flag domains stay dirty (days AND sessions), so
    /// the retry recomputes everything. The old code only ever failed one side
    /// or the other at the CLEAR step; this pins the "push never landed ⇒ both
    /// stay dirty" half of "either both cleared or both dirty".
    #[test]
    fn push_usage_failure_keeps_both_flags_dirty() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let dev = "aaaaaaaaaaaa";
        let paths = crate::config::Paths::resolve(&tmp.path().join("dev"));
        let cfg = ConfigData {
            repo_url: Some(url.clone()),
            github_token: Some("tok".into()),
            device_id: dev.into(),
            ..Default::default()
        };
        let _repo = open_or_clone(&url, &paths.repo, "").unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();

        // Day dirty (one usage row) + session dirty (favorited + one message).
        let book = crate::pricing::seed_book();
        let rec = crate::collect::ingest::recordify(&raw_usage("a-1"), dev, &book);
        store
            .ingest_marking_dirty(std::slice::from_ref(&rec))
            .unwrap();
        let sys = crate::model::SessionSystemData {
            id: "sx".into(),
            source: "claude_code".into(),
            project_dir: "/p".into(),
            title_orig: "Title".into(),
            started_at: "2026-08-01T00:00:00.000Z".into(),
            last_active_at: "2026-08-02T00:00:00.000Z".into(),
            agent_type: String::new(),
            parent_session_id: String::new(),
        };
        let msg = crate::model::SessionMessage {
            uuid: "m1".into(),
            session_id: "sx".into(),
            role: crate::model::SessionMessageRole::User,
            ts: "2026-08-01T10:00:00.000Z".into(),
            model: None,
            name: None,
            content: "hello".into(),
        };
        crate::collect::ingest::ingest_sessions(&store, dev, &[sys], &[msg]).unwrap();
        store.set_session_favorited(dev, "sx", true).unwrap();

        // The remote dies between clone and push ⇒ commit lands locally, the
        // push fails, and push_usage must return the error with BOTH flag
        // domains untouched for the next retry.
        std::fs::remove_dir_all(&remote).unwrap();
        assert!(matches!(
            push_usage(&store, &paths, &cfg).unwrap_err(),
            AppError::Sync(_)
        ));
        assert_eq!(
            store.dirty_days().unwrap(),
            vec!["2026-07-13".to_string()],
            "failed push leaves the dirty day dirty"
        );
        assert_eq!(
            store.dirty_sessions().unwrap(),
            vec!["sx".to_string()],
            "failed push leaves the dirty session dirty"
        );
    }

    /// The split-clear crash window self-heals: if the process died between
    /// the commit and the clear, a flag can be stale while its materialized
    /// snapshot is ALREADY on the remote (nothing to push). The clear must run
    /// even on a no-op push — gating it on `pushed` would strand the flag
    /// forever. Construction: push once (snapshot committed), then re-mark the
    /// session dirty WITHOUT changing its content (the production
    /// `set_session_favorited` re-mark) — the materialization is byte-identical,
    /// commit_and_push no-ops, and the flag must still clear.
    #[test]
    fn push_usage_clears_stale_flags_on_noop_push_after_split_clear() {
        use crate::model::{SessionMessage, SessionMessageRole, SessionSystemData};

        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        let dev = "aaaaaaaaaaaa";
        let paths = crate::config::Paths::resolve(&tmp.path().join("dev"));
        let cfg = ConfigData {
            repo_url: Some(url.clone()),
            github_token: Some("tok".into()),
            device_id: dev.into(),
            ..Default::default()
        };
        let _repo = open_or_clone(&url, &paths.repo, "").unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();

        let sys = SessionSystemData {
            id: "sx".into(),
            source: "claude_code".into(),
            project_dir: "/p".into(),
            title_orig: "Title".into(),
            started_at: "2026-08-01T00:00:00.000Z".into(),
            last_active_at: "2026-08-02T00:00:00.000Z".into(),
            agent_type: String::new(),
            parent_session_id: String::new(),
        };
        let msg = SessionMessage {
            uuid: "m1".into(),
            session_id: "sx".into(),
            role: SessionMessageRole::User,
            ts: "2026-08-01T10:00:00.000Z".into(),
            model: None,
            name: None,
            content: "hello".into(),
        };
        crate::collect::ingest::ingest_sessions(&store, dev, &[sys], &[msg]).unwrap();
        store.set_session_favorited(dev, "sx", true).unwrap();
        assert!(
            push_usage(&store, &paths, &cfg).unwrap(),
            "first push ships"
        );
        assert!(
            store.dirty_sessions().unwrap().is_empty(),
            "successful push clears the session"
        );

        // Simulate the split-clear residue: the session is re-marked dirty
        // while its snapshot (already committed + pushed) is byte-identical to
        // what the store would recompute.
        store.set_session_favorited(dev, "sx", true).unwrap();
        assert_eq!(store.dirty_sessions().unwrap(), vec!["sx".to_string()]);
        assert!(!has_changes(&open_or_clone(&url, &paths.repo, "").unwrap()).unwrap());

        // No-op push (nothing to ship) must STILL clear the stale flag — the
        // unconditional clear heals the crash window instead of stranding it.
        let pushed = push_usage(&store, &paths, &cfg).unwrap();
        assert!(!pushed, "nothing new to push");
        assert!(
            store.dirty_sessions().unwrap().is_empty(),
            "stale flag cleared on a no-op push"
        );
    }
}
