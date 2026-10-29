//! What one chat's agent sees of the browser, decided message by
//! message, with no connection in sight: its chat's pages, the tabs that
//! hold them, and the targets the agent attaches to within them, such as
//! the pages' frames and workers. Other chats' pages, and the rest of the
//! browser, stay hidden: the agent hears nothing of them, and what it
//! asks of them is refused, as if they were not there.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::{Value, json};

use super::Known;

/// What one chat's agent sees of the browser, as it has come to know it.
#[derive(Debug, Default)]
pub(super) struct Scope {
    /// The chat's pages and the tabs that hold them, each with its page:
    /// the agent closes them through the stage.
    own: HashMap<String, String>,
    /// How many of the chat's pages are in `own`.
    learned: usize,
    /// The targets the agent sees.
    targets: HashSet<String>,
    /// The sessions, attached to those targets, that the agent sees, each
    /// with the page it is in, if it is in one.
    sessions: HashMap<String, Option<String>>,
    /// The ids of the agent's requests for the list of targets, whose
    /// answers name every target.
    listings: HashSet<u64>,
}

/// What becomes of a message from the agent.
#[derive(Debug, PartialEq)]
pub(super) enum Request {
    Pass,
    /// Not for the browser: the answer goes back to the agent.
    Refuse(String),
    /// For the stage instead of the browser.
    Ask(Ask),
}

/// What the agent asks of the stage.
#[derive(Debug, PartialEq)]
pub(super) enum Ask {
    Open { url: String, background: bool },
    Close { page: String },
}

/// What becomes of a message from the browser.
#[derive(Debug, PartialEq)]
pub(super) enum Reply {
    Pass,
    /// The agent gets this instead.
    Replace(String),
    Drop,
    /// The browser attached to a target the agent does not see, in
    /// `session`: the agent hears nothing of it, and the relay lets go of
    /// it, starting it first if it is `waiting`.
    Hide {
        session: String,
        waiting: bool,
    },
}

/// The parts of a message from the browser that say whom it is about,
/// read without taking a screenshot's data apart.
#[derive(Deserialize)]
struct Head<'a> {
    id: Option<u64>,
    method: Option<&'a str>,
    #[serde(rename = "sessionId")]
    session: Option<&'a str>,
}

impl Scope {
    /// Takes in the chat's pages it has not seen yet.
    pub(super) fn learn(&mut self, known: &Known) {
        for (page, tab) in known.pages.iter().skip(self.learned) {
            for target in [Some(page), tab.as_ref()].into_iter().flatten() {
                self.targets.insert(target.clone());
                self.own.insert(target.clone(), page.clone());
            }
        }
        self.learned = known.pages.len();
    }

    /// The page that `session` is in, if any.
    pub(super) fn page_of(&self, session: &str) -> Option<String> {
        self.sessions.get(session).cloned().flatten()
    }

    /// What becomes of `request`, from the agent.
    pub(super) fn request(&mut self, request: &Value) -> Request {
        let params = &request["params"];
        let session = request["sessionId"].as_str();
        let target = params["targetId"].as_str();
        let refused = match request["method"].as_str().unwrap_or_default() {
            _ if session.is_some_and(|session| !self.sessions.contains_key(session)) => {
                Some("No session with given id")
            }
            "Target.createTarget" => {
                let url = params["url"].as_str().filter(|url| !url.is_empty());
                return Request::Ask(Ask::Open {
                    url: url.unwrap_or("about:blank").to_owned(),
                    background: params["background"] == true,
                });
            }
            "Browser.close" | "Browser.crash" | "Browser.crashGpuProcess" => {
                Some("The browser is Cox's, and stays open")
            }
            "Target.closeTarget"
                if let Some(page) = target.and_then(|target| self.own.get(target)) =>
            {
                return Request::Ask(Ask::Close { page: page.clone() });
            }
            _ if target.is_some_and(|target| !self.targets.contains(target)) => {
                Some("No target with given id found")
            }
            "Target.detachFromTarget" | "Target.sendMessageToTarget"
                if params["sessionId"]
                    .as_str()
                    .is_some_and(|session| !self.sessions.contains_key(session)) =>
            {
                Some("No session with given id")
            }
            "Target.getTargets" => {
                if let Some(id) = request["id"].as_u64() {
                    self.listings.insert(id);
                }
                None
            }
            _ => None,
        };
        let Some(refused) = refused else {
            return Request::Pass;
        };
        let mut answer = json!({
            "id": request["id"],
            "error": { "code": -32000, "message": refused },
        });
        if let Some(session) = session {
            answer["sessionId"] = session.into();
        }
        Request::Refuse(answer.to_string())
    }

