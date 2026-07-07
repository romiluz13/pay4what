//! Transcript discovery — resolve Claude Code project dirs and find JSONL sessions.
//!
//! MIRROR: codeburn src/providers/claude.ts (expandHome + dedupeResolved +
//! CLAUDE_CONFIG_DIRS precedence), ccusage rust/crates/ccusage/src (project dir
//! resolution). Claude Code encodes cwd as a path-with-dashes directory name
//! under ~/.claude/projects (or each CLAUDE_CONFIG_DIRS entry).
use std::path::{Path, PathBuf};

/// Resolve every config dir to scan, in priority order with duplicates removed.
/// Precedence: `CLAUDE_CONFIG_DIRS` (path-delimiter list), then `~/.claude`.
/// MIRROR: codeburn providers/claude.ts `dedupeResolved`.
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
    // dedupe by resolved (canonical) path
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

/// Discover all `*.jsonl` session files under `<config_dir>/*/*.jsonl`.
/// Returns paths sorted for deterministic output.
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

/// Discover across ALL config dirs (the real entry point).
pub fn discover_all() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in config_dirs() {
        out.extend(discover_sessions(&d));
    }
    out.sort();
    out
}
