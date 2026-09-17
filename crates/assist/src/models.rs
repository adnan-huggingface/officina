//! The Claude models Assist offers, in words, with what a paragraph costs.
//!
//! Opus 5 is the default: the best result is what a person asking for help
//! with their writing should get unless they choose otherwise, and the choice
//! is offered with its price beside it. The prices are Anthropic's list prices
//! for its own API, in US cents per million tokens.

use crate::event::Usage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaudeModel {
    pub id: &'static str,
    /// The model as the settings box offers it.
    pub words: &'static str,
    pub input_cents: u64,
    pub output_cents: u64,
    /// Whether the model can be told to think less for a quick verb.
    pub effort: bool,
    /// Whether the server can hand a request this model declines to another.
    pub fallback: bool,
}

pub const DEFAULT_CLAUDE_MODEL: &str = "claude-opus-5";

pub const CLAUDE_MODELS: [ClaudeModel; 3] = [
    ClaudeModel {
        id: "claude-opus-5",
        words: "Opus 5 (best)",
        input_cents: 500,
        output_cents: 2500,
        effort: true,
        fallback: true,
    },
    ClaudeModel {
        id: "claude-sonnet-5",
        words: "Sonnet 5 (faster, cheaper)",
        input_cents: 200,
        output_cents: 1000,
        effort: true,
        fallback: false,
    },
    ClaudeModel {
        id: "claude-haiku-4-5",
        words: "Haiku 4.5 (cheapest)",
        input_cents: 100,
        output_cents: 500,
        effort: false,
        fallback: false,
    },
];

pub fn claude_model(id: &str) -> Option<&'static ClaudeModel> {
    CLAUDE_MODELS.iter().find(|model| model.id == id)
}

/// A paragraph rewritten with the two around it on each side: about two
/// thousand tokens in and five hundred out.
const PARAGRAPH: Usage = Usage {
    input: 2000,
    output: 500,
    cache_read: 0,
    cache_write: 0,
};

impl ClaudeModel {
    /// What `usage` costs on this model, in US cents. Input read from the
    /// cache costs a tenth, and input written to it a quarter more.
    pub fn cents(&self, usage: Usage) -> f64 {
        let input = self.input_cents as f64 / 1e6;
        let output = self.output_cents as f64 / 1e6;
        usage.input as f64 * input
            + usage.output as f64 * output
            + usage.cache_read as f64 * input * 0.1
            + usage.cache_write as f64 * input * 1.25
    }

    pub fn paragraph_cents(&self) -> f64 {
        self.cents(PARAGRAPH)
    }
}

/// An amount in cents as the settings box says it.
pub fn cost_words(cents: f64) -> String {
    if cents < 0.5 {
        "under half a cent".to_owned()
    } else if cents < 1.0 {
        "under a cent".to_owned()
    } else {
        match cents.round() as u64 {
            1 => "about a cent".to_owned(),
            n => format!("about {n} cents"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paragraph_costs_about_two_cents_on_opus_and_under_one_on_sonnet() {
        let words = |id: &str| cost_words(claude_model(id).unwrap().paragraph_cents());
        assert_eq!(words("claude-opus-5"), "about 2 cents");
        assert_eq!(words("claude-sonnet-5"), "under a cent");
        assert_eq!(words("claude-haiku-4-5"), "under half a cent");
        assert_eq!(
            CLAUDE_MODELS[0].id, DEFAULT_CLAUDE_MODEL,
            "the best is offered first"
        );
        let cached = claude_model("claude-opus-5").unwrap().cents(Usage {
            cache_read: 2000,
            output: 500,
            ..Usage::default()
        });
        assert!(
            cached < CLAUDE_MODELS[0].paragraph_cents(),
            "a paragraph whose context comes from the cache costs less"
        );
    }
}
