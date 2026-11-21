//! Pricing table + cost calculator.
//!
//! Cost is computed at ingest time by a pure, unit-testable function
//! (`CostCalculator`), looking up per-million-token rates with model-key
//! normalization + prefix fallback, and cache heuristics when cache rates are
//! unspecified. A whole model with no rate ⇒ cost 0 (recorded for later top-up,
//! freeze + top-up zero-cost only). Rates are `rust_decimal::Decimal`
//! for precision; the DB column is TEXT, the DTO exposes `f64`.

use std::collections::HashMap;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use crate::error::{AppError, AppResult};
use crate::model::{CostBreakdown, PricingEntry, TokenCounts};

/// Per-million-token USD rates for one model.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPricing {
    pub model_key: String,
    pub display_name: String,
    pub input: Decimal,
    pub output: Decimal,
    pub cache_read: Decimal,
    pub cache_creation: Decimal,
    pub is_builtin: bool,
}

impl ModelPricing {
    /// Convert to the boundary DTO (f64 rates).
    pub fn to_entry(&self) -> PricingEntry {
        PricingEntry {
            model_key: self.model_key.clone(),
            display_name: self.display_name.clone(),
            input_per_million: self.input.to_f64().unwrap_or(0.0),
            output_per_million: self.output.to_f64().unwrap_or(0.0),
            cache_read_per_million: self.cache_read.to_f64().unwrap_or(0.0),
            cache_creation_per_million: self.cache_creation.to_f64().unwrap_or(0.0),
            is_builtin: self.is_builtin,
        }
    }

    /// Parse from a DTO (UI writes back f64 → Decimal).
    pub fn from_entry(e: &PricingEntry) -> AppResult<Self> {
        let d = |f: f64| AppResult::Ok(Decimal::try_from(f)?);
        Ok(Self {
            model_key: e.model_key.clone(),
            display_name: e.display_name.clone(),
            input: d(e.input_per_million)?,
            output: d(e.output_per_million)?,
            cache_read: d(e.cache_read_per_million)?,
            cache_creation: d(e.cache_creation_per_million)?,
            is_builtin: e.is_builtin,
        })
    }
}

/// In-memory pricing book keyed by normalized model key (used at ingest).
#[derive(Debug, Clone, Default)]
pub struct PricingBook {
    by_key: HashMap<String, ModelPricing>,
}

impl PricingBook {
    pub fn from_iter<I: IntoIterator<Item = ModelPricing>>(iter: I) -> Self {
        let mut by_key = HashMap::new();
        for p in iter {
            by_key.insert(crate::model::normalize_model_key(&p.model_key), p);
        }
        Self { by_key }
    }

    /// Resolve a runtime model string to usable rates:
    /// normalization candidates → exact → prefix fallback. Cache rates left at
    /// zero are filled with the documented heuristics (cache write ≈ 1.25×
    /// input, cache read ≈ 0.1× input). Returns `None` if no model matches at
    /// all (⇒ zero-cost placeholder).
    ///
    /// When nothing in the book matches, the model's published list price from
    /// `llm-prices` is used before giving up, so a model released after the
    /// book was seeded or refreshed is billed instead of recorded at $0. Rows
    /// in the book (user edits, LiteLLM refresh) always win.
    pub fn resolve(&self, model: &str) -> Option<ResolvedRate> {
        for candidate in normalization_candidates(model) {
            if let Some(p) = self.by_key.get(&candidate) {
                return Some(ResolvedRate::from_pricing(p));
            }
        }
        listed_price(model)
            .or_else(|| listed_price(&crate::model::normalize_model_key(model)))
            .map(|p| ResolvedRate::from_pricing(&p))
    }
}

/// Resolved, ingest-ready rates with cache heuristics applied.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedRate {
    pub input: Decimal,
    pub output: Decimal,
    pub cache_read: Decimal,
    pub cache_creation: Decimal,
}

impl ResolvedRate {
    fn from_pricing(p: &ModelPricing) -> Self {
        // Cache heuristics only kick in when the rate is unset (0).
        let zero = Decimal::ZERO;
        let cache_read = if p.cache_read == zero {
            p.input * dec("0.1")
        } else {
            p.cache_read
        };
        let cache_creation = if p.cache_creation == zero {
            p.input * dec("1.25")
        } else {
            p.cache_creation
        };
        Self {
            input: p.input,
            output: p.output,
            cache_read,
            cache_creation,
        }
    }
}

