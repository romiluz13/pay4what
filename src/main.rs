//! pay4what — See what each feature cost you in Claude Code.
//!
//! Token spend per activity, not per session. Powered by LLM categorization
//! + a persisted bucket store (incremental, queryable).
//!
//! Architecture: classify segments → persist rich records {activity, tags,
//! summary, confidence, cost} into ~/.pay4what/store.json → query the buckets
//! at question time (no re-reading 7M tokens of raw session context).
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "pay4what",
    version,
    about = "See what each feature cost you in Claude Code — token spend per activity, not per session."
)]
struct Cli {
    /// Show spend since this long. e.g. `7d`, `today`, `2026-07-01`. Default: 7d.
    #[arg(long, short = 's', global = true)]
    since: Option<String>,

    /// Categorizer model. Default: DeepSeek-V4-Flash (Grove) / deepseek/deepseek-v4-flash (OpenRouter).
    #[arg(long, global = true)]
    model: Option<String>,

    /// Also show the cost-by-file table.
    #[arg(long, global = true)]
    files: bool,

    /// Skip the LLM categorizer (rules only — DEGRADED MODE, no activity tags).
    #[arg(long, global = true)]
    no_llm: bool,

    /// Force re-classification of all segments (ignores the bucket cache).
    #[arg(long, global = true)]
    rebuild: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Query the bucket store: "how much did <phrase> cost?"
    Query { phrase: String },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        eprintln!("pay4what: {e}");
        std::process::exit(1);
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    match &cli.command {
        Some(Command::Query { phrase }) => run_query(cli, phrase),
        None => run_default(cli),
    }
}

/// A classification job: one session's segments to classify.
struct ClassifyJob {
    uuid: String,
    segments: Vec<pay4what::segment::Segment>,
}

/// Default mode: classify (incremental, parallel) + render cost-by-activity.
fn run_default(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    let sessions = pay4what::discover::discover_all();
    if sessions.is_empty() {
        eprintln!("No Claude Code sessions found under ~/.claude/projects.");
        return Ok(());
    }

    let since = parse_since(cli.since.as_deref().or(Some("7d")));
    let pricing = pay4what::cost::bundled_pricing();
    let model = cli
        .model
        .as_deref()
        .unwrap_or("DeepSeek-V4-Flash")
        .to_string();

    let mut store = pay4what::store::BucketStore::load();
    if cli.rebuild {
        eprintln!("Rebuilding: clearing all cached buckets.");
        store = pay4what::store::BucketStore::default();
    }

    // Phase 1 (serial): parse + segment all sessions in range, collect jobs
    // that need (re)classification.
    let mut jobs: Vec<ClassifyJob> = Vec::new();
    let mut total_sessions = 0usize;
    let mut _total_subagent_files = 0usize;

    for session_path in &sessions {
        let Ok(session) = pay4what::parse::parse_session(session_path) else {
            continue;
        };
        if let Some(cutoff) = since
            && let Some(last) = session.last_ts.as_deref()
            && let Some(last_dt) = parse_ts(last)
            && last_dt < cutoff
        {
            continue;
        }
        total_sessions += 1;

        let uuid = pay4what::store::session_uuid(session_path);
        let segments = pay4what::segment::segment_session_with_pricing(&session, &pricing);
        _total_subagent_files += pay4what::discover::discover_subagents(session_path).len();

        let already = store.classified_count(&uuid);
        if already < segments.len() || cli.rebuild {
            jobs.push(ClassifyJob { uuid, segments });
        }

        // subagent sessions
        for sub_path in pay4what::discover::discover_subagents(session_path) {
            if let Ok(sub_session) = pay4what::parse::parse_session(&sub_path) {
                let sub_uuid = pay4what::store::session_uuid(&sub_path);
                let sub_segs =
                    pay4what::segment::segment_session_with_pricing(&sub_session, &pricing);
                let sub_already = store.classified_count(&sub_uuid);
                if sub_already < sub_segs.len() || cli.rebuild {
                    jobs.push(ClassifyJob {
                        uuid: sub_uuid,
                        segments: sub_segs,
                    });
                }
            }
        }
    }

    if total_sessions == 0 {
        println!("No Claude Code sessions in range.");
        return Ok(());
    }

    eprintln!(
        "Classifying {} session(s) across {} jobs...",
        total_sessions,
        jobs.len()
    );
    if jobs.len() > 50 && !cli.no_llm {
        eprintln!(
            "⚠️  {} jobs — this will take a while. Use --no-llm for instant totals.",
            jobs.len()
        );
        eprintln!("    The incremental cache means this big run only happens once.");
    }

    // Phase 2 (parallel via rayon): classify each job. Sessions are independent.
    // rayon bounds concurrency to CPU core count (work-stealing) — prevents
    // overwhelming the gateway with 1500 concurrent requests.
    let results: Vec<(String, usize, Vec<pay4what::store::Bucket>)> = if cli.no_llm {
        eprintln!("⚠️  --no-llm: rules-only degraded mode (no tags, no summaries).");
        jobs.iter().map(classify_job_rules).collect()
    } else {
        use rayon::prelude::*;
        jobs.par_iter()
            .map(|job| classify_job_llm(job, &model))
            .collect()
    };

    // Phase 3 (serial): merge results into the store
    let mut total_new_segments = 0usize;
    for (uuid, n_segs, buckets) in results {
        total_new_segments += buckets.len();
        store.remove_session(&uuid);
        for b in buckets {
            store.upsert_bucket(b);
        }
        store.mark_classified(&uuid, n_segs);
    }

    eprintln!(
        "Classified {total_new_segments} segments. Bucket store: {} buckets total.",
        store.buckets.len()
    );
    store.save()?;

    // Render from ALL buckets in range (not just this run's)
    let labeled = buckets_to_labeled(&store, since);
    print!("{}", pay4what::render::render_activity_table(&labeled));
    if cli.files {
        print!("{}", pay4what::render::render_file_table(&labeled));
    }
    Ok(())
}

