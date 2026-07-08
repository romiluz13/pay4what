//! PROOF: dump raw turn-level usage from one real AST-Bench session and
//! hand-verify pay4what's cost math against the pricing table. No README
//! claims — only what's in the actual JSONL + the actual code.
use pay4what::cost::{bundled_pricing, cost_for_usage};
use pay4what::parse::parse_session;

fn main() {
    let p = "/Users/rom.iluz/.claude/projects/-Users-rom-iluz-Dev-sql-hidden-cost-benchmark-runs-ast-bench-v1-data-access-audit-export-claude-code-repeat-1-mongo-workspace/2a96f358-05d3-4f7a-a3dc-b33478d68b60.jsonl";
    let session = parse_session(std::path::Path::new(p)).unwrap();
    let pricing = bundled_pricing();

    println!("PRICING TABLE (the thing that MUST be verified vs anthropic.com/pricing):");
    for (m, mp) in &pricing.models {
        println!(
            "  {m}: input=${}/MTok output=${}/MTok cache_read=${}/MTok cache_creation=${}/MTok",
            mp.input_per_mtok,
            mp.output_per_mtok,
            mp.cache_read_per_mtok,
            mp.cache_creation_per_mtok
        );
    }
    println!("  as_of: {}", pricing.as_of);
    println!(
        "\n⚠️  These rates are UNVERIFIED list prices. Every dollar below is only as good as these numbers.\n"
    );

    println!("RAW TURNS (first 8 assistant turns with usage) from the actual JSONL:");
    let mut shown = 0;
    for t in &session.turns {
        if shown >= 8 {
            break;
        }
        if let Some(u) = &t.usage {
            let cost = cost_for_usage(u, &pricing);
            shown += 1;
            println!(
                "  turn: model={} in={} out={} cache_read={} cache_creation={} (5m={:?} 1h={:?}) -> ${:.6}",
                u.model,
                u.input_tokens,
                u.output_tokens,
                u.cache_read_input_tokens,
                u.cache_creation_input_tokens,
                u.cache_creation_5m,
                u.cache_creation_1h,
                cost
            );
        }
    }

    // hand-verify turn 1's math explicitly
    println!("\nHAND-VERIFY turn 1 math (showing the formula):");
    if let Some(u) = session.turns.iter().filter_map(|t| t.usage.as_ref()).next() {
        let mp = pricing.models.get(&u.model).unwrap();
        let per = 1_000_000.0;
        let cc_5m = u.cache_creation_5m.unwrap_or(0) as f64;
        let cc_1h = u.cache_creation_1h.unwrap_or(0) as f64;
        let cc_flat = u.cache_creation_input_tokens as f64;
        let (cc5_used, cc1_used) = if u.cache_creation_5m.is_some() {
            (cc_5m, cc_1h)
        } else {
            (cc_flat, 0.0)
        };
        println!(
            "  input:       {} / 1M * ${} = ${:.6}",
            u.input_tokens,
            mp.input_per_mtok,
            u.input_tokens as f64 / per * mp.input_per_mtok
        );
        println!(
            "  output:      {} / 1M * ${} = ${:.6}",
            u.output_tokens,
            mp.output_per_mtok,
            u.output_tokens as f64 / per * mp.output_per_mtok
        );
        println!(
            "  cache_read:  {} / 1M * ${} = ${:.6}",
            u.cache_read_input_tokens,
            mp.cache_read_per_mtok,
            u.cache_read_input_tokens as f64 / per * mp.cache_read_per_mtok
        );
        println!(
            "  cache_5m:    {} / 1M * ${} = ${:.6}",
            cc5_used,
            mp.cache_creation_per_mtok,
            cc5_used / per * mp.cache_creation_per_mtok
        );
        println!(
            "  cache_1h:    {} / 1M * ${} (input*2.0) = ${:.6}",
            cc1_used,
            mp.input_per_mtok * 2.0,
            cc1_used / per * mp.input_per_mtok * 2.0
        );
        let total: f64 = u.input_tokens as f64 / per * mp.input_per_mtok
            + u.output_tokens as f64 / per * mp.output_per_mtok
            + u.cache_read_input_tokens as f64 / per * mp.cache_read_per_mtok
            + cc5_used / per * mp.cache_creation_per_mtok
            + cc1_used / per * mp.input_per_mtok * 2.0;
        println!(
            "  SUM = ${:.6}  (pay4what said ${:.6})",
            total,
            cost_for_usage(u, &pricing)
        );
    }

    // full session total via the library
    let full = pay4what::cost::cost_for_session(&session, &pricing);
    println!(
        "\nFULL SESSION (parent, all turns): ${:.4}  (sum of {} usage-bearing turns)",
        full,
        session.turns.iter().filter(|t| t.usage.is_some()).count()
    );

    // subagents
    let subs = pay4what::discover::discover_subagents(std::path::Path::new(p));
    let mut sub_total = 0.0;
    for s in &subs {
        if let Ok(ss) = parse_session(s) {
            sub_total += pay4what::cost::cost_for_session(&ss, &pricing);
        }
    }
    println!("SUBAGENTS: {} files, ${:.4}", subs.len(), sub_total);
    println!("COMPLETE:  ${:.4}", full + sub_total);
    println!("\nThis number is REAL COMPUTATION on the actual JSONL. But its accuracy hinges on");
    println!("the pricing table above being correct — and those rates are NOT yet web-verified.");
}
