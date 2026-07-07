//! Categorization — LLM-primary via OpenRouter, rules fallback.
//!
//! USER DECISION (2026-07-07): LLM-PRIMARY. Every segment goes through the LLM
//! (DeepSeek V4 Flash on OpenRouter by default); deterministic rules are the
//! fallback (no OPENROUTER_API_KEY) + obvious-case pre-tag (branch-name-clear
//! cases skip the LLM to save cost).
//!
//! MIRROR (anti-pattern to beat): codeburn src/classifier.ts — regex + tool-set
//! heuristics, firstMatchingCategory picks earliest regex index (the #196
//! keyword-order bug), NEVER inspects tool args/files. pay4what's LLM reads the
//! full segment context (user message + tool verbs + touched files + branch).
//!
//! Activity labels match the handoff's viral screenshot.
use crate::segment::Segment;
use std::collections::BTreeSet;

/// The activity categories pay4what attributes spend to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Activity {
    Feature,
    Bugfix,
    Migration,
    Refactor,
    Debugging,
    Exploration,
    Planning,
    Unattributed,
}

impl Activity {
    /// Emoji + business noun for the viral table (MIRROR: handoff screenshot).
    pub fn emoji(&self) -> &'static str {
        match self {
            Activity::Feature => "🚀",
            Activity::Bugfix => "🐛",
            Activity::Migration => "📦",
            Activity::Refactor => "♻️",
            Activity::Debugging => "🔍",
            Activity::Exploration => "🧭",
            Activity::Planning => "📝",
            Activity::Unattributed => "❓",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Activity::Feature => "feature",
            Activity::Bugfix => "bugfix",
            Activity::Migration => "migration",
            Activity::Refactor => "refactor",
            Activity::Debugging => "debugging",
            Activity::Exploration => "exploration",
            Activity::Planning => "planning",
            Activity::Unattributed => "unattributed",
        }
    }

    /// Parse a label string from the LLM response into an Activity.
    pub fn parse(s: &str) -> Activity {
        let s = s.trim().to_lowercase();
        match s.as_str() {
            "feature" => Activity::Feature,
            "bugfix" | "bug" | "fix" => Activity::Bugfix,
            "migration" | "migrate" => Activity::Migration,
            "refactor" => Activity::Refactor,
            "debugging" | "debug" => Activity::Debugging,
            "exploration" | "explore" => Activity::Exploration,
            "planning" | "plan" => Activity::Planning,
            _ => Activity::Unattributed,
        }
    }
}

/// A segment with its attributed activity + cost (the render unit).
#[derive(Debug, Clone)]
pub struct LabeledSegment {
    pub index: usize,
    pub activity: Activity,
    pub user_message: String,
    pub cost: f64,
    pub tokens: u64,
    pub git_branch: Option<String>,
    pub touched_files: BTreeSet<String>,
}

/// Categorizer trait — rules or LLM both implement this.
pub trait Categorizer {
    fn categorize(&self, segment: &Segment) -> Activity;
    /// Categorize a batch (default: per-segment; LLM overrides to batch).
    fn categorize_batch(&self, segments: &[Segment]) -> Vec<Activity> {
        segments.iter().map(|s| self.categorize(s)).collect()
    }
}

// ─── Rules categorizer (fallback + obvious-case pre-tag) ───────────────────

/// Deterministic rules: branch-name patterns + user-message keywords + tool
/// verbs. Sharper than CodeBurn (it reads touched files + branch) but still
/// no LLM. The no-key path + the obvious-case pre-tagger.
pub struct RulesCategorizer;

impl RulesCategorizer {
    fn from_branch(branch: &str) -> Option<Activity> {
        let b = branch.to_lowercase();
        if b.starts_with("feat/") || b.starts_with("feature/") {
            return Some(Activity::Feature);
        }
        if b.starts_with("fix/") || b.starts_with("bugfix/") || b.starts_with("hotfix/") {
            return Some(Activity::Bugfix);
        }
        if b.starts_with("refactor/") {
            return Some(Activity::Refactor);
        }
        if b.starts_with("migrat") {
            return Some(Activity::Migration);
        }
        None
    }

    fn from_message(msg: &str) -> Option<Activity> {
        let m = msg.to_lowercase();
        // order matters less than CodeBurn (we check branch first), but keep
        // migration before refactor ("migrate the schema" not "refactor")
        if m.contains("migrat") || m.contains("upgrade ") || m.contains("bump ") {
            return Some(Activity::Migration);
        }
        if m.contains("refactor") || m.contains("clean up") || m.contains("rename") {
            return Some(Activity::Refactor);
        }
        if m.contains("fix") || m.contains("bug") || m.contains("broken") || m.contains("failing") {
            return Some(Activity::Bugfix);
        }
        if m.contains("debug") || m.contains("trace") || m.contains("why is") {
            return Some(Activity::Debugging);
        }
        if m.contains("add")
            || m.contains("implement")
            || m.contains("create")
            || m.contains("new ")
        {
            return Some(Activity::Feature);
        }
        if m.contains("explore")
            || m.contains("understand")
            || m.contains("investigate")
            || m.contains("how does")
        {
            return Some(Activity::Exploration);
        }
        if m.contains("plan") || m.contains("design") || m.contains("how should") {
            return Some(Activity::Planning);
        }
        None
    }

