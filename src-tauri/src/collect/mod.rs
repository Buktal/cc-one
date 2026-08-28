//! Collect orchestration: parse Sources into the Local Store, then sync with
//! peer devices.
//!
//! `collect_into` is the single ingest path shared by the manual actions and
//! the background scheduler. `align` is the full manual action (collect, then
//! pull+push in Synced mode). One pull+push pass is [`sync_round`], wrapped by
//! [`run_sync_round`] — the single entry every caller takes its posture from
//! ([`SyncRoundPosture`]: retry-with-backoff for the manual action, once for
//! the scheduler, once + outcome logging for the `set_sync_repo` bind), so the
//! round's policy lives in one place and the command layer only binds. Collect
//! and sync are DECOUPLED at the scheduler — collect is a short seconds-level
//! local cadence, sync is a longer minutes-level Git cadence (Synced only), so
//! the scheduler triggers them on independent deadlines rather than chaining
//! them.

pub mod artifact;
pub mod ingest;
pub mod jsonl;

use std::time::Duration;

use self::ingest::IngestReport;
use crate::config::ConfigStore;
use crate::db::Store;
use crate::error::AppResult;
use crate::source_parser::SourceParser;
use crate::sync;

/// Parse Source → Local Store. No network. Shared by the manual `collect_now`
/// command and the background scheduler so both follow the exact same ingest
/// path.
///
/// Iterates every enabled parser. The per-file cursor table is loaded once
/// and shared (keys are file paths, disjoint across parsers); each parser's
/// cursor advances are merged and persisted AFTER all ingests — so a failed
/// ingest leaves cursors untouched (next collect re-parses the same lines; the
/// store's primary-key dedup absorbs the re-read). First run / empty table ⇒
/// full scan.
pub fn collect_into(store: &Store, config: &ConfigStore) -> AppResult<IngestReport> {
    collect_into_with(store, config, crate::source_parser::all_source_parsers()?)
}

/// Same orchestration as [`collect_into`], but with an explicit parser set —
/// the root-injection seam for testing: production reaches it via
/// [`collect_into`] (real-home parsers); the orchestration test injects
/// tempdir-rooted parsers (`source_parser::all_source_parsers_at`). Every
/// invariant below — per-parser incremental collect, cursor deltas merged and
/// saved AFTER all ingests — runs on the exact production code path, so it is
/// provable against a fixture dir instead of a real `~/.claude`.
pub fn collect_into_with(
    store: &Store,
    config: &ConfigStore,
    parsers: Vec<Box<dyn SourceParser>>,
) -> AppResult<IngestReport> {
    let cfg = config.get();
    let paths = config.paths();
    let progress = store.load_scan_progress()?;
    let book = store.load_pricing_book()?;

    let mut merged = IngestReport::default();
    let mut merged_delta = crate::source_parser::ScanProgressDelta::new();
    let mut sources_with_rows: Vec<String> = Vec::new();
    for parser in &parsers {
        let (result, delta) = parser.collect_incremental(&progress)?;
        let report = ingest::ingest_collected(store, &paths, &cfg.device_id, &book, result)?;
        if report.rows_inserted > 0 {
            sources_with_rows.push(report.source.clone());
        }
        merged.events_collected += report.events_collected;
        merged.rows_inserted += report.rows_inserted;
        merged.turn_durations_collected += report.turn_durations_collected;
        merged.turn_durations_inserted += report.turn_durations_inserted;
        merged.files_scanned += report.files_scanned;
        merged.lines_skipped += report.lines_skipped;
        merged_delta.extend(delta);
    }
    merged.source = sources_with_rows.join(",");
    // Post-collect device-registry maintenance — NON-destructive by design:
    // touch self + discover usage-only devices, nothing that can delete data
    // (ADR-0013: the destructive reconcile lives at the single post-pull point
    // `devices::reload_devices_into_store`, so no collect-tick misread of a
    // jittering worktree can ever fire a forget). Runs here, on the collect
    // path — not on the read-only list_devices command — so a query never
    // mutates the DB. Worst-case latency to surface a new device is one
    // collect interval.
    crate::devices::refresh_device_registry(store, &cfg)?;
    store.save_scan_progress(&merged_delta)?;
    Ok(merged)
}

