//! Render — the viral artifact (the table that gets screenshotted).
//!
//! MIRROR: the viral screenshot in /tmp/handoff-pay4what.md. v1.0 PR column is
//! "—" (commit/issue attribution deferred to v1.1).
//!
//! Two outputs (v1.0 scope = activity + file):
//!   - cost-by-activity (the headline table + surprise-ratio footer)
//!   - cost-by-file/area (the second table)
use crate::categorize::{Activity, LabeledSegment};
use std::collections::BTreeMap;

/// One aggregated row of the cost-by-activity table.
#[derive(Debug, Clone)]
pub struct ActivityRow {
    pub activity: Activity,
    pub cost: f64,
    pub tokens: u64,
    pub count: usize,
    pub description: String,
}

fn aggregate_by_activity(segments: &[LabeledSegment]) -> Vec<ActivityRow> {
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
        // keep the first non-empty description; if multiple segments share an
        // activity, note the count (e.g. "OAuth rotation (+1 more)")
        if row.description.is_empty() {
            // fallback for segments with no captured user message: derive a
            // description from touched files / tool verbs so the row isn't blank
            row.description = if !s.user_message.is_empty() {
                s.user_message.clone()
            } else if let Some(f) = s.touched_files.iter().next() {
                truncate(f, 36)
            } else {
                format!(
                    "({} segment{})",
                    s.activity.label(),
                    if s.index > 1 { "s" } else { "" }
                )
            };
        } else if row.count == 2 {
            row.description = format!("{} (+1 more)", truncate(&row.description, 40));
        }
    }
    // sort by cost desc (the surprise: the biggest activity at the top)
    let mut rows: Vec<ActivityRow> = map.into_values().collect();
    rows.sort_by(|a, b| {
        b.cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}

fn fmt_cost(n: f64) -> String {
    format!("${:.2}", n)
}

fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}…")
    }
}

/// Render the cost-by-activity table + the surprise-ratio footer.
/// MIRROR: handoff viral screenshot shape.
pub fn render_activity_table(segments: &[LabeledSegment]) -> String {
    if segments.is_empty() {
        return "No Claude Code sessions found in range.\n".to_string();
    }
    let rows = aggregate_by_activity(segments);
    let total_cost: f64 = rows.iter().map(|r| r.cost).sum();
    let total_tokens: u64 = rows.iter().map(|r| r.tokens).sum();

    if total_cost <= 0.0 {
        return "No spend recorded in range (sessions may lack usage data).\n".to_string();
    }

    let mut out = String::new();
    out.push_str("\n  cost by activity\n");
    out.push_str("  ┌──────────────────────────────────────────────┬──────────┬────────┐\n");
    out.push_str("  │ Activity                                      │ Cost     │ Tokens │\n");
    out.push_str("  ├──────────────────────────────────────────────┼──────────┼────────┤\n");
    for r in &rows {
        let label = format!("{} {}", r.activity.emoji(), r.activity.label());
        let desc = truncate(&r.description, 36);
        let activity_col = format!("{label:<11} {desc:<33}");
        out.push_str(&format!(
            "  │ {:<44} │ {:>8} │ {:>6} │\n",
            activity_col,
            fmt_cost(r.cost),
            fmt_tokens(r.tokens),
        ));
    }
    out.push_str("  ├──────────────────────────────────────────────┼──────────┼────────┤\n");
    out.push_str(&format!(
        "  │ {:<44} │ {:>8} │ {:>6} │\n",
        "TOTAL",
        fmt_cost(total_cost),
        fmt_tokens(total_tokens),
    ));
    out.push_str("  └──────────────────────────────────────────────┴──────────┴────────┘\n");

    // The surprise ratio footer — the line that gets screenshotted.
    // Skip 'unattributed' as the headline (show the top REAL activity).
    let headline = rows.iter().find(|r| r.activity != Activity::Unattributed);
    if let Some(top) = headline {
        let pct = (top.cost / total_cost * 100.0).round() as u64;
        if pct >= 20 {
            out.push_str(&format!(
                "\n  💸 1 {} = {}% of the spend.\n",
                top.activity.label(),
                pct,
            ));
        }
    }
    out
}

/// Render the cost-by-file/area table (the second v1.0 view).
pub fn render_file_table(segments: &[LabeledSegment]) -> String {
    // Aggregate by lowercased path (case-insensitive: /Dev/SDR-AI and
    // /dev/sdr-ai are the same repo). Keep the first-seen original casing for display.
    let mut by_file: BTreeMap<String, (f64, u64, String)> = BTreeMap::new();
    for s in segments {
        for f in &s.touched_files {
            let key = f.to_lowercase();
            let e = by_file.entry(key).or_insert((0.0, 0, f.clone()));
            e.0 += s.cost / s.touched_files.len().max(1) as f64;
            e.1 += s.tokens;
            // keep the first-seen display casing (don't overwrite)
        }
    }
    if by_file.is_empty() {
        return "No files touched in range.\n".to_string();
    }
    let mut entries: Vec<(String, f64, u64)> = by_file
        .into_iter()
        .map(|(_, (c, t, disp))| (disp, c, t))
        .collect();
    entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = String::new();
    out.push_str("\n  cost by file\n");
    out.push_str("  ┌──────────────────────────────────────────────┬──────────┐\n");
    out.push_str("  │ File                                          │ Cost     │\n");
    out.push_str("  ├──────────────────────────────────────────────┼──────────┤\n");
    for (f, c, _t) in entries.iter().take(15) {
        out.push_str(&format!(
            "  │ {:<44} │ {:>8} │\n",
            truncate(f, 44),
            fmt_cost(*c),
        ));
    }
    out.push_str("  └──────────────────────────────────────────────┴──────────┘\n");
    out
}
