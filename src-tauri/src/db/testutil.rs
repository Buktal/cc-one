//! Cross-domain test fixtures shared by every `db/<domain>.rs` test module.
//!
//! Each `pub fn` here is a small builder that hides noisy struct literals so the
//! domain tests read just the behavior under test. `mem` opens a fresh
//! in-memory `Store`; `rec`/`msg` build flat records/messages; the `seed_*`
//! helpers insert minimal session rows (+ bound usage row) for session/transcript
//! tests.

use super::*;
use crate::model::{ServerToolUse, TokenCounts, UsageRecord};
use std::path::Path;

pub fn mem() -> Store {
    Store::open(Path::new(":memory:")).unwrap()
}

/// Build a stored record with a flat (input-only) cost for test simplicity.
pub fn rec(
    uuid: &str,
    day: &str,
    model: &str,
    device: &str,
    input: u32,
    output: u32,
    cost_usd: f64,
) -> UsageRecord {
    let total = rust_decimal::Decimal::try_from(cost_usd).unwrap_or(rust_decimal::Decimal::ZERO);
    UsageRecord {
        uuid: uuid.into(),
        timestamp: format!("{day}T10:00:00.000Z"),
        day: day.into(),
        model: model.into(),
        pricing_model: crate::model::normalize_model_key(model),
        source: "claude_code".into(),
        session_id: String::new(),
        device_id: device.into(),
        tokens: TokenCounts {
            input,
            output,
            cache_creation: 0,
            cache_read: 0,
        },
        server_tool_use: ServerToolUse::default(),
        stop_reason: "end_turn".into(),
        service_tier: "standard".into(),
        iterations: 0,
        cost: crate::model::CostBreakdown {
            input_usd: total,
            output_usd: rust_decimal::Decimal::ZERO,
            cache_read_usd: rust_decimal::Decimal::ZERO,
            cache_creation_usd: rust_decimal::Decimal::ZERO,
            total_usd: total,
        },
    }
}

pub fn msg(uuid: &str, sid: &str, role: SessionMessageRole, ts: &str) -> SessionMessage {
    SessionMessage {
        uuid: uuid.into(),
        session_id: sid.into(),
        role,
        ts: ts.into(),
        model: (role == SessionMessageRole::Assistant).then(|| "glm-5.2".to_string()),
        name: (role == SessionMessageRole::Tool).then(|| "Bash".to_string()),
        content: format!("content-{uuid}"),
    }
}

/// Helper: insert one session row with an explicit source.
pub fn seed_session_source(store: &Store, id: &str, device: &str, source: &str, last_active: &str) {
    store
        .upsert_session(
            device,
            &SessionSystemData {
                id: id.into(),
                source: source.into(),
                project_dir: "/proj".into(),
                title_orig: "Title".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: last_active.into(),
                agent_type: String::new(),
                parent_session_id: String::new(),
            },
        )
        .unwrap();
}

/// Helper: insert one session row with an explicit project_dir (the raw launch
/// dir — worktree suffixes stay raw in storage, the project dimension
/// collapses them at read time).
pub fn seed_session_project(
    store: &Store,
    id: &str,
    device: &str,
    project_dir: &str,
    last_active: &str,
) {
    store
        .upsert_session(
            device,
            &SessionSystemData {
                id: id.into(),
                source: "claude_code".into(),
                project_dir: project_dir.into(),
                title_orig: "Title".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: last_active.into(),
                agent_type: String::new(),
                parent_session_id: String::new(),
            },
        )
        .unwrap();
}

/// Helper: insert one session row with a given last_active_at.
pub fn seed_session(store: &Store, id: &str, device: &str, last_active: &str) {
    seed_session_source(store, id, device, "claude_code", last_active)
}

/// Build a system-data row for a session (the 6 refreshable columns). Unlike
/// `seed_session`, returns the value so a test can vary `last_active_at`
/// across calls (re-extract scenarios).
pub fn sys_session(id: &str, last_active_at: &str) -> SessionSystemData {
    SessionSystemData {
        id: id.into(),
        source: "claude_code".into(),
        project_dir: "/proj".into(),
        title_orig: "orig-title".into(),
        started_at: "2026-08-01T00:00:00.000Z".into(),
        last_active_at: last_active_at.into(),
        agent_type: String::new(),
        parent_session_id: String::new(),
    }
}

/// Helper: seed one session row + one usage record bound to it.
pub fn seed_session_with_record(store: &Store, sid: &str, device: &str, model: &str) {
    seed_session(store, sid, device, "2026-08-15T10:00:00.000Z");
    let mut r = rec(
        &format!("u-{sid}-{model}"),
        "2026-08-15",
        model,
        device,
        10,
        10,
        0.001,
    );
    r.session_id = sid.into();
    store.ingest_marking_dirty(&[r]).unwrap();
}
