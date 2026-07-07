//! Cache-aware cost computation + bundled versioned pricing table.
//!
//! MIRROR: ~/Dev/ai-cost-compare/src/cost/tracker.ts:3-9 (costForUsage formula),
//! ~/Dev/sql-hidden-cost/scripts/agent-usage.mjs:54-66 (cache separation).
//!
//! ccusage's `models-dev-pricing.json` LACKS cache rates — pay4what bundles a
//! versioned table WITH `cacheReadPerMTok` + `cacheCreationPerMTok` per model.
//! Source: LiteLLM pricing snapshot, asOf-dated for reproducibility.
use std::collections::HashMap;

use crate::parse::{Session, TurnUsage};

/// Per-model pricing in USD per million tokens. Cache rates are FIRST-CLASS —
/// Claude reports cache_read/cache_creation separate from fresh input, and each
/// is priced at its own rate (cache_read is ~10x cheaper than input;
/// cache_creation is ~1.25x input).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_creation_per_mtok: f64,
}

/// Versioned pricing table. `as_of` dates the snapshot for reproducibility
/// (MIRROR: ai-cost-compare/pricing.json `asOf` field).
#[derive(Debug, Clone)]
pub struct PricingTable {
    pub as_of: String,
    pub models: HashMap<String, ModelPricing>,
}

/// 1h cache-creation is priced at 2.0x the INPUT rate (NOT the cache_create
/// rate). MIRROR: ccusage cost.rs:5 `CACHE_CREATE_1H_INPUT_MULTIPLIER = 2.0`.
const CACHE_CREATE_1H_INPUT_MULTIPLIER: f64 = 2.0;

/// Compute USD cost for one turn's usage. Prices each token bucket at its OWN
/// rate. NEVER double-counts: input_tokens is fresh (excludes cache), so the
/// four terms are additive.
///
/// When the 5m/1h cache-creation split is present, 5m tokens are priced at the
/// `cache_creation` rate and 1h tokens at `input * 2.0` (per ccusage). When no
/// split, all cache-creation tokens are priced at the flat `cache_creation` rate.
///
/// Unknown model -> $0 (tolerant; caller may log a warning).
pub fn cost_for_usage(usage: &TurnUsage, pricing: &PricingTable) -> f64 {
    let Some(mp) = pricing.models.get(&usage.model) else {
        return 0.0;
    };
    let per_mtok = 1_000_000.0;

    // cache-creation: use 5m/1h split if present, else flat total.
    let (cc_5m, cc_1h) = match (usage.cache_creation_5m, usage.cache_creation_1h) {
        (Some(m5), Some(h1)) => (m5 as f64, h1 as f64),
        _ => (usage.cache_creation_input_tokens as f64, 0.0),
    };

    (usage.input_tokens as f64 / per_mtok) * mp.input_per_mtok
        + (usage.output_tokens as f64 / per_mtok) * mp.output_per_mtok
        + (usage.cache_read_input_tokens as f64 / per_mtok) * mp.cache_read_per_mtok
        + (cc_5m / per_mtok) * mp.cache_creation_per_mtok
        + (cc_1h / per_mtok) * (mp.input_per_mtok * CACHE_CREATE_1H_INPUT_MULTIPLIER)
}

/// Sum cost across all turns in a session that carry usage. Turns without usage
/// contribute $0 (they're typically user/system turns).
pub fn cost_for_session(session: &Session, pricing: &PricingTable) -> f64 {
    session
        .turns
        .iter()
        .filter_map(|t| t.usage.as_ref())
        .map(|u| cost_for_usage(u, pricing))
        .sum()
}

/// Bundled default pricing table (LiteLLM-sourced snapshot, asOf 2026-07-07).
/// Includes cache rates — the gap ccusage's table has.
///
/// Rates are Anthropic API list prices for the Claude models seen in Rom's
/// local transcripts (sonnet-4-6, opus-4-8, haiku-4-5). Verify against
/// anthropic.com/pricing before publishing claims (AGENTS.md: $0 may be a bug).
pub fn bundled_pricing() -> PricingTable {
    PricingTable {
        as_of: "2026-07-07".into(),
        models: [
            ("claude-sonnet-4-6", 3.0, 15.0, 0.30, 3.75),
            ("claude-opus-4-8", 15.0, 75.0, 1.50, 18.75),
            ("claude-haiku-4-5", 0.80, 4.0, 0.08, 1.0),
            ("claude-haiku-4-5-20251001", 0.80, 4.0, 0.08, 1.0),
        ]
        .into_iter()
        .map(|(m, i, o, cr, cc)| {
            (
                m.to_string(),
                ModelPricing {
                    input_per_mtok: i,
                    output_per_mtok: o,
                    cache_read_per_mtok: cr,
                    cache_creation_per_mtok: cc,
                },
            )
        })
        .collect(),
    }
}
