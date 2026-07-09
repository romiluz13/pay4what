//! pay4what — See what each feature cost you in Claude Code.
//!
//! Token spend per activity, not per session.
//!
//! v1.0: cost-by-activity + cost-by-file (high confidence). Commit/issue
//! attribution deferred to v1.1.
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "pay4what",
    version,
    about = "See what each feature cost you in Claude Code — token spend per activity, not per session."
)]
struct Cli {
    /// Show spend since this long. e.g. `7d`, `2026-07-01`. Defaults to all.
    #[arg(long, short = 's')]
    since: Option<String>,

    /// Output format: table (default), json, markdown.
    #[arg(long, default_value = "table")]
    format: String,

    /// Categorizer model. Default: DeepSeek-V4-Flash (Grove). For OpenRouter,
    /// pass e.g. `deepseek/deepseek-v4-flash`. Falls back to rules if no key.
    #[arg(long, default_value = "DeepSeek-V4-Flash")]
    model: String,

    /// Also show the cost-by-file table.
    #[arg(long)]
    files: bool,

    /// Skip the LLM categorizer (rules only). Fast, no API key needed.
    /// Use this for quick cost totals; the LLM sharpens unattributed segments.
    #[arg(long)]
    no_llm: bool,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        eprintln!("pay4what: {e}");
        std::process::exit(1);
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    let sessions = pay4what::discover::discover_all();
    if sessions.is_empty() {
        println!("No Claude Code sessions found under ~/.claude/projects.");
        println!("Set CLAUDE_CONFIG_DIRS or run from a machine with Claude Code transcripts.");
        return Ok(());
    }

    let since = parse_since(cli.since.as_deref());
    let pricing = pay4what::cost::bundled_pricing();
    let mut all_labeled: Vec<pay4what::categorize::LabeledSegment> = Vec::new();
    let mut total_sessions = 0usize;
    let mut total_subagent_files = 0usize;

    let categorizer = if cli.no_llm {
        Box::new(pay4what::categorize::RulesCategorizer)
            as Box<dyn pay4what::categorize::Categorizer>
    } else {
        pick_categorizer(&cli.model)
    };

    for session_path in &sessions {
        let Ok(session) = pay4what::parse::parse_session(session_path) else {
            continue;
        };
        // date filter: skip sessions whose last_ts is before `since`
        if let Some(cutoff) = since
            && let Some(last) = session.last_ts.as_deref()
            && let Some(last_dt) = parse_ts(last)
            && last_dt < cutoff
        {
            continue;
        }
        total_sessions += 1;

        // parent session segments
        let segments = pay4what::segment::segment_session_with_pricing(&session, &pricing);
        let mut labeled =
            pay4what::categorize::categorize_segments(&segments, categorizer.as_ref());

        // subagent spend (separate files, no double-count)
        let sub_paths = pay4what::discover::discover_subagents(session_path);
        total_subagent_files += sub_paths.len();
        for sub_path in &sub_paths {
            if let Ok(sub_session) = pay4what::parse::parse_session(sub_path) {
                let sub_segs =
                    pay4what::segment::segment_session_with_pricing(&sub_session, &pricing);
                let mut sub_labeled =
                    pay4what::categorize::categorize_segments(&sub_segs, categorizer.as_ref());
                labeled.append(&mut sub_labeled);
            }
        }

        all_labeled.extend(labeled);
    }