    /// What becomes of `reply`, from the browser.
    pub(super) fn reply(&mut self, reply: &str) -> Reply {
        let Ok(head) = serde_json::from_str::<Head<'_>>(reply) else {
            return Reply::Pass;
        };
        if head
            .session
            .is_some_and(|session| !self.sessions.contains_key(session))
        {
            return Reply::Drop;
        }
        if let Some(id) = head.id {
            if head.session.is_none() && self.listings.remove(&id) {
                return self.listing(reply);
            }
            return Reply::Pass;
        }
        match head.method {
            Some(method) if method.starts_with("Target.") => {
                let Ok(event) = serde_json::from_str::<Value>(reply) else {
                    return Reply::Pass;
                };
                self.target_event(method, head.session, &event["params"])
            }
            _ => Reply::Pass,
        }
    }

    /// The answer to the agent's request for the list of targets, naming
    /// those it sees only.
    fn listing(&self, reply: &str) -> Reply {
        let Ok(mut reply) = serde_json::from_str::<Value>(reply) else {
            return Reply::Pass;
        };
        if let Some(infos) = reply["result"]["targetInfos"].as_array_mut() {
            infos.retain(|info| self.sees(info));
        }
        Reply::Replace(reply.to_string())
    }

    /// Whether the agent sees the target `info` describes. The browser
    /// itself has nothing of other chats in it. A page CEF has just made is
    /// a target of the type `other` until it turns into a page: the agent
    /// sees it once it has, through its tab, or its tools never take it
    /// for a page.
    fn sees(&self, info: &Value) -> bool {
        info["type"] == "browser"
            || (info["type"] != "other"
                && info["targetId"]
                    .as_str()
                    .is_some_and(|target| self.targets.contains(target)))
    }

    /// What becomes of an event of the `Target` domain, sent in `session`
    /// (`None` for the browser's own) with `params`.
    fn target_event(&mut self, method: &str, session: Option<&str>, params: &Value) -> Reply {
        let shown = |seen: bool| if seen { Reply::Pass } else { Reply::Drop };
        match method {
            "Target.targetCreated" | "Target.targetInfoChanged" => {
                shown(self.sees(&params["targetInfo"]))
            }
            "Target.targetDestroyed" | "Target.targetCrashed" => shown(
                params["targetId"]
                    .as_str()
                    .is_some_and(|target| self.targets.contains(target)),
            ),
            "Target.attachedToTarget" => {
                let Some(attached) = params["sessionId"].as_str() else {
                    return Reply::Drop;
                };
                let info = &params["targetInfo"];
                // What attaches within a session the agent sees is part of
                // its pages.
                if session.is_some() || self.sees(info) {
                    let target = info["targetId"].as_str();
                    if let Some(target) = target {
                        self.targets.insert(target.to_owned());
                    }
                    // A page's frames and workers are in the page.
                    let page = if info["type"] == "page" {
                        target.map(str::to_owned)
                    } else {
                        session.and_then(|session| self.page_of(session))
                    };
                    self.sessions.insert(attached.to_owned(), page);
                    Reply::Pass
                } else {
                    Reply::Hide {
                        session: attached.to_owned(),
                        waiting: params["waitingForDebugger"] == true,
                    }
                }
            }
            "Target.detachedFromTarget" | "Target.receivedMessageFromTarget" => shown(
                params["sessionId"]
                    .as_str()
                    .is_some_and(|attached| self.sessions.contains_key(attached)),
            ),
            _ => Reply::Pass,
        }
    }
}

