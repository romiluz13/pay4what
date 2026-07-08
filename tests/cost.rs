//! Task 3 tests — cache-aware cost computation + bundled pricing table.
//!
//! MIRROR: ~/Dev/ai-cost-compare/src/cost/tracker.ts:3-9 (costForUsage formula),
//! ~/Dev/sql-hidden-cost/scripts/agent-usage.mjs:54-66 (cache separation).
//!
//! CRITICAL: input_tokens is FRESH (does not include cache). Cost prices each
//! bucket at its OWN rate:
//!   cost = input*in_rate + cache_read*cache_read_rate
//!          + cache_creation*cache_creation_rate + output*out_rate
//! NEVER double-count (do not subtract cache from input — Claude reports them
//! separate, per agent-usage.mjs:54-66).
use pay4what::cost::{PricingTable, cost_for_session, cost_for_usage};
use pay4what::parse::parse_session;
use std::fs;
use tempfile::TempDir;

/// A pricing table with cache rates. ccusage's table LACKS cache rates;
/// pay4what adds cacheReadPerMTok + cacheCreationPerMTok.
fn sample_pricing() -> PricingTable {
    PricingTable {
        as_of: "2026-07-07".into(),
        models: vec![(
            "claude-sonnet-4-6".into(),
            pay4what::cost::ModelPricing {
                input_per_mtok: 3.0,
                output_per_mtok: 15.0,
                cache_read_per_mtok: 0.30,
                cache_creation_per_mtok: 3.75,
            },
        )]
        .into_iter()
        .collect(),
    }
}

#[test]
fn cost_prices_each_bucket_at_its_own_rate() {
    let usage = pay4what::parse::TurnUsage {
        model: "claude-sonnet-4-6".into(),
        input_tokens: 1_000_000,
        output_tokens: 1_000_000,
        cache_read_input_tokens: 1_000_000,
        cache_creation_input_tokens: 1_000_000,
        ..Default::default()
    };
    let pricing = sample_pricing();
    // 1M of each bucket at the sample rates:
    //   input 1M * 3.0   = $3.00
    //   output 1M * 15.0 = $15.00
    //   cache_read 1M * 0.30 = $0.30
    //   cache_creation 1M * 3.75 = $3.75
    //   total = $22.05
    let cost = cost_for_usage(&usage, &pricing);
    assert!((cost - 22.05).abs() < 1e-6, "expected $22.05, got ${cost}");
}

#[test]
fn cost_does_not_double_count_cache() {
    // input_tokens is FRESH — it does NOT include cache. So cost must NOT
    // subtract cache from input. The four buckets are additive, each at its rate.
    let usage = pay4what::parse::TurnUsage {
        model: "claude-sonnet-4-6".into(),
        input_tokens: 100,
        output_tokens: 50,
        cache_read_input_tokens: 10_000,
        cache_creation_input_tokens: 0,
        ..Default::default()
    };
    let pricing = sample_pricing();
    let cost = cost_for_usage(&usage, &pricing);
    // 100/1M * 3.0 + 50/1M * 15.0 + 10000/1M * 0.30 + 0
    // = 0.0003 + 0.00075 + 0.003 + 0 = 0.00405
    assert!(
        (cost - 0.00405).abs() < 1e-9,
        "expected $0.00405, got ${cost}"
    );
}

#[test]
fn cost_for_session_sums_all_turns_with_usage() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("s.jsonl");
    fs::write(
        &p,
        [
            r#"{"type":"user","timestamp":"2026-07-07T10:00:00Z","cwd":"/x","gitBranch":"main","message":{"role":"user","content":"hi"}}"#,
            r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":2000,"cache_creation_input_tokens":100}}}"#,
            r#"{"type":"assistant","timestamp":"2026-07-07T10:00:10Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":2000,"output_tokens":1000,"cache_read_input_tokens":5000,"cache_creation_input_tokens":0}}}"#,
        ]
        .join("\n"),
    )
    .unwrap();
    let session = parse_session(&p).unwrap();
    let pricing = sample_pricing();
    let cost = cost_for_session(&session, &pricing);
    // turn1: 1000/1M*3 + 500/1M*15 + 2000/1M*0.30 + 100/1M*3.75
    //      = 0.003 + 0.0075 + 0.0006 + 0.000375 = 0.011475
    // turn2: 2000/1M*3 + 1000/1M*15 + 5000/1M*0.30 + 0
    //      = 0.006 + 0.015 + 0.0015 + 0 = 0.0225
    // total = 0.033975
    assert!(
        (cost - 0.033975).abs() < 1e-9,
        "expected $0.033975, got ${cost}"
    );
}

