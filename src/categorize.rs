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
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    Default,
)]
pub enum Activity {
    Feature,
    Bugfix,
    Migration,
    Refactor,
    Debugging,
    Exploration,
    Planning,
    #[default]
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

/// Rich LLM categorization record — one per segment. The full output that goes
/// into the bucket store: not just an activity label, but tags + summary +
/// confidence. This is what makes the bucket queryable ("how much did the
/// login bug cost?" -> match on tags+summary).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct RichRecord {
    pub activity: Activity,
    pub tags: Vec<String>,
    pub summary: String,
    pub confidence: f64,
}

/// Categorizer trait — rules or LLM both implement this.
pub trait Categorizer {
    fn categorize(&self, segment: &Segment) -> Activity;
    /// Categorize a batch (default: per-segment; LLM overrides to batch).
    fn categorize_batch(&self, segments: &[Segment]) -> Vec<Activity> {
        segments.iter().map(|s| self.categorize(s)).collect()
    }
    /// Categorize with rich output {activity, tags, summary, confidence}.
    /// Default: rules-only records (no tags, no real summary, confidence=0).
    /// LLM overrides this to read the full session arc in one call.
    fn categorize_rich(&self, segments: &[Segment]) -> Vec<RichRecord> {
        segments
            .iter()
            .map(|s| RichRecord {
                activity: self.categorize(s),
                tags: Vec::new(),
                summary: s.user_message.chars().take(80).collect(),
                confidence: 0.0,
            })
            .collect()
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
pub trait LlmCaller: Send {
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

    /// Build the LLM prompt for a session's segments. One call per session —
    /// the LLM sees the FULL arc (user messages + tool verbs + files + branch +
    /// assistant text replies) so it can detect topic pivots and compaction
    /// boundaries. Returns rich records {activity, tags, summary, confidence}.
    pub fn build_prompt(&self, segments: &[Segment]) -> String {
        let items: Vec<String> = segments
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let files: Vec<String> = s.touched_files.iter().take(8).cloned().collect();
                let verbs: Vec<String> = s.tool_verbs.iter().cloned().collect();
                // collect the first assistant text reply in the segment
                let assistant_text = s
                    .turns
                    .iter()
                    .filter(|t| t.kind.as_deref() == Some("assistant"))
                    .find_map(|t| t.text.as_ref().map(|t| truncate(t, 200)))
                    .unwrap_or_default();
                format!(
                    "[{}] user: {:?} | branch: {:?} | tools: {} | files: {} | assistant: {:?}",
                    i + 1,
                    truncate(&s.user_message, 300),
                    s.git_branch,
                    verbs.join(","),
                    files.join(","),
                    assistant_text,
                )
            })
            .collect();
        format!(
            "Categorize each segment of this Claude Code session. For EACH segment return a JSON object with:\n\
             - activity: one of feature, bugfix, migration, refactor, debugging, exploration, planning, unattributed\n\
             - tags: 1-3 short lowercase keywords (e.g. [\"auth\",\"oauth\"])\n\
             - summary: one-line description of what this segment actually did\n\
             - confidence: 0.0 to 1.0\n\
             Read the full arc — sessions pivot topics mid-stream. If a segment starts as feature work but\n\
             becomes debugging, label it by what the segment ACTUALLY did.\n\
             Segments:\n{}\n\
             Return JSON: {{\"results\":[{{\"activity\":\"feature\",\"tags\":[\"auth\"],\"summary\":\"implement oauth\",\"confidence\":0.9}}, ...]}}",
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

    /// Parse a rich LLM response ({"results":[{activity,tags,summary,confidence},...]})
    /// into a Vec<RichRecord>. Strips code fences + extracts the JSON block.
    /// Falls back to empty vec on parse failure (caller falls back to rules).
    pub fn parse_rich(text: &str) -> Vec<RichRecord> {
        let text = text.trim();
        let text = text
            .strip_prefix("```")
            .map(|t| t.strip_prefix("json").unwrap_or(t).trim_start())
            .unwrap_or(text);
        let text = text
            .strip_suffix("```")
            .map(|t| t.trim_end())
            .unwrap_or(text);
        let json_str = if let Some(start) = text.find('{')
            && let Some(end) = text.rfind('}')
            && start < end
        {
            &text[start..=end]
        } else {
            return Vec::new();
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
            return Vec::new();
        };
        let Some(arr) = v.get("results").and_then(|l| l.as_array()) else {
            return Vec::new();
        };
        arr.iter()
            .map(|obj| {
                let activity = obj
                    .get("activity")
                    .and_then(|a| a.as_str())
                    .map(Activity::parse)
                    .unwrap_or(Activity::Unattributed);
                let tags = obj
                    .get("tags")
                    .and_then(|t| t.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|t| t.as_str())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                let summary = obj
                    .get("summary")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                let confidence = obj
                    .get("confidence")
                    .and_then(|c| c.as_f64())
                    .unwrap_or(0.5);
                RichRecord {
                    activity,
                    tags,
                    summary,
                    confidence,
                }
            })
            .collect()
    }

    /// Categorize all segments, returning rich records {activity, tags, summary,
    /// confidence}. The LLM sees the full session arc. Chunked at 20 segments
    /// per call to keep prompts manageable (a 50-segment session = 3 calls).
    /// Every segment goes through the LLM — no rules-pre-tagging (the LLM IS
    /// the product). Falls back to rules-only records on error or count mismatch.
    pub fn categorize_rich(&self, segments: &[Segment]) -> Vec<RichRecord> {
        if segments.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(segments.len());
        for chunk in segments.chunks(20) {
            let prompt = self.build_prompt(chunk);
            let records = match self.caller.categorize_batch(&prompt) {
                Ok(text) => {
                    let parsed = Self::parse_rich(&text);
                    if parsed.len() == chunk.len() {
                        parsed
                    } else {
                        // count mismatch — rules fallback for this chunk
                        chunk
                            .iter()
                            .map(|s| RichRecord {
                                activity: self.rules.categorize(s),
                                tags: Vec::new(),
                                summary: s.user_message.chars().take(80).collect(),
                                confidence: 0.0,
                            })
                            .collect()
                    }
                }
                Err(_) => chunk
                    .iter()
                    .map(|s| RichRecord {
                        activity: self.rules.categorize(s),
                        tags: Vec::new(),
                        summary: s.user_message.chars().take(80).collect(),
                        confidence: 0.0,
                    })
                    .collect(),
            };
            out.extend(records);
        }
        out
    }
}

impl Categorizer for LlmCategorizer {
    fn categorize(&self, seg: &Segment) -> Activity {
        // single-segment path: batch of one
        let labels = self.categorize_batch(std::slice::from_ref(seg));
        labels.into_iter().next().unwrap_or(Activity::Unattributed)
    }

    fn categorize_batch(&self, segments: &[Segment]) -> Vec<Activity> {
        // Delegate to categorize_rich — the LLM sees every segment in full arc
        // context and returns rich records. We extract just the activity here.
        // No rules-pre-tagging: the LLM IS the product (user's explicit call).
        self.categorize_rich(segments)
            .into_iter()
            .map(|r| r.activity)
            .collect()
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
