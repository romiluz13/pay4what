//! Find the actual Supabase-task user message + identify a clean Supabase-focused session.
use pay4what::parse::parse_session;

fn scan_session_user_msgs(path: &std::path::Path) -> Vec<(String, String)> {
    // returns (timestamp, text) for real user messages
    let Ok(s) = parse_session(path) else {
        return vec![];
    };
    let mut out = Vec::new();
    for t in &s.turns {
        if t.kind.as_deref() == Some("user")
            && let Some(text) = &t.text
            && !text.is_empty()
            && !text.starts_with('<')
        {
            let ts = t.timestamp.clone().unwrap_or_default();
            out.push((ts, text.clone()));
        }
    }
    out
}

fn main() {
    let candidates = [
        (
            "-Users-rom-iluz-Dev-SDR-AI",
            "8cab6361-80ae-4db8-a83d-917ee7a092e0",
        ),
        ("-Users-rom-iluz", "b1a649bf-6fb9-4200-a210-e27250f49044"),
        (
            "-Users-rom-iluz-Dev",
            "20ffb608-a23e-48c4-b523-e744b65cba83",
        ),
        ("-Users-rom-iluz", "2a0ec53e-6012-4572-8d9c-3f359ef98431"),
    ];
    for (proj, sess) in &candidates {
        let p = std::path::Path::new(&format!(
            "/Users/rom.iluz/.claude/projects/{proj}/{sess}.jsonl"
        ))
        .to_path_buf();
        if !p.exists() {
            println!("MISSING: {}", p.display());
            continue;
        }
        let msgs = scan_session_user_msgs(&p);
        println!("\n=== {sess} ({} user msgs) ===", msgs.len());
        // print user msgs that mention supabase or a DB task
        for (ts, text) in &msgs {
            let l = text.to_lowercase();
            if l.contains("supabase")
                || l.contains("postgres")
                || l.contains("database")
                || l.contains("schema")
            {
                let preview: String = text.chars().take(200).collect();
                println!("  [{ts}] {preview}");
            }
        }
    }
}
