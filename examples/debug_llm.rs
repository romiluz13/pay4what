//! Debug: call the live OpenRouter categorizer on a few real segments and
//! print the RAW response so we can see why it's falling back to rules.
use pay4what::categorize::{Categorizer, LlmCategorizer, OpenRouterCaller, RulesCategorizer};
use pay4what::cost::bundled_pricing;
use pay4what::discover::discover_all;
use pay4what::parse::parse_session;
use pay4what::segment::segment_session_with_pricing;

fn main() {
    let key = std::env::var("GROVE_API_KEY")
        .or(std::env::var("OPENROUTER_API_KEY"))
        .expect("set GROVE_API_KEY or OPENROUTER_API_KEY");
    let model = std::env::args()
        .nth(1)
        .unwrap_or("DeepSeek-V4-Flash".into());
    let sessions = discover_all();
    // pick the first session in the last 7d that has segments
    let pricing = bundled_pricing();
    let mut all_segs = Vec::new();
    for p in &sessions {
        if let Ok(s) = parse_session(p) {
            let segs = segment_session_with_pricing(&s, &pricing);
            if !segs.is_empty() {
                all_segs.extend(segs);
                if all_segs.len() >= 6 {
                    break;
                }
            }
        }
    }
    println!("testing {} segments with model {}", all_segs.len(), model);
    for (i, s) in all_segs.iter().take(6).enumerate() {
        println!("\n--- segment {i} ---");
        println!(
            "  user: {:?}",
            s.user_message.chars().take(100).collect::<String>()
        );
        println!(
            "  branch: {:?}, tools: {:?}, files: {}",
            s.git_branch,
            s.tool_verbs,
            s.touched_files.len()
        );
    }

    // build the prompt the same way the categorizer does
    let cat = LlmCategorizer::new(
        &model,
        Box::new(OpenRouterCaller {
            api_key: key.clone(),
            model: model.clone(),
        }),
    );
    // call the caller directly to see the raw response
    let prompt = cat.build_prompt(&all_segs[..6.min(all_segs.len())]);
    println!(
        "\n\n=== PROMPT (first 800 chars) ===\n{}",
        prompt.chars().take(800).collect::<String>()
    );
    println!("\n=== CALLING provider (model={}) ===", model);
    let labels = cat.categorize_batch(&all_segs[..6.min(all_segs.len())]);
    println!("\n=== CATEGORIZER RESULT ===");
    println!(
        "labels: {:?}",
        labels.iter().map(|l| l.label()).collect::<Vec<_>>()
    );
    use std::time::Instant;
    let t = Instant::now();
    use pay4what::categorize::LlmCaller;
    let caller: Box<dyn LlmCaller> = if let (Ok(k), Ok(b)) = (
        std::env::var("GROVE_API_KEY"),
        std::env::var("GROVE_BASE_URL"),
    ) {
        println!("(using Grove)");
        Box::new(pay4what::categorize::GroveCaller {
            api_key: k,
            base_url: b,
            model: model.clone(),
        })
    } else {
        println!("(using OpenRouter)");
        Box::new(OpenRouterCaller {
            api_key: std::env::var("OPENROUTER_API_KEY").unwrap(),
            model: model.clone(),
        })
    };
    match caller.categorize_batch(&prompt) {
        Ok(raw) => {
            println!(
                "\n=== RAW RESPONSE (first 1500 chars) [{}ms] ===\n{}",
                t.elapsed().as_millis(),
                raw.chars().take(1500).collect::<String>()
            );
            println!("\n=== PARSED ===");
            let parsed = pay4what::categorize::LlmCategorizer::parse_labels(&raw);
            println!(
                "parsed {} labels: {:?}",
                parsed.len(),
                parsed.iter().map(|l| l.label()).collect::<Vec<_>>()
            );
        }
        Err(e) => println!("\nCALLER ERR: {e} [{}ms]", t.elapsed().as_millis()),
    }
    // compare to rules
    let rules = RulesCategorizer;
    let rule_labels: Vec<_> = all_segs
        .iter()
        .take(6)
        .map(|s| rules.categorize(s).label())
        .collect();
    println!("\n=== RULES fallback labels ===\n{:?}", rule_labels);
}