/// Outcome of one「采集 / 同步」action, surfaced to the UI. Best-effort: every
/// step runs independently, so `errors` carries per-step failures rather than
/// aborting early. Empty on full success.
#[derive(Debug, Clone, Default, serde::Serialize, specta::Type)]
pub struct AlignReport {
    /// Local collect outcome (zeroed if collect itself failed — see `errors`).
    pub collected: IngestReport,
    /// Items imported from peer devices this round, summed across every sync
    /// domain (usage rows / session snapshots / provider entries / registry
    /// rows — the per-domain grains are documented on the sync domain table's
    /// `import` contract) (Synced only).
    pub imported: u32,
    /// True iff a local change was committed and pushed (Synced only).
    pub pushed: bool,
    /// Per-step failures (`collect: …`, `pull: …`, `push: …`). Empty on success.
    pub errors: Vec<String>,
}

/// Outcome of [`sync_round`] — the pull/push half of an [`AlignReport`].
#[derive(Debug, Clone, Default)]
pub(crate) struct SyncRoundOutcome {
    pub(crate) imported: u32,
    pub(crate) pushed: bool,
    pub(crate) errors: Vec<String>,
}

/// The step whose failure an [`AlignReport`] error entry describes — the wire
/// prefix protocol of `AlignReport.errors` (`collect:` / `pull:` / `push:`),
/// single home of the step names. A new step joins by adding an arm here, not
/// by string-pasting a prefix at a call site.
#[derive(Debug, Clone, Copy)]
enum AlignStep {
    Collect,
    Pull,
    Push,
}

impl AlignStep {
    /// Format one step failure as an `AlignReport.errors` entry.
    fn error(self, e: &impl std::fmt::Display) -> String {
        let prefix = match self {
            AlignStep::Collect => "collect",
            AlignStep::Pull => "pull",
            AlignStep::Push => "push",
        };
        format!("{prefix}: {e}")
    }
}

/// One best-effort sync round: pull peer devices' Artifacts, then push this
/// device's. Both steps run independently — a pull failure does NOT skip push
/// (a failed pull usually means nothing new to push, but push may still succeed
/// on a flaky network). Errors land in `errors` rather than aborting the round.
/// The plain round — callers pick the posture (once / once+log / retry) via
/// [`run_sync_round`]. Synced only; a no-op (zeroed outcome) in Standalone.
pub(crate) fn sync_round(store: &Store, config: &ConfigStore) -> SyncRoundOutcome {
    let mut out = SyncRoundOutcome::default();
    let cfg = config.get();
    if !cfg.is_synced() {
        return out;
    }
    let paths = config.paths();
    match sync::pull_and_import(store, &paths, &cfg) {
        Ok(n) => out.imported = n,
        Err(e) => out.errors.push(AlignStep::Pull.error(&e)),
    }
    match sync::push_usage(store, &paths, &cfg) {
        Ok(p) => out.pushed = p,
        Err(e) => out.errors.push(AlignStep::Push.error(&e)),
    }
    out
}

/// How one sync round is run — the posture. Every caller of a sync round (the
/// manual `align`, the background scheduler, the `set_sync_repo` bind) picks
/// ONE posture here, so the round's policy (retry? outcome logging?) lives in
/// one place instead of being re-decided per call site.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SyncRoundPosture {
    /// Run exactly once; the outcome is returned for the caller to handle.
    /// The background scheduler's choice — the push cadence IS the retry.
    Once,
    /// Run exactly once, then log the outcome under `label` (diagnostic
    /// prefix). `set_sync_repo`'s choice: the bind-time round's result goes
    /// only to the log — a failure is left for the next startup sync to
    /// retry, not retried in place.
    OnceLogged(&'static str),
    /// Run up to 3 attempts with a short backoff (1 s, 2 s); `imported`
    /// aggregates across attempts. The manual「采集 / 同步」choice (`align`).
    Retry,
}