    match cli.format.as_str() {
        "json" => {
            let json = serde_json::json!({
                "sessions": total_sessions,
                "subagent_files": total_subagent_files,
                "pricing_as_of": pricing.as_of,
                "activities": aggregate_json(&all_labeled),
            });
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        _ => {
            println!(
                "pay4what — {} sessions (+{} subagent files), pricing as-of {}",
                total_sessions, total_subagent_files, pricing.as_of,
            );
            print!("{}", pay4what::render::render_activity_table(&all_labeled));
            if cli.files {
                print!("{}", pay4what::render::render_file_table(&all_labeled));
            }
        }
    }
    Ok(())
}

/// Pick the categorizer: prefer Grove (Rom's personal gateway) when its env
/// is set, else OpenRouter (the public-path default) when its key is set, else
/// rules fallback. Published app uses OpenRouter; Grove is for local dogfooding.
fn pick_categorizer(model: &str) -> Box<dyn pay4what::categorize::Categorizer> {
    #[cfg(feature = "categorize")]
    {
        if let (Ok(key), Ok(base)) = (
            std::env::var("GROVE_API_KEY"),
            std::env::var("GROVE_BASE_URL"),
        ) {
            return Box::new(pay4what::categorize::LlmCategorizer::new(
                model,
                Box::new(pay4what::categorize::GroveCaller {
                    api_key: key,
                    base_url: base,
                    model: model.to_string(),
                }),
            ));
        }
        if let Ok(key) = std::env::var("OPENROUTER_API_KEY") {
            return Box::new(pay4what::categorize::LlmCategorizer::new(
                model,
                Box::new(pay4what::categorize::OpenRouterCaller {
                    api_key: key,
                    model: model.to_string(),
                }),
            ));
        }
    }
    let _ = model;
    Box::new(pay4what::categorize::RulesCategorizer)
}

fn aggregate_json(labeled: &[pay4what::categorize::LabeledSegment]) -> serde_json::Value {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, (f64, u64, usize)> = BTreeMap::new();
    for s in labeled {
        let e = map
            .entry(s.activity.label().to_string())
            .or_insert((0.0, 0, 0));
        e.0 += s.cost;
        e.1 += s.tokens;
        e.2 += 1;
    }
    let arr: Vec<serde_json::Value> = map
        .into_iter()
        .map(|(label, (cost, tokens, count))| {
            serde_json::json!({ "activity": label, "cost_usd": (cost * 100.0).round() / 100.0, "tokens": tokens, "segments": count })
        })
        .collect();
    serde_json::Value::Array(arr)
}

/// Parse a `--since` value into a cutoff datetime.
/// Supports: `7d` (N days ago), `2026-07-01` (absolute date).
fn parse_since(since: Option<&str>) -> Option<chrono_like::DateTime> {
    let s = since?;
    // N days ago
    if let Some(days) = s.strip_suffix('d').and_then(|n| n.parse::<u64>().ok()) {
        return chrono_like::now_minus_days(days);
    }
    // absolute date
    chrono_like::parse_date(s)
}

fn parse_ts(s: &str) -> Option<chrono_like::DateTime> {
    chrono_like::parse_rfc3339(s)
}

/// Minimal date handling without pulling chrono — use std::time + manual parse
/// for the RFC3339 timestamps Claude Code emits. For now, a lightweight module.
mod chrono_like {
    #[derive(Clone, Copy)]
    pub struct DateTime {
        pub epoch_secs: i64,
    }
    impl DateTime {
        fn from_ymd_hms(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Self {
            // days since 1970-01-01 (civil-from-days algorithm)
            let y = if mo <= 2 { y - 1 } else { y };
            let era = if y >= 0 { y } else { y - 399 } / 400;
            let yoe = (y - era * 400) as u32;
            let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
            let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
            let days = era as i64 * 146097 + doe as i64 - 719468;
            Self {
                epoch_secs: days * 86400 + h as i64 * 3600 + mi as i64 * 60 + s as i64,
            }
        }
    }
    impl PartialEq for DateTime {
        fn eq(&self, o: &Self) -> bool {
            self.epoch_secs == o.epoch_secs
        }
    }
    impl PartialOrd for DateTime {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            self.epoch_secs.partial_cmp(&o.epoch_secs)
        }
    }
    pub fn now_minus_days(days: u64) -> Option<DateTime> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        Some(DateTime {
            epoch_secs: now.as_secs() as i64 - days as i64 * 86400,
        })
    }
    pub fn parse_date(s: &str) -> Option<DateTime> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 3 {
            return None;
        }
        let y = parts[0].parse().ok()?;
        let mo = parts[1].parse().ok()?;
        let d = parts[2].parse().ok()?;
        Some(DateTime::from_ymd_hms(y, mo, d, 0, 0, 0))
    }
    pub fn parse_rfc3339(s: &str) -> Option<DateTime> {
        // 2026-07-07T10:00:00Z or with fractional/.timezone
        let s = s.trim();
        if s.len() < 19 {
            return None;
        }
        let y = s[0..4].parse().ok()?;
        let mo = s[5..7].parse().ok()?;
        let d = s[8..10].parse().ok()?;
        let h = s[11..13].parse().ok()?;
        let mi = s[14..16].parse().ok()?;
        let se = s[17..19].parse().ok()?;
        Some(DateTime::from_ymd_hms(y, mo, d, h, mi, se))
    }
}
