//! Low-level libgit2 primitives for repo sync: credential callback, repo
//! open/clone/init, pull (fast-forward) + `rebase_and_push` (diverge self-heal),
//! commit, push, and worktree status queries. Pure git — this layer knows
//! nothing about the Local Store, dirty flags, or snapshot_policy. The
//! high-level sync flow that composes these primitives with the store lives in
//! `super::flow`.

use std::collections::HashSet;
use std::path::Path;

use git2::build::{CheckoutBuilder, RepoBuilder};
use git2::{
    AnnotatedCommit, Cred, FetchOptions, Index, ObjectType, Oid, ProxyOptions, PushOptions,
    RemoteCallbacks, Repository, ResetType, Signature, Status,
};

use crate::config::ConfigData;
use crate::error::{AppError, AppResult};

// ---------------------------------------------------------------------------
// Credential callback (in-process PAT)
// ---------------------------------------------------------------------------

/// Build a GitHub PAT credential. GitHub accepts the fine-grained PAT as the
/// password under any username; we use the conventional `x-access-token` when
/// libgit2 does not hand us one from the URL.
fn pat_credential(username_from_url: Option<&str>, token: &str) -> Result<Cred, git2::Error> {
    let user = username_from_url.unwrap_or("x-access-token");
    Cred::userpass_plaintext(user, token)
}

/// Remote callbacks that inject the PAT, with a one-shot guard so a rejected
/// token does not loop forever (libgit2 may re-invoke the callback on auth
/// failure). git2 0.19's `RemoteCallbacks` holds a `'static` callback, so the
/// token is cloned into the closure (cheap; sync is low-frequency).
// The borrowed `&str` is unrelated to the returned `RemoteCallbacks` (its
// callback is 'static), so rustc's mismatched_lifetime_syntaxes misfires here.
#[allow(mismatched_lifetime_syntaxes)]
pub(super) fn build_callbacks(token: &str) -> RemoteCallbacks {
    let token = token.to_string();
    let mut attempts = 0u32;
    let mut cb = RemoteCallbacks::new();
    cb.credentials(move |_url, username_from_url, _allowed| {
        if attempts > 0 {
            return Err(git2::Error::from_str(
                "git credentials rejected: PAT invalid or expired",
            ));
        }
        attempts += 1;
        pat_credential(username_from_url, &token)
    });
    cb
}

/// Declare libgit2 transport options (`FetchOptions` or `PushOptions`, named
/// `$opt`) wired with the PAT callback AND the system proxy discovered at this
/// instant. A macro, not a function: libgit2's `ProxyOptions` borrows the
/// proxy URL by reference, so the URL must outlive the options — expanding
/// inline keeps the borrowed URL and the options in the caller's scope, where
/// the subsequent `fetch` / `clone` / `push` consumes them before either can
/// drop.
macro_rules! options_with_proxy {
    ($opt:ident, $type:ty, $token:expr) => {
        let mut $opt = <$type>::new();
        $opt.remote_callbacks(build_callbacks($token));
        let __proxy_url = crate::proxy::discover_system_proxy();
        if let Some(ref __pu) = __proxy_url {
            let mut __p = ProxyOptions::new();
            __p.url(__pu);
            $opt.proxy_options(__p);
        }
    };
}

// ---------------------------------------------------------------------------
// clone / open
// ---------------------------------------------------------------------------

/// Default branch we bootstrap (empty remote / Standalone→Synced switch).
/// libgit2's `init` defaults to `master`; we pin `main` to match the GitHub
/// default. A non-empty remote is always followed verbatim (`pick_origin_branch`).
const DEFAULT_BRANCH: &str = "main";

/// Open the local repo at `local`, or clone it from `repo_url` on first use.
/// Idempotent: once `.git` exists, reopens instead of re-cloning.
pub fn open_or_clone(repo_url: &str, local: &Path, token: &str) -> AppResult<Repository> {
    open_or_clone_impl(repo_url, local, token)
}