/// Run one sync round under the given posture — the single entry for "a round
/// of pull+push". The retry policy and the [`SyncRoundPosture::OnceLogged`]
/// outcome logging live here; every caller gets back the same
/// [`SyncRoundOutcome`] regardless of posture.
pub(crate) fn run_sync_round(
    store: &Store,
    config: &ConfigStore,
    posture: SyncRoundPosture,
) -> SyncRoundOutcome {
    let out = match posture {
        SyncRoundPosture::Once | SyncRoundPosture::OnceLogged(_) => sync_round(store, config),
        SyncRoundPosture::Retry => {
            retry_rounds(|| sync_round(store, config), 3, std::thread::sleep)
        }
    };
    if let SyncRoundPosture::OnceLogged(label) = posture {
        if out.imported > 0 {
            eprintln!("[cc-one] {label} imported {} row(s)", out.imported);
        }
        if out.pushed {
            eprintln!("[cc-one] {label} pushed local changes");
        }
        for e in &out.errors {
            eprintln!("[cc-one] {label} sync error: {e}");
        }
    }
    out
}

/// Bounded retry over a sync round, factored out of [`align`] so the
/// retry-aggregation logic is unit-testable without real git IO or real time.
///
/// `round` produces one [`SyncRoundOutcome`] per call; `sleep` backs off
/// between attempts (production passes [`std::thread::sleep`]; tests inject a
/// no-op so the 3-attempt retry is instant). Stops early once a round returns
/// no errors; otherwise runs `max_attempts` times.
///
/// `imported` is SUMMED across retries: pull is uuid-deduped, so a row pulled
/// on attempt 1 (then lost to a push failure) reads 0 on attempt 2 — taking
/// only the last round's imported would report "0 imported" despite real new
/// rows. The returned outcome carries the sum in `imported`, plus the final
/// round's `pushed` / `errors`.
fn retry_rounds<R, S>(mut round: R, max_attempts: u32, mut sleep: S) -> SyncRoundOutcome
where
    R: FnMut() -> SyncRoundOutcome,
    S: FnMut(Duration),
{
    let mut last = SyncRoundOutcome::default();
    let mut imported = 0u32;
    for attempt in 0u32..max_attempts {
        last = round();
        imported += last.imported;
        if last.errors.is_empty() {
            break;
        }
        // Back off before the next attempt (1 s, 2 s); skip after the last.
        if attempt + 1 < max_attempts {
            sleep(Duration::from_secs(1u64 << attempt));
        }
    }
    last.imported = imported;
    last
}

