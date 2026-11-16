//! Questions the agent asks the human, as ACP's form elicitation carries
//! them: Claude's `AskUserQuestion`, which `claude-agent-acp` sends as a
//! form of a field of options per question, each with a companion field
//! for the human's own words, and forms MCP servers ask the same way.

use std::collections::BTreeMap;

use agent_client_protocol::Responder;
use agent_client_protocol::schema::v1::{
    CreateElicitationRequest, CreateElicitationResponse, ElicitationAcceptAction,
    ElicitationAction, ElicitationContentValue, ElicitationMode, ElicitationPropertySchema,
    ElicitationSchema, EnumOption, MultiSelectItems, StringPropertySchema,
};

/// One question the agent asks the human.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Question {
    /// A short label for it, as "Approach", if the agent gave one.
    pub header: Option<String>,
    /// The question, when the request's message does not say it, as when
    /// the agent asks several.
    pub text: Option<String>,
    /// What the human chooses among: none for a question they answer in
    /// their own words alone.
    pub options: Vec<QuestionOption>,
    /// Whether the human may choose more than one option.
    pub multiple: bool,
    /// Whether the human may answer in their own words.
    pub other: bool,
}

/// An option of a [`Question`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

/// What the human picked of one question: the options at `chosen`, in
/// [`Question::options`], and their own words, if any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picked {
    pub chosen: Vec<usize>,
    pub other: Option<String>,
}

/// An answer to a [`QuestionRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestionAnswer {
    /// What the human picked of each question, in their order.
    Given(Vec<Picked>),
    /// The human chose not to answer, which the agent is told, and goes
    /// on.
    Declined,
    /// The question went unanswered, as when the human stopped the turn:
    /// the agent gives up what it asked for.
    Cancel,
}

/// A form the agent asks the human to fill in, read as questions. Answer
/// it with [`QuestionRequest::answer`]; the agent waits until then.
pub struct QuestionRequest {
    message: String,
    questions: Vec<Question>,
    /// Where the answer to each question goes in the form, in the same
    /// order.
    fields: Vec<Field>,
    responder: Box<Responder<CreateElicitationResponse>>,
}

/// Where the answer to a question goes in the form the agent sent.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Field {
    key: String,
    /// What the form takes for each option, as its `const`.
    values: Vec<String>,
    multiple: bool,
    /// The field the human's own words go in, if they may write any: the
    /// question's companion, or, for a question of no options, its own.
    other: Option<String>,
}

impl QuestionRequest {
    /// Reads `request` as questions, or, when it is not a form Cox can
    /// show, as a link to open or a field of a kind Cox does not ask,
    /// tells the agent no through `responder` at once, for it not to wait
    /// on the human.
    pub(crate) fn new(
        request: CreateElicitationRequest,
        responder: Responder<CreateElicitationResponse>,
    ) -> Option<Self> {
        let read = match &request.mode {
            ElicitationMode::Form(form) => read(&form.requested_schema),
            _ => None,
        };
        let Some((questions, fields)) = read else {
            let refused =
                responder.respond(CreateElicitationResponse::new(ElicitationAction::Cancel));
            if let Err(error) = refused {
                tracing::warn!("could not turn down a form: {error}");
            }
            return None;
        };
        Some(Self {
            message: request.message,
            questions,
            fields,
            responder: Box::new(responder),
        })
    }

    /// What the agent says as it asks: the question itself, when it asks
    /// one.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub fn questions(&self) -> &[Question] {
        &self.questions
    }

    /// Sends `answer` to the agent.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    pub fn answer(self, answer: &QuestionAnswer) -> anyhow::Result<()> {
        let action = match answer {
            QuestionAnswer::Given(picked) => ElicitationAction::Accept(
                ElicitationAcceptAction::new().content(content(&self.fields, picked)),
            ),
            QuestionAnswer::Declined => ElicitationAction::Decline,
            QuestionAnswer::Cancel => ElicitationAction::Cancel,
        };
        self.responder
            .respond(CreateElicitationResponse::new(action))?;
        Ok(())
    }
}

