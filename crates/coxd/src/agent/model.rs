//! The model a chat's agent is asked to run with, as the agent that opened
//! the chat names it, in its own words, as `opus`, `sonnet[1m]` or
//! `claude-opus-5-5-xhigh`, read against the settings the session offers:
//! the choice of its model it names, and of its effort, when the name ends
//! with one.

use cox_acp::{Choice, Setting};

/// The words of a name that say how hard the model thinks, not which
/// model it is.
const EFFORTS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

/// The words of a name that say nothing of which model it is: the maker,
/// and a speed no session offers as a setting of its own.
const NOISE: [&str; 2] = ["claude", "fast"];

/// What of the asked model the session is set to, and what it does not
/// offer.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Chosen {
    /// The settings to set, by id, each with the id of its choice.
    pub set: Vec<(String, String)>,
    /// What the session does not offer of what was asked, to tell, if
    /// anything.
    pub unoffered: Option<String>,
}

/// What `settings`, a session's, are to be set to for the model `asked`:
/// the choice of the model setting whose id or name is `asked`, or holds
/// each word of it that names a model, and the choice of the effort
/// setting that the last word of `asked` names, if it names an effort.
/// `inherit`, or nothing, asks for nothing.
#[must_use]
pub fn chosen(asked: &str, settings: &[Setting]) -> Chosen {
    let asked = asked.trim();
    if asked.is_empty() || asked.eq_ignore_ascii_case("inherit") {
        return Chosen::default();
    }
    let model = settings
        .iter()
        .find(|setting| setting.category.as_deref() == Some("model"));
    let effort = settings.iter().find(|setting| is_effort(setting));
    if let Some(model) = model
        && let Some(choice) = model.choices.iter().find(|choice| names(choice, asked))
    {
        return Chosen {
            set: vec![(model.id.clone(), choice.id.clone())],
            unoffered: None,
        };
    }
    let mut words: Vec<String> = asked
        .split(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '/'))
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .filter(|word| !NOISE.contains(&word.as_str()))
        .collect();
    let asked_effort = words
        .last()
        .filter(|word| EFFORTS.contains(&word.as_str()))
        .cloned();
    if asked_effort.is_some() {
        words.pop();
    }
    // A version, as `5` of `opus-5-5`, is not how a session names its
    // models, which go by their family.
    let family: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .filter(|word| word.chars().any(char::is_alphabetic))
        .collect();
    let mut chosen = Chosen::default();
    let mut unoffered = Vec::new();
    if !family.is_empty() {
        match model {
            None => unoffered.push(format!(
                "The agent offers no choice of model, so `{asked}` is not set: the chat runs \
                 with the model the agent uses."
            )),
            Some(model) => match model.choices.iter().find(|choice| holds(choice, &family)) {
                Some(choice) => chosen.set.push((model.id.clone(), choice.id.clone())),
                None => unoffered.push(format!(
                    "The agent offers no model `{asked}` ({}): the chat runs with {}.",
                    offers(model),
                    now(model)
                )),
            },
        }
    }
    if let Some(asked_effort) = asked_effort {
        match effort {
            None => unoffered.push(format!(
                "The agent offers no choice of effort, so `{asked_effort}` is not set."
            )),
            Some(effort) => match effort
                .choices
                .iter()
                .find(|choice| names(choice, &asked_effort))
            {
                Some(choice) => chosen.set.push((effort.id.clone(), choice.id.clone())),
                None => unoffered.push(format!(
                    "The agent offers no effort `{asked_effort}` ({}): the chat runs with {}.",
                    offers(effort),
                    now(effort)
                )),
            },
        }
    }
    chosen.unoffered = (!unoffered.is_empty()).then(|| unoffered.join(" "));
    chosen
}

/// Whether `setting` says how hard the model thinks.
fn is_effort(setting: &Setting) -> bool {
    setting.category.as_deref() == Some("thought_level")
        || [&setting.id, &setting.name]
            .iter()
            .any(|said| said.to_lowercase().contains("effort"))
}