fn open_or_clone_impl(repo_url: &str, local: &Path, token: &str) -> AppResult<Repository> {
    if local.join(".git").exists() {
        return Ok(Repository::open(local)?);
    }
    // Standalone collects write JSONL artifacts into `local/data/`.
    // When the user later switches to Synced, `local` is non-empty but has no
    // `.git`, and libgit2's `clone` (which demands an empty target) fails with
    // "exists and is not an empty directory". Detect that and bootstrap the repo
    // in place instead — preserving the locally-collected artifacts.
    let dir_has_entries = local
        .read_dir()
        .map(|mut it| it.next().is_some())
        .unwrap_or(false);
    if dir_has_entries {
        return init_with_remote(repo_url, local, token);
    }
    options_with_proxy!(fo, FetchOptions, token);
    let mut builder = RepoBuilder::new();
    builder.fetch_options(fo);
    let repo = builder.clone(repo_url, local)?;
    // Force LF so JSONL artifacts round-trip byte-identically across Windows /
    // POSIX (deterministic interop). libgit2's platform-default text
    // conversion would otherwise flip \n ↔ \r\n and corrupt line-oriented JSONL.
    repo.config()?.set_str("core.autocrlf", "false")?;
    // The initial checkout ran under libgit2's platform-default autocrlf; under
    // the new LF policy the worktree can look "modified" vs the index until we
    // re-materialize it (force is safe — a fresh clone has no local changes).
    let mut co = CheckoutBuilder::new();
    co.force();
    repo.checkout_head(Some(&mut co))?;
    Ok(repo)
}

/// Drop the local `.git` so a fresh re-bind starts clean (used by
/// `clear_sync_repo`). `data/` and `config/` are preserved — Standalone keeps
/// writing artifacts to `data/`, and they carry no per-repo identity. Only
/// `.git` pins the worktree to the old remote + branch; the DB is the source of
/// truth for usage rows, so this loses git history, never data. Best-effort: a
/// removal failure is logged, not fatal (the unbind's primary effect — clearing
/// the config — already succeeded).
pub fn reset_local_git(repo: &Path) {
    let dot_git = repo.join(".git");
    if dot_git.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dot_git) {
            eprintln!("[cc-one] reset_local_git: failed to remove .git: {e}");
        }
    }
}

/// Bootstrap a sync repo inside an already-populated `local` — the Standalone →
/// Synced switch, or the unbind→re-bind case. `clone` refuses a non-empty target,
/// so init in place, fetch the remote, and force-checkout the remote tip. Force
/// is safe even though it may overwrite this device's own `data/<deviceId>/`
/// files: collect writes the store, not the Artifact, so unpushed rows live in
/// SQLite (flagged in `dirty_days`) and the next push recomputes this device's
/// files from the store. No snapshot/restore is needed — the store is the
/// source of truth.
fn init_with_remote(repo_url: &str, local: &Path, token: &str) -> AppResult<Repository> {
    let repo = Repository::init(local)?;
    repo.config()?.set_str("core.autocrlf", "false")?;
    {
        let mut remote = repo.remote("origin", repo_url)?;
        options_with_proxy!(fo, FetchOptions, token);
        remote.fetch(
            &["+refs/heads/*:refs/remotes/origin/*"],
            Some(&mut fo),
            None,
        )?;
    }
    // Point HEAD at the remote's default branch and force-checkout its tree. If
    // the remote is unborn (empty repo) there is nothing to check out — pin HEAD
    // at our `main` (unborn) so the first commit+push creates `main`, not
    // libgit2's hardcoded `master`. Force: the worktree may already hold files
    // the remote also carries (the unbind→re-bind case — `.git` was dropped but
    // `data/` remains, so those files are now untracked and a SAFE checkout
    // rejects them as conflicts). Overwriting this device's own (possibly
    // staler) files is fine — see the doc comment above: push recomputes them.
    if let Some((branch, tip)) = pick_origin_branch(&repo)? {
        let commit = repo.find_commit(tip)?;
        repo.branch(&branch, &commit, true)?;
        repo.set_head(&format!("refs/heads/{branch}"))?;
        let mut co = CheckoutBuilder::new();
        co.force();
        repo.checkout_head(Some(&mut co))?;
    } else {
        // Empty remote: libgit2's init default (`master`) would otherwise win.
        // Pin to our `main` as an unborn HEAD; the first commit lands on it.
        repo.set_head(&format!("refs/heads/{DEFAULT_BRANCH}"))?;
    }
    Ok(repo)
}

