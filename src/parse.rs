//! Tolerant JSONL parser for Claude Code transcripts.
//!
//! MIRROR: ~/Dev/sql-hidden-cost/scripts/agent-usage.mjs:54-66 (field names +
//! cache separation), ccusage summary.rs (tolerant missing-field access).
//!
//! Schema is volatile (Anthropic GitHub #53516). Parse tolerantly: never hard-
//! fail on a missing field; skip unparseable lines; keep what's there.
use serde_json::Value;
use std::path::Path;

/// Per-turn token usage. Cache buckets are SEPARATE from input — never collapse.
/// MIRROR: agent-usage.mjs:54-66: input_tokens / cache_read_input_tokens /
/// cache_creation_input_tokens are distinct; `tokensRead = input + cache_read +
/// cache_creation` (but cost prices each bucket at its OWN rate).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TurnUsage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

/// One parsed JSONL line (a turn). Fields are optional — volatile schema.
#[derive(Debug, Clone, Default)]
pub struct Turn {
    pub kind: Option<String>, // type: user/assistant/system/summary/result/...
    pub timestamp: Option<String>, // RFC3339
    pub cwd: Option<String>,
    pub git_branch: Option<String>,  // gitBranch
    pub is_sidechain: bool,          // isSidechain — subagent
    pub agent_id: Option<String>,    // agentId
    pub parent_uuid: Option<String>, // parentUuid (conversation tree)
    pub usage: Option<TurnUsage>,    // message.usage (assistant turns only)
    pub tool_uses: Vec<ToolUse>,     // message.content[].tool_use
}

#[derive(Debug, Clone, Default)]
pub struct ToolUse {
    pub name: String,
    pub input: Value, // raw input (file paths, bash commands, etc.)
}

#[derive(Debug, Clone, Default)]
pub struct Session {
    pub path: std::path::PathBuf,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub turns: Vec<Turn>,
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
}

/// Read a u64 from a JSON value, defaulting to 0 on any missing/non-numeric.
fn u64_of(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

/// Parse one JSONL line into a Turn (tolerant). Returns None for unparseable.
fn parse_line(line: &str) -> Option<Turn> {
    let v: Value = serde_json::from_str(line).ok()?;
    let mut tool_uses = Vec::new();
    let mut usage = None;
    if let Some(msg) = v.get("message") {
        // usage lives on message for assistant turns. Only set usage when the
        // usage block actually exists — a turn with a model but no usage stays
        // usage=None (per spec; model-mix is derived from turns that DO have usage).
        if let (Some(model), Some(u)) = (str_of(msg, "model"), msg.get("usage")) {
            usage = Some(TurnUsage {
                model,
                input_tokens: u64_of(u, "input_tokens"),
                output_tokens: u64_of(u, "output_tokens"),
                cache_read_input_tokens: u64_of(u, "cache_read_input_tokens"),
                cache_creation_input_tokens: u64_of(u, "cache_creation_input_tokens"),
            });
        }
        // tool_use blocks live in message.content[]
        if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if block.get("type").and_then(|x| x.as_str()) == Some("tool_use") {
                    tool_uses.push(ToolUse {
                        name: block
                            .get("name")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                        input: block.get("input").cloned().unwrap_or(Value::Null),
                    });
                }
            }
        }
    }
    Some(Turn {
        kind: str_of(&v, "type"),
        timestamp: str_of(&v, "timestamp"),
        cwd: str_of(&v, "cwd"),
        git_branch: str_of(&v, "gitBranch"),
        is_sidechain: v
            .get("isSidechain")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        agent_id: str_of(&v, "agentId"),
        parent_uuid: str_of(&v, "parentUuid"),
        usage,
        tool_uses,
    })
}

/// Parse a whole session JSONL file. Tolerant: skips bad lines, never panics.
pub fn parse_session(path: &Path) -> std::io::Result<Session> {
    let text = std::fs::read_to_string(path)?;
    let mut session = Session {
        path: path.to_path_buf(),
        ..Default::default()
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(turn) = parse_line(line) else {
            continue;
        };
        // adopt session-wide fields from any line that carries them (last write wins)
        if turn.cwd.is_some() {
            session.cwd = turn.cwd.clone();
        }
        if session.git_branch.is_none() && turn.git_branch.is_some() {
            session.git_branch = turn.git_branch.clone();
        }
        if session.first_ts.is_none() && turn.timestamp.is_some() {
            session.first_ts = turn.timestamp.clone();
        }
        if turn.timestamp.is_some() {
            session.last_ts = turn.timestamp.clone();
        }
        session.turns.push(turn);
    }
    Ok(session)
}
