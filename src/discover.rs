//! Transcript discovery — resolve Claude Code project dirs and find JSONL sessions.
//!
//! MIRROR: codeburn src/providers/claude.ts (expandHome + dedupeResolved +
//! CLAUDE_CONFIG_DIRS precedence), ccusage rust/crates/ccusage/src (project dir
//! resolution). Claude Code encodes cwd as a path-with-dashes directory name
//! under ~/.claude/projects (or each CLAUDE_CONFIG_DIRS entry).
//!
//! ARCHITECTURE (verified 2026-07-07 against Rom's 1,885 local files):
//!   ~/.claude/projects/<enc-cwd>/<session-uuid>.jsonl          <- top-level parent
//!   ~/.claude/projects/<enc-cwd>/<session-uuid>/subagents/agent-<id>.jsonl  <- subagent
//!   ~/.claude/projects/<enc-cwd>/<session-uuid>/subagents/workflows/wf_*/journal.jsonl  <- NOT a transcript (skip)
use std::path::{Path, PathBuf};

/// Resolve every config dir to scan, in priority order with duplicates removed.
fn config_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(list) = std::env::var("CLAUDE_CONFIG_DIRS") {
        for sep in [':', ';'] {
            if list.contains(sep) {
                dirs.extend(list.split(sep).filter(|s| !s.is_empty()).map(PathBuf::from));
                break;
            }
        }
        if dirs.is_empty() && !list.is_empty() {
            dirs.push(PathBuf::from(list));
        }
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".claude").join("projects"));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|p| {
        let key = match p.canonicalize() {
            Ok(c) => c,
            Err(_) => p.clone(),
        };
        seen.insert(key)
    });
    dirs
}

/// Discover all top-level `*.jsonl` session files under `<root>/*/*.jsonl`.
/// These are PARENT sessions (isSidechain:false). Sorted for deterministic output.
pub fn discover_sessions(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let proj = entry.path();
            if !proj.is_dir() {
                continue;
            }
            if let Ok(sess_entries) = std::fs::read_dir(&proj) {
                for s in sess_entries.flatten() {
                    let p = s.path();
                    if p.extension().is_some_and(|e| e == "jsonl") {
                        out.push(p);
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// Discover subagent transcript files for a given top-level session.
///
/// Given `.../<enc-cwd>/<session-uuid>.jsonl`, looks for a sibling directory
/// `.../<enc-cwd>/<session-uuid>/subagents/agent-*.jsonl`. Skips
/// `workflows/wf_*/journal.jsonl` (loop-engine journals, not transcripts).
///
/// Returns paths sorted for deterministic output.
pub fn discover_subagents(session_path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    // session_path = .../<enc-cwd>/<uuid>.jsonl -> session dir = .../<enc-cwd>/<uuid>/
    let Some(stem) = session_path.file_stem() else {
        return out;
    };
    let session_dir = session_path.with_file_name(stem);
    let subagents_dir = session_dir.join("subagents");
    if !subagents_dir.is_dir() {
        return out;
    }
    if let Ok(entries) = std::fs::read_dir(&subagents_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            // only agent-*.jsonl (skip workflows/ subdirs, .meta.json, etc.)
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if name.starts_with("agent-") && p.extension().is_some_and(|e| e == "jsonl") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Discover across ALL config dirs (the real entry point). Returns top-level
/// parent session files only — use `discover_subagents` per session to get the
/// full tree.
pub fn discover_all() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in config_dirs() {
        out.extend(discover_sessions(&d));
    }
    out.sort();
    out
}
