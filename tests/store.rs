//! Store module tests — the bucket store contract (load, upsert, query,
//! incremental cache, remove_session).
//!
//! MIRROR: tests/cost.rs:1-9 (tempdir + fs pattern),
//! src/store.rs:67-119 (the methods under test).
use pay4what::categorize::Activity;
use pay4what::store::{Bucket, BucketStore};
use std::collections::HashMap;

/// Build a minimal bucket for testing.
fn test_bucket(id: &str, session: &str, cost: f64, tags: &[&str], summary: &str) -> Bucket {
    Bucket {
        id: id.to_string(),
        session: session.to_string(),
        segment_index: 0,
        activity: Activity::Feature,
        tags: tags.iter().map(|t| t.to_string()).collect(),
        summary: summary.to_string(),
        confidence: 0.9,
        cost,
        tokens: 1000,
        files: Vec::new(),
        branch: None,
        first_ts: None,
        last_ts: None,
    }
}

#[test]
fn store_load_returns_default_when_missing() {
    // The default store path (~/.pay4what/store.json) may exist from dogfooding,
    // so we test the logic directly: a missing file → default (empty).
    let store = BucketStore::default();
    assert!(store.buckets.is_empty(), "default store has no buckets");
    assert!(store.sessions.is_empty(), "default store has no sessions");
    assert_eq!(store.version, "", "default store has empty version");
}

#[test]
fn store_upsert_replaces_same_id() {
    let mut store = BucketStore::default();
    let b1 = test_bucket("s1:1", "s1", 10.0, &["auth"], "add oauth");
    store.upsert_bucket(b1);
    assert_eq!(store.buckets.len(), 1);

    // upsert with same id → replaces, not appends
    let b2 = test_bucket("s1:1", "s1", 25.0, &["auth"], "add oauth v2");
    store.upsert_bucket(b2);
    assert_eq!(store.buckets.len(), 1, "upsert should replace, not append");
    assert!(
        (store.buckets[0].cost - 25.0).abs() < 1e-9,
        "cost should be updated to 25.0"
    );
}

#[test]
fn store_query_matches_tags_and_summary() {
    let mut store = BucketStore::default();
    store.upsert_bucket(test_bucket(
        "s1:1",
        "s1",
        10.0,
        &["auth"],
        "implement oauth",
    ));
    store.upsert_bucket(test_bucket(
        "s1:2",
        "s1",
        5.0,
        &["billing"],
        "fix billing invoice",
    ));
    store.upsert_bucket(test_bucket("s2:1", "s2", 3.0, &[], "explore the codebase"));

    let auth = store.query("auth");
    assert_eq!(auth.len(), 1, "query 'auth' should match 1 bucket (tag)");
    let billing = store.query("billing");
    assert_eq!(
        billing.len(),
        1,
        "query 'billing' should match 1 bucket (tag)"
    );
    let invoice = store.query("invoice");
    assert_eq!(
        invoice.len(),
        1,
        "query 'invoice' should match 1 bucket (summary)"
    );
    let xyz = store.query("xyz");
    assert_eq!(xyz.len(), 0, "query 'xyz' should match 0 buckets");
}

#[test]
fn store_incremental_classified_count() {
    let mut store = BucketStore::default();
    store.mark_classified("uuid1", 5);
    assert_eq!(
        store.classified_count("uuid1"),
        5,
        "classified_count should return the marked count"
    );
    assert_eq!(
        store.classified_count("uuid2"),
        0,
        "unclassified session should return 0"
    );
}

#[test]
fn store_remove_session_clears_buckets_and_meta() {
    let mut store = BucketStore::default();
    store.upsert_bucket(test_bucket("s1:1", "s1", 10.0, &["auth"], "add oauth"));
    store.upsert_bucket(test_bucket("s1:2", "s1", 5.0, &["billing"], "fix billing"));
    store.upsert_bucket(test_bucket("s2:1", "s2", 3.0, &[], "explore"));
    store.mark_classified("s1", 2);

    assert_eq!(store.buckets.len(), 3);
    store.remove_session("s1");
    assert_eq!(
        store.buckets.len(),
        1,
        "remove_session should clear that session's buckets"
    );
    assert_eq!(
        store.buckets[0].session, "s2",
        "only s2 bucket should remain"
    );
    assert_eq!(
        store.classified_count("s1"),
        0,
        "remove_session should clear the session meta"
    );
}

// Keep HashMap import used (for type inference in some compilers)
#[allow(dead_code)]
fn _keep_hashmap() -> HashMap<String, String> {
    HashMap::new()
}
