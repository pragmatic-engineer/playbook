// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Token counts to dollars. Transcripts carry no cost field, so cost comes
//! from this hand-maintained per-model table, taken from
//! <https://platform.claude.com/docs/en/about-claude/pricing> on 2026-10-02.
//! It can drift from published pricing. A model not in the table is never
//! guessed: it is reported as unpriced by the callers.

/// (normalized model name, input USD per MTok, output USD per MTok, cache read
/// as a fraction of the input price).
const PRICES: [(&str, f64, f64, f64); 20] = [
    ("fable-5-1", 10.0, 50.0, 0.025),
    ("mythos-5-1", 10.0, 50.0, 0.025),
    ("fable-5", 10.0, 50.0, 0.1),
    ("mythos-5", 10.0, 50.0, 0.1),
    ("opus-5-5", 4.0, 20.0, 0.05),
    ("opus-5", 5.0, 25.0, 0.1),
    ("opus-4-8", 5.0, 25.0, 0.1),
    ("opus-4-7", 5.0, 25.0, 0.1),
    ("opus-4-6", 5.0, 25.0, 0.1),
    ("opus-4-5", 5.0, 25.0, 0.1),
    ("opus-4-1", 15.0, 75.0, 0.1),
    ("opus-4", 15.0, 75.0, 0.1),
    ("sonnet-5-5", 2.0, 10.0, 0.1),
    ("sonnet-5", 2.0, 10.0, 0.1),
    ("sonnet-4-6", 3.0, 15.0, 0.1),
    ("sonnet-4-5", 3.0, 15.0, 0.1),
    ("sonnet-4", 3.0, 15.0, 0.1),
    ("haiku-4-5", 1.0, 5.0, 0.1),
    ("3-5-haiku", 0.8, 4.0, 0.1),
    ("haiku-3-5", 0.8, 4.0, 0.1),
];

const CACHE_WRITE_5M_MULTIPLIER: f64 = 1.25;
const CACHE_WRITE_1H_MULTIPLIER: f64 = 2.0;
const TOKENS_PER_UNIT: f64 = 1_000_000.0;
const DATE_SUFFIX_LEN: usize = 8;

/// One message's token counts, with cache writes split by cache lifetime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }
}

/// `claude-opus-4-1-20250805` to `opus-4-1`: drops the `claude-` prefix, any
/// `[1m]`-style or `@` suffix, and a trailing eight digit date. The name is
/// matched exactly afterwards, so an unlisted version stays unpriced.
fn normalize(model: &str) -> String {
    let lower = model.to_lowercase();
    let end = lower.find(['[', '@', ':']).unwrap_or(lower.len());
    let mut name = lower[..end].trim_start_matches("claude-");
    if let Some((head, tail)) = name.rsplit_once('-') {
        if tail.len() == DATE_SUFFIX_LEN && tail.bytes().all(|b| b.is_ascii_digit()) {
            name = head;
        }
    }
    name.to_string()
}