/// Pure cost calculator (unit-testable). `rate` is per 1M tokens.
pub struct CostCalculator;

impl CostCalculator {
    /// Compute the cost breakdown for one request. `None` rate ⇒ all-zero
    /// (model missing from book) — the row is stored as a zero-cost placeholder.
    pub fn calc(tokens: TokenCounts, rate: Option<ResolvedRate>) -> CostBreakdown {
        let Some(rate) = rate else {
            return CostBreakdown::default();
        };
        let per_token = |rate: Decimal, tokens: u32| {
            // rate is USD per 1M tokens → cost = rate * tokens / 1_000_000.
            rate * Decimal::from(tokens) / Decimal::from(1_000_000)
        };
        CostBreakdown::from_buckets(
            per_token(rate.input, tokens.input),
            per_token(rate.output, tokens.output),
            per_token(rate.cache_read, tokens.cache_read),
            per_token(rate.cache_creation, tokens.cache_creation),
        )
    }
}

fn dec(s: &str) -> Decimal {
    // Used only on compile-time string literals (built-in seed + heuristic
    // coefficients + tests). A typo must panic at startup, not silently zero
    // the rate and skew every cost calculation.
    Decimal::from_str_exact(s).expect("seed literal: not a valid decimal")
}

/// Ordered candidates to try: full normalized key, then progressively shorter
/// `-`-delimited prefixes (prefix fallback).
fn normalization_candidates(model: &str) -> Vec<String> {
    let norm = crate::model::normalize_model_key(model);
    let mut out = vec![norm.clone()];
    let parts: Vec<&str> = norm.split('-').collect();
    // build prefixes by dropping trailing segments (keep ≥1 segment)
    for end in (1..parts.len()).rev() {
        let prefix = parts[..end].join("-");
        if !out.contains(&prefix) {
            out.push(prefix);
        }
    }
    out
}

/// A model's published list price from `llm-prices` as a `ModelPricing` row.
/// Cache writes map to the 5-minute lifetime, which is what a single
/// `cache_creation` bucket bills at. A column the provider does not list stays
/// 0, so the cache heuristics in `ResolvedRate` apply as for any other row.
fn listed_price(model: &str) -> Option<ModelPricing> {
    use llm_prices::Rate;
    let p = llm_prices::lookup(model)?;
    let rate = |r: Rate<'_>| p.rate_decimal(r).unwrap_or(Decimal::ZERO);
    Some(ModelPricing {
        model_key: p.model.to_string(),
        display_name: p.model.to_string(),
        input: p.rate_decimal(Rate::Input)?,
        output: rate(Rate::Output),
        cache_read: rate(Rate::CacheRead),
        cache_creation: rate(Rate::CacheWrite("5m")),
        is_builtin: true,
    })
}

/// A seed row taken from the `llm-prices` table, so it carries the provider's
/// current list price rather than a hand-copied one. Panics if the model is
/// not in the table: a seed key that stops resolving must fail tests, not
/// silently bill at $0.
fn listed_row(key: &str, name: &str) -> ModelPricing {
    let mut p = listed_price(key).unwrap_or_else(|| panic!("seed model {key} not in llm-prices"));
    p.model_key = key.to_string();
    p.display_name = name.to_string();
    p
}

