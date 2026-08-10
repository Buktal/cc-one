//! Remote probe: validate a repo URL + PAT WITHOUT touching the real
//! sync repo. A pure read (ls-remote) powering the Settings「测试连接」button so the
//! user can verify credentials before binding — and re-check after.
//!
//! This is an independent feature with its own types (`VerifyReport`) and its
//! own error model (a failed probe is a business result `ok: false`, never an
//! `AppError`). The git primitives live in [`super::git`]; this module only
//! borrows [`super::git::build_callbacks`] to inject the PAT into the one-shot
//! connection.

use git2::{Direction, ProxyOptions, Repository};

// ---------------------------------------------------------------------------
// Public probe entry
// ---------------------------------------------------------------------------

/// Outcome of a remote probe, surfaced to the UI. Always returned as `Ok`: a
/// failed probe is a business result (`ok: false`), not an `AppError`, so the
/// frontend reads `report.ok` instead of catching an exception.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct VerifyReport {
    /// True iff the repo was reachable, the PAT authenticated, and the caller
    /// has read access.
    pub ok: bool,
    /// Human-readable status (zh), shown verbatim in the Settings banner.
    pub message: String,
}

/// Probe a `(repo_url, token)` pair: validate the inputs, then open a fetch
/// connection to the remote. Never mutates config and NEVER touches the real
/// sync repo — the throwaway bare repo under the OS temp dir is the only git2
/// anchor. Why not reuse `paths.repo`: the background scheduler (lib.rs)
/// periodically pulls and pushes it, and libgit2 does not guarantee concurrent
/// access to one `.git` directory; the temp anchor path-isolates the probe.
///
/// This is the module's public interface; [`try_verify_remote`] and
/// [`friendly_git_error`] are its private implementation. Re-exported by
/// [`super`] as `crate::sync::verify_remote` so the command layer's import path
/// (and thus the tauri-specta binding for `verify_sync_repo`) is unchanged.
pub fn verify_remote(repo_url: &str, token: &str) -> VerifyReport {
    let url = repo_url.trim();
    let tok = token.trim();
    if url.is_empty() {
        return deny("请填写仓库地址");
    }
    if tok.is_empty() {
        return deny("请填写访问令牌");
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return deny("仓库地址需以 http(s):// 开头（暂不支持 SSH）");
    }
    match try_verify_remote(url, tok) {
        Ok(()) => VerifyReport {
            ok: true,
            message: "连接成功：仓库可访问、令牌可读".to_string(),
        },
        Err(e) => deny(&friendly_git_error(&e)),
    }
}

