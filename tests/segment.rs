//! Task 4 tests — segmentation: split a session at user-turn boundaries.
//!
//! A segment = the work done in response to one user message (the assistant
//! turns + tool calls that follow it, until the next user turn). This is the
//! unit of categorization.
//!
//! Subagent spend is handled separately (separate files, discovered via
//! discover_subagents) — segmentation operates on a single session file.
use pay4what::parse::parse_session;
use pay4what::segment::segment_session;
use std::fs;
use tempfile::TempDir;

fn write_session(lines: &[&str]) -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("s.jsonl");
    fs::write(&p, lines.join("\n")).unwrap();
    (tmp, p)
}

#[test]
fn splits_at_user_turn_boundaries() {
    let (_tmp, p) = write_session(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","cwd":"/x","gitBranch":"main","message":{"role":"user","content":"add oauth"}}"#,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":100,"output_tokens":50}}}"#,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:00:10Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":50,"output_tokens":30}}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:01:00Z","message":{"role":"user","content":"now fix the login bug"}}"#,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:01:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":80,"output_tokens":40}}}"#,
    ]);
    let session = parse_session(&p).unwrap();
    let segments = segment_session(&session);
    assert_eq!(segments.len(), 2, "two user turns -> two segments");
    // segment 0: the oauth work (2 assistant turns)
    assert_eq!(segments[0].turns.len(), 3, "segment 0 = user + 2 assistant");
    assert_eq!(segments[0].user_message, "add oauth");
    // segment 1: the bugfix (1 assistant turn)
    assert_eq!(segments[1].turns.len(), 2, "segment 1 = user + 1 assistant");
    assert_eq!(segments[1].user_message, "now fix the login bug");
}

#[test]
fn segment_carries_cost_and_tool_uses() {
    let (_tmp, p) = write_session(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"feat/oauth","message":{"role":"user","content":"add oauth"}}"#,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":2000,"cache_creation_input_tokens":100}}}"#,
    ]);
    let session = parse_session(&p).unwrap();
    let pricing = pay4what::cost::bundled_pricing();
    let segments = segment_session(&session);
    assert_eq!(segments.len(), 1);
    let seg = &segments[0];
    // cost is computed from the assistant turns in the segment
    let expected = pay4what::cost::cost_for_usage(seg.turns[1].usage.as_ref().unwrap(), &pricing);
    assert!(
        (seg.cost - expected).abs() < 1e-9,
        "segment carries its cost"
    );
    assert_eq!(seg.git_branch.as_deref(), Some("feat/oauth"));
}

#[test]
fn collects_touched_files_from_edit_tool_uses() {
    let (_tmp, p) = write_session(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","message":{"role":"user","content":"add oauth"}}"#,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":100,"output_tokens":50},"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/x/src/auth.rs","oldText":"a","newText":"b"}},{"type":"tool_use","name":"Write","input":{"file_path":"/x/src/oauth.rs","content":"..."}}]}}"#,
    ]);
    let session = parse_session(&p).unwrap();
    let segments = segment_session(&session);
    let files = &segments[0].touched_files;
    assert_eq!(files.len(), 2, "two files touched");
    assert!(files.contains(&"/x/src/auth.rs".to_string()));
    assert!(files.contains(&"/x/src/oauth.rs".to_string()));
}

#[test]
fn empty_session_yields_no_segments() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("s.jsonl");
    fs::write(&p, "").unwrap();
    let session = parse_session(&p).unwrap();
    let segments = segment_session(&session);
    assert!(segments.is_empty());
}

#[test]
fn leading_non_user_turns_grouped_into_first_segment() {
    // If the session starts with system/attachment turns before the first user
    // turn, those prefatory turns attach to the first segment (don't lose them).
    let (_tmp, p) = write_session(&[
        r#"{"type":"system","timestamp":"2026-07-07T09:59:00Z","content":"init"}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","message":{"role":"user","content":"hi"}}"#,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":10,"output_tokens":5}}}"#,
    ]);
    let session = parse_session(&p).unwrap();
    let segments = segment_session(&session);
    assert_eq!(segments.len(), 1);
    // system turn is included in the segment (prefatory)
    assert_eq!(segments[0].turns.len(), 3, "system + user + assistant");
}