/// A small built-in seed so cost calc works offline before the LiteLLM fetch.
/// These are **bootstrap placeholders** — refresh via LiteLLM or
/// edit in the Pricing UI; treat values as approximate.
///
/// `glm-5.2` is included because it is the transit model used via CC-Switch in
/// the Claude Code session-log sample.
pub fn builtin_seed() -> Vec<ModelPricing> {
    let row = |key: &str, name: &str, i: &str, o: &str, cr: &str, cc: &str| ModelPricing {
        model_key: key.to_string(),
        display_name: name.to_string(),
        input: dec(i),
        output: dec(o),
        cache_read: dec(cr),
        cache_creation: dec(cc),
        is_builtin: true,
    };
    // (key, display, input, output, cache_read/1M, cache_creation/1M) — USD.
    vec![
        // The user's transit model via CC-Switch. Placeholder price, verify/override.
        row("glm-5.2", "GLM 5.2", "0.60", "2.20", "0.06", "0.75"),
        // Anthropic Claude family: current list prices from llm-prices.
        listed_row("claude-fable-5", "Claude Fable 5"),
        listed_row("claude-opus-4-8", "Claude Opus 4.8"),
        listed_row("claude-sonnet-5", "Claude Sonnet 5"),
        listed_row("claude-haiku-4-5", "Claude Haiku 4.5"),
        row(
            "claude-3-5-haiku",
            "Claude 3.5 Haiku",
            "0.80",
            "4.00",
            "0.08",
            "1.00",
        ),
        row(
            "claude-3-5-sonnet",
            "Claude 3.5 Sonnet",
            "3.00",
            "15.00",
            "0.30",
            "3.75",
        ),
        // OpenAI (approx).
        row("gpt-5.6", "GPT-5.6", "2.50", "10.00", "0.25", "2.50"),
        listed_row("gpt-4o", "GPT-4o"),
        // OpenAI Codex variants (gpt-5.x) — base names; the prefix-fallback in
        // `resolve` maps variant suffixes (`-codex`, `-low/medium/high/…`) back
        // to these. Codex logs never expose cache_creation, so it is 0 here and
        // the cache-write heuristic does not affect Codex cost.
        listed_row("gpt-5.5", "GPT-5.5"),
        listed_row("gpt-5.4", "GPT-5.4"),
        listed_row("gpt-5.2", "GPT-5.2"),
        // Google: list price from llm-prices.
        listed_row("gemini-2.5-pro", "Gemini 2.5 Pro"),
        // DeepSeek (OpenCode's common backend). Values from CC-Switch's table;
        // v4 and newer are not listed here: `resolve` prices them from
        // llm-prices when it lists them, otherwise they fall to cost 0 and are
        // top-up rebilled once a price is added via the Pricing UI / LiteLLM.
        row(
            "deepseek-v3.2",
            "DeepSeek V3.2",
            "0.28",
            "0.42",
            "0.028",
            "0",
        ),
        row(
            "deepseek-v3.1",
            "DeepSeek V3.1",
            "0.55",
            "1.67",
            "0.055",
            "0",
        ),
        row("deepseek-v3", "DeepSeek V3", "0.28", "1.11", "0.028", "0"),
        // xAI Grok Build (Grok CLI). Verified against the CLI's self-reported
        // costUsdTicks — grok-4.5-build at 2/6/0.30 per 1M (cache_creation 0:
        // Grok exposes no write bucket). Other aliases (grok-4.3, …) fall to
        // cost 0 and are rebilled once a price is added.
        row("grok-4.5-build", "Grok 4.5 Build", "2", "6", "0.30", "0"),
    ]
}

/// Convenience: a book preloaded with the built-in seed.
pub fn seed_book() -> PricingBook {
    PricingBook::from_iter(builtin_seed())
}

/// Parse a JSON pricing document (the local `pricing.json` shape) into entries.
/// Accepts either an array of `PricingEntry` or `{ "models": [...] }`.
pub fn parse_pricing_doc(json: &str) -> AppResult<Vec<ModelPricing>> {
    #[derive(serde::Deserialize)]
    struct Wrapper {
        #[serde(default)]
        models: Vec<PricingEntry>,
    }
    let trimmed = json.trim_start();
    let entries: Vec<PricingEntry> = if trimmed.starts_with('{') {
        serde_json::from_str::<Wrapper>(json)?.models
    } else {
        serde_json::from_str(json)?
    };
    entries.iter().map(ModelPricing::from_entry).collect()
}

/// Serialize pricing entries back to the pricing.json shape (`{ "models": [...] }`).
pub fn write_pricing_doc(entries: &[ModelPricing]) -> AppResult<String> {
    let dto: Vec<PricingEntry> = entries.iter().map(ModelPricing::to_entry).collect();
    Ok(serde_json::to_string_pretty(
        &serde_json::json!({ "models": dto }),
    )?)
}

