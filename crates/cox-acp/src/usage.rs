//! What the agent tells of what its session spends: how full its context
//! is and what the session cost so far, in ACP's `usage_update`, and the
//! tokens a turn spent, in the answer to its prompt. `claude-agent-acp`
//! tells both; an agent that tells neither leaves the chat without.

use agent_client_protocol::schema::v1::{Cost, UsageUpdate};

/// How a session stands, as the agent's `usage_update` tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    /// The tokens in the context now.
    pub used: u64,
    /// The tokens the context holds at most.
    pub size: u64,
    /// What the session cost so far, from its start, as the agent counts
    /// it, when it tells it in US dollars: `claude-agent-acp` tells it
    /// as each turn ends, and goes on counting from it when the session
    /// is resumed.
    pub cost: Option<Microdollars>,
}

impl From<UsageUpdate> for Usage {
    fn from(update: UsageUpdate) -> Self {
        Self {
            used: update.used,
            size: update.size,
            cost: update.cost.as_ref().and_then(Microdollars::of),
        }
    }
}

/// An amount of US dollars, in millionths of a dollar, so that amounts
/// add up as they are.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct Microdollars(pub u64);

impl Microdollars {
    /// `cost` in millionths of a dollar, when it is in US dollars, and
    /// an amount that can be one.
    fn of(cost: &Cost) -> Option<Self> {
        if cost.currency != "USD" || !cost.amount.is_finite() || cost.amount < 0.0 {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a cost that is finite and not negative, of far fewer than 2^64 millionths"
        )]
        let micros = (cost.amount * 1_000_000.0).round() as u64;
        Some(Self(micros))
    }

    /// The amount in dollars, as the window shows it.
    #[must_use]
    #[expect(clippy::cast_precision_loss, reason = "a cost shown to the cent")]
    pub fn dollars(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }
}

/// The amount as the human reads it, to the cent, as `$0.42`: less
/// than half a cent, but more than none, as `<$0.01`.
impl std::fmt::Display for Microdollars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cents = self.0.saturating_add(5_000) / 10_000;
        if cents == 0 && self.0 > 0 {
            return f.write_str("<$0.01");
        }
        write!(f, "${}.{:02}", cents / 100, cents % 100)
    }
}

impl std::ops::Add for Microdollars {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

/// The tokens a turn spent, as the agent tells them in the answer to its
/// prompt: those of the turn alone, not of the session so far.
/// `claude-agent-acp` counts its main loop only, not the subagents it
/// runs itself, whose cost the session's cost holds all the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    /// The input read from the prompt cache.
    pub cache_read: u64,
    /// The input written to the prompt cache.
    pub cache_write: u64,
}

impl Tokens {
    /// All of them.
    #[must_use]
    pub fn total(self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }
}

impl std::ops::AddAssign for Tokens {
    fn add_assign(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
    }
}

impl From<agent_client_protocol::schema::v1::Usage> for Tokens {
    fn from(usage: agent_client_protocol::schema::v1::Usage) -> Self {
        Self {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cache_read: usage.cached_read_tokens.unwrap_or_default(),
            cache_write: usage.cached_write_tokens.unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::v1::{PromptResponse, SessionNotification, SessionUpdate};

    use super::*;

    /// A `usage_update` as `claude-agent-acp` 0.107.3 sent it as a turn
    /// ended, its session id made up.
    const TURN_ENDED: &str = r#"{"sessionId":"00000000-0000-4000-8000-000000000001","update":{"sessionUpdate":"usage_update","used":24595,"size":1000000,"cost":{"amount":0.17947570000000002,"currency":"USD"},"_meta":{"_claude/origin":{"kind":"human"},"_claude/model":"claude-opus-5-5"}}}"#;

    /// One it sent as the reply streamed, which tells no cost.
    const STREAMING: &str = r#"{"sessionId":"00000000-0000-4000-8000-000000000001","update":{"sessionUpdate":"usage_update","used":22401,"size":1000000,"_meta":{"_claude/model":"claude-opus-5-5"}}}"#;

    /// The answer to the prompt of that turn.
    const ANSWER: &str = r#"{"stopReason":"end_turn","usage":{"inputTokens":2,"outputTokens":3,"cachedReadTokens":14006,"cachedWriteTokens":10584,"totalTokens":24595},"_meta":{"quota":{"token_count":{"totalTokens":24595,"inputTokens":2,"cachedInputTokens":14006,"cachedWriteTokens":10584,"outputTokens":3,"reasoningOutputTokens":0},"model_usage":[{"model":"claude-opus-5-5","token_count":{"totalTokens":24595,"inputTokens":2,"cachedInputTokens":14006,"cachedWriteTokens":10584,"outputTokens":3,"reasoningOutputTokens":0}}]}}}"#;

    fn usage(notification: &str) -> Usage {
        let notification: SessionNotification = serde_json::from_str(notification).unwrap();
        let SessionUpdate::UsageUpdate(update) = notification.update else {
            panic!("not a usage update");
        };
        Usage::from(update)
    }

    #[test]
    fn a_turn_that_ends_tells_the_context_and_what_the_session_cost() {
        assert_eq!(
            usage(TURN_ENDED),
            Usage {
                used: 24_595,
                size: 1_000_000,
                cost: Some(Microdollars(179_476)),
            }
        );
    }

    #[test]
    fn a_reply_that_streams_tells_the_context_alone() {
        assert_eq!(
            usage(STREAMING),
            Usage {
                used: 22_401,
                size: 1_000_000,
                cost: None,
            }
        );
    }

    #[test]
    fn a_cost_in_another_currency_or_no_amount_is_left_out() {
        let euros = TURN_ENDED.replace("\"USD\"", "\"EUR\"");
        assert_eq!(usage(&euros).cost, None);
        let negative = TURN_ENDED.replace("0.17947570000000002", "-1");
        assert_eq!(usage(&negative).cost, None);
    }

    #[test]
    fn an_amount_reads_to_the_cent() {
        let read = |micros| Microdollars(micros).to_string();
        assert_eq!(read(0), "$0.00");
        assert_eq!(read(3_000), "<$0.01");
        assert_eq!(read(179_476), "$0.18");
        assert_eq!(read(12_345_678), "$12.35");
    }

    #[test]
    fn the_answer_tells_the_tokens_of_its_turn() {
        let answer: PromptResponse = serde_json::from_str(ANSWER).unwrap();
        let tokens = Tokens::from(answer.usage.unwrap());
        assert_eq!(
            tokens,
            Tokens {
                input: 2,
                output: 3,
                cache_read: 14_006,
                cache_write: 10_584,
            }
        );
        assert_eq!(tokens.total(), 24_595);
    }
}
