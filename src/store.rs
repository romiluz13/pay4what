//! Bucket store — the persisted, queryable index of classified segments.
//!
//! ARCHITECTURE (user-driven redesign 2026-07-08):
//! Instead of re-classifying whole sessions at query time, pay4what classifies
//! each segment ONCE (with the LLM reading the full session arc), persists a
//! rich record {activity, tags, summary, confidence, cost, ...} into a local
//! bucket store. At query time ("how much did the login bug cost?"), you query
//! the buckets — no re-reading 7M tokens of raw session context.
//!
//! Location: ~/.pay4what/store.json (global, regenerable — delete to re-scan).
//! Incremental: per-session, tracks segments_classified; only new segments
//! hit the LLM on subsequent runs.

use crate::categorize::Activity;
use std::collections::HashMap;
use std::path::PathBuf;

/// One classified segment = one bucket. The queryable unit.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Bucket {
    /// "{session_uuid}:{segment_index}" — stable identifier
    pub id: String,
    pub session: String,
    pub segment_index: usize,
    pub activity: Activity,
    /// 1-3 short lowercase keywords (e.g. ["auth", "oauth"])
    pub tags: Vec<String>,
    /// One-line description of what this segment did
    pub summary: String,
    /// LLM confidence 0.0-1.0
    pub confidence: f64,
    pub cost: f64,
    pub tokens: u64,
    pub files: Vec<String>,
    pub branch: Option<String>,
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
}

/// Per-session metadata for incremental classification.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SessionMeta {
    pub session_uuid: String,
    /// How many segments have been classified (0-indexed count).
    /// Segments with index >= this are NEW and need classification.
    pub segments_classified: usize,
    pub last_modified: String,
}

/// The persisted store. Loaded from ~/.pay4what/store.json, modified, saved back.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct BucketStore {
    pub version: String,
    pub as_of: String,
    /// Keyed by session uuid (the JSONL filename stem).
    pub sessions: HashMap<String, SessionMeta>,
    pub buckets: Vec<Bucket>,
}

impl BucketStore {
    /// Load from the default location (~/.pay4what/store.json).
    /// Returns an empty store if the file doesn't exist (first run).
    pub fn load() -> Self {
        let path = store_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save to ~/.pay4what/store.json (creates the dir if needed).
    pub fn save(&self) -> std::io::Result<()> {
        let path = store_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// How many segments are already classified for a session?
    pub fn classified_count(&self, session_uuid: &str) -> usize {
        self.sessions
            .get(session_uuid)
            .map(|m| m.segments_classified)
            .unwrap_or(0)
    }

    /// Record that a session has been classified up to N segments.
    pub fn mark_classified(&mut self, session_uuid: &str, count: usize) {
        let meta = SessionMeta {
            session_uuid: session_uuid.to_string(),
            segments_classified: count,
            last_modified: now_iso(),
        };
        self.sessions.insert(session_uuid.to_string(), meta);
    }

    /// Append a new bucket (replacing any existing bucket with the same id).
    pub fn upsert_bucket(&mut self, bucket: Bucket) {
        // remove any existing bucket with the same id (re-classification)
        self.buckets.retain(|b| b.id != bucket.id);
        self.buckets.push(bucket);
    }

    /// Query buckets by substring match on tags + summary.
    /// Returns matching buckets sorted by cost descending.
    pub fn query(&self, phrase: &str) -> Vec<&Bucket> {
        let p = phrase.to_lowercase();
        self.buckets
            .iter()
            .filter(|b| {
                b.summary.to_lowercase().contains(&p)
                    || b.tags.iter().any(|t| t.to_lowercase().contains(&p))
            })
            .collect()
    }

    /// Remove all buckets for a session (for re-classification).
    pub fn remove_session(&mut self, session_uuid: &str) {
        self.buckets.retain(|b| b.session != session_uuid);
        self.sessions.remove(session_uuid);
    }
}

fn store_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pay4what")
        .join("store.json")
}

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch:{secs}")
}

/// Extract the session uuid (filename stem) from a session file path.
pub fn session_uuid(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}