/// Fetch the LiteLLM upstream price sheet and convert it to per-million-token
/// pricing entries. LiteLLM lists costs **per token**, so each
/// is multiplied by 1e6. Best-effort: the caller treats network failure as
/// "keep using the existing book" (offline-first).
pub fn fetch_litellm() -> AppResult<Vec<ModelPricing>> {
    const URL: &str =
        "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
    let resp = ureq::get(URL)
        .timeout(std::time::Duration::from_secs(20))
        .call()
        .map_err(|e| AppError::Pricing(format!("litellm fetch failed: {e}")))?;
    let body = resp
        .into_string()
        .map_err(|e| AppError::Pricing(format!("litellm read failed: {e}")))?;
    parse_litellm_body(&body)
}

/// Parse a LiteLLM price sheet body into per-million pricing entries.
fn parse_litellm_body(body: &str) -> AppResult<Vec<ModelPricing>> {
    let v: serde_json::Value = serde_json::from_str(body)?;
    let obj = v
        .as_object()
        .ok_or_else(|| AppError::Pricing("litellm body is not a JSON object".into()))?;
    let mut out = Vec::new();
    for (key, val) in obj {
        // Skip the doc's template pseudo-entry and anything without costs.
        if key.eq_ignore_ascii_case("sample_provider/claude-3-5-sonnet") || key.contains("sample") {
            continue;
        }
        let o = match val.as_object() {
            Some(o) => o,
            None => continue,
        };
        let get_f = |k: &str| o.get(k).and_then(|x| x.as_f64());
        let Some(input) = get_f("input_cost_per_token") else {
            continue;
        };
        // Drop free / placeholder entries (0 input cost ⇒ not a real price).
        if input <= 0.0 {
            continue;
        }
        let output = get_f("output_cost_per_token").unwrap_or(0.0);
        let cache_creation = get_f("cache_creation_input_token_cost").unwrap_or(0.0);
        let cache_read = get_f("cache_read_input_token_cost").unwrap_or(0.0);
        // Key form is often `provider/model`; keep the model tail.
        let model_key = key.rsplit('/').next().unwrap_or(key).to_string();
        out.push(ModelPricing {
            display_name: model_key.clone(),
            model_key,
            input: per_million(input),
            output: per_million(output),
            cache_read: per_million(cache_read),
            cache_creation: per_million(cache_creation),
            is_builtin: true,
        });
    }
    Ok(out)
}

