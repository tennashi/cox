//! What the relay sees of the agent's input to a page, on the agent's
//! own connection: the clicks, keys and text it sends, and the element
//! it types into, which the relay asks the page for itself and keeps
//! from the agent.

use serde_json::{Value, json};

use super::AgentInput;

/// The ids of the relay's own requests, far above those the agent's tools
/// count up from, and still within the 32-bit ids the browser takes.
pub(super) const OWN_IDS: u64 = 2_000_000_000;

/// Finds the focused element, inside shadow roots too, and answers its
/// bounds in the viewport, or `null` when the page has no focus.
const FOCUSED_BOUNDS: &str = "(() => {\
    let element = document.activeElement;\
    while (element?.shadowRoot?.activeElement) element = element.shadowRoot.activeElement;\
    if (!element || element === document.body) return null;\
    const bounds = element.getBoundingClientRect();\
    return [bounds.x, bounds.y, bounds.width, bounds.height];\
})()";

/// The work a CDP message from the agent does on a page, if any, and the
/// session of the page it goes to: the input it sends the page, or `None`
/// when it navigates the page or brings it to the front.
pub(super) fn observe(message: &Value) -> Option<(Option<AgentInput>, Option<String>)> {
    let params = &message["params"];
    let at = || Some((float(&params["x"])?, float(&params["y"])?));
    let input = match message["method"].as_str()? {
        "Input.dispatchMouseEvent" => {
            let (x, y) = at()?;
            Some(match params["type"].as_str()? {
                "mousePressed" => AgentInput::Press { x, y },
                "mouseWheel" => AgentInput::Wheel { x, y },
                _ => AgentInput::Move { x, y },
            })
        }
        "Input.dispatchKeyEvent" | "Input.insertText" | "Input.imeSetComposition" => {
            Some(AgentInput::Type)
        }
        "Page.navigate" | "Page.navigateToHistoryEntry" | "Page.reload" | "Page.bringToFront" => {
            None
        }
        _ => return None,
    };
    let session = message["sessionId"].as_str().map(str::to_owned);
    Some((input, session))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "positions on a page fit an f32"
)]
fn float(value: &Value) -> Option<f32> {
    value.as_f64().map(|value| value as f32)
}

/// The relay's request, numbered `id`, for the bounds of the focused
/// element in the page of `session`.
pub(super) fn ask_focused(id: u64, session: &str) -> String {
    json!({
        "id": id,
        "sessionId": session,
        "method": "Runtime.evaluate",
        "params": { "expression": FOCUSED_BOUNDS, "returnByValue": true },
    })
    .to_string()
}

/// Whom a message from the browser is for.
#[derive(Debug, PartialEq)]
pub(super) enum Answer {
    ToAgent,
    /// The answer to the relay's own request, with the field it names, in
    /// the page of `session`.
    ToRelay {
        field: Option<AgentInput>,
        session: Option<String>,
    },
}

pub(super) fn answer(message: &str) -> Answer {
    // The relay's answers are short; the agent's screenshots are not.
    if message.len() > 1024 || !message.contains("\"id\"") {
        return Answer::ToAgent;
    }
    let Ok(message) = serde_json::from_str::<Value>(message) else {
        return Answer::ToAgent;
    };
    if message["id"].as_u64().is_none_or(|id| id < OWN_IDS) {
        return Answer::ToAgent;
    }
    let bounds = &message["result"]["result"]["value"];
    let field = || {
        Some(AgentInput::Field {
            x: float(&bounds[0])?,
            y: float(&bounds[1])?,
            width: float(&bounds[2])?,
            height: float(&bounds[3])?,
        })
    };
    Answer::ToRelay {
        field: field(),
        session: message["sessionId"].as_str().map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(message: &str) -> Option<(Option<AgentInput>, Option<String>)> {
        observe(&serde_json::from_str(message).unwrap())
    }

    fn input(message: &str) -> Option<AgentInput> {
        observed(message).and_then(|(input, _)| input)
    }

    #[test]
    fn mouse_events_are_seen_where_they_happen() {
        let event = |kind: &str| {
            format!(
                r#"{{"id":7,"method":"Input.dispatchMouseEvent","sessionId":"S","params":{{"type":"{kind}","x":120.5,"y":40,"button":"left"}}}}"#
            )
        };
        assert_eq!(
            input(&event("mouseMoved")),
            Some(AgentInput::Move { x: 120.5, y: 40. })
        );
        assert_eq!(
            observed(&event("mousePressed")),
            Some((
                Some(AgentInput::Press { x: 120.5, y: 40. }),
                Some("S".to_owned())
            ))
        );
        assert_eq!(
            input(&event("mouseReleased")),
            Some(AgentInput::Move { x: 120.5, y: 40. })
        );
        assert_eq!(
            input(&event("mouseWheel")),
            Some(AgentInput::Wheel { x: 120.5, y: 40. })
        );
    }

    #[test]
    fn keys_and_text_are_typing() {
        assert_eq!(
            input(r#"{"id":8,"method":"Input.insertText","params":{"text":"Rust"}}"#),
            Some(AgentInput::Type)
        );
        assert_eq!(
            input(
                r#"{"id":9,"method":"Input.dispatchKeyEvent","params":{"type":"keyDown","key":"Enter"}}"#
            ),
            Some(AgentInput::Type)
        );
    }

    #[test]
    fn navigating_is_work_on_the_page_with_no_input() {
        assert_eq!(
            observed(
                r#"{"id":1,"sessionId":"S","method":"Page.navigate","params":{"url":"https://example.com"}}"#
            ),
            Some((None, Some("S".to_owned())))
        );
        assert_eq!(
            observed(r#"{"id":2,"sessionId":"S","method":"Page.bringToFront"}"#),
            Some((None, Some("S".to_owned())))
        );
    }

    #[test]
    fn reading_the_page_is_no_work_on_it() {
        assert_eq!(
            observed(
                r#"{"id":2,"method":"Runtime.evaluate","params":{"expression":"\"Input.x\""}}"#
            ),
            None
        );
        assert_eq!(
            observed(r#"{"id":3,"sessionId":"S","method":"Page.captureScreenshot"}"#),
            None
        );
    }

    #[test]
    fn the_relay_keeps_its_own_answers() {
        assert_eq!(
            answer(
                r#"{"id":2000000003,"result":{"result":{"type":"object","value":[10,20.5,300,24]}},"sessionId":"S"}"#
            ),
            Answer::ToRelay {
                field: Some(AgentInput::Field {
                    x: 10.,
                    y: 20.5,
                    width: 300.,
                    height: 24.
                }),
                session: Some("S".to_owned()),
            }
        );
        assert_eq!(
            answer(
                r#"{"id":2000000004,"result":{"result":{"type":"object","subtype":"null","value":null}}}"#
            ),
            Answer::ToRelay {
                field: None,
                session: None
            }
        );
        assert_eq!(
            answer(r#"{"id":12,"result":{"result":{"type":"number","value":3}}}"#),
            Answer::ToAgent
        );
        assert_eq!(
            answer(r#"{"method":"Page.loadEventFired","params":{"timestamp":1}}"#),
            Answer::ToAgent
        );
    }
}
