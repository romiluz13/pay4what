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
///
/// `cache_creation` can carry a 5m/1h split (ccusage cost.rs:5 — 1h priced at
/// input*2.0, NOT the flat cache_create rate). When the split is present,
/// `cache_creation_input_tokens` is the FLAT total and the split overrides it
/// for cost math (the same tokens are never counted both ways).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TurnUsage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    /// Optional 5m/1h breakdown of cache_creation. When present, cost math uses
    /// these instead of the flat total (1h priced at input*2.0 per ccusage).
    pub cache_creation_5m: Option<u64>,
    pub cache_creation_1h: Option<u64>,
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
    pub text: Option<String>,        // message.content as string or first text block
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

/// Detect injected command output / version banners that Claude Code wraps into
/// user-role text blocks but are NOT real user requests. Heuristic: a leading
/// `$ ` shell prompt, a `Version:` banner line, or a `<command-*>` wrapper.
/// Conservative — only filters obvious noise so real multi-line user messages
/// (which often start with a word) pass through.
fn looks_like_command_output(s: &str) -> bool {
    let trimmed = s.trim_start();
    trimmed.starts_with("$ ")
        || trimmed.starts_with("Version:")
        || trimmed.starts_with("<command-")
        // multi-line where line 1 is a shell command (cd/npm/cargo/git/python)
        && trimmed.lines().next().is_some_and(|first| {
            let f = first.trim();
            f.starts_with("cd ") || f.starts_with("npm ") || f.starts_with("cargo ")
                || f.starts_with("git ") || f.starts_with("python ") || f.starts_with("node ")
        })
}

/// Parse one JSONL line into a Turn (tolerant). Returns None for unparseable.
fn parse_line(line: &str) -> Option<Turn> {
    let v: Value = serde_json::from_str(line).ok()?;
    let mut tool_uses = Vec::new();
    let mut usage = None;
    let mut text = None;
    if let Some(msg) = v.get("message") {
        // usage lives on message for assistant turns. Only set usage when the
        // usage block actually exists — a turn with a model but no usage stays
        // usage=None (per spec; model-mix is derived from turns that DO have usage).
        if let (Some(model), Some(u)) = (str_of(msg, "model"), msg.get("usage")) {
            // 5m/1h cache-creation split (ccusage cost.rs:5 — 1h priced at input*2.0)
            let cc_split = u.get("cache_creation").and_then(|c| c.as_object());
            let (cc_5m, cc_1h) = if let Some(obj) = cc_split {
                (
                    obj.get("ephemeral_5m_input_tokens")
                        .and_then(|x| x.as_u64()),
                    obj.get("ephemeral_1h_input_tokens")
                        .and_then(|x| x.as_u64()),
                )
            } else {
                (None, None)
            };
            usage = Some(TurnUsage {
                model,
                input_tokens: u64_of(u, "input_tokens"),
                output_tokens: u64_of(u, "output_tokens"),
                cache_read_input_tokens: u64_of(u, "cache_read_input_tokens"),
                cache_creation_input_tokens: u64_of(u, "cache_creation_input_tokens"),
                cache_creation_5m: cc_5m,
                cache_creation_1h: cc_1h,
            });
        }
        // tool_use blocks live in message.content[]
        if let Some(content) = msg.get("content") {
            // content can be a string (user request) or an array of blocks.
            // Skip injected markers (<local-command-*>, <command-message>, etc.)
            // and multi-line command output (lines starting with $ or containing
            // 'Version:' boot lines) — these are NOT user requests.
            if let Some(s) = content.as_str() {
                if !s.is_empty() && !s.starts_with('<') && !looks_like_command_output(s) {
                    text = Some(s.to_string());
                }
            } else if let Some(arr) = content.as_array() {
                // first text block (skip tool_result continuations AND injected
                // local-command markers — Claude Code wraps command output/caveats
                // in <local-command-*> tags; those are NOT user requests)
                for block in arr {
                    if block.get("type").and_then(|x| x.as_str()) == Some("text")
                        && let Some(t) = block.get("text").and_then(|x| x.as_str())
                        && !t.is_empty()
                        && !t.starts_with("<local-command-")
                        && !t.starts_with("<command-")
                        && !looks_like_command_output(t)
                    {
                        text = Some(t.to_string());
                        break;
                    }
                }
                for block in arr {
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
        text,
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