/// Convert a per-token USD cost to a per-1M-token `Decimal`.
fn per_million(per_token: f64) -> Decimal {
    let per_million_f = per_token * 1_000_000.0;
    Decimal::try_from(per_million_f).unwrap_or(Decimal::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_is_zero_when_rate_missing() {
        let tokens = TokenCounts {
            input: 1000,
            output: 500,
            cache_creation: 0,
            cache_read: 0,
        };
        let cost = CostCalculator::calc(tokens, None);
        assert_eq!(cost.total_f64(), 0.0);
    }

    #[test]
    fn calc_applies_per_million_rates() {
        // input @ 3 USD/1M × 1000 tokens = 0.003 USD.
        let tokens = TokenCounts {
            input: 1000,
            output: 0,
            cache_creation: 0,
            cache_read: 0,
        };
        let rate = ResolvedRate {
            input: dec("3"),
            output: dec("0"),
            cache_read: dec("0"),
            cache_creation: dec("0"),
        };
        let cost = CostCalculator::calc(tokens, Some(rate));
        assert!((cost.total_f64() - 0.003).abs() < 1e-9);
    }

    #[test]
    fn resolve_fills_unset_cache_rates_via_heuristics() {
        let book = PricingBook::from_iter([ModelPricing {
            model_key: "m".into(),
            display_name: "M".into(),
            input: dec("10"),
            output: dec("30"),
            cache_read: Decimal::ZERO,
            cache_creation: Decimal::ZERO,
            is_builtin: true,
        }]);
        let rate = book.resolve("m").expect("seeded model resolves");
        // cache_read ≈ 0.1×input, cache_creation ≈ 1.25×input.
        assert_eq!(rate.cache_read, dec("1.0"));
        assert_eq!(rate.cache_creation, dec("12.5"));
        assert_eq!(rate.input, dec("10"));
        assert_eq!(rate.output, dec("30"));
    }

    #[test]
    fn resolve_uses_prefix_fallback_and_none_for_unknown() {
        let book = PricingBook::from_iter([ModelPricing {
            model_key: "gpt-4o".into(),
            display_name: "GPT-4o".into(),
            input: dec("2.5"),
            output: dec("10"),
            cache_read: dec("1.25"),
            cache_creation: dec("2.5"),
            is_builtin: true,
        }]);
        assert!(
            book.resolve("gpt-4o-mini").is_some(),
            "longer string falls back to gpt-4o"
        );
        assert!(book.resolve("gpt-4o").is_some());
        assert!(book.resolve("totally-unknown").is_none());
        // A listed model the book lacks resolves via llm-prices.
        assert!(book.resolve("claude-opus-5-5").is_some());
    }

    #[test]
    fn seed_carries_current_list_prices() {
        let book = seed_book();
        let rates = |m: &str| {
            let r = book.resolve(m).expect("seeded model resolves");
            (r.input, r.output, r.cache_read, r.cache_creation)
        };
        // The hand-copied seed had 5/25, 15/75, 3/15 and Gemini output 5.
        assert_eq!(
            rates("claude-fable-5"),
            (dec("10"), dec("50"), dec("1"), dec("12.5"))
        );
        assert_eq!(
            rates("claude-opus-4-8"),
            (dec("5"), dec("25"), dec("0.5"), dec("6.25"))
        );
        assert_eq!(
            rates("claude-sonnet-5"),
            (dec("2"), dec("10"), dec("0.2"), dec("2.5"))
        );
        assert_eq!(book.resolve("gemini-2.5-pro").unwrap().output, dec("10"));
    }

    #[test]
    fn model_missing_from_book_bills_at_listed_price() {
        // Not in the seed: used to resolve to None and be stored at $0.
        let book = seed_book();
        let rate = book
            .resolve("claude-opus-5-5")
            .expect("listed model resolves");
        assert_eq!((rate.input, rate.output), (dec("4"), dec("20")));
        let tokens = TokenCounts {
            input: 1_000_000,
            output: 0,
            cache_creation: 0,
            cache_read: 0,
        };
        assert!((CostCalculator::calc(tokens, Some(rate)).total_f64() - 4.0).abs() < 1e-9);
        // DeepSeek v4 (the seed comment's known $0 case) now prices too.
        assert!(book.resolve("deepseek-v4-pro").is_some());
        // A book row still wins over the listed price.
        let custom = PricingBook::from_iter([ModelPricing {
            model_key: "claude-opus-5-5".into(),
            display_name: "custom".into(),
            input: dec("1"),
            output: dec("2"),
            cache_read: Decimal::ZERO,
            cache_creation: Decimal::ZERO,
            is_builtin: false,
        }]);
        assert_eq!(custom.resolve("claude-opus-5-5").unwrap().input, dec("1"));
    }

    #[test]
    fn seed_book_resolves_transit_model() {
        let book = seed_book();
        // The CC-Switch transit model carries a [1m] bracket that normalizes away.
        assert!(book.resolve("glm-5.2[1m]").is_some());
    }

    #[test]
    fn pricing_doc_round_trip() {
        let entries: Vec<_> = builtin_seed().into_iter().take(2).collect();
        let doc = write_pricing_doc(&entries).unwrap();
        let parsed = parse_pricing_doc(&doc).unwrap();
        assert_eq!(parsed.len(), entries.len());
        assert_eq!(parsed[0].model_key, entries[0].model_key);
        assert_eq!(parsed[0].is_builtin, entries[0].is_builtin);
    }

    #[test]
    fn parse_litellm_body_skips_sample_and_free_entries() {
        let body = r#"{
            "sample_provider/claude-3-5-sonnet": {"input_cost_per_token": 0.000003},
            "claude-3-5-haiku": {"input_cost_per_token": 0.0000008, "output_cost_per_token": 0.000004},
            "free-model": {"input_cost_per_token": 0}
        }"#;
        let entries = parse_litellm_body(body).unwrap();
        assert_eq!(entries.len(), 1, "sample + free entries must be skipped");
        assert_eq!(entries[0].model_key, "claude-3-5-haiku");
        let per_million_input = entries[0].input.to_f64().unwrap();
        assert!(
            (per_million_input - 0.8).abs() < 1e-6,
            "per-token 0.0000008 → per-million ≈0.8"
        );
    }
}