/// Resolve the remote's default branch + tip. `clone` records `origin/HEAD`, but
/// an in-place init+fetch does not, so prefer `main`, then `master`, then any
/// remote branch. `None` when the remote carries no branches yet (unborn).
fn pick_origin_branch(repo: &Repository) -> AppResult<Option<(String, Oid)>> {
    for name in ["main", "master"] {
        if let Ok(oid) = repo.refname_to_id(&format!("refs/remotes/origin/{name}")) {
            return Ok(Some((name.to_string(), oid)));
        }
    }
    for item in repo.branches(Some(git2::BranchType::Remote))? {
        let (branch, _) = item?;
        let raw = branch.name_bytes()?;
        let s = String::from_utf8_lossy(raw);
        if let Some(rest) = s.strip_prefix("origin/") {
            if let Ok(oid) = repo.refname_to_id(&format!("refs/remotes/origin/{rest}")) {
                return Ok(Some((rest.to_string(), oid)));
            }
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// pull (fetch + fast-forward) + rebase_and_push (diverge self-heal)
// ---------------------------------------------------------------------------

/// Outcome of [`pull`]. `pull` is fetch + fast-forward only — it never rebases
/// or pushes. When the local and remote histories have diverged (a lost push
/// race — another device pushed between our last pull and push) it does NOT
/// mutate, returning [`PullOutcome::Diverged`] with the upstream tip so the
/// caller can resolve it explicitly with [`rebase_and_push`]. This keeps `pull`
/// honest about its name: no hidden rebase, no hidden push.
pub enum PullOutcome<'a> {
    /// Local is already at the remote tip — or there is no branch/upstream to
    /// advance (unborn HEAD, first push pending). No mutation.
    UpToDate,
    /// Local branch was fast-forwarded to the remote tip and the worktree
    /// synced to it.
    FastForwarded,
    /// Histories diverged. `pull` did nothing; the caller decides whether to
    /// rebase + push via [`rebase_and_push`].
    Diverged(AnnotatedCommit<'a>),
}

/// Fetch `origin` and advance the current branch to its tip when possible.
/// Fast-forwards when it can; returns [`PullOutcome::Diverged`] WITHOUT
/// mutating when the local branch has commits the remote doesn't (a lost push
/// race). The caller — typically `super::flow::pull_and_import` — then resolves
/// the diverge explicitly with [`rebase_and_push`]. Device isolation
/// (`data/<deviceId>/`) means a local-only commit only touches files the remote
/// didn't, so that rebase applies without conflict. Usage artifacts are the
/// only thing in the repo and they are per-device isolated (`data/<deviceId>/`),
/// so no shared file two devices could diverge on.
pub fn pull<'a>(repo: &'a Repository, token: &str) -> AppResult<PullOutcome<'a>> {
    // Unborn HEAD (fresh init, first commit still pending): no local branch to
    // fast-forward, so there is nothing to pull — the first commit+push creates
    // the branch. Covers the Standalone→Synced switch against an empty remote,
    // where `head()` would otherwise error on the missing HEAD ref.
    let mut head = match repo.head() {
        Ok(h) => h,
        Err(ref e) if e.code() == git2::ErrorCode::UnbornBranch => {
            return Ok(PullOutcome::UpToDate)
        }
        Err(e) => return Err(e.into()),
    };
    options_with_proxy!(fo, FetchOptions, token);
    repo.find_remote("origin")?.fetch(
        &["+refs/heads/*:refs/remotes/origin/*"],
        Some(&mut fo),
        None,
    )?;
    let branch = head
        .shorthand()
        .ok_or_else(|| AppError::Sync("HEAD is detached; cannot pull".into()))?;
    let upstream_ref = format!("refs/remotes/origin/{branch}");
    // Remote may not yet have this branch (first push pending) — nothing to pull.
    let upstream_oid = match repo.refname_to_id(&upstream_ref) {
        Ok(oid) => oid,
        Err(_) => return Ok(PullOutcome::UpToDate),
    };

    let upstream = repo.find_annotated_commit(upstream_oid)?;
    let (analysis, _pref) = repo.merge_analysis(&[&upstream])?;
    if analysis.is_up_to_date() {
        return Ok(PullOutcome::UpToDate);
    }
    if !analysis.is_fast_forward() {
        // Diverged: surface the upstream tip only. `pull` declines to rebase/push
        // — the caller resolves it via `rebase_and_push`.
        return Ok(PullOutcome::Diverged(upstream));
    }
    // Fast-forward: move the branch ref to the remote tip, then sync the tree.
    head.set_target(upstream_oid, "pull: fast-forward")?;
    let mut co = CheckoutBuilder::new();
    co.force();
    repo.checkout_head(Some(&mut co))?;
    Ok(PullOutcome::FastForwarded)
}

/// Rebase this branch's local-only commits onto `upstream` and push. The
/// explicit diverge step [`pull`] declines to do — `super::flow::pull_and_import`
/// invokes it when [`pull`] returns [`PullOutcome::Diverged`].
///
/// `git rebase` needs a clean worktree, so any in-worktree change here is
/// hard-reset away before rebasing. That is safe: usage rows live in the store
/// (not the worktree Artifacts), and the next push recomputes this device's
/// files from it — the worktree is always regenerable. Device isolation
/// (`data/<deviceId>/`) guarantees the rebase applies without conflict. A
/// commit whose diff is already on the upstream tip (e.g. a device-cleanup a
/// peer pushed first) is dropped as already-applied; any other failure is a
/// real conflict, so we abort and surface it instead of silently merging, and
/// the caller reports it as a plain failed sync.
///
/// `author_name` / `author_email` are this device's commit identity, reused as
/// the rebaser's signature (authors of replayed commits are preserved). Synced
/// callers pass `cfg.display_name` / `author_email(cfg)`.
pub(crate) fn rebase_and_push(
    repo: &Repository,
    upstream: &AnnotatedCommit,
    token: &str,
    author_name: &str,
    author_email: &str,
) -> AppResult<()> {
    if has_changes(repo)? {
        let head_oid = repo
            .head()?
            .target()
            .ok_or_else(|| AppError::Sync("HEAD has no target; cannot rebase".into()))?;
        let head_obj = repo.find_object(head_oid, None)?;
        repo.reset(&head_obj, ResetType::Hard, None)?;
    }
    let committer = Signature::now(author_name, author_email)?;
    let mut rebase = repo.rebase(None, Some(upstream), Some(upstream), None)?;
    while let Some(op) = rebase.next() {
        op?;
        match rebase.commit(None, &committer, None) {
            Ok(_) => {}
            // This commit's diff is already on the upstream tip — e.g. a
            // device-cleanup a peer pushed first — so libgit2 reports it as
            // already-applied. Drop it and keep rebasing; the device-isolation
            // layout means the surviving commits apply cleanly, so any other
            // error here is a real conflict we refuse to auto-merge.
            Err(ref e) if e.code() == git2::ErrorCode::Applied => continue,
            Err(e) => {
                let _ = rebase.abort();
                return Err(AppError::Sync(format!(
                    "rebase onto remote tip would conflict; refusing to auto-merge: {e}"
                )));
            }
        }
    }
    rebase.finish(Some(&committer))?;
    push(repo, token)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// commit
// ---------------------------------------------------------------------------

/// Stage every worktree change (add / modify / delete) and commit it. Supports
/// an unborn HEAD (first commit). Usage artifacts are keyed by `<deviceId>/<day>`
/// so files are only added or appended in place — never renamed — hence no
/// rename handling.
pub fn commit_all(
    repo: &Repository,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> AppResult<git2::Oid> {
    let mut index = repo.index()?;
    stage_all(repo, &mut index)?;
    index.write()?;
    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;
    let sig = Signature::now(author_name, author_email)?;
    let oid = match repo.head() {
        Ok(head) => {
            let parent = head.peel_to_commit()?;
            repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])?
        }
        Err(_) => repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[])?, // unborn HEAD
    };
    Ok(oid)
}

