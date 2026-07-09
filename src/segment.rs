//! Segmentation — split a session at user-turn boundaries.
//!
//! A segment = the work done in response to one user message: the user turn
//! plus all subsequent assistant/system/attachment turns (and their tool calls)
//! until the next user turn. This is the unit of categorization.
//!
//! Subagent spend is handled separately (separate files, discovered via
//! discover_subagents) — segmentation operates on a single session file.
//!
//! MIRROR: the handoff's "segment at user-turn / subagent / compact boundaries"
//! design. User-turn boundaries are the primary split; compact boundaries
//! (type:summary) start a new segment too.
use std::collections::BTreeSet;

use crate::cost::PricingTable;
use crate::parse::{Session, Turn};

/// One segment of work: a user message + the assistant turns that answer it.
#[derive(Debug, Clone)]
pub struct Segment {
    /// 1-bounded index of this segment within the session (1-based for display).
    pub index: usize,
    /// The user message that initiated this segment (empty for prefatory).
    pub user_message: String,
    /// All turns in this segment (the initiating user turn + assistant/tool turns).
    pub turns: Vec<Turn>,
    /// USD cost summed across assistant turns with usage.
    pub cost: f64,
    /// gitBranch captured for this segment (from any turn that carries it).
    pub git_branch: Option<String>,
    /// File paths touched by Edit/Write/FileWriteTool tool_use in this segment.
    pub touched_files: BTreeSet<String>,
    /// Tool-call verbs used in this segment (Edit, Bash, Read, Grep, ...).
    pub tool_verbs: BTreeSet<String>,
}

impl Segment {
    /// Total tokens consumed by this segment (input + output + cache_read +
    /// cache_creation across all LOGICAL assistant turns — chunk-deduped so
    /// thinking+text+tool_use chunks of one turn count once, not 3×).
    pub fn total_tokens(&self) -> u64 {
        crate::cost::dedup_usage_iter(&self.turns)
            .map(|u| {
                u.input_tokens
                    + u.output_tokens
                    + u.cache_read_input_tokens
                    + u.cache_creation_input_tokens
            })
            .sum()
    }
}

/// Edit/Write tool names whose `input` carries a file path.
const FILE_EDIT_TOOLS: &[&str] = &[
    "Edit",
    "Write",
    "FileEditTool",
    "FileWriteTool",
    "NotebookEdit",
    "MultiEditTool",
];

fn extract_file_path(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    // Edit/Write/MultiEdit use `file_path`; some tools use `path`.
    if !FILE_EDIT_TOOLS.contains(&tool_name) {
        return None;
    }
    input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|p| !is_memory_or_session_file(p))
}

/// Claude Code maintains memory files under `.claude/projects/<enc-cwd>/memory/`
/// (MEMORY.md, project_*.md) and journal files under `subagents/workflows/`.
/// These are bookkeeping the agent reads/writes, NOT source files the user's
/// task targets. Exclude them from cost-by-file so the table reflects actual
/// work, not memory maintenance.
fn is_memory_or_session_file(path: &str) -> bool {
    (path.contains("/.claude/projects/")
        && (path.contains("/memory/") || path.contains("/subagents/")))
        || path.contains("/subagents/workflows/")
}

/// Segment a session at user-turn boundaries. A new segment starts at each
/// `type:user` turn (or `type:summary` compact boundary). Prefatory turns
/// before the first user turn attach to the first segment.
pub fn segment_session(session: &Session) -> Vec<Segment> {
    let pricing = crate::cost::bundled_pricing();
    segment_session_with_pricing(session, &pricing)
}

/// Segment with an explicit pricing table (for testing / custom pricing).
pub fn segment_session_with_pricing(session: &Session, pricing: &PricingTable) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut current: Option<Segment> = None;

    for turn in &session.turns {
        let is_user_boundary = turn.kind.as_deref() == Some("user");
        let is_compact_boundary = turn.kind.as_deref() == Some("summary");
        let is_first = current.is_none();

        // Start a new segment at a user turn ONLY if the current segment already
        // has a user message (complete prior segment). Prefatory turns before
        // the first user turn attach to the first user segment. Compact
        // (summary) boundaries always start a new segment.
        let close_and_start = if is_first {
            false
        } else if is_compact_boundary {
            true
        } else if is_user_boundary {
            current
                .as_ref()
                .is_some_and(|seg| !seg.user_message.is_empty())
        } else {
            false
        };

        if close_and_start && let Some(mut seg) = current.take() {
            finalize_segment(&mut seg, pricing);
            segments.push(seg);
        }

        let seg = current.get_or_insert_with(|| Segment {
            index: segments.len() + 1,
            user_message: String::new(),
            turns: Vec::new(),
            cost: 0.0,
            git_branch: None,
            touched_files: BTreeSet::new(),
            tool_verbs: BTreeSet::new(),
        });

        // capture the user message text from the initiating user turn
        if turn.kind.as_deref() == Some("user")
            && let Some(msg) = turn_message_text(turn)
            && seg.user_message.is_empty()
        {
            seg.user_message = msg;
        }
        if seg.git_branch.is_none() && turn.git_branch.is_some() {
            seg.git_branch = turn.git_branch.clone();
        }
        // collect tool verbs + touched files
        for tu in &turn.tool_uses {
            seg.tool_verbs.insert(tu.name.clone());
            if let Some(path) = extract_file_path(&tu.name, &tu.input) {
                seg.touched_files.insert(path);
            }
        }
        seg.turns.push(turn.clone());
    }

    if let Some(mut seg) = current {
        finalize_segment(&mut seg, pricing);
        segments.push(seg);
    }
    // renumber after potential compact-boundary splits
    for (i, seg) in segments.iter_mut().enumerate() {
        seg.index = i + 1;
    }
    segments
}

fn finalize_segment(seg: &mut Segment, pricing: &PricingTable) {
    seg.cost = crate::cost::dedup_usage_iter(&seg.turns)
        .map(|u| crate::cost::cost_for_usage(u, pricing))
        .sum();
}

/// Extract the user message text from a user turn. Returns the captured
/// `message.content` string (or first text block). Tool-result continuations
/// (content is an array of tool_result with no text block) yield None — they
/// are continuations, not new requests.
fn turn_message_text(turn: &Turn) -> Option<String> {
    turn.text.clone()
}
