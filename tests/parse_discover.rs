//! Task 2 tests — transcript discovery + tolerant JSONL parser.
//!
//! MIRROR: ~/Dev/sql-hidden-cost/scripts/agent-usage.mjs:54-66 (field names +
//! cache separation), ccusage summary.rs (tolerant missing-field access).
//!
//! The parser MUST tolerate:
//!   - missing fields (volatile schema, Anthropic #53516)
//!   - subagent turns (isSidechain / agentId)
//!   - compact boundaries
//!   - cache tokens reported separate from input (never double-count)
use pay4what::discover::discover_sessions;
use pay4what::parse::parse_session;
use std::fs;
use std::io::Write;
use tempfile::TempDir;

/// Build a fake ~/.claude/projects tree and confirm discovery finds the JSONL.
#[test]
fn discovers_jsonl_under_encoded_cwd_dir() {
    let root = TempDir::new().unwrap();
    // Claude Code encodes cwd as path-with-dashes: /Users/x/Dev/foo -> -Users-x-Dev-foo
    let proj_dir = root.path().join("-Users-rom-iluz-Dev-pay4what");
    fs::create_dir_all(&proj_dir).unwrap();
    let sess = proj_dir.join("abc123.jsonl");
    let mut f = fs::File::create(&sess).unwrap();
    writeln!(f, r#"{{"type":"user","timestamp":"2026-07-07T10:00:00Z","cwd":"/Users/rom.iluz/Dev/pay4what","gitBranch":"main"}}"#).unwrap();
    drop(f);

    let found = discover_sessions(root.path());
    assert_eq!(found.len(), 1, "should find exactly one session file");
    assert!(found[0].ends_with("abc123.jsonl"));
}

/// Tolerant parse: a session with only a user turn + a minimal assistant turn
/// with usage. Must not panic on missing fields.
#[test]
fn parses_minimal_session_with_usage() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("s.jsonl");
    fs::write(
        &p,
        [
            r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","cwd":"/x","gitBranch":"main","message":{"role":"user","content":"add oauth"}}"#,
            r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":200,"cache_creation_input_tokens":10}}}"#,
        ]
        .join("\n"),
    )
    .unwrap();

    let session = parse_session(&p).expect("parse should succeed");
    assert_eq!(session.cwd.as_deref(), Some("/x"));
    assert_eq!(session.git_branch.as_deref(), Some("main"));
    assert_eq!(session.turns.len(), 2);

    let usage = session.turns[1]
        .usage
        .as_ref()
        .expect("assistant turn has usage");
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.output_tokens, 50);
    assert_eq!(usage.cache_read_input_tokens, 200);
    assert_eq!(usage.cache_creation_input_tokens, 10);
    assert_eq!(usage.model, "claude-sonnet-4-6");
}

/// Tolerant parse: lines with missing/extra fields, an unparseable line, and a
/// subagent turn. Must skip bad lines, keep good ones, and flag subagent.
#[test]
fn tolerates_missing_fields_bad_lines_and_subagent() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("s.jsonl");
    fs::write(
        &p,
        [
            // user turn, no gitBranch (must not panic)
            r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","cwd":"/x","message":{"role":"user","content":"hi"}}"#,
            // garbage line (must skip)
            r#"this is not json"#,
            // assistant turn, no usage (must still record the turn, usage=None)
            r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-haiku-4-5"}}"#,
            // subagent turn (isSidechain=true) — must be flagged
            r#"{"type":"assistant","timestamp":"2026-07-07T10:00:10Z","isSidechain":true,"agentId":"sub-1","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":10,"output_tokens":5}}}"#,
        ]
        .join("\n"),
    )
    .unwrap();

    let session = parse_session(&p).expect("parse should succeed");
    // 3 good lines (garbage skipped)
    assert_eq!(session.turns.len(), 3, "garbage line should be skipped");
    assert!(session.git_branch.is_none(), "no gitBranch in this session");
    // subagent detected
    assert!(
        session.turns.iter().any(|t| t.is_sidechain),
        "subagent turn flagged"
    );
    // assistant turn without usage must have usage=None (not panic)
    assert!(
        session.turns[1].usage.is_none(),
        "turn without usage -> None"
    );
}

/// Cache separation: input_tokens is FRESH (does not include cache). The parser
/// must expose all four buckets so cost math never double-counts.
/// MIRROR: agent-usage.mjs:54-66 — input/cache_read/cache_creation are separate.
#[test]
fn exposes_cache_buckets_separately() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("s.jsonl");
    fs::write(
        &p,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:00:00Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":1000,"output_tokens":200,"cache_read_input_tokens":5000,"cache_creation_input_tokens":500}}}"#,
    )
    .unwrap();
    let session = parse_session(&p).unwrap();
    let u = session.turns[0].usage.as_ref().unwrap();
    // Four distinct buckets, NOT collapsed. cost math will price each at its own rate.
    assert_eq!(u.input_tokens, 1000);
    assert_eq!(u.cache_read_input_tokens, 5000);
    assert_eq!(u.cache_creation_input_tokens, 500);
    assert_eq!(u.output_tokens, 200);
}
