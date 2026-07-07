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

/// Compute USD cost for one turn's usage. Prices each token bucket at its OWN
/// rate. NEVER double-counts: input_tokens is fresh (excludes cache), so the
/// four terms are additive.
///
/// Unknown model -> $0 (tolerant; caller may log a warning). This matches the
/// handoff's "treat $0/empty as suspect" correction — unknown is NOT free, it's
/// unpriced; the real CLI will surface a warning + skip the turn's cost.
pub fn cost_for_usage(usage: &TurnUsage, pricing: &PricingTable) -> f64 {
    let Some(mp) = pricing.models.get(&usage.model) else {
        return 0.0;
    };
    let per_mtok = 1_000_000.0;
    (usage.input_tokens as f64 / per_mtok) * mp.input_per_mtok
        + (usage.output_tokens as f64 / per_mtok) * mp.output_per_mtok
        + (usage.cache_read_input_tokens as f64 / per_mtok) * mp.cache_read_per_mtok
        + (usage.cache_creation_input_tokens as f64 / per_mtok) * mp.cache_creation_per_mtok
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