    fn from_tools(seg: &Segment) -> Option<Activity> {
        // read-only tools only -> exploration
        let has_edit = seg.tool_verbs.iter().any(|v| {
            matches!(
                v.as_str(),
                "Edit" | "Write" | "FileEditTool" | "FileWriteTool" | "MultiEditTool"
            )
        });
        let has_read = seg
            .tool_verbs
            .iter()
            .any(|v| matches!(v.as_str(), "Read" | "Grep" | "Glob"));
        if !has_edit
            && has_read
            && seg.tool_verbs.iter().all(|v| {
                matches!(
                    v.as_str(),
                    "Read" | "Grep" | "Glob" | "WebSearch" | "WebFetch"
                )
            })
        {
            return Some(Activity::Exploration);
        }
        None
    }
}

impl Categorizer for RulesCategorizer {
    fn categorize(&self, seg: &Segment) -> Activity {
        if let Some(b) = seg.git_branch.as_deref()
            && let Some(a) = Self::from_branch(b)
        {
            return a;
        }
        if !seg.user_message.is_empty()
            && let Some(a) = Self::from_message(&seg.user_message)
        {
            return a;
        }
        if let Some(a) = Self::from_tools(seg) {
            return a;
        }
        Activity::Unattributed
    }
}

// ─── LLM categorizer (OpenRouter, mockable caller) ─────────────────────────

/// Abstracts the OpenRouter HTTP call so tests can inject a mock.
pub trait LlmCaller {
    /// Given a prompt, return the raw response text (JSON: {"labels":[...]}).
    fn categorize_batch(&self, prompt: &str) -> Result<String, Box<dyn std::error::Error>>;
}

/// LLM-primary categorizer. Batches a session's segments into one OpenRouter
/// call (1 call/session, not 1/segment) to cut latency + cost.
pub struct LlmCategorizer {
    pub model: String,
    pub caller: Box<dyn LlmCaller>,
    pub rules: RulesCategorizer,
}

impl LlmCategorizer {
    pub fn new(model: &str, caller: Box<dyn LlmCaller>) -> Self {
        Self {
            model: model.to_string(),
            caller,
            rules: RulesCategorizer,
        }
    }

    /// Build the OpenRouter prompt for a batch of segments.
    fn build_prompt(&self, segments: &[Segment]) -> String {
        let items: Vec<String> = segments
            .iter()
            .map(|s| {
                let files: Vec<String> = s.touched_files.iter().take(8).cloned().collect();
                let verbs: Vec<String> = s.tool_verbs.iter().cloned().collect();
                format!(
                    "- user: {:?} | branch: {:?} | tools: {} | files: {}",
                    truncate(&s.user_message, 300),
                    s.git_branch,
                    verbs.join(","),
                    files.join(","),
                )
            })
            .collect();
        format!(
            "Categorize each development segment by activity. Respond as JSON: {{\"labels\":[\"feature\",\"bugfix\",...]}}\n\
             Allowed labels: feature, bugfix, migration, refactor, debugging, exploration, planning, unattributed.\n\
             Segments:\n{}",
            items.join("\n")
        )
    }

    fn parse_labels(text: &str) -> Vec<Activity> {
        // parse {"labels":[...]} ; fall back to per-line parsing
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text)
            && let Some(arr) = v.get("labels").and_then(|l| l.as_array())
        {
            return arr
                .iter()
                .filter_map(|x| x.as_str())
                .map(Activity::parse)
                .collect();
        }
        // fall back: split by comma/newline
        text.split([',', '\n']).map(Activity::parse).collect()
    }
}

impl Categorizer for LlmCategorizer {
    fn categorize(&self, seg: &Segment) -> Activity {
        // single-segment path: batch of one
        let labels = self.categorize_batch(std::slice::from_ref(seg));
        labels.into_iter().next().unwrap_or(Activity::Unattributed)
    }

    fn categorize_batch(&self, segments: &[Segment]) -> Vec<Activity> {
        if segments.is_empty() {
            return Vec::new();
        }
        let prompt = self.build_prompt(segments);
        match self.caller.categorize_batch(&prompt) {
            Ok(text) => {
                let labels = Self::parse_labels(&text);
                // if the LLM returned the wrong count or was unparseable, fall back to rules
                if labels.len() == segments.len() {
                    labels
                } else {
                    segments.iter().map(|s| self.rules.categorize(s)).collect()
                }
            }
            Err(_) => segments.iter().map(|s| self.rules.categorize(s)).collect(),
        }
    }
}

/// The real OpenRouter HTTP caller (used when OPENROUTER_API_KEY is set).
#[cfg(feature = "categorize")]
pub struct OpenRouterCaller {
    pub api_key: String,
    pub model: String,
}

#[cfg(feature = "categorize")]
impl LlmCaller for OpenRouterCaller {
    fn categorize_batch(&self, prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "max_completion_tokens": 1024,
        });
        let resp = client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()?;
        let v: serde_json::Value = resp.json()?;
        // OpenAI-compatible: choices[0].message.content
        let text = v
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        Ok(text.to_string())
    }
}

// ─── Entry point: label all segments ───────────────────────────────────────

/// Categorize all segments in a session, preserving cost + files.
pub fn categorize_segments(segments: &[Segment], cat: &dyn Categorizer) -> Vec<LabeledSegment> {
    let activities = cat.categorize_batch(segments);
    segments
        .iter()
        .zip(
            activities
                .into_iter()
                .chain(std::iter::repeat(Activity::Unattributed)),
        )
        .map(|(seg, activity)| LabeledSegment {
            index: seg.index,
            activity,
            user_message: seg.user_message.clone(),
            cost: seg.cost,
            tokens: seg.total_tokens(),
            git_branch: seg.git_branch.clone(),
            touched_files: seg.touched_files.clone(),
        })
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}