/// The questions `schema` asks, with where each one's answer goes, or
/// `None` when it has a field Cox does not ask, as a number. A field of
/// options asks a question, and a text field named as one of those with
/// `_custom` after it, as `claude-agent-acp` names its "Other" box, takes
/// the human's own words for it; any other text field is a question of
/// its own, answered in words alone.
fn read(schema: &ElicitationSchema) -> Option<(Vec<Question>, Vec<Field>)> {
    let properties = &schema.properties;
    let companion = |key: &str| -> Option<String> {
        let of = key.strip_suffix("_custom")?;
        let ElicitationPropertySchema::String(text) = properties.get(key)? else {
            return None;
        };
        let asks = match properties.get(of)? {
            ElicitationPropertySchema::String(of) => !options_of(of).is_empty(),
            ElicitationPropertySchema::Array(_) => true,
            _ => false,
        };
        (asks && options_of(text).is_empty()).then(|| of.to_owned())
    };
    let mut questions = Vec::new();
    let mut fields = Vec::new();
    for (key, property) in properties {
        if companion(key).is_some() {
            continue;
        }
        let other_key = format!("{key}_custom");
        let other = (companion(&other_key).as_ref() == Some(key)).then_some(other_key);
        let (title, description, options, multiple) = match property {
            ElicitationPropertySchema::String(text) => {
                (&text.title, &text.description, options_of(text), false)
            }
            ElicitationPropertySchema::Array(many) => {
                let options = match &many.items {
                    MultiSelectItems::Titled(titled) => titled.options.clone(),
                    MultiSelectItems::String(plain) => plain
                        .values
                        .iter()
                        .map(|value| EnumOption::new(value, value))
                        .collect(),
                    _ => return None,
                };
                (&many.title, &many.description, options, true)
            }
            _ => return None,
        };
        let other = if options.is_empty() {
            Some(key.clone())
        } else {
            other
        };
        questions.push(Question {
            header: title.clone(),
            text: description.clone(),
            options: options
                .iter()
                .map(|option| QuestionOption {
                    label: option.title.clone(),
                    description: option.description.clone(),
                })
                .collect(),
            multiple,
            other: other.is_some(),
        });
        fields.push(Field {
            key: key.clone(),
            values: options.into_iter().map(|option| option.value).collect(),
            multiple,
            other,
        });
    }
    Some((questions, fields))
}

/// The options of a text field: its titled `oneOf`, or else its plain
/// `enum`, each titled with its value.
fn options_of(text: &StringPropertySchema) -> Vec<EnumOption> {
    if let Some(options) = &text.one_of {
        return options.clone();
    }
    text.enum_values
        .iter()
        .flatten()
        .map(|value| EnumOption::new(value, value))
        .collect()
}