/// `git add -A` over the worktree: stage new + modified files, drop deleted ones.
fn stage_all(repo: &Repository, index: &mut Index) -> AppResult<()> {
    let statuses = repo.statuses(None)?;
    for entry in statuses.iter() {
        let Some(p) = entry.path() else { continue };
        let s = entry.status();
        if s.contains(Status::WT_NEW) || s.contains(Status::WT_MODIFIED) {
            index.add_path(Path::new(p))?;
        } else if s.contains(Status::WT_DELETED) {
            index.remove_path(Path::new(p))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// push
// ---------------------------------------------------------------------------

/// Push the current branch to `origin` (creating the remote branch on first push).
pub fn push(repo: &Repository, token: &str) -> AppResult<()> {
    let head = repo.head()?;
    let refname = head
        .name()
        .ok_or_else(|| AppError::Sync("HEAD has no symbolic name; cannot push".into()))?;
    let refspec = format!("{refname}:{refname}");
    options_with_proxy!(po, PushOptions, token);
    repo.find_remote("origin")?
        .push(&[&refspec], Some(&mut po))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Synced-mode guard
// ---------------------------------------------------------------------------

/// Return the configured repo URL + PAT, or an error in Standalone mode.
/// S2b command-layer callers that must be no-ops in Standalone check
/// `ConfigData::is_synced()` directly instead of erroring.
pub fn require_synced(cfg: &ConfigData) -> AppResult<(String, String)> {
    if !cfg.is_synced() {
        return Err(AppError::Sync(
            "not in Synced mode: no repo URL / PAT configured".into(),
        ));
    }
    // `is_synced` guarantees both are present and non-blank.
    let url = cfg.repo_url.as_deref().unwrap().trim().to_string();
    let token = cfg.github_token.as_deref().unwrap().trim().to_string();
    Ok((url, token))
}

/// Open or clone the configured sync repo into `local`. Synced-only.
#[cfg(test)]
pub fn ensure_repo(cfg: &ConfigData, local: &Path) -> AppResult<Repository> {
    let (url, token) = require_synced(cfg)?;
    open_or_clone(&url, local, &token)
}

// ---------------------------------------------------------------------------
// Worktree status queries
// ---------------------------------------------------------------------------

/// Deterministic commit identity for this device (device-scoped).
pub(crate) fn author_email(cfg: &ConfigData) -> String {
    format!("{}@devices.cc-one", cfg.device_id)
}

/// Whether the worktree has any change to commit.
pub(crate) fn has_changes(repo: &Repository) -> AppResult<bool> {
    Ok(!repo.statuses(None)?.is_empty())
}

/// Whether the local branch has commits the remote tip lacks — the state a
/// failed push leaves behind: the commit landed locally, the worktree is clean,
/// and `has_changes` alone would no-op the retry forever. An unborn HEAD or a
/// never-fetched remote ref is conservatively "not ahead" (there is nothing
/// pushable either way).
pub(super) fn is_ahead_of_origin(repo: &Repository) -> AppResult<bool> {
    let Ok(head) = repo.head() else {
        return Ok(false); // unborn HEAD: nothing to be ahead with
    };
    let local = head.peel_to_commit()?;
    let remote_ref = format!(
        "refs/remotes/origin/{}",
        head.shorthand().unwrap_or("master")
    );
    let remote = match repo.find_reference(&remote_ref) {
        Ok(r) => r.peel_to_commit()?,
        Err(_) => return Ok(false),
    };
    let (ahead, _behind) = repo.graph_ahead_behind(local.id(), remote.id())?;
    Ok(ahead > 0)
}

// ---------------------------------------------------------------------------
// Membership oracle: which devices git itself carries
// ---------------------------------------------------------------------------

/// Which device ids git itself still carries locally, read off the HEAD tree:
/// devices with a `config/devices_<id>.json` name artifact (plus the read-only
/// legacy `config/devices/<id>.json` layout the artifact reader also accepts)
/// or a `data/<id>/` subtree. This is the committed tree, NOT the worktree —
/// device membership ([`crate::devices::reconcile_devices`]) is decided here so
/// a failed force-checkout, an interrupted rebase, or an external branch switch
/// (worktree files transiently missing while HEAD still carries them) can never
/// read as a device leaving the repo. Local HEAD is the right truth in both
/// modes: Synced keeps it at the pulled remote tip, and Standalone-after-unbind
/// still has no repo (see `None` below).
///
/// `None` = no usable local git state: no `.git` (Standalone never opened a
/// repo), an unborn HEAD (bound to an empty remote, first commit pending), or
/// an unreadable tree. The caller must then degrade to the worktree
/// approximation instead of reading the failure as "nobody is present" — the
/// consumer (reconcile) forgets absent devices destructively, so an unreadable
/// truth falls back to the old lenient source, never to aggressive deletion.
pub fn head_tree_device_ids(repo_path: &Path) -> Option<HashSet<String>> {
    let repo = Repository::open(repo_path).ok()?;
    let tree = repo.head().ok()?.peel_to_tree().ok()?;
    let mut ids: HashSet<String> = HashSet::new();
    let mut insert_if_valid = |id: &str| {
        if crate::devices::is_valid_device_id(id) {
            ids.insert(id.to_string());
        }
    };
    for entry in tree.iter() {
        match entry.name() {
            // Name artifacts: flat `config/devices_<id>.json` blobs, plus the
            // legacy `config/devices/<id>.json` layout (the same two layouts
            // `crate::devices::read_all_device_artifacts` reads from the
            // worktree, so both membership sources accept the same shapes).
            Some("config") => {
                let Ok(config) = entry.to_object(&repo).and_then(|o| o.peel_to_tree()) else {
                    continue;
                };
                for e in config.iter() {
                    let Some(name) = e.name() else { continue };
                    if let Some(id) = name
                        .strip_prefix("devices_")
                        .and_then(|s| s.strip_suffix(".json"))
                    {
                        insert_if_valid(id);
                    } else if name == "devices" {
                        // Legacy layout subtree: <id>.json blobs.
                        let Ok(obj) = e.to_object(&repo) else {
                            continue;
                        };
                        let Ok(legacy) = obj.peel_to_tree() else {
                            continue;
                        };
                        for f in legacy.iter() {
                            if let Some(id) = f.name().and_then(|n| n.strip_suffix(".json")) {
                                insert_if_valid(id);
                            }
                        }
                    }
                }
            }
            // Per-device data subtrees: `data/<id>/`.
            Some("data") => {
                let Ok(data) = entry.to_object(&repo).and_then(|o| o.peel_to_tree()) else {
                    continue;
                };
                for e in data.iter() {
                    if e.kind() == Some(ObjectType::Tree) {
                        if let Some(id) = e.name() {
                            insert_if_valid(id);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(ids)
}

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

/// Seed a bare "remote" with one initial commit so it has a cloneable HEAD.
/// Module-level (not inside `mod tests`) and `pub(crate)` so the sibling test
/// modules (`super::tests` and `sync::remote_probe::tests`) can build a
/// `file://` remote without duplicating the fixture. Compiled only under
/// `cfg(test)`.
#[cfg(test)]
pub(crate) fn seed_remote(remote_path: &Path) {
    Repository::init_bare(remote_path).unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = Repository::init(work.path()).unwrap();
    repo.remote("origin", &remote_path.to_string_lossy())
        .unwrap();
    std::fs::write(work.path().join("README"), "cc-one sync seed\n").unwrap();
    commit_all(&repo, "seed", "cc one", "seed@devices.cc-one").unwrap();
    push(&repo, "").unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pat_credential_builds_userpass() {
        // pat_credential is a thin wrapper over Cred::userpass_plaintext; we
        // assert it succeeds (and forwards an explicit username). git2 0.19's
        // Cred::credtype returns a raw c_int that does not compare to the
        // CredentialType constants, so we don't assert the enum here.
        assert!(pat_credential(None, "ghp_token").is_ok());
        assert!(pat_credential(Some("octocat"), "ghp_token").is_ok());
    }

    /// Standalone → Synced switch: `local` already holds collected artifacts and no
    /// `.git`. `init_with_remote` must bootstrap the repo, pull the remote, and
    /// keep the local data intact.
    #[test]
    fn init_with_remote_preserves_local_data_and_pulls_remote() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();

        let local = tmp.path().join("device");
        let local_data = local.join("data").join("localdev");
        std::fs::create_dir_all(&local_data).unwrap();
        std::fs::write(
            local_data.join("usage-2026-07-22.jsonl"),
            "{\"uuid\":\"local-1\"}\n",
        )
        .unwrap();

        let repo = init_with_remote(&url, &local, "").unwrap();
        assert!(local.join(".git").exists());
        // Local artifact survives the SAFE checkout (untracked, not clobbered).
        assert!(local_data.join("usage-2026-07-22.jsonl").exists());
        // Remote content landed (seed_remote committed a README).
        assert!(local.join("README").exists());
        drop(repo);
    }

    #[test]
    fn init_with_remote_handles_unborn_remote() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        Repository::init_bare(&remote).unwrap(); // unborn — no commits
        let url = remote.to_string_lossy().to_string();

        let local = tmp.path().join("device");
        let local_data = local.join("data").join("localdev");
        std::fs::create_dir_all(&local_data).unwrap();
        std::fs::write(local_data.join("usage.jsonl"), "{}\n").unwrap();

        // No branches on the remote ⇒ no checkout, but local data survives + repo
        // is init'd (first commit+push will create the branch).
        let repo = init_with_remote(&url, &local, "").unwrap();
        assert!(local.join(".git").exists());
        assert!(local_data.join("usage.jsonl").exists());
        drop(repo);
    }

    /// Against an empty remote the bootstrapped repo has an unborn HEAD; `pull`
    /// must short-circuit instead of erroring on the missing HEAD ref.
    #[test]
    fn pull_is_noop_on_unborn_head() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        Repository::init_bare(&remote).unwrap();
        let url = remote.to_string_lossy().to_string();
        let local = tmp.path().join("dev");
        let repo = init_with_remote(&url, &local, "").unwrap();
        assert!(
            matches!(pull(&repo, "").unwrap(), PullOutcome::UpToDate),
            "unborn HEAD must short-circuit as UpToDate"
        );
    }
}
