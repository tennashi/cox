//! What a chat's agent spent, as it told Cox (see [`cox_acp::Usage`]):
//! the cost of the chat's sessions, how full the context of the one it
//! has now is, and the tokens of its turns. A chat whose agent tells none
//! of it has none.

use cox_acp::{Microdollars, Tokens};
use serde::{Deserialize, Serialize};

use crate::agent::{AgentEvent, Opened};

/// What a chat's agent spent, from the chat's start, across the agent
/// sessions it had.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spent {
    /// What the chat cost before the agent's count of [`Spent::session`]
    /// started: the sessions it had before, which the agent could not go
    /// on with, or of another agent, and what the session cost before
    /// the agent counted it from nothing again, as on resuming it.
    #[serde(default)]
    pub earlier: Microdollars,
    /// What the session the chat has now cost so far, as the agent last
    /// told it.
    #[serde(default)]
    pub session: Microdollars,
    /// How full the context of the session is, as the agent last told it,
    /// until another session starts.
    #[serde(default)]
    pub context: Option<Context>,
    /// The tokens the turns spent, all added up.
    #[serde(default)]
    pub tokens: Tokens,
}

/// How full an agent's context is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// The tokens in it.
    pub used: u64,
    /// The tokens it holds at most.
    pub size: u64,
}

impl Context {
    /// How full it is, in hundredths, rounded.
    #[must_use]
    pub fn percent(self) -> u64 {
        if self.size == 0 {
            return 0;
        }
        let percent =
            (u128::from(self.used) * 100 + u128::from(self.size) / 2) / u128::from(self.size);
        u64::try_from(percent).unwrap_or(u64::MAX)
    }
}

impl Spent {
    /// What the chat's sessions cost, all of them.
    #[must_use]
    pub fn cost(&self) -> Microdollars {
        self.earlier + self.session
    }

    /// Takes in `event`, from the chat's agent, into `spent`: the chat
    /// spent nothing until its agent tells something of it. Answers
    /// whether it changed.
    pub fn hear(spent: &mut Option<Self>, event: &AgentEvent) -> bool {
        let was = spent.clone();
        match event {
            AgentEvent::Usage(usage) => {
                let spent = spent.get_or_insert_default();
                spent.context = Some(Context {
                    used: usage.used,
                    size: usage.size,
                });
                if let Some(cost) = usage.cost {
                    // Claude Code goes on counting a session it resumes
                    // from where it was only when it kept the cost for
                    // it, which another session in the same folder takes
                    // the place of: a cost below the last is of a count
                    // that started again, the session's cost before it
                    // spent all the same.
                    if cost < spent.session {
                        spent.earlier = spent.cost();
                    }
                    spent.session = cost;
                }
            }
            AgentEvent::TurnTokens(tokens) => spent.get_or_insert_default().tokens += *tokens,
            // The agent counts the cost of a session it goes on with from
            // where it was, and that of a new one from nothing.
            AgentEvent::Opened { how, .. } if *how != Opened::Resumed => {
                if let Some(spent) = spent {
                    spent.earlier = spent.cost();
                    spent.session = Microdollars::default();
                    spent.context = None;
                }
            }
            _ => {}
        }
        *spent != was
    }
}

#[cfg(test)]
mod tests {
    use cox_acp::Usage;

    use super::*;

    fn usage(used: u64, cost: Option<u64>) -> AgentEvent {
        AgentEvent::Usage(Usage {
            used,
            size: 1_000_000,
            cost: cost.map(Microdollars),
        })
    }

    fn opened(how: Opened) -> AgentEvent {
        AgentEvent::Opened {
            id: "s".to_owned(),
            how,
            steers: false,
        }
    }

    fn tokens(input: u64, output: u64) -> AgentEvent {
        AgentEvent::TurnTokens(Tokens {
            input,
            output,
            cache_read: 11_219,
            cache_write: 11_177,
        })
    }

    #[test]
    fn a_chat_whose_agent_tells_nothing_spent_nothing() {
        let mut spent = None;
        for event in [opened(Opened::New), AgentEvent::ReplyText("one".to_owned())] {
            assert!(!Spent::hear(&mut spent, &event));
        }
        assert_eq!(spent, None);
    }

    /// The costs `claude-agent-acp` told of one session over three turns,
    /// the last after Cox resumed it: what the session cost so far, each.
    #[test]
    fn the_cost_of_a_session_is_the_last_the_agent_told_resumed_or_not() {
        let mut spent = None;
        for event in [
            opened(Opened::New),
            usage(22_401, None),
            usage(22_401, Some(91_728)),
            tokens(2, 3),
            usage(24_595, Some(179_476)),
            tokens(2, 3),
            opened(Opened::Resumed),
            usage(25_162, Some(206_010)),
        ] {
            Spent::hear(&mut spent, &event);
        }
        let spent = spent.unwrap();
        assert_eq!(spent.cost(), Microdollars(206_010));
        assert_eq!(
            spent.context,
            Some(Context {
                used: 25_162,
                size: 1_000_000
            })
        );
        assert_eq!(spent.context.unwrap().percent(), 3);
        assert_eq!(
            (
                spent.tokens.input,
                spent.tokens.output,
                spent.tokens.cache_read
            ),
            (4, 6, 22_438)
        );
    }

    /// What Cox heard as it resumed a session whose cost Claude Code did
    /// not keep, as another chat in the same folder ended after it: the
    /// agent counted from nothing again.
    #[test]
    fn a_resumed_session_that_counts_again_from_nothing_keeps_what_it_cost() {
        let mut spent = None;
        for event in [
            usage(34_299, Some(201_259)),
            opened(Opened::Resumed),
            usage(40_000, Some(120_000)),
            usage(41_000, Some(130_000)),
        ] {
            Spent::hear(&mut spent, &event);
        }
        assert_eq!(spent.unwrap().cost(), Microdollars(331_259));
    }

    #[test]
    fn a_new_session_adds_its_cost_to_that_of_the_sessions_before() {
        let mut spent = None;
        for event in [
            usage(22_401, Some(179_476)),
            opened(Opened::Retold {
                lost: true,
                instructions: Vec::new(),
                context: None,
            }),
        ] {
            Spent::hear(&mut spent, &event);
        }
        assert_eq!(spent.as_ref().unwrap().context, None);
        Spent::hear(&mut spent, &usage(30_000, Some(50_000)));
        let spent = spent.unwrap();
        assert_eq!(
            (spent.earlier, spent.session, spent.cost()),
            (
                Microdollars(179_476),
                Microdollars(50_000),
                Microdollars(229_476)
            )
        );
    }

    #[test]
    fn spent_is_kept_and_read_again_as_it_was() {
        let mut spent = None;
        Spent::hear(&mut spent, &usage(24_595, Some(179_476)));
        Spent::hear(&mut spent, &tokens(2, 3));
        let json = serde_json::to_string(&spent).unwrap();
        assert_eq!(serde_json::from_str::<Option<Spent>>(&json).unwrap(), spent);
    }
}
