// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Token counts to dollars. Transcripts carry no cost field, so cost comes
//! from this hand-maintained per-family table and can drift from published
//! pricing. Matching is by model family so a new version name still prices.

/// USD per million tokens: (input, output). Cache write is 1.25x input and
/// cache read is 0.1x input.
const FAMILY_PRICES: [(&str, f64, f64); 3] = [
    ("opus", 5.0, 25.0),
    ("sonnet", 3.0, 15.0),
    ("haiku", 1.0, 5.0),
];

const CACHE_WRITE_MULTIPLIER: f64 = 1.25;
const CACHE_READ_MULTIPLIER: f64 = 0.1;
const TOKENS_PER_UNIT: f64 = 1_000_000.0;

/// Dollar cost of one message. An unrecognized model costs 0.0.
pub fn cost_usd(model: &str, input: u64, output: u64, cache_write: u64, cache_read: u64) -> f64 {
    let lower = model.to_lowercase();
    let Some((_, in_price, out_price)) = FAMILY_PRICES
        .iter()
        .find(|(family, _, _)| lower.contains(family))
    else {
        return 0.0;
    };
    let total = input as f64 * in_price
        + output as f64 * out_price
        + cache_write as f64 * in_price * CACHE_WRITE_MULTIPLIER
        + cache_read as f64 * in_price * CACHE_READ_MULTIPLIER;
    total / TOKENS_PER_UNIT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn prices_each_family_with_hand_computed_totals() {
        assert!(close(
            cost_usd("claude-sonnet-5", 2, 770, 29653, 22178),
            0.12940815
        ));
        assert!(close(cost_usd("claude-opus-5", 10, 200, 0, 5000), 0.00755));
        assert!(close(cost_usd("claude-haiku-5", 1000, 500, 0, 0), 0.0035));
    }

    #[test]
    fn unknown_model_costs_zero() {
        assert_eq!(cost_usd("mystery-model", 1000, 1000, 1000, 1000), 0.0);
    }
}