/// Whether `choice` goes by `name`, its id or its own name.
fn names(choice: &Choice, name: &str) -> bool {
    choice.id.eq_ignore_ascii_case(name) || choice.name.eq_ignore_ascii_case(name)
}

/// Whether `choice`'s id or name holds each of `words`.
fn holds(choice: &Choice, words: &[&str]) -> bool {
    let id = choice.id.to_lowercase();
    let name = choice.name.to_lowercase();
    words.iter().all(|word| id.contains(word)) || words.iter().all(|word| name.contains(word))
}

/// What `setting` offers, by name, as `it offers Large, Small`.
fn offers(setting: &Setting) -> String {
    let names: Vec<&str> = setting
        .choices
        .iter()
        .map(|choice| choice.name.as_str())
        .collect();
    format!("it offers {}", names.join(", "))
}

/// The name of the choice `setting` is set to.
fn now(setting: &Setting) -> &str {
    setting
        .choices
        .iter()
        .find(|choice| choice.id == setting.value)
        .map_or(setting.value.as_str(), |choice| choice.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(id: &str, category: &str, value: &str, choices: &[(&str, &str)]) -> Setting {
        Setting {
            id: id.to_owned(),
            name: id.to_owned(),
            description: None,
            category: Some(category.to_owned()),
            value: value.to_owned(),
            choices: choices
                .iter()
                .map(|(id, name)| Choice {
                    id: (*id).to_owned(),
                    name: (*name).to_owned(),
                    description: None,
                    group: None,
                })
                .collect(),
        }
    }

    fn claude() -> Vec<Setting> {
        vec![
            setting(
                "model",
                "model",
                "default",
                &[
                    ("default", "Default (recommended)"),
                    ("opus", "Opus"),
                    ("sonnet", "Sonnet"),
                    ("haiku", "Haiku"),
                ],
            ),
            setting(
                "effort",
                "thought_level",
                "high",
                &[("low", "Low"), ("high", "High"), ("xhigh", "Extra high")],
            ),
            setting("mode", "mode", "default", &[("default", "Default")]),
        ]
    }

    fn set(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(id, value)| ((*id).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn sets_the_model_named_as_the_agent_tool_names_it() {
        assert_eq!(
            chosen("opus", &claude()),
            Chosen {
                set: set(&[("model", "opus")]),
                unoffered: None,
            }
        );
        assert_eq!(chosen("Sonnet", &claude()).set, set(&[("model", "sonnet")]));
    }

    #[test]
    fn sets_the_effort_a_name_ends_with() {
        assert_eq!(
            chosen("claude-opus-5-5-xhigh-fast", &claude()),
            Chosen {
                set: set(&[("model", "opus"), ("effort", "xhigh")]),
                unoffered: None,
            }
        );
        assert_eq!(
            chosen("opus low", &claude()).set,
            set(&[("model", "opus"), ("effort", "low")])
        );
    }

    #[test]
    fn says_what_is_not_offered() {
        let told = chosen("fable max", &claude());
        assert_eq!(told.set, Vec::new());
        assert_eq!(
            told.unoffered.as_deref(),
            Some(
                "The agent offers no model `fable max` (it offers Default (recommended), Opus, \
                 Sonnet, Haiku): the chat runs with Default (recommended). The agent offers no \
                 effort `max` (it offers Low, High, Extra high): the chat runs with High."
            )
        );
        let none = chosen("opus", &[]);
        assert_eq!(none.set, Vec::new());
        assert!(
            none.unoffered
                .is_some_and(|told| told.starts_with("The agent offers no choice of model")),
        );
    }

    #[test]
    fn asks_nothing_to_inherit() {
        assert_eq!(chosen("inherit", &claude()), Chosen::default());
        assert_eq!(chosen(" ", &claude()), Chosen::default());
    }
}