/// Classify one job using the LLM (runs in its own thread).
fn classify_job_llm(
    job: &ClassifyJob,
    model: &str,
) -> (String, usize, Vec<pay4what::store::Bucket>) {
    let cat = pick_categorizer(model);
    let records = cat.categorize_rich(&job.segments);
    let buckets = job
        .segments
        .iter()
        .zip(records)
        .map(|(seg, rec)| pay4what::store::Bucket {
            id: format!("{}:{}", job.uuid, seg.index),
            session: job.uuid.clone(),
            segment_index: seg.index,
            activity: rec.activity,
            tags: rec.tags,
            summary: rec.summary,
            confidence: rec.confidence,
            cost: seg.cost,
            tokens: seg.total_tokens(),
            files: seg.touched_files.iter().cloned().collect(),
            branch: seg.git_branch.clone(),
            first_ts: seg.turns.first().and_then(|t| t.timestamp.clone()),
            last_ts: seg.turns.last().and_then(|t| t.timestamp.clone()),
        })
        .collect();
    (job.uuid.clone(), job.segments.len(), buckets)
}

/// Classify one job using rules only (instant, no LLM).
fn classify_job_rules(job: &ClassifyJob) -> (String, usize, Vec<pay4what::store::Bucket>) {
    let cat = pay4what::categorize::RulesCategorizer;
    let buckets = job
        .segments
        .iter()
        .map(|seg| {
            let rec = pay4what::categorize::RichRecord {
                activity: pay4what::categorize::Categorizer::categorize(&cat, seg),
                tags: Vec::new(),
                summary: seg.user_message.chars().take(80).collect(),
                confidence: 0.0,
            };
            pay4what::store::Bucket {
                id: format!("{}:{}", job.uuid, seg.index),
                session: job.uuid.clone(),
                segment_index: seg.index,
                activity: rec.activity,
                tags: rec.tags,
                summary: rec.summary,
                confidence: rec.confidence,
                cost: seg.cost,
                tokens: seg.total_tokens(),
                files: seg.touched_files.iter().cloned().collect(),
                branch: seg.git_branch.clone(),
                first_ts: seg.turns.first().and_then(|t| t.timestamp.clone()),
                last_ts: seg.turns.last().and_then(|t| t.timestamp.clone()),
            }
        })
        .collect();
    (job.uuid.clone(), job.segments.len(), buckets)
}

