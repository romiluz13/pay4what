//! Analysis: find a Supabase session + MongoDB sessions, compute complete
//! costs (parent + subagents), report the task, and compare.
//!
//! Uses pay4what's own library — a real dogfood of the cost pipeline.
use pay4what::cost::{bundled_pricing, cost_for_session};
use pay4what::discover::{discover_all, discover_subagents};
use pay4what::parse::parse_session;
use std::collections::HashMap;

fn count_mentions(path: &std::path::Path, terms: &[&str]) -> usize {
    // cheap line-level grep (don't load huge files fully)
    let Ok(text) = std::fs::read_to_string(path) else {
        return 0;
    };
    let l = text.to_lowercase();
    terms.iter().map(|t| l.matches(t).count()).sum()
}

fn first_real_user_msg(path: &std::path::Path) -> String {
    let Ok(session) = parse_session(path) else {
        return "(unparseable)".into();
    };
    for t in &session.turns {
        if t.kind.as_deref() == Some("user")
            && let Some(text) = &t.text
            && !text.is_empty()
        {
            // take first ~280 chars of the first real user request
            let clean = text.replace('\n', " ");
            return clean.chars().take(280).collect();
        }
    }
    "(no user message captured)".into()
}

fn complete_cost(
    session_path: &std::path::Path,
    pricing: &pay4what::cost::PricingTable,
) -> (f64, f64, usize, u64) {
    let Ok(session) = parse_session(session_path) else {
        return (0.0, 0.0, 0, 0);
    };
    let parent_cost = cost_for_session(&session, pricing);
    let mut sub_cost = 0.0;
    let mut sub_turns = 0u64;
    for sub in discover_subagents(session_path) {
        if let Ok(sub_session) = parse_session(&sub) {
            sub_turns += sub_session.turns.len() as u64;
            sub_cost += cost_for_session(&sub_session, pricing);
        }
    }
    let parent_turns = session.turns.len() as u64;
    (
        parent_cost,
        sub_cost,
        discover_subagents(session_path).len(),
        parent_turns + sub_turns,
    )
}

fn main() {
    let sessions = discover_all();
    let pricing = bundled_pricing();
    println!(
        "Scanning {} sessions for database context...\n",
        sessions.len()
    );

    // Score each session by Supabase and MongoDB mention counts.
    let mut supabase_scores: Vec<(usize, std::path::PathBuf)> = Vec::new();
    let mut mongo_scores: Vec<(usize, std::path::PathBuf)> = Vec::new();

    for p in &sessions {
        let supa = count_mentions(p, &["supabase"]);
        let mongo = count_mentions(p, &["mongodb", "mongo db", ".mongo", "atlas", "memongo"]);
        if supa > 0 {
            supabase_scores.push((supa, p.clone()));
        }
        if mongo > 0 {
            mongo_scores.push((mongo, p.clone()));
        }
    }
    supabase_scores.sort_by(|a, b| b.0.cmp(&a.0));
    mongo_scores.sort_by(|a, b| b.0.cmp(&a.0));

    println!("== Top Supabase-mentioning sessions ==");
    for (n, p) in supabase_scores.iter().take(5) {
        println!(
            "  {n:5} mentions  {}",
            p.file_name().unwrap().to_string_lossy()
        );
    }
    println!("\n== Top MongoDB-mentioning sessions ==");
    for (n, p) in mongo_scores.iter().take(5) {
        println!(
            "  {n:5} mentions  {}",
            p.file_name().unwrap().to_string_lossy()
        );
    }

    // Pick the top Supabase session for the deep-dive.
    let supa_session = &supabase_scores[0].1;
    println!("\n\n========== SUPABASE SESSION DEEP-DIVE ==========");
    println!("file: {}", supa_session.display());
    println!("cwd:  {:?}", parse_session(supa_session).unwrap().cwd);
    println!(
        "gitBranch: {:?}",
        parse_session(supa_session).unwrap().git_branch
    );
    println!(
        "first_ts: {:?}",
        parse_session(supa_session).unwrap().first_ts
    );
    println!(
        "last_ts:  {:?}",
        parse_session(supa_session).unwrap().last_ts
    );
    println!("\nTASK (first user message):");
    println!("  \"{}\"", first_real_user_msg(supa_session));
    let (pcost, scost, nsubs, turns) = complete_cost(supa_session, &pricing);
    println!("\nCOST BREAKDOWN:");
    println!("  parent session:   ${:.2}  ({} turns)", pcost, turns);
    println!("  subagent files:   {} (cost ${:.2})", nsubs, scost);
    println!("  COMPLETE COST:    ${:.2}", pcost + scost);

    // Compare: aggregate ALL Supabase-mentioning sessions vs ALL MongoDB-mentioning.
    let mut supa_total = 0.0;
    let mut supa_sessions = 0usize;
    let mut mongo_total = 0.0;
    let mut mongo_sessions = 0usize;
    for (_, p) in &supabase_scores {
        let (pc, sc, _, _) = complete_cost(p, &pricing);
        supa_total += pc + sc;
        supa_sessions += 1;
    }
    for (_, p) in &mongo_scores {
        let (pc, sc, _, _) = complete_cost(p, &pricing);
        mongo_total += pc + sc;
        mongo_sessions += 1;
    }

    println!("\n\n========== DATABASE COMPARISON ==========");
    println!(
        "  {:<28} {:>10} {:>12} {:>14}",
        "Database", "sessions", "total cost", "avg/session"
    );
    println!("  {}", "-".repeat(68));
    println!(
        "  {:<28} {:>10} {:>12} {:>14}",
        "Supabase (mentions)",
        supa_sessions,
        format!("${:.2}", supa_total),
        format!(
            "${:.2}",
            if supa_sessions > 0 {
                supa_total / supa_sessions as f64
            } else {
                0.0
            }
        ),
    );
    println!(
        "  {:<28} {:>10} {:>12} {:>14}",
        "MongoDB (mentions)",
        mongo_sessions,
        format!("${:.2}", mongo_total),
        format!(
            "${:.2}",
            if mongo_sessions > 0 {
                mongo_total / mongo_sessions as f64
            } else {
                0.0
            }
        ),
    );

    // Model mix for the top Supabase session.
    let mut models: HashMap<String, u64> = HashMap::new();
    if let Ok(s) = parse_session(supa_session) {
        for t in &s.turns {
            if let Some(u) = &t.usage {
                *models.entry(u.model.clone()).or_default() += 1;
            }
        }
    }
    println!("\nModel mix (Supabase session): {:?}", models);
}