#[test]
fn unknown_model_falls_back_gracefully() {
    let usage = pay4what::parse::TurnUsage {
        model: "claude-unknown-99".into(),
        input_tokens: 1_000_000,
        output_tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
        ..Default::default()
    };
    let pricing = sample_pricing();
    // unknown model -> cost 0 (tolerant; do NOT panic). Real impl will log a warning.
    let cost = cost_for_usage(&usage, &pricing);
    assert_eq!(cost, 0.0, "unknown model should yield $0, not panic");
}

/// 1h cache-creation is priced at input*2.0, NOT the flat cache_create rate.
/// MIRROR: ccusage cost.rs:5 CACHE_CREATE_1H_INPUT_MULTIPLIER = 2.0.
/// When the 5m/1h split is present, 5m uses cache_creation rate, 1h uses input*2.0.
/// Chunked-turn dedup: Claude Code emits one logical turn as multiple JSONL
/// lines (thinking/text/tool_use) sharing identical cumulative usage.
/// cost_for_session must count each logical turn ONCE, not sum every chunk.
#[test]
fn cost_for_session_dedups_chunked_turns() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("s.jsonl");
    fs::write(
        &p,
        [
            r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"thinking"}],"usage":{"input_tokens":2,"output_tokens":490,"cache_read_input_tokens":17926,"cache_creation_input_tokens":4912}}}"#,
            r#"{"type":"assistant","timestamp":"2026-07-07T10:00:05Z","message":{"role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"text"}],"usage":{"input_tokens":2,"output_tokens":490,"cache_read_input_tokens":17926,"cache_creation_input_tokens":4912}}}"#,
            r#"{"type":"assistant","timestamp":"2026-07-07T10:00:08Z","message":{"role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"tool_use","name":"Read"}],"usage":{"input_tokens":2,"output_tokens":490,"cache_read_input_tokens":17926,"cache_creation_input_tokens":4912}}}"#,
            r#"{"type":"assistant","timestamp":"2026-07-07T10:00:20Z","message":{"role":"assistant","model":"claude-sonnet-4-6","usage":{"input_tokens":10,"output_tokens":100,"cache_read_input_tokens":20000,"cache_creation_input_tokens":0}}}"#,
        ]
        .join("\n"),
    )
    .unwrap();
    let session = parse_session(&p).unwrap();
    let pricing = sample_pricing();
    let cost = cost_for_session(&session, &pricing);
    let u1 = pay4what::parse::TurnUsage {
        model: "claude-sonnet-4-6".into(),
        input_tokens: 2,
        output_tokens: 490,
        cache_read_input_tokens: 17926,
        cache_creation_input_tokens: 4912,
        ..Default::default()
    };
    let u2 = pay4what::parse::TurnUsage {
        model: "claude-sonnet-4-6".into(),
        input_tokens: 10,
        output_tokens: 100,
        cache_read_input_tokens: 20000,
        cache_creation_input_tokens: 0,
        ..Default::default()
    };
    let expected = cost_for_usage(&u1, &pricing) + cost_for_usage(&u2, &pricing);
    assert!(
        (cost - expected).abs() < 1e-9,
        "dedup: expected ${expected}, got ${cost}"
    );
}

#[test]
fn cache_creation_1h_priced_at_input_times_2() {
    let usage = pay4what::parse::TurnUsage {
        model: "claude-sonnet-4-6".into(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 1_000_000, // flat total (ignored when split present)
        cache_creation_5m: Some(0),
        cache_creation_1h: Some(1_000_000), // all 1h
    };
    let pricing = sample_pricing();
    // 1M 1h-cache at input*2.0 = 1M/1M * 3.0 * 2.0 = $6.00
    // (NOT 1M * 3.75 = $3.75 which would be the flat cache_create rate)
    let cost = cost_for_usage(&usage, &pricing);
    assert!(
        (cost - 6.0).abs() < 1e-6,
        "1h cache at input*2.0 = $6.00, got ${cost}"
    );
}

/// When no 5m/1h split, all cache-creation uses the flat cache_create rate.
#[test]
fn cache_creation_flat_when_no_split() {
    let usage = pay4what::parse::TurnUsage {
        model: "claude-sonnet-4-6".into(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 1_000_000,
        cache_creation_5m: None,
        cache_creation_1h: None,
    };
    let pricing = sample_pricing();
    // 1M at flat cache_create rate 3.75 = $3.75
    let cost = cost_for_usage(&usage, &pricing);
    assert!(
        (cost - 3.75).abs() < 1e-6,
        "flat cache_create = $3.75, got ${cost}"
    );
}
