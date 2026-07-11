//! Task 5 tests — categorization (rules fallback + LLM-primary via OpenRouter).
//!
//! LLM-PRIMARY: every segment goes through the LLM; rules are fallback (no key)
//! + obvious-case pre-tag (skip the LLM for branch-name-clear cases).
//!
//! Activity labels match the handoff's viral screenshot:
//!   feature, bugfix, migration, refactor, debugging, exploration, planning,
//!   unattributed.
use pay4what::categorize::{Activity, Categorizer, RulesCategorizer, categorize_segments};
use pay4what::parse::parse_session;
use pay4what::segment::segment_session;
use std::fs;
use tempfile::TempDir;

fn seg_of(lines: &[&str]) -> Vec<pay4what::segment::Segment> {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("s.jsonl");
    fs::write(&p, lines.join("\n")).unwrap();
    let session = parse_session(&p).unwrap();
    segment_session(&session)
}

// ─── Rules categorizer (no API key path) ───────────────────────────────────

#[test]
fn rules_branch_feat_tags_feature() {
    let segs = seg_of(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"feat/oauth-rotation","message":{"role":"user","content":"add the new flow"}}"#,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":100,"output_tokens":50}}}"#,
    ]);
    let cat = RulesCategorizer;
    assert_eq!(cat.categorize(&segs[0]), Activity::Feature);
}

#[test]
fn rules_branch_fix_tags_bugfix() {
    let segs = seg_of(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"fix/login-redirect","message":{"role":"user","content":"the login loop"}}"#,
    ]);
    assert_eq!(RulesCategorizer.categorize(&segs[0]), Activity::Bugfix);
}

#[test]
fn rules_message_keyword_tags_when_no_branch_signal() {
    // master branch (no signal), but user message says "refactor"
    let segs = seg_of(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"master","message":{"role":"user","content":"refactor the billing service"}}"#,
    ]);
    assert_eq!(RulesCategorizer.categorize(&segs[0]), Activity::Refactor);
}

#[test]
fn rules_read_only_tools_tag_exploration() {
    let segs = seg_of(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"main","message":{"role":"user","content":"how does this work"}}"#,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"tool_use","name":"Read","input":{"file_path":"/x/a.rs"}},{"type":"tool_use","name":"Grep","input":{"pattern":"foo"}}]}}"#,
    ]);
    assert_eq!(RulesCategorizer.categorize(&segs[0]), Activity::Exploration);
}

#[test]
fn rules_unattributed_when_no_signal() {
    let segs = seg_of(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"HEAD","message":{"role":"user","content":"ok"}}"#,
    ]);
    assert_eq!(
        RulesCategorizer.categorize(&segs[0]),
        Activity::Unattributed
    );
}

// ─── Categorize-all (the entry point) ──────────────────────────────────────

#[test]
fn categorize_segments_labels_each() {
    let segs = seg_of(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"feat/oauth","message":{"role":"user","content":"add oauth"}}"#,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":100,"output_tokens":50}}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:01:00Z","gitBranch":"fix/login","message":{"role":"user","content":"fix the login bug"}}"#,
        r#"{"type":"assistant","timestamp":"2026-07-07T10:01:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":50,"output_tokens":20}}}"#,
    ]);
    let cat = RulesCategorizer;
    let labeled = categorize_segments(&segs, &cat);
    assert_eq!(labeled.len(), 2);
    assert_eq!(labeled[0].activity, Activity::Feature);
    assert_eq!(labeled[1].activity, Activity::Bugfix);
    // labeled segment keeps cost + files from the original segment
    assert!(labeled[0].cost > 0.0);
}

// ─── LLM categorizer (mockable caller — no network in tests) ───────────────

/// A mock caller that returns a canned response, simulating OpenRouter.
struct MockCaller {
    response: String,
}

impl pay4what::categorize::LlmCaller for MockCaller {
    fn categorize_batch(&self, _prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
        Ok(self.response.clone())
    }
}

#[test]
fn llm_categorizer_parses_json_response() {
    // OpenRouter returns one label per segment (batched). Mock returns JSON array.
    let mock = MockCaller {
        response: r#"{"results":[{"activity":"feature","tags":["auth"],"summary":"add oauth","confidence":0.9},{"activity":"bugfix","tags":["login"],"summary":"fix login","confidence":0.85}]}"#.to_string(),
    };
    let cat =
        pay4what::categorize::LlmCategorizer::new("deepseek/deepseek-v4-flash", Box::new(mock));
    let segs = seg_of(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"main","message":{"role":"user","content":"add oauth"}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:01:00Z","gitBranch":"main","message":{"role":"user","content":"fix login"}}"#,
    ]);
    let labeled = categorize_segments(&segs, &cat);
    assert_eq!(labeled.len(), 2);
    assert_eq!(labeled[0].activity, Activity::Feature);
    assert_eq!(labeled[1].activity, Activity::Bugfix);
}

