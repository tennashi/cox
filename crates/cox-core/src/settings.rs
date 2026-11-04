//! What a chat's agent lets the human choose of its session, such as the
//! model it uses: the settings the agent says the session has, each set to
//! one of the choices it offers. Cox shows them as the agent says them, so
//! it knows nothing of any agent's models; it only keeps what each chat
//! was set to, to set it again when the chat's agent starts again, and
//! what the human chose last of the model, for a new chat to start with.

use serde::{Deserialize, Serialize};

/// Something of a session the agent lets the human choose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Setting {
    /// The id the agent knows it by.
    pub id: String,
    /// What the human knows it by, as `Model`.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// What it is, as ACP names it, such as `model`, `mode` or
    /// `thought_level`, if the agent says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The id of the choice it is set to.
    pub value: String,
    /// What it can be set to.
    pub choices: Vec<Choice>,
}

/// One of the values a [`Setting`] can be set to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// The id the agent knows it by.
    pub id: String,
    /// What the human knows it by.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The group of choices it is listed under, if the agent groups them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// The categories of the settings that say which model the agent uses and
/// how, as ACP names them: what a new chat takes of the human's last
/// choices. A chat's mode, as one that plans before it edits, is the
/// chat's own.
const MODEL_CATEGORIES: [&str; 3] = ["model", "model_config", "thought_level"];

impl Setting {
    /// The choice the setting is set to, if it is one it offers.
    #[must_use]
    pub fn chosen(&self) -> Option<&Choice> {
        self.choices.iter().find(|choice| choice.id == self.value)
    }

    /// Whether `value` is one of the choices it offers.
    #[must_use]
    pub fn offers(&self, value: &str) -> bool {
        self.choices.iter().any(|choice| choice.id == value)
    }

    /// Whether it says which model the agent uses, or how: what a new chat
    /// starts with as the human chose it last.
    #[must_use]
    pub fn is_of_model(&self) -> bool {
        self.category
            .as_deref()
            .is_some_and(|category| MODEL_CATEGORIES.contains(&category))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(value: &str) -> Setting {
        Setting {
            id: "model".to_owned(),
            name: "Model".to_owned(),
            description: None,
            category: Some("model".to_owned()),
            value: value.to_owned(),
            choices: ["large", "small"]
                .map(|id| Choice {
                    id: id.to_owned(),
                    name: id.to_uppercase(),
                    description: None,
                    group: None,
                })
                .to_vec(),
        }
    }

    #[test]
    fn knows_the_choice_it_is_set_to() {
        assert_eq!(
            model("small").chosen().map(|c| c.name.as_str()),
            Some("SMALL")
        );
        assert_eq!(model("gone").chosen(), None);
        assert!(model("small").offers("large"));
        assert!(!model("small").offers("gone"));
    }

    #[test]
    fn a_mode_is_not_of_the_model() {
        assert!(model("small").is_of_model());
        let mode = Setting {
            category: Some("mode".to_owned()),
            ..model("small")
        };
        assert!(!mode.is_of_model());
        let unsaid = Setting {
            category: None,
            ..model("small")
        };
        assert!(!unsaid.is_of_model());
    }
}