/// The form's content for what the human `picked` of each question whose
/// answer goes in `fields`: the options chosen, and their own words, in
/// the fields that take them. What they left blank is left out.
fn content(fields: &[Field], picked: &[Picked]) -> BTreeMap<String, ElicitationContentValue> {
    let mut content = BTreeMap::new();
    for (field, picked) in fields.iter().zip(picked) {
        let values: Vec<String> = picked
            .chosen
            .iter()
            .filter_map(|&index| field.values.get(index).cloned())
            .collect();
        if field.multiple && !values.is_empty() {
            content.insert(field.key.clone(), ElicitationContentValue::from(values));
        } else if let Some(value) = values.into_iter().next() {
            content.insert(field.key.clone(), ElicitationContentValue::from(value));
        }
        let words = picked.other.as_deref().map(str::trim);
        if let (Some(key), Some(words)) = (&field.other, words)
            && !words.is_empty()
        {
            content.insert(key.clone(), ElicitationContentValue::from(words));
        }
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request `claude-agent-acp` sends for an `AskUserQuestion` of
    /// two questions, the second taking several options, as it writes it.
    fn asked_twice() -> CreateElicitationRequest {
        serde_json::from_value(serde_json::json!({
            "mode": "form",
            "sessionId": "s",
            "toolCallId": "toolu_1",
            "message": "Please answer the following questions.",
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "question_0": {
                        "type": "string",
                        "title": "Language",
                        "description": "Which language?",
                        "oneOf": [
                            { "const": "Rust", "title": "Rust", "description": "Fast and safe" },
                            { "const": "Go", "title": "Go" }
                        ]
                    },
                    "question_0_custom": {
                        "type": "string",
                        "title": "Other",
                        "description": "Type your own answer, or add a note to the option you chose above (optional)."
                    },
                    "question_1": {
                        "type": "array",
                        "title": "Fruit",
                        "description": "Which fruits?",
                        "items": { "anyOf": [
                            { "const": "Apple", "title": "Apple" },
                            { "const": "Pear", "title": "Pear" },
                            { "const": "Plum", "title": "Plum" }
                        ] }
                    },
                    "question_1_custom": {
                        "type": "string",
                        "title": "Other",
                        "description": "Type your own answer to add to your selection above (optional).",
                        "_meta": { "jetbrains": { "air": { "customAnswer": { "questionId": "question_1", "isCustomAnswer": true } } } }
                    }
                }
            }
        }))
        .unwrap()
    }

    fn read_request(request: &CreateElicitationRequest) -> Option<(Vec<Question>, Vec<Field>)> {
        match &request.mode {
            ElicitationMode::Form(form) => read(&form.requested_schema),
            _ => None,
        }
    }

    #[test]
    fn a_question_of_claude_reads_as_its_options_with_room_for_other_words() {
        let (questions, _) = read_request(&asked_twice()).unwrap();
        assert_eq!(
            questions,
            [
                Question {
                    header: Some("Language".to_owned()),
                    text: Some("Which language?".to_owned()),
                    options: vec![
                        QuestionOption {
                            label: "Rust".to_owned(),
                            description: Some("Fast and safe".to_owned()),
                        },
                        QuestionOption {
                            label: "Go".to_owned(),
                            description: None,
                        },
                    ],
                    multiple: false,
                    other: true,
                },
                Question {
                    header: Some("Fruit".to_owned()),
                    text: Some("Which fruits?".to_owned()),
                    options: ["Apple", "Pear", "Plum"]
                        .map(|label| QuestionOption {
                            label: label.to_owned(),
                            description: None,
                        })
                        .to_vec(),
                    multiple: true,
                    other: true,
                },
            ]
        );
    }

    #[test]
    fn the_answer_fills_the_fields_claude_reads_it_from() {
        let (_, fields) = read_request(&asked_twice()).unwrap();
        let answer = content(
            &fields,
            &[
                Picked {
                    chosen: vec![1],
                    other: Some("  ".to_owned()),
                },
                Picked {
                    chosen: vec![0, 2],
                    other: Some("Kiwi".to_owned()),
                },
            ],
        );
        assert_eq!(
            serde_json::to_value(&answer).unwrap(),
            serde_json::json!({
                "question_0": "Go",
                "question_1": ["Apple", "Plum"],
                "question_1_custom": "Kiwi",
            })
        );
    }

    #[test]
    fn a_question_skipped_leaves_its_fields_out() {
        let (_, fields) = read_request(&asked_twice()).unwrap();
        let answer = content(
            &fields,
            &[
                Picked {
                    chosen: Vec::new(),
                    other: Some("Zig".to_owned()),
                },
                Picked::default(),
            ],
        );
        assert_eq!(
            serde_json::to_value(&answer).unwrap(),
            serde_json::json!({ "question_0_custom": "Zig" })
        );
    }

    #[test]
    fn a_text_field_alone_is_a_question_answered_in_words() {
        let request: CreateElicitationRequest = serde_json::from_value(serde_json::json!({
            "mode": "form",
            "sessionId": "s",
            "message": "Sign in to the server.",
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "title": "Name" },
                    "color": { "type": "string", "enum": ["red", "blue"] }
                }
            }
        }))
        .unwrap();
        let (questions, fields) = read_request(&request).unwrap();
        assert_eq!(
            questions
                .iter()
                .map(|question| (question.options.len(), question.other))
                .collect::<Vec<_>>(),
            [(2, false), (0, true)]
        );
        let answer = content(
            &fields,
            &[
                Picked {
                    chosen: vec![1],
                    other: None,
                },
                Picked {
                    chosen: Vec::new(),
                    other: Some("Ada".to_owned()),
                },
            ],
        );
        assert_eq!(
            serde_json::to_value(&answer).unwrap(),
            serde_json::json!({ "color": "blue", "name": "Ada" })
        );
    }

    #[test]
    fn a_form_of_a_field_cox_does_not_ask_is_not_read() {
        let request: CreateElicitationRequest = serde_json::from_value(serde_json::json!({
            "mode": "form",
            "sessionId": "s",
            "message": "How many?",
            "requestedSchema": {
                "type": "object",
                "properties": { "count": { "type": "integer" } }
            }
        }))
        .unwrap();
        assert_eq!(read_request(&request), None);
        let link: CreateElicitationRequest = serde_json::from_value(serde_json::json!({
            "mode": "url",
            "sessionId": "s",
            "message": "Sign in.",
            "url": "https://example.com",
            "elicitationId": "e"
        }))
        .unwrap();
        assert_eq!(read_request(&link), None);
    }
}
