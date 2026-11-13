//! First-party Claude API list prices, for an API-equivalent cost estimate of Claude Code usage.
//!
//! Subscriptions are not billed per token; this answers "what would this have cost on the API".
//! Prices are USD per million tokens as published on 2026-11-03. Models without a known price
//! are reported as unpriced instead of being guessed.
use crate::code_usage::Tokens;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Price {
    input: f64,
    output: f64,
    cache_read: f64,
}

const fn p(input: f64, output: f64, cache_read: f64) -> Price { Price { input, output, cache_read } }

const PRICES: [(&str, Price); 15] = [
    ("claude-fable-5-2", p(8.0, 40.0, 0.20)),
    ("claude-mythos-5-2", p(8.0, 40.0, 0.20)),
    ("claude-fable-5-1", p(10.0, 50.0, 0.25)),
    ("claude-mythos-5-1", p(10.0, 50.0, 0.25)),
    ("claude-fable-5", p(10.0, 50.0, 1.00)),
    ("claude-mythos-5", p(10.0, 50.0, 1.00)),
    ("claude-opus-5-5", p(4.0, 20.0, 0.20)),
    ("claude-opus-5", p(5.0, 25.0, 0.50)),
    ("claude-opus-4-8", p(5.0, 25.0, 0.50)),
    ("claude-opus-4-7", p(5.0, 25.0, 0.50)),
    ("claude-opus-4-6", p(5.0, 25.0, 0.50)),
    ("claude-sonnet-5-5", p(2.0, 10.0, 0.10)),
    ("claude-sonnet-5", p(2.0, 10.0, 0.20)),
    ("claude-sonnet-4-6", p(3.0, 15.0, 0.30)),
    ("claude-haiku-4-5", p(1.0, 5.0, 0.10)),
];

/// Claude Haiku 5.5 is priced by prompt size: up to 100K prompt tokens, and above.
const HAIKU_5_5: &str = "claude-haiku-5-5";
const HAIKU_5_5_SHORT: Price = p(0.10, 0.50, 0.01);
const HAIKU_5_5_LONG: Price = p(0.50, 2.50, 0.05);
const HAIKU_5_5_LIMIT: u64 = 100_000;

/// The model id without a date snapshot (`-20251001`) or a variant marker (`[1m]`), but only
/// when what follows the known id is exactly that: `claude-opus-5-7` is not `claude-opus-5`.
fn matches(model: &str, id: &str) -> bool {
    let Some(rest) = model.strip_prefix(id) else { return false };
    rest.is_empty() || rest.starts_with('[') || rest.strip_prefix('-').is_some_and(|d| d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit()))
}

fn price(model: &str, prompt: u64) -> Option<Price> {
    if matches(model, HAIKU_5_5) { return Some(if prompt > HAIKU_5_5_LIMIT { HAIKU_5_5_LONG } else { HAIKU_5_5_SHORT }); }
    PRICES.iter().find(|(id, _)| matches(model, id)).map(|(_, price)| *price)
}

/// API-equivalent cost in USD of one response. `write_1h` is the part of the cache writes made
/// with the 1-hour TTL (2× input; 5-minute writes are 1.25×). Fast mode bills 2× on every model
/// that offers it.
pub fn cost(model: &str, t: &Tokens, write_1h: u64, fast: bool) -> Option<f64> {
    let price = price(model, t.input + t.cache_creation + t.cache_read)?;
    let write_1h = write_1h.min(t.cache_creation);
    let write_5m = t.cache_creation - write_1h;
    let micro = t.input as f64 * price.input
        + t.output as f64 * price.output
        + write_5m as f64 * price.input * 1.25
        + write_1h as f64 * price.input * 2.0
        + t.cache_read as f64 * price.cache_read;
    Some(micro / 1_000_000.0 * if fast { 2.0 } else { 1.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, output: u64, cache_creation: u64, cache_read: u64) -> Tokens {
        Tokens { input, output, cache_creation, cache_read }
    }

    #[test]
    fn snapshot_and_variant_ids_use_their_model_price_but_unknown_versions_do_not() {
        // Arrange
        let ids = ["claude-opus-5-5", "claude-opus-5-5[1m]", "claude-haiku-4-5-20251001", "claude-opus-5-7", "claude-sonnet-4-5", "gpt-5"];

        // Act
        let priced: Vec<bool> = ids.iter().map(|m| price(m, 0).is_some()).collect();

        // Assert
        assert_eq!(priced, [true, true, true, false, false, false]);
        assert_ne!(price("claude-sonnet-5-5", 0), price("claude-sonnet-5", 0));
    }

    #[test]
    fn cache_writes_cost_more_than_input_and_reads_less() {
        // Arrange
        let million = 1_000_000;
        let model = "claude-opus-5-5";

        // Act
        let input = cost(model, &tokens(million, 0, 0, 0), 0, false).unwrap();
        let write_5m = cost(model, &tokens(0, 0, million, 0), 0, false).unwrap();
        let write_1h = cost(model, &tokens(0, 0, million, 0), million, false).unwrap();
        let read = cost(model, &tokens(0, 0, 0, million), 0, false).unwrap();

        // Assert
        assert!(read < input && input < write_5m && write_5m < write_1h);
        assert!((write_5m / input - 1.25).abs() < 1e-9);
        assert!((write_1h / input - 2.0).abs() < 1e-9);
    }

    #[test]
    fn fast_mode_doubles_the_cost() {
        // Arrange
        let t = tokens(1000, 2000, 300, 4000);

        // Act
        let (normal, fast) = (cost("claude-opus-5-5", &t, 0, false).unwrap(), cost("claude-opus-5-5", &t, 0, true).unwrap());

        // Assert
        assert!((fast / normal - 2.0).abs() < 1e-9);
    }

    #[test]
    fn haiku_5_5_switches_price_above_the_prompt_limit() {
        // Arrange
        let short = tokens(HAIKU_5_5_LIMIT, 0, 0, 0);
        let long = tokens(HAIKU_5_5_LIMIT + 1, 0, 0, 0);

        // Act
        let (a, b) = (cost(HAIKU_5_5, &short, 0, false).unwrap(), cost(HAIKU_5_5, &long, 0, false).unwrap());

        // Assert
        assert!(b / (HAIKU_5_5_LIMIT + 1) as f64 > a / HAIKU_5_5_LIMIT as f64);
    }
}
