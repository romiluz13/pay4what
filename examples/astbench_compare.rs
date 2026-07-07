//! AST-Bench cost comparison: same task, same agent, MongoDB vs Postgres.
//! The cleanest apples-to-apples database cost comparison in Rom's data.
use pay4what::cost::{bundled_pricing, cost_for_session};
use pay4what::discover::discover_subagents;
use pay4what::parse::parse_session;
use std::path::PathBuf;

fn complete_cost(
    session_path: &std::path::Path,
    pricing: &pay4what::cost::PricingTable,
) -> (f64, f64, usize, u64, u64) {
    let Ok(session) = parse_session(session_path) else {
        return (0.0, 0.0, 0, 0, 0);
    };
    let parent_cost = cost_for_session(&session, pricing);
    let parent_turns = session.turns.len() as u64;
    let subs = discover_subagents(session_path);
    let mut sub_cost = 0.0;
    let mut sub_turns = 0u64;
    for sub in &subs {
        if let Ok(sub_session) = parse_session(sub) {
            sub_turns += sub_session.turns.len() as u64;
            sub_cost += cost_for_session(&sub_session, pricing);
        }
    }
    (parent_cost, sub_cost, subs.len(), parent_turns, sub_turns)
}

fn first_user_msg(path: &std::path::Path) -> String {
    let Ok(s) = parse_session(path) else {
        return "(unparseable)".into();
    };
    for t in &s.turns {
        if t.kind.as_deref() == Some("user")
            && let Some(text) = &t.text
            && !text.is_empty()
            && !text.starts_with('<')
        {
            return text.replace('\n', " ").chars().take(300).collect();
        }
    }
    "(none)".into()
}

fn main() {
    let pricing = bundled_pricing();
    let base = "/Users/rom.iluz/.claude/projects";
    let scenarios = ["data-access-audit", "invoice-dispute-workflow"];
    let repeats = [1usize, 2, 3];

    println!("AST-BENCH: SAME TASK × SAME AGENT × MongoDB vs Postgres\n");
    println!(
        "Task: Claude Code builds the 'strategic-account-rescue' workflow against a live DB.\n"
    );

    let mut mongo_total = 0.0;
    let mut pg_total = 0.0;
    let mut n = 0usize;

    for scenario in &scenarios {
        println!("═══ Scenario: {scenario} ═══");
        for rep in &repeats {
            let mongo_dir = format!(
                "{base}/-Users-rom-iluz-Dev-sql-hidden-cost-benchmark-runs-ast-bench-v1-{scenario}-export-claude-code-repeat-{rep}-mongo-workspace"
            );
            let pg_dir = format!(
                "{base}/-Users-rom-iluz-Dev-sql-hidden-cost-benchmark-runs-ast-bench-v1-{scenario}-export-claude-code-repeat-{rep}-postgres-workspace"
            );
            let m = find_session(&mongo_dir);
            let p = find_session(&pg_dir);
            if let (Some(mp), Some(pp)) = (m, p) {
                let (mc, msc, mns, mt, mst) = complete_cost(&mp, &pricing);
                let (pc, psc, pns, pt, pst) = complete_cost(&pp, &pricing);
                let mtot = mc + msc;
                let ptot = pc + psc;
                mongo_total += mtot;
                pg_total += ptot;
                n += 1;
                println!("  repeat {rep}:");
                println!(
                    "    MongoDB:  ${mtot:>7.2}  (parent ${mc:.2} + sub ${msc:.2}, {mns} sub files, {} turns)",
                    mt + mst
                );
                println!(
                    "    Postgres: ${ptot:>7.2}  (parent ${pc:.2} + sub ${psc:.2}, {pns} sub files, {} turns)",
                    pt + pst
                );
                let delta = ptot - mtot;
                let pct = if mtot > 0.0 {
                    (delta / mtot * 100.0).round() as i64
                } else {
                    0
                };
                let arrow = if delta > 0.0 {
                    "Postgres +$"
                } else {
                    "Postgres -$"
                };
                println!(
                    "    delta:    {arrow}{:.2}  ({pct}% vs MongoDB)",
                    delta.abs()
                );
            }
        }
    }

    println!("\n═══ AGGREGATE (n={n} matched pairs) ═══");
    println!("  MongoDB total:  ${:.2}", mongo_total);
    println!("  Postgres total: ${:.2}", pg_total);
    if mongo_total > 0.0 {
        let avg_m = mongo_total / n as f64;
        let avg_p = pg_total / n as f64;
        println!("  MongoDB avg/pair:  ${:.2}", avg_m);
        println!("  Postgres avg/pair: ${:.2}", avg_p);
        let diff = avg_p - avg_m;
        let pct = (diff / avg_m * 100.0).round() as i64;
        let word = if diff > 0.0 { "more" } else { "less" };
        println!("  Postgres is {pct}% {word} expensive per task than MongoDB");
    }

    // Deep-dive one pair
    println!("\n═══ DEEP-DIVE: data-access-audit repeat 1 ═══");
    let m = find_session(&format!("{base}/-Users-rom-iluz-Dev-sql-hidden-cost-benchmark-runs-ast-bench-v1-data-access-audit-export-claude-code-repeat-1-mongo-workspace")).unwrap();
    let p = find_session(&format!("{base}/-Users-rom-iluz-Dev-sql-hidden-cost-benchmark-runs-ast-bench-v1-data-access-audit-export-claude-code-repeat-1-postgres-workspace")).unwrap();
    println!("\nMongoDB session task (first user msg):");
    println!("  \"{}\"", first_user_msg(&m));
    println!("\nPostgres session task (first user msg):");
    println!("  \"{}\"", first_user_msg(&p));

    // model mix
    for (label, path) in [("MongoDB", &m), ("Postgres", &p)] {
        let Ok(s) = parse_session(path) else { continue };
        let mut inp = 0u64;
        let mut out = 0u64;
        let mut cr = 0u64;
        let mut cc = 0u64;
        for t in &s.turns {
            if let Some(u) = &t.usage {
                inp += u.input_tokens;
                out += u.output_tokens;
                cr += u.cache_read_input_tokens;
                cc += u.cache_creation_input_tokens;
            }
        }
        println!("\n{label} tokens: input={inp} output={out} cache_read={cr} cache_creation={cc}");
    }
}

fn find_session(dir: &str) -> Option<PathBuf> {
    let d = std::path::Path::new(dir);
    if !d.is_dir() {
        return None;
    }
    for entry in std::fs::read_dir(d).ok()?.flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "jsonl") && p.is_file() {
            return Some(p);
        }
    }
    None
}