/// Query mode: "how much did <phrase> cost?" — hits the bucket store, no LLM.
fn run_query(cli: &Cli, phrase: &str) -> Result<(), Box<dyn std::error::Error>> {
    let store = pay4what::store::BucketStore::load();
    if store.buckets.is_empty() {
        eprintln!("No buckets found. Run `pay4what --since 7d` first to classify your sessions.");
        return Ok(());
    }
    let since = parse_since(cli.since.as_deref().or(Some("365d")));
    let matches: Vec<&pay4what::store::Bucket> = store
        .buckets
        .iter()
        .filter(|b| {
            if let Some(cutoff) = since
                && let Some(ts) = &b.last_ts
                && let Some(dt) = parse_ts(ts)
                && dt < cutoff
            {
                return false;
            }
            let p = phrase.to_lowercase();
            b.summary.to_lowercase().contains(&p)
                || b.tags.iter().any(|t| t.to_lowercase().contains(&p))
                || b.activity.label().contains(&p)
        })
        .collect();

    if matches.is_empty() {
        println!("No segments matched \"{phrase}\".");
        return Ok(());
    }

    let total: f64 = matches.iter().map(|b| b.cost).sum();
    let total_tokens: u64 = matches.iter().map(|b| b.tokens).sum();
    println!("\n  \"{}\" — {} segment(s)", phrase, matches.len());
    println!("  ┌──────────────────────────────────────────────┬──────────┬────────┐");
    println!("  │ Activity     Summary                          │ Cost     │ Tokens │");
    println!("  ├──────────────────────────────────────────────┼──────────┼────────┤");
    for b in &matches {
        let act = format!("{} {}", b.activity.emoji(), b.activity.label());
        let summary = truncate_str(&b.summary, 32);
        println!(
            "  │ {:<11} {:<33} │ {:>8} │ {:>6} │",
            act,
            summary,
            format!("${:.2}", b.cost),
            fmt_tokens(b.tokens)
        );
    }
    println!("  ├──────────────────────────────────────────────┼──────────┼────────┤");
    println!(
        "  │ {:<45} │ {:>8} │ {:>6} │",
        "TOTAL",
        format!("${:.2}", total),
        fmt_tokens(total_tokens)
    );
    println!("  └──────────────────────────────────────────────┴──────────┴────────┘");
    Ok(())
}

fn buckets_to_labeled(
    store: &pay4what::store::BucketStore,
    since: Option<chrono_like::DateTime>,
) -> Vec<pay4what::categorize::LabeledSegment> {
    store
        .buckets
        .iter()
        .filter(|b| {
            if let Some(cutoff) = since
                && let Some(ts) = &b.last_ts
                && let Some(dt) = parse_ts(ts)
                && dt < cutoff
            {
                return false;
            }
            true
        })
        .map(|b| pay4what::categorize::LabeledSegment {
            index: b.segment_index,
            activity: b.activity,
            user_message: b.summary.clone(),
            cost: b.cost,
            tokens: b.tokens,
            git_branch: b.branch.clone(),
            touched_files: b.files.iter().cloned().collect(),
        })
        .collect()
}

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

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
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

// ─── date helpers ───────────────────────────────────────────────────────────
fn parse_since(since: Option<&str>) -> Option<chrono_like::DateTime> {
    let s = since?;
    if s == "today" {
        return chrono_like::today_midnight();
    }
    if let Some(days) = s.strip_suffix('d').and_then(|n| n.parse::<u64>().ok()) {
        return chrono_like::now_minus_days(days);
    }
    chrono_like::parse_date(s)
}
fn parse_ts(s: &str) -> Option<chrono_like::DateTime> {
    chrono_like::parse_rfc3339(s)
}

mod chrono_like {
    #[derive(Clone, Copy)]
    pub struct DateTime {
        pub epoch_secs: i64,
    }
    impl DateTime {
        fn from_ymd_hms(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Self {
            let y = if mo <= 2 { y - 1 } else { y };
            let era = if y >= 0 { y } else { y - 399 } / 400;
            let yoe = (y - era * 400) as u32;
            let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
            let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
            Self {
                epoch_secs: (era as i64 * 146097 + doe as i64 - 719468) * 86400
                    + h as i64 * 3600
                    + mi as i64 * 60
                    + s as i64,
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
    pub fn today_midnight() -> Option<DateTime> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        let secs = now.as_secs() as i64;
        Some(DateTime {
            epoch_secs: secs - (secs % 86400),
        })
    }
    pub fn parse_date(s: &str) -> Option<DateTime> {
        let p: Vec<&str> = s.split('-').collect();
        if p.len() != 3 {
            return None;
        }
        Some(DateTime::from_ymd_hms(
            p[0].parse().ok()?,
            p[1].parse().ok()?,
            p[2].parse().ok()?,
            0,
            0,
            0,
        ))
    }
    pub fn parse_rfc3339(s: &str) -> Option<DateTime> {
        let s = s.trim();
        if s.len() < 19 {
            return None;
        }
        Some(DateTime::from_ymd_hms(
            s[0..4].parse().ok()?,
            s[5..7].parse().ok()?,
            s[8..10].parse().ok()?,
            s[11..13].parse().ok()?,
            s[14..16].parse().ok()?,
            s[17..19].parse().ok()?,
        ))
    }
}
