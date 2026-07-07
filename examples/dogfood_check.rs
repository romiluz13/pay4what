// Quick dogfood: parse one REAL local session, report what we got.
use pay4what::discover::discover_all;
use pay4what::parse::parse_session;
fn main() {
    let sessions = discover_all();
    println!("discovered {} sessions", sessions.len());
    // pick a SDR-AI session (real dev repo with 777 commits)
    let sdr = sessions
        .iter()
        .find(|p| p.to_string_lossy().contains("SDR-AI"))
        .unwrap();
    let s = parse_session(sdr).unwrap();
    println!("session: {}", s.path.file_name().unwrap().to_string_lossy());
    println!("  cwd:        {:?}", s.cwd);
    println!("  git_branch: {:?}", s.git_branch);
    println!("  turns:      {}", s.turns.len());
    println!("  first_ts:   {:?}", s.first_ts);
    println!("  last_ts:    {:?}", s.last_ts);
    let with_usage = s.turns.iter().filter(|t| t.usage.is_some()).count();
    println!("  turns w/ usage: {}", with_usage);
    let sub = s.turns.iter().filter(|t| t.is_sidechain).count();
    println!("  subagent turns: {}", sub);
    let tools = s.turns.iter().map(|t| t.tool_uses.len()).sum::<usize>();
    println!("  tool_use blocks: {}", tools);
    // model mix
    use std::collections::HashMap;
    let mut models: HashMap<String, u64> = HashMap::new();
    for t in &s.turns {
        if let Some(u) = &t.usage {
            *models.entry(u.model.clone()).or_default() += 1;
        }
    }
    println!("  models: {:?}", models);
    // token totals (cache-separated)
    let (mut inp, mut out, mut cr, mut cc) = (0u64, 0u64, 0u64, 0u64);
    for t in &s.turns {
        if let Some(u) = &t.usage {
            inp += u.input_tokens;
            out += u.output_tokens;
            cr += u.cache_read_input_tokens;
            cc += u.cache_creation_input_tokens;
        }
    }
    println!(
        "  tokens: input={} output={} cache_read={} cache_creation={}",
        inp, out, cr, cc
    );
}