/// The relay's requests that let go of the target attached in `session`,
/// which the agent does not see: it may be waiting to start until someone
/// lets it, as a new page does when the agent asked to attach to every
/// new target.
pub(super) fn let_go(session: &str, waiting: bool, mut own_id: impl FnMut() -> u64) -> Vec<String> {
    let mut requests = Vec::new();
    if waiting {
        requests.push(
            json!({
                "id": own_id(),
                "sessionId": session,
                "method": "Runtime.runIfWaitingForDebugger",
            })
            .to_string(),
        );
    }
    requests.push(
        json!({
            "id": own_id(),
            "method": "Target.detachFromTarget",
            "params": { "sessionId": session },
        })
        .to_string(),
    );
    requests
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scope of a chat with the page `PAGE` in the tab `TAB`.
    fn page_scope() -> Scope {
        let mut scope = Scope::default();
        scope.learn(&Known {
            opening: 0,
            pages: vec![("PAGE".to_owned(), Some("TAB".to_owned()))],
        });
        scope
    }

    fn request(scope: &mut Scope, request: &str) -> Request {
        scope.request(&serde_json::from_str(request).unwrap())
    }

    #[test]
    fn the_agent_sees_its_tab_and_page_and_what_attaches_in_them() {
        let mut scope = page_scope();
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","params":{"sessionId":"T1","targetInfo":{"targetId":"TAB","type":"tab"},"waitingForDebugger":false}}"#
            ),
            Reply::Pass
        );
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":5,"sessionId":"T1","method":"Target.setAutoAttach","params":{}}"#
            ),
            Request::Pass
        );
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","sessionId":"T1","params":{"sessionId":"P1","targetInfo":{"targetId":"PAGE","type":"page"},"waitingForDebugger":false}}"#
            ),
            Reply::Pass
        );
        // A frame of the page, attached within it.
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","sessionId":"P1","params":{"sessionId":"F1","targetInfo":{"targetId":"FRAME","type":"iframe"},"waitingForDebugger":true}}"#
            ),
            Reply::Pass
        );
        assert_eq!(
            scope.reply(r#"{"method":"Page.loadEventFired","params":{},"sessionId":"F1"}"#),
            Reply::Pass
        );
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.targetInfoChanged","params":{"targetInfo":{"targetId":"FRAME","type":"iframe"}}}"#
            ),
            Reply::Pass
        );
    }

    #[test]
    fn other_pages_are_hidden_and_let_go() {
        let mut scope = page_scope();
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"OTHER","type":"page"}}}"#
            ),
            Reply::Drop
        );
        // Another session opened its page while the agent asked to attach
        // to every new tab: it waits to start until someone lets it.
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","params":{"sessionId":"T2","targetInfo":{"targetId":"OTHER-TAB","type":"tab"},"waitingForDebugger":true}}"#
            ),
            Reply::Hide {
                session: "T2".to_owned(),
                waiting: true
            }
        );
        assert_eq!(
            scope.reply(r#"{"method":"Target.detachedFromTarget","params":{"sessionId":"T2"}}"#),
            Reply::Drop
        );
        assert_eq!(
            scope.reply(r#"{"method":"Runtime.consoleAPICalled","params":{},"sessionId":"T2"}"#),
            Reply::Drop
        );
        let mut id = 9;
        assert_eq!(
            let_go("T2", true, || {
                id += 1;
                id
            }),
            [
                r#"{"id":10,"sessionId":"T2","method":"Runtime.runIfWaitingForDebugger"}"#,
                r#"{"id":11,"method":"Target.detachFromTarget","params":{"sessionId":"T2"}}"#,
            ]
        );
    }

    #[test]
    fn the_list_of_targets_names_only_those_the_agent_sees() {
        let mut scope = page_scope();
        assert_eq!(
            request(&mut scope, r#"{"id":7,"method":"Target.getTargets"}"#),
            Request::Pass
        );
        let Reply::Replace(listing) = scope.reply(
            r#"{"id":7,"result":{"targetInfos":[{"targetId":"PAGE","type":"page"},{"targetId":"OTHER","type":"page"},{"targetId":"B","type":"browser"}]}}"#,
        ) else {
            panic!("the list is cut down");
        };
        assert_eq!(
            listing,
            r#"{"id":7,"result":{"targetInfos":[{"targetId":"PAGE","type":"page"},{"targetId":"B","type":"browser"}]}}"#
        );
        // Other answers go through as they are.
        assert_eq!(scope.reply(r#"{"id":8,"result":{}}"#), Reply::Pass);
    }

    #[test]
    fn what_the_agent_asks_of_other_targets_is_refused() {
        let mut scope = page_scope();
        let refused = |scope: &mut Scope, message: &str| {
            matches!(request(scope, message), Request::Refuse(_))
        };
        assert!(refused(
            &mut scope,
            r#"{"id":1,"method":"Target.attachToTarget","params":{"targetId":"OTHER","flatten":true}}"#
        ));
        assert!(refused(
            &mut scope,
            r#"{"id":2,"sessionId":"T2","method":"Runtime.evaluate","params":{"expression":"1"}}"#
        ));
        assert!(refused(
            &mut scope,
            r#"{"id":3,"method":"Target.closeTarget","params":{"targetId":"OTHER"}}"#
        ));
        assert!(refused(&mut scope, r#"{"id":5,"method":"Browser.close"}"#));
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":6,"method":"Target.attachToTarget","params":{"targetId":"PAGE","flatten":true}}"#
            ),
            Request::Pass
        );
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":7,"method":"Target.attachToTarget","params":{"targetId":"OTHER"}}"#
            ),
            Request::Refuse(
                r#"{"id":7,"error":{"code":-32000,"message":"No target with given id found"}}"#
                    .to_owned()
            )
        );
    }

    #[test]
    fn the_agent_opens_and_closes_its_tabs_through_the_stage() {
        let mut scope = page_scope();
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":1,"method":"Target.createTarget","params":{"url":"https://example.com/"}}"#
            ),
            Request::Ask(Ask::Open {
                url: "https://example.com/".to_owned(),
                background: false
            })
        );
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":2,"method":"Target.createTarget","params":{"url":"","background":true}}"#
            ),
            Request::Ask(Ask::Open {
                url: "about:blank".to_owned(),
                background: true
            })
        );
        // A page closes by its own id, or its tab's.
        for target in ["PAGE", "TAB"] {
            assert_eq!(
                request(
                    &mut scope,
                    &format!(
                        r#"{{"id":3,"method":"Target.closeTarget","params":{{"targetId":"{target}"}}}}"#
                    )
                ),
                Request::Ask(Ask::Close {
                    page: "PAGE".to_owned()
                })
            );
        }
    }

    #[test]
    fn the_agent_sees_the_pages_its_chat_opens_later() {
        let mut scope = page_scope();
        let mut known = Known {
            opening: 0,
            pages: vec![("PAGE".to_owned(), Some("TAB".to_owned()))],
        };
        let created = r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"NEW-TAB","type":"tab"}}}"#;
        assert_eq!(scope.reply(created), Reply::Drop);

        known
            .pages
            .push(("NEW".to_owned(), Some("NEW-TAB".to_owned())));
        scope.learn(&known);
        assert_eq!(scope.reply(created), Reply::Pass);
        // CEF makes the page as a target of another type first, which the
        // browser attaches the agent to; it attaches again as a page in
        // its tab.
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"NEW","type":"other"}}}"#
            ),
            Reply::Drop
        );
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","params":{"sessionId":"O1","targetInfo":{"targetId":"NEW","type":"other"},"waitingForDebugger":true}}"#
            ),
            Reply::Hide {
                session: "O1".to_owned(),
                waiting: true
            }
        );
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.targetInfoChanged","params":{"targetInfo":{"targetId":"NEW","type":"page"}}}"#
            ),
            Reply::Pass
        );
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":1,"method":"Target.closeTarget","params":{"targetId":"NEW-TAB"}}"#
            ),
            Request::Ask(Ask::Close {
                page: "NEW".to_owned()
            })
        );
    }

    #[test]
    fn a_session_is_in_the_page_it_is_attached_to_or_within() {
        let mut scope = page_scope();
        for attached in [
            r#"{"method":"Target.attachedToTarget","params":{"sessionId":"T1","targetInfo":{"targetId":"TAB","type":"tab"},"waitingForDebugger":false}}"#,
            r#"{"method":"Target.attachedToTarget","sessionId":"T1","params":{"sessionId":"P1","targetInfo":{"targetId":"PAGE","type":"page"},"waitingForDebugger":false}}"#,
            r#"{"method":"Target.attachedToTarget","sessionId":"P1","params":{"sessionId":"F1","targetInfo":{"targetId":"FRAME","type":"iframe"},"waitingForDebugger":false}}"#,
        ] {
            assert_eq!(scope.reply(attached), Reply::Pass);
        }
        assert_eq!(scope.page_of("T1"), None);
        assert_eq!(scope.page_of("P1"), Some("PAGE".to_owned()));
        assert_eq!(scope.page_of("F1"), Some("PAGE".to_owned()));
    }
}
