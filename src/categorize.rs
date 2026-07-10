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
    pub fn build_prompt(&self, segments: &[Segment]) -> String {
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

    pub fn parse_labels(text: &str) -> Vec<Activity> {
        // Strip a markdown code fence if present (LLMs often wrap JSON in ```json ... ```)
        let text = text.trim();
        let text = text
            .strip_prefix("```")
            .map(|t| t.strip_prefix("json").unwrap_or(t).trim_start())
            .unwrap_or(text);
        let text = text
            .strip_suffix("```")
            .map(|t| t.trim_end())
            .unwrap_or(text);
        // Try the simple path first: the WHOLE text is a JSON object with a labels array.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text)
            && let Some(arr) = v.get("labels").and_then(|l| l.as_array())
        {
            return arr
                .iter()
                .filter_map(|x| x.as_str())
                .map(Activity::parse)
                .collect();
        }
        // Fall back: extract the first {...} block and try again.
        if let Some(start) = text.find('{')
            && let Some(end) = text.rfind('}')
            && start < end
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[start..=end])
            && let Some(arr) = v.get("labels").and_then(|l| l.as_array())
        {
            return arr
                .iter()
                .filter_map(|x| x.as_str())
                .map(Activity::parse)
                .collect();
        }
        // Last resort: split by comma/newline (rarely reached now).
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
        // Cost saver + latency fix: rules-pre-tag obvious segments (feat/ branch,
        // "fix" keyword, read-only tools) and send ONLY the rules-unattributed
        // segments to the LLM. This cuts LLM calls ~5-10x — a 7d run drops from
        // >5min to <60s — without contorting the categorizer around a slow model.
        // The LLM still sees every AMBIGUOUS segment in full context; rules-confident
        // segments don't need it (they're unambiguous by definition).
        let rule_labels: Vec<Activity> =
            segments.iter().map(|s| self.rules.categorize(s)).collect();
        let ambiguous: Vec<Segment> = segments
            .iter()
            .zip(rule_labels.iter())
            .filter(|(_, a)| **a == Activity::Unattributed)
            .map(|(s, _)| s.clone())
            .collect();
        if ambiguous.is_empty() {
            return rule_labels;
        }
        // batch ambiguous segments in chunks of 20
        let mut llm_labels: Vec<Activity> = Vec::with_capacity(ambiguous.len());
        for chunk in ambiguous.chunks(20) {
            let prompt = self.build_prompt(chunk);
            let labels = match self.caller.categorize_batch(&prompt) {
                Ok(text) => {
                    let parsed = Self::parse_labels(&text);
                    if parsed.len() == chunk.len() {
                        parsed
                    } else {
                        chunk.iter().map(|s| self.rules.categorize(s)).collect()
                    }
                }
                Err(_) => chunk.iter().map(|s| self.rules.categorize(s)).collect(),
            };
            llm_labels.extend(labels);
        }
        // merge LLM labels back into the rule_labels at the ambiguous positions
        let mut merged = rule_labels;
        let mut li = 0;
        for a in merged.iter_mut() {
            if *a == Activity::Unattributed && li < llm_labels.len() {
                *a = llm_labels[li];
                li += 1;
            }
        }
        merged
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
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("pay4what/0.1")
            .build()?;
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "max_completion_tokens": 2048,
        });
        let resp = client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://github.com/romiluz13/pay4what")
            .header("X-Title", "pay4what")
            .json(&body)
            .send()?;
        let status = resp.status();
        let text = resp.text()?;
        if !status.is_success() {
            return Err(format!("HTTP {status}: {text}").into());
        }
        let v: serde_json::Value = serde_json::from_str(&text)?;
        let msg = v
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"));
        let content = msg
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                msg.and_then(|m| m.get("reasoning"))
                    .and_then(|r| r.as_str())
            })
            .unwrap_or("");
        Ok(content.to_string())
    }
}

/// Grove gateway caller (Rom's personal gateway — OpenAI-compatible). Used when
/// GROVE_API_KEY + GROVE_BASE_URL are set. NOT the public-path default; the
/// published app uses OpenRouter. Grove is for Rom's local dogfooding.
#[cfg(feature = "categorize")]
pub struct GroveCaller {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

#[cfg(feature = "categorize")]
impl LlmCaller for GroveCaller {
    fn categorize_batch(&self, prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .user_agent("pay4what/0.1")
            .build()?;
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "max_completion_tokens": 2048,
        });
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("api-key", &self.api_key)
            .json(&body)
            .send()?;
        let status = resp.status();
        let text = resp.text()?;
        if !status.is_success() {
            return Err(format!("HTTP {status}: {text}").into());
        }
        let v: serde_json::Value = serde_json::from_str(&text)?;
        let msg = v
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"));
        let content = msg
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                msg.and_then(|m| m.get("reasoning"))
                    .and_then(|r| r.as_str())
            })
            .unwrap_or("");
        Ok(content.to_string())
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
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}…")
    }
}