/// Full manual「同步 / 采集」: collect locally, then (Synced only) run a sync
/// round under the [`SyncRoundPosture::Retry`] posture — up to 3 attempts with
/// a short backoff (1 s, 2 s; the retry loop + no-op sleeper live in
/// `retry_rounds`, unit-tested). Retry covers only the network steps
/// (pull/push); collect runs once (a local disk failure won't fix itself on
/// retry). Best-effort: every step's outcome is reported independently in
/// `errors`, none aborts the others.
///
/// Shared by the dashboard button and the Settings「立即同步」entry — the run
/// mode decides what it means (Standalone ⇒ collect only; Synced ⇒ collect +
/// sync). The caller emits `usage_changed` after this returns.
pub fn align(store: &Store, config: &ConfigStore) -> AlignReport {
    let mut report = AlignReport::default();
    match collect_into(store, config) {
        Ok(r) => report.collected = r,
        Err(e) => report.errors.push(AlignStep::Collect.error(&e)),
    }
    if config.get().is_synced() {
        let outcome = run_sync_round(store, config, SyncRoundPosture::Retry);
        report.imported = outcome.imported;
        report.pushed = outcome.pushed;
        report.errors.extend(outcome.errors);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::{
        collect_into_with, retry_rounds, run_sync_round, SyncRoundOutcome, SyncRoundPosture,
    };
    use std::cell::{Cell, RefCell};
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use crate::config::{ConfigData, ConfigStore, Paths};
    use crate::db::Store;
    use crate::error::AppResult;
    use crate::source_parser::{CollectResult, ScanProgress, ScanProgressDelta, SourceParser};

    /// Parser wrapper whose `collect_incremental` fails once on demand — the
    /// orchestration-failure injector for the cursor-not-advanced invariant.
    struct FlakyParser {
        inner: Box<dyn SourceParser>,
        fail_next: AtomicBool,
    }

    impl SourceParser for FlakyParser {
        fn name(&self) -> &'static str {
            self.inner.name()
        }

        fn discover(&self) -> AppResult<Vec<PathBuf>> {
            self.inner.discover()
        }

        fn session_identity(&self) -> crate::source_parser::SessionIdentity {
            self.inner.session_identity()
        }

        fn collect_incremental(
            &self,
            progress: &ScanProgress,
        ) -> AppResult<(CollectResult, ScanProgressDelta)> {
            if self.fail_next.swap(false, Ordering::SeqCst) {
                return Err(crate::error::AppError::SourceParser("flaky collect".into()));
            }
            self.inner.collect_incremental(progress)
        }
    }

    /// One Claude JSONL assistant line (the minimal shape the parser needs).
    fn claude_line(uuid: &str, ts: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","uuid":"{uuid}","cwd":"/home/me/proj","summary":"Build a thing","message":{{"id":"msg_{uuid}","model":"glm-5.2","role":"assistant","stop_reason":"end_turn","usage":{{"input_tokens":100,"output_tokens":50}}}}}}"#
        )
    }

    /// tempdir home + claude projects fixture + in-memory store + test config —
    /// the collect-orchestration fixture: production paths (`collect_into`)
    /// and this test share `collect_into_with`, differing only in the injected
    /// parser roots (real `~` vs tempdir).
    fn orchestration_fixture(home: &Path) -> (Store, ConfigStore, PathBuf) {
        let projects = home.join(".claude").join("projects").join("proj-a");
        std::fs::create_dir_all(&projects).unwrap();
        let session_file = projects.join("s-001.jsonl");
        let body = format!(
            "{}\n{}\n",
            claude_line("a1", "2026-08-01T11:00:00Z"),
            claude_line("a2", "2026-08-01T11:01:00Z")
        );
        std::fs::write(&session_file, &body).unwrap();

        let store = Store::open(Path::new(":memory:")).unwrap();
        let paths = Paths::resolve(home);
        let data = ConfigData {
            device_id: "0123456789ab".into(),
            ..Default::default()
        };
        (store, ConfigStore::for_test(paths, data), session_file)
    }

    /// The full collect chain runs over tempdir-rooted parsers — the seam the
    /// orchestration invariants were previously untestable at. First pass
    /// ingests everything; a no-change pass ingests nothing (the saved cursor +
    /// mtime gate skip unchanged files); an append ingests only the new line.
    #[test]
    fn collect_into_with_ingests_incrementally_over_tempdir_parsers() {
        let tmp = tempfile::tempdir().unwrap();
        let (store, config, session_file) = orchestration_fixture(tmp.path());

        let parsers = crate::source_parser::all_source_parsers_at(tmp.path());
        let r1 = collect_into_with(&store, &config, parsers).unwrap();
        assert_eq!(r1.rows_inserted, 2);
        assert_eq!(r1.files_scanned, 1);
        assert_eq!(r1.source, "claude_code");

        let parsers = crate::source_parser::all_source_parsers_at(tmp.path());
        let r2 = collect_into_with(&store, &config, parsers).unwrap();
        assert_eq!(
            r2.rows_inserted, 0,
            "saved cursor + mtime gate skip the file"
        );
        assert_eq!(r2.files_scanned, 1, "discovered, then gated");

        let body = std::fs::read_to_string(&session_file).unwrap();
        std::fs::write(
            &session_file,
            format!("{body}{}\n", claude_line("a3", "2026-08-01T12:00:00Z")),
        )
        .unwrap();
        let parsers = crate::source_parser::all_source_parsers_at(tmp.path());
        let r3 = collect_into_with(&store, &config, parsers).unwrap();
        assert_eq!(r3.rows_inserted, 1, "append ingests only the new line");
    }

    /// A failed collect aborts the orchestration BEFORE any cursor delta is
    /// saved — the cursor table is written only after all ingests succeed. A
    /// subsequent healthy collect therefore re-parses in full (a wrongly saved
    /// cursor would gate the file and ingest zero rows).
    #[test]
    fn failed_collect_leaves_no_cursor_advance() {
        let tmp = tempfile::tempdir().unwrap();
        let (store, config, _session_file) = orchestration_fixture(tmp.path());

        let flaky = FlakyParser {
            // First parser in factory order is claude (the fixture's source).
            inner: crate::source_parser::all_source_parsers_at(tmp.path())
                .into_iter()
                .next()
                .unwrap(),
            fail_next: AtomicBool::new(true),
        };
        let err = collect_into_with(&store, &config, vec![Box::new(flaky)]);
        assert!(err.is_err(), "a failing parser aborts the orchestration");

        let parsers = crate::source_parser::all_source_parsers_at(tmp.path());
        let r = collect_into_with(&store, &config, parsers).unwrap();
        assert_eq!(
            r.rows_inserted, 2,
            "failed pass left no cursor → the retry re-parses in full"
        );
    }

    // Scripted always-errors round; `imported = n` makes aggregation observable.
    fn err_round(n: u32) -> SyncRoundOutcome {
        SyncRoundOutcome {
            imported: n,
            pushed: false,
            errors: vec!["e".to_string()],
        }
    }

    /// On the first clean round we stop, having aggregated imported across the
    /// retries that ran — and we only slept between attempts that actually
    /// happened (1→2), not after the terminating clean round.
    #[test]
    fn retry_rounds_breaks_on_clean_round_and_aggregates_imported() {
        let script = [
            SyncRoundOutcome {
                imported: 5,
                pushed: false,
                errors: vec!["pull: x".to_string()],
            },
            SyncRoundOutcome {
                imported: 0,
                pushed: true,
                errors: vec![],
            },
        ];
        let idx = Cell::new(0usize);
        let sleeps = Cell::new(0u32);
        let out = retry_rounds(
            || {
                let i = idx.get();
                idx.set(i + 1);
                script[i].clone()
            },
            3,
            |_| sleeps.set(sleeps.get() + 1),
        );
        assert_eq!(idx.get(), 2, "stopped after the clean 2nd round, no 3rd");
        assert_eq!(sleeps.get(), 1, "slept once between attempts 1→2 only");
        assert_eq!(out.imported, 5, "imported aggregated across retries");
        assert!(out.pushed, "final round's pushed carried through");
        assert!(
            out.errors.is_empty(),
            "final round's clean errors carried through"
        );
    }

    /// When every round errors we exhaust all attempts, sleeping only between
    /// them (not after the last), and imported accumulates from every attempt.
    #[test]
    fn retry_rounds_exhausts_attempts_when_always_errors() {
        let calls = Cell::new(0u32);
        let sleeps = Cell::new(0u32);
        let out = retry_rounds(
            || {
                calls.set(calls.get() + 1);
                err_round(1)
            },
            3,
            |_| sleeps.set(sleeps.get() + 1),
        );
        assert_eq!(calls.get(), 3, "all 3 attempts used");
        assert_eq!(
            sleeps.get(),
            2,
            "slept between attempts only, not after the last"
        );
        assert_eq!(out.imported, 3, "1 imported per attempt × 3");
        assert_eq!(out.errors, vec!["e".to_string()]);
    }

    /// The backoff doubles (1 s, 2 s) and never fires after the final attempt.
    #[test]
    fn retry_rounds_backoff_is_1s_then_2s() {
        let sleeps: Rc<RefCell<Vec<Duration>>> = Rc::new(RefCell::new(Vec::new()));
        let cap = sleeps.clone();
        let _out = retry_rounds(|| err_round(0), 3, move |d| cap.borrow_mut().push(d));
        assert_eq!(
            *sleeps.borrow(),
            vec![Duration::from_secs(1), Duration::from_secs(2)],
            "backoff doubles (1s, 2s); nothing after the last attempt"
        );
    }

    /// The posture entry's Standalone guard: every posture is a zeroed no-op
    /// without a Synced config — no network, no errors, no retry attempts.
    #[test]
    fn run_sync_round_is_zeroed_noop_in_standalone_for_every_posture() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(Path::new(":memory:")).unwrap();
        let config = ConfigStore::for_test(Paths::resolve(tmp.path()), ConfigData::default());
        for posture in [
            SyncRoundPosture::Once,
            SyncRoundPosture::OnceLogged("test"),
            SyncRoundPosture::Retry,
        ] {
            let out = run_sync_round(&store, &config, posture);
            assert_eq!(out.imported, 0, "{posture:?}: no network in Standalone");
            assert!(!out.pushed, "{posture:?}: nothing pushed");
            assert!(out.errors.is_empty(), "{posture:?}: no errors");
        }
    }
}