fn deny(message: &str) -> VerifyReport {
    VerifyReport {
        ok: false,
        message: message.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Connection probe (private)
// ---------------------------------------------------------------------------

/// Open a fetch connection to an arbitrary URL using a throwaway bare repo as
/// the git2 anchor. A successful `connect_auth` IS the whole probe: the URL
/// resolved, the PAT authenticated, and the caller has read access (GitHub
/// returns 404 at this stage for a missing repo or an insufficient token scope).
/// Errors stay as raw [`git2::Error`] (NOT promoted to `AppError`) so the caller
/// can read `code()` / `class()` for a user-facing diagnosis — those are lost
/// once `From<git2::Error>` flattens the error to a string.
///
/// We intentionally do NOT call `RemoteConnection::list` / `default_branch`:
/// git2 0.19.0 aborts the process (unsafe-precondition UB via a null-pointer
/// `slice::from_raw_parts`) when a remote advertises zero refs, and a brand-new
/// empty GitHub repo can. Reachability + auth + access already fully answers
/// "is this repo + token valid". `connect_auth` (git2 0.19) returns a
/// [`git2::RemoteConnection`] that disconnects on drop; we let it drop at the
/// `;`. The PAT callbacks are moved in by value, so the token lives only inside
/// that closure.
fn try_verify_remote(url: &str, token: &str) -> Result<(), git2::Error> {
    let dir = std::env::temp_dir().join(format!(
        "cc-one-verify-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    // `_guard` is dropped LAST (after repo/remote below), so the .git file
    // handles are released before the temp dir is removed (Windows file locks).
    let _guard = TmpBare(dir.clone());
    let repo = Repository::init_bare(&dir)?;
    let mut remote = repo.remote_anonymous(url)?;
    // Proxy URL borrowed locally (libgit2's ProxyOptions holds a &str); the
    // options and the URL are consumed together within this call.
    let proxy_url = crate::proxy::discover_system_proxy();
    let proxy_opts = proxy_url.as_ref().map(|u| {
        let mut p = ProxyOptions::new();
        p.url(u);
        p
    });
    remote.connect_auth(
        Direction::Fetch,
        Some(super::git::build_callbacks(token)),
        proxy_opts,
    )?;
    Ok(())
}

/// RAII guard removing a temp dir on drop. `tempfile` is dev-only, so the probe
/// builds its throwaway bare-repo anchor under the OS temp dir instead.
struct TmpBare(std::path::PathBuf);
impl Drop for TmpBare {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Translate a git2 probe failure into a zh user hint. Prefer `code()` / `class()`
/// over matching `message()`: libgit2's English wording drifts between versions,
/// and "not found" collides across DNS failure and HTTP 404 (whose fixes differ).
fn friendly_git_error(e: &git2::Error) -> String {
    use git2::{ErrorClass, ErrorCode};
    if e.message().contains("git credentials rejected") || e.code() == ErrorCode::Auth {
        return "访问令牌无效或已过期".into();
    }
    if e.code() == ErrorCode::Timeout {
        return "连接超时，请检查网络".into();
    }
    if e.code() == ErrorCode::NotFound {
        return "无法解析主机名或地址不可达（请检查仓库地址拼写）".into();
    }
    if e.class() == ErrorClass::Http {
        return "仓库不存在，或令牌无权访问该仓库（GitHub 对二者均返回 404）".into();
    }
    if e.class() == ErrorClass::Net {
        return "网络连接失败，请检查网络".into();
    }
    if e.class() == ErrorClass::Ssl {
        return "TLS/SSL 握手失败".into();
    }
    e.message().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- remote probe tests (「测试连接」) ----
    //
    // The auth / 404 / DNS / timeout branches need a live network, so they are
    // covered only by the manual checklist below — NOT by these unit tests:
    //   · 坏 PAT → 真实 GitHub + 错令牌（应提示「访问令牌无效或已过期」）
    //   · 不存在 / 无权私有仓 → 真实 GitHub + 不存在仓（应提示 404 / 无权）
    //   · DNS 失败 → https://nonexistent.invalid/x/y.git
    //   · 超时 → 死/慢主机

    #[test]
    fn verify_remote_validates_inputs() {
        let r = verify_remote("", "tok");
        assert!(!r.ok && r.message.contains("仓库地址"));
        let r = verify_remote("https://github.com/x/y", "");
        assert!(!r.ok && r.message.contains("访问令牌"));
        // SSH-style URLs are rejected (http(s) only).
        let r = verify_remote("git@github.com:x/y.git", "tok");
        assert!(!r.ok && r.message.contains("http"));
    }

    /// `try_verify_remote` bypasses the https:// input gate, so a local file://
    /// bare repo can exercise the connect path without network.
    #[test]
    fn try_verify_remote_connects_to_local_bare() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        crate::sync::seed_remote(&remote);
        let url = remote.to_string_lossy().to_string();
        // Local file transport needs no auth; the token is unused.
        try_verify_remote(&url, "local-no-auth").unwrap();
    }

    #[test]
    fn try_verify_remote_fails_on_missing_path() {
        let tmp = tempfile::tempdir().unwrap();
        let url = tmp
            .path()
            .join("does-not-exist.git")
            .to_string_lossy()
            .to_string();
        assert!(try_verify_remote(&url, "tok").is_err());
    }
}
