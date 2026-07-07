//! Task 7 tests — the viral render (the artifact that gets screenshotted).
//!
//! MIRROR: the viral screenshot in /tmp/handoff-pay4what.md — the exact table
//! shape: emoji + activity + description + cost + tokens + PR column ("—" v1.0)
//! + TOTAL row + footer "💸 1 feature = N% of the week's spend."
use pay4what::categorize::{Activity, LabeledSegment};
use pay4what::render::{ActivityRow, render_activity_table};
use std::collections::BTreeSet;

fn labeled(activity: Activity, msg: &str, cost: f64, tokens: u64) -> LabeledSegment {
    LabeledSegment {
        index: 1,
        activity,
        user_message: msg.to_string(),
        cost,
        tokens,
        git_branch: None,
        touched_files: BTreeSet::new(),
    }
}

#[test]
fn aggregates_by_activity_summing_cost_and_tokens() {
    let segments = vec![
        labeled(
            Activity::Feature,
            "OAuth refresh-token rotation",
            30.0,
            2_000_000,
        ),
        labeled(Activity::Feature, "add login flow", 17.20, 1_100_000),
        labeled(Activity::Bugfix, "login redirect loop", 3.40, 220_000),
        labeled(
            Activity::Migration,
            "Prisma 5 to 6 schema bump",
            12.80,
            880_000,
        ),
        labeled(Activity::Refactor, "extract billing service", 6.10, 410_000),
        labeled(Activity::Unattributed, "small / interrupted", 2.60, 175_000),
    ];
    let rows = aggregate(&segments);
    // feature aggregated: 30 + 17.20 = 47.20, tokens 3.1M
    let feature = rows
        .iter()
        .find(|r| r.activity == Activity::Feature)
        .unwrap();
    assert!((feature.cost - 47.20).abs() < 1e-6);
    assert_eq!(feature.tokens, 3_100_000);
    assert_eq!(feature.count, 2);
    // total
    let total: f64 = rows.iter().map(|r| r.cost).sum();
    assert!((total - 72.10).abs() < 1e-6);
}

#[test]
fn footer_shows_surprise_ratio() {
    let segments = vec![
        labeled(
            Activity::Feature,
            "OAuth refresh-token rotation",
            47.20,
            3_100_000,
        ),
        labeled(Activity::Bugfix, "login redirect loop", 3.40, 220_000),
        labeled(
            Activity::Migration,
            "Prisma 5 to 6 schema bump",
            12.80,
            880_000,
        ),
        labeled(Activity::Refactor, "extract billing service", 6.10, 410_000),
        labeled(Activity::Unattributed, "small", 2.60, 175_000),
    ];
    let out = render_activity_table(&segments);
    // total = 72.10 ; feature = 47.20 -> 65%
    assert!(
        out.contains("1 feature = 65% of the spend"),
        "expected surprise ratio footer, got:\n{out}"
    );
    assert!(out.contains("TOTAL"), "table has a TOTAL row");
    assert!(out.contains("🚀"), "emoji rendered");
    assert!(out.contains("OAuth"), "description rendered");
}

#[test]
fn empty_segments_render_empty_state() {
    let out = render_activity_table(&[]);
    assert!(
        out.contains("No Claude Code sessions found") || out.contains("no spend"),
        "empty state"
    );
}

// helper: aggregate into rows (mirrors render's internal aggregation)
fn aggregate(segments: &[LabeledSegment]) -> Vec<ActivityRow> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<Activity, ActivityRow> = BTreeMap::new();
    for s in segments {
        let row = map.entry(s.activity).or_insert_with(|| ActivityRow {
            activity: s.activity,
            cost: 0.0,
            tokens: 0,
            count: 0,
            description: String::new(),
        });
        row.cost += s.cost;
        row.tokens += s.tokens;
        row.count += 1;
        if row.description.is_empty() {
            row.description = s.user_message.clone();
        }
    }
    map.into_values().collect()
}