/// Dollar cost of one message, or `None` when the model is not in the table.
pub fn cost_usd(model: &str, tokens: &Tokens) -> Option<f64> {
    let name = normalize(model);
    let (_, input, output, read_fraction) = PRICES.iter().find(|(n, _, _, _)| *n == name)?;
    let total = tokens.input as f64 * input
        + tokens.output as f64 * output
        + tokens.cache_write_5m as f64 * input * CACHE_WRITE_5M_MULTIPLIER
        + tokens.cache_write_1h as f64 * input * CACHE_WRITE_1H_MULTIPLIER
        + tokens.cache_read as f64 * input * read_fraction;
    Some(total / TOKENS_PER_UNIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn per_million(model: &str, tokens: Tokens) -> f64 {
        cost_usd(model, &tokens).unwrap()
    }

    #[test]
    fn prices_a_mixed_message_with_hand_computed_totals() {
        // Sonnet 5 is 2/10: 2*2 + 770*10 + 29653*2*2 (1h write) + 22178*2*0.1.
        let sonnet = Tokens {
            input: 2,
            output: 770,
            cache_write_5m: 0,
            cache_write_1h: 29653,
            cache_read: 22178,
        };
        assert!(close(per_million("claude-sonnet-5", sonnet), 0.1307516));

        // Opus 5.5 is 4/20 with reads at 0.05x:
        // 1000*4 + 500*20 + 2000*4*1.25 + 4000*4*2 + 10000*4*0.05 = 58000.
        let opus = Tokens {
            input: 1000,
            output: 500,
            cache_write_5m: 2000,
            cache_write_1h: 4000,
            cache_read: 10000,
        };
        assert!(close(per_million("claude-opus-5-5", opus), 0.058));
    }

    #[test]
    fn each_published_row_prices_one_million_input_and_output_tokens() {
        let input = Tokens {
            input: 1_000_000,
            ..Tokens::default()
        };
        let output = Tokens {
            output: 1_000_000,
            ..Tokens::default()
        };
        let rows = [
            ("claude-fable-5-1", 10.0, 50.0),
            ("claude-fable-5", 10.0, 50.0),
            ("claude-opus-5-5", 4.0, 20.0),
            ("claude-opus-5", 5.0, 25.0),
            ("claude-opus-4-8", 5.0, 25.0),
            ("claude-opus-4-5-20251101", 5.0, 25.0),
            ("claude-opus-4-1-20250805", 15.0, 75.0),
            ("claude-opus-4-20250514", 15.0, 75.0),
            ("claude-sonnet-5-5", 2.0, 10.0),
            ("claude-sonnet-5", 2.0, 10.0),
            ("claude-sonnet-4-6", 3.0, 15.0),
            ("claude-sonnet-4-20250514", 3.0, 15.0),
            ("claude-haiku-4-5-20251001", 1.0, 5.0),
            ("claude-3-5-haiku-20241022", 0.8, 4.0),
        ];
        for (model, in_price, out_price) in rows {
            assert!(close(per_million(model, input), in_price), "{model} input");
            assert!(
                close(per_million(model, output), out_price),
                "{model} output"
            );
        }
    }

    #[test]
    fn cache_reads_use_the_per_model_fraction() {
        let reads = Tokens {
            cache_read: 1_000_000,
            ..Tokens::default()
        };
        assert!(close(per_million("claude-fable-5-1", reads), 0.25));
        assert!(close(per_million("claude-fable-5", reads), 1.0));
        assert!(close(per_million("claude-opus-5-5", reads), 0.2));
        assert!(close(per_million("claude-sonnet-5-5", reads), 0.2));
        assert!(close(per_million("claude-haiku-4-5", reads), 0.1));
    }

    #[test]
    fn cache_writes_use_the_two_lifetimes() {
        let five = Tokens {
            cache_write_5m: 1_000_000,
            ..Tokens::default()
        };
        let hour = Tokens {
            cache_write_1h: 1_000_000,
            ..Tokens::default()
        };
        assert!(close(per_million("claude-sonnet-5", five), 2.5));
        assert!(close(per_million("claude-sonnet-5", hour), 4.0));
    }

    #[test]
    fn a_suffix_after_the_model_name_does_not_change_the_match() {
        let tokens = Tokens {
            input: 1_000_000,
            ..Tokens::default()
        };
        assert!(close(per_million("claude-opus-4-8[1m]", tokens), 5.0));
        assert!(close(per_million("CLAUDE-SONNET-5", tokens), 2.0));
    }

    #[test]
    fn a_model_not_in_the_table_is_unpriced_not_zero() {
        let tokens = Tokens {
            input: 1000,
            ..Tokens::default()
        };
        assert_eq!(cost_usd("mystery-model", &tokens), None);
        assert_eq!(cost_usd("<synthetic>", &tokens), None);
        // A near-miss version must not borrow a neighbour's price.
        assert_eq!(cost_usd("claude-opus-4-9", &tokens), None);
        assert_eq!(cost_usd("claude-haiku-5", &tokens), None);
    }
}