#[test]
fn llm_categorizer_falls_back_on_unparseable_response() {
    let mock = MockCaller {
        response: "garbage not json".to_string(),
    };
    let cat =
        pay4what::categorize::LlmCategorizer::new("deepseek/deepseek-v4-flash", Box::new(mock));
    let segs = seg_of(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"main","message":{"role":"user","content":"do something"}}"#,
    ]);
    let labeled = categorize_segments(&segs, &cat);
    // unparseable LLM response -> fall back to rules; rules see no signal -> unattributed
    assert_eq!(labeled[0].activity, Activity::Unattributed);
}

// ─── Count-mismatch partial-result handling ────────────────────────────────

/// A mock caller that returns an error, simulating a timeout or network failure.
struct ErrMockCaller;
impl pay4what::categorize::LlmCaller for ErrMockCaller {
    fn categorize_batch(&self, _prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
        Err("simulated timeout".into())
    }
}

#[test]
fn count_mismatch_uses_partial_results() {
    // LLM returns 3 records for 5 segments — the 3 that parsed should be USED,
    // the 2 gaps filled with rules (not all 5 discarded).
    let mock = MockCaller {
        response: r#"{"results":[
            {"activity":"feature","tags":["auth"],"summary":"add oauth","confidence":0.9},
            {"activity":"bugfix","tags":["login"],"summary":"fix login","confidence":0.85},
            {"activity":"refactor","tags":["billing"],"summary":"extract billing","confidence":0.8}
        ]}"#
            .to_string(),
    };
    let cat =
        pay4what::categorize::LlmCategorizer::new("deepseek/deepseek-v4-flash", Box::new(mock));
    let segs = seg_of(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"main","message":{"role":"user","content":"add oauth"}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:01:00Z","gitBranch":"main","message":{"role":"user","content":"fix login"}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:02:00Z","gitBranch":"main","message":{"role":"user","content":"extract billing"}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:03:00Z","gitBranch":"main","message":{"role":"user","content":"do something else"}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:04:00Z","gitBranch":"main","message":{"role":"user","content":"and another thing"}}"#,
    ]);
    let records = pay4what::categorize::Categorizer::categorize_rich(&cat, &segs);
    assert_eq!(records.len(), 5, "should return 5 records (3 LLM + 2 rules-fill)");
    // first 3 are from LLM — have tags + summaries
    assert_eq!(records[0].tags, vec!["auth"], "record 0 should have LLM tags");
    assert_eq!(records[0].summary, "add oauth");
    assert_eq!(records[1].tags, vec!["login"]);
    assert_eq!(records[2].tags, vec!["billing"]);
    // last 2 are rules-filled — empty tags, summary = user message
    assert!(records[3].tags.is_empty(), "record 3 should be rules-filled (no tags)");
    assert!(records[4].tags.is_empty(), "record 4 should be rules-filled (no tags)");
}

#[test]
fn llm_error_falls_back_to_rules_for_chunk() {
    // LLM call errors (e.g. timeout) -> all records rules-filled, no panic.
    let cat = pay4what::categorize::LlmCategorizer::new(
        "deepseek/deepseek-v4-flash",
        Box::new(ErrMockCaller),
    );
    let segs = seg_of(&[
        r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","gitBranch":"main","message":{"role":"user","content":"do something"}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:01:00Z","gitBranch":"main","message":{"role":"user","content":"do another thing"}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:02:00Z","gitBranch":"main","message":{"role":"user","content":"and a third"}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:03:00Z","gitBranch":"main","message":{"role":"user","content":"fourth task"}}"#,
        r#"{"type":"user","timestamp":"2026-07-07T10:04:00Z","gitBranch":"main","message":{"role":"user","content":"fifth task"}}"#,
    ]);
    let records = pay4what::categorize::Categorizer::categorize_rich(&cat, &segs);
    assert_eq!(records.len(), 5, "error should produce 5 rules-filled records");
    assert!(records[0].tags.is_empty(), "error fallback should have no tags");
    assert!(records[4].tags.is_empty());
}
