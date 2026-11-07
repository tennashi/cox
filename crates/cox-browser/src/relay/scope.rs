//! What one chat's agent sees of the browser, decided message by
//! message, with no connection in sight: its chat's pages, the tabs that
//! hold them, and the targets the agent attaches to within them, such as
//! the pages' frames and workers. Other chats' pages, and the rest of the
//! browser, stay hidden: the agent hears nothing of them, and what it
//! asks of them is refused, as if they were not there. Of a page another
//! chat's agent is at work on, what acts on it is refused, saying so.

use std::cmp::Reverse;
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
    /// The chat's pages, as the scope learned them.
    pages: HashSet<String>,
    /// The targets the agent sees, each with the page it is in, if it is
    /// in one.
    targets: HashMap<String, Option<String>>,
    /// The sessions, attached to those targets, that the agent sees.
    sessions: HashMap<String, Attached>,
    /// What the browser last said of each target, the agent's or not, for
    /// the agent to hear of a page handed to its chat.
    infos: HashMap<String, Value>,
    /// How the agent asked the browser to attach it to the targets, if it
    /// did, for the browser to attach it to a page handed to its chat.
    auto_attach: Option<Value>,
    /// The ids of the agent's requests for the list of targets, whose
    /// answers name every target.
    listings: HashSet<u64>,
    /// The pages another chat's agent is at work on, each with that
    /// chat's name.
    locked: HashMap<String, String>,
}

/// A session the agent sees.
#[derive(Debug)]
struct Attached {
    /// The page it is in, if it is in one.
    page: Option<String>,
    /// The session it was attached within, if any.
    parent: Option<String>,
}

/// What the scope says, as it learns of the chat's pages.
#[derive(Debug, PartialEq)]
pub(super) enum Said {
    ToAgent(String),
    ToBrowser(String),
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
    /// Takes in the chat's pages as `known` has them now, and returns what
    /// it says to either end of it: the agent hears of a page handed to
    /// the chat, and the browser attaches the agent to it, as it asked to
    /// be attached to its pages; and of a page the chat no longer has, the
    /// agent hears that it is gone, and the browser lets go of it, the
    /// relay's own requests numbered by `own_id`.
    pub(super) fn learn(&mut self, known: &Known, mut own_id: impl FnMut() -> u64) -> Vec<Said> {
        let mut said = Vec::new();
        let mut handed = false;
        self.locked.clone_from(&known.locked);
        for known in &known.pages {
            let page = &known.page;
            if !self.pages.insert(page.clone()) {
                continue;
            }
            // The tab first, as the browser tells of it.
            for target in [known.tab.as_ref(), Some(page)].into_iter().flatten() {
                self.targets.insert(target.clone(), Some(page.clone()));
                self.own.insert(target.clone(), page.clone());
                if known.handed
                    && let Some(info) = self.infos.get(target)
                {
                    said.push(Said::ToAgent(
                        json!({ "method": "Target.targetCreated", "params": { "targetInfo": info } })
                            .to_string(),
                    ));
                }
            }
            handed |= known.handed;
        }
        // The browser attaches to the targets it is not attached to yet,
        // the handed page among them, as it was asked to.
        if handed && let Some(params) = &self.auto_attach {
            said.push(Said::ToBrowser(
                json!({ "id": own_id(), "method": "Target.setAutoAttach", "params": params })
                    .to_string(),
            ));
        }
        let mut gone: Vec<String> = self
            .pages
            .iter()
            .filter(|page| !known.has(page))
            .cloned()
            .collect();
        gone.sort();
        for page in gone {
            said.extend(self.forget(&page, &mut own_id));
        }
        said
    }

    /// Forgets the page `page`, and what attached within it, and returns
    /// what it says of that to either end.
    fn forget(&mut self, page: &str, own_id: &mut impl FnMut() -> u64) -> Vec<Said> {
        let mut said = Vec::new();
        self.pages.remove(page);
        self.own.retain(|_, of| of != page);
        let in_page = |of: &Option<String>| of.as_deref() == Some(page);
        let sessions: HashMap<String, Attached> = self
            .sessions
            .extract_if(|_, attached| in_page(&attached.page))
            .collect();
        // What attached within another session goes before it, as the
        // browser would say.
        let depth = |session: &str| {
            std::iter::successors(Some(session), |session| {
                sessions.get(*session)?.parent.as_deref()
            })
            .count()
        };
        let mut order: Vec<&String> = sessions.keys().collect();
        order.sort_by_key(|session| (Reverse(depth(session)), *session));
        for session in order {
            let parent = sessions[session].parent.as_ref();
            let mut detached = json!({
                "method": "Target.detachedFromTarget",
                "params": { "sessionId": session },
            });
            if let Some(parent) = parent {
                detached["sessionId"] = parent.as_str().into();
            }
            said.push(Said::ToAgent(detached.to_string()));
            // Letting go of a session lets go of those within it.
            if parent.is_none_or(|parent| !sessions.contains_key(parent)) {
                said.push(Said::ToBrowser(
                    json!({
                        "id": own_id(),
                        "method": "Target.detachFromTarget",
                        "params": { "sessionId": session },
                    })
                    .to_string(),
                ));
            }
        }
        let mut targets: Vec<String> = self
            .targets
            .extract_if(|_, of| in_page(of))
            .map(|(target, _)| target)
            .collect();
        targets.sort();
        for target in targets {
            said.push(Said::ToAgent(
                json!({ "method": "Target.targetDestroyed", "params": { "targetId": target } })
                    .to_string(),
            ));
        }
        said
    }

    /// The page that `session` is in, if any.
    pub(super) fn page_of(&self, session: &str) -> Option<String> {
        self.sessions
            .get(session)
            .and_then(|attached| attached.page.clone())
    }

    /// The chat whose agent is at work on the page `request` goes to, if
    /// it acts on the page and another chat's agent is at work on it.
    fn locked_by(&self, request: &Value) -> Option<&String> {
        let method = request["method"].as_str()?;
        if !acts(method) {
            return None;
        }
        let page = match request["sessionId"].as_str() {
            Some(session) => self.page_of(session),
            None => self
                .targets
                .get(request["params"]["targetId"].as_str()?)
                .cloned()
                .flatten(),
        }?;
        self.locked.get(&page)
    }

    /// What becomes of `request`, from the agent.
    pub(super) fn request(&mut self, request: &Value) -> Request {
        let params = &request["params"];
        let session = request["sessionId"].as_str();
        let target = params["targetId"].as_str();
        let method = request["method"].as_str().unwrap_or_default();
        let locked = self.locked_by(request).map(|chat| {
            format!(
                "This page is in use by the agent of the chat \"{chat}\", which is at work on it: \
                 read it, but leave it be until that chat's turn ends"
            )
        });
        let refused = match method {
            _ if session.is_some_and(|session| !self.sessions.contains_key(session)) => {
                Some("No session with given id".to_owned())
            }
            // Bringing the page to the front is how the agent's tools
            // choose it, to read it too: it stays where it is, and the
            // agent goes on as if it had come to the front.
            "Page.bringToFront" if locked.is_some() => {
                let mut answer = json!({ "id": request["id"], "result": {} });
                if let Some(session) = session {
                    answer["sessionId"] = session.into();
                }
                return Request::Refuse(answer.to_string());
            }
            _ if locked.is_some() => locked,
            "Target.createTarget" => {
                let url = params["url"].as_str().filter(|url| !url.is_empty());
                return Request::Ask(Ask::Open {
                    url: url.unwrap_or("about:blank").to_owned(),
                    background: params["background"] == true,
                });
            }
            "Browser.close" | "Browser.crash" | "Browser.crashGpuProcess" => {
                Some("The browser is Cox's, and stays open".to_owned())
            }
            "Target.closeTarget"
                if let Some(page) = target.and_then(|target| self.own.get(target)) =>
            {
                return Request::Ask(Ask::Close { page: page.clone() });
            }
            _ if target.is_some_and(|target| !self.targets.contains_key(target)) => {
                Some("No target with given id found".to_owned())
            }
            "Target.detachFromTarget" | "Target.sendMessageToTarget"
                if params["sessionId"]
                    .as_str()
                    .is_some_and(|session| !self.sessions.contains_key(session)) =>
            {
                Some("No session with given id".to_owned())
            }
            "Target.setAutoAttach" if session.is_none() => {
                self.auto_attach = (params["autoAttach"] == true).then(|| params.clone());
                None
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
    fn listing(&mut self, reply: &str) -> Reply {
        let Ok(mut reply) = serde_json::from_str::<Value>(reply) else {
            return Reply::Pass;
        };
        if let Some(infos) = reply["result"]["targetInfos"].as_array_mut() {
            for info in infos.iter() {
                self.heard_of(info);
            }
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
                    .is_some_and(|target| self.targets.contains_key(target)))
    }

    /// Keeps what the browser said of the target `info` describes.
    fn heard_of(&mut self, info: &Value) {
        if let Some(target) = info["targetId"].as_str() {
            self.infos.insert(target.to_owned(), info.clone());
        }
    }

    /// What becomes of an event of the `Target` domain, sent in `session`
    /// (`None` for the browser's own) with `params`.
    fn target_event(&mut self, method: &str, session: Option<&str>, params: &Value) -> Reply {
        let shown = |seen: bool| if seen { Reply::Pass } else { Reply::Drop };
        match method {
            "Target.targetCreated" | "Target.targetInfoChanged" => {
                self.heard_of(&params["targetInfo"]);
                shown(self.sees(&params["targetInfo"]))
            }
            "Target.targetDestroyed" | "Target.targetCrashed" => {
                let target = params["targetId"].as_str();
                if method == "Target.targetDestroyed"
                    && let Some(target) = target
                {
                    self.infos.remove(target);
                }
                shown(target.is_some_and(|target| self.targets.contains_key(target)))
            }
            "Target.attachedToTarget" => {
                let Some(attached) = params["sessionId"].as_str() else {
                    return Reply::Drop;
                };
                let info = &params["targetInfo"];
                self.heard_of(info);
                // What attaches within a session the agent sees is part of
                // its pages.
                if session.is_some() || self.sees(info) {
                    let target = info["targetId"].as_str();
                    // A page is its own; its tab holds it; its frames and
                    // workers are in it.
                    let page = if info["type"] == "page" {
                        target.map(str::to_owned)
                    } else if let Some(page) = target.and_then(|target| self.own.get(target)) {
                        Some(page.clone())
                    } else {
                        session.and_then(|session| self.page_of(session))
                    };
                    if let Some(target) = target {
                        self.targets.insert(target.to_owned(), page.clone());
                    }
                    self.sessions.insert(
                        attached.to_owned(),
                        Attached {
                            page,
                            parent: session.map(str::to_owned),
                        },
                    );
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

/// Whether `method` acts on the page it goes to, as the human would, rather
/// than reads it: input, going to another address, a dialog's answer, a
/// file chosen, coming to the front, or closing it. A script the agent
/// runs on the page is not told apart from the scripts its tools read the
/// page with, and is not refused.
fn acts(method: &str) -> bool {
    method.starts_with("Input.")
        || matches!(
            method,
            "Page.navigate"
                | "Page.navigateToHistoryEntry"
                | "Page.reload"
                | "Page.stopLoading"
                | "Page.bringToFront"
                | "Page.close"
                | "Page.handleJavaScriptDialog"
                | "Page.setDocumentContent"
                | "DOM.setFileInputFiles"
                | "Target.activateTarget"
                | "Target.closeTarget"
        )
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
    use crate::relay::KnownPage;

    /// The page `page` in the tab `tab`, handed to the chat if `handed`.
    fn known(page: &str, tab: &str, handed: bool) -> KnownPage {
        KnownPage {
            page: page.to_owned(),
            tab: Some(tab.to_owned()),
            handed,
        }
    }

    /// The scope of a chat with the page `PAGE` in the tab `TAB`.
    fn page_scope() -> Scope {
        let mut scope = Scope::default();
        let said = scope.learn(
            &Known {
                pages: vec![known("PAGE", "TAB", false)],
                ..Known::default()
            },
            || 0,
        );
        assert_eq!(said, []);
        scope
    }

    /// What the agent is told, of what `said` says.
    fn to_agent(said: &[Said]) -> Vec<&str> {
        said.iter()
            .filter_map(|said| match said {
                Said::ToAgent(text) => Some(text.as_str()),
                Said::ToBrowser(_) => None,
            })
            .collect()
    }

    /// What the browser is asked, of what `said` says.
    fn to_browser(said: &[Said]) -> Vec<&str> {
        said.iter()
            .filter_map(|said| match said {
                Said::ToBrowser(text) => Some(text.as_str()),
                Said::ToAgent(_) => None,
            })
            .collect()
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
    fn a_page_another_chat_is_at_work_on_is_read_but_not_acted_on() {
        let mut scope = page_scope();
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","params":{"sessionId":"P1","targetInfo":{"targetId":"PAGE","type":"page"},"waitingForDebugger":false}}"#
            ),
            Reply::Pass
        );
        let click = r#"{"id":4,"sessionId":"P1","method":"Input.dispatchMouseEvent","params":{"type":"mousePressed","x":1,"y":2}}"#;
        let read =
            r#"{"id":5,"sessionId":"P1","method":"Accessibility.getFullAXTree","params":{}}"#;
        let front = r#"{"id":6,"sessionId":"P1","method":"Page.bringToFront","params":{}}"#;
        let close = r#"{"id":7,"method":"Target.closeTarget","params":{"targetId":"TAB"}}"#;
        assert_eq!(request(&mut scope, click), Request::Pass);

        let mut known = Known {
            pages: vec![known("PAGE", "TAB", false)],
            ..Known::default()
        };
        known
            .locked
            .insert("PAGE".to_owned(), "fix the build".to_owned());
        assert_eq!(scope.learn(&known, || 0), []);
        let Request::Refuse(refused) = request(&mut scope, click) else {
            panic!("a click on the page is not refused");
        };
        let refused: Value = serde_json::from_str(&refused).unwrap();
        assert_eq!(refused["id"], 4);
        assert_eq!(refused["sessionId"], "P1");
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("\"fix the build\""),
            "{refused}"
        );
        assert!(matches!(
            request(&mut scope, close),
            Request::Refuse(refused) if refused.contains("in use")
        ));
        // The agent reads it, and its tools choose it as they would, the
        // page staying where it is.
        assert_eq!(request(&mut scope, read), Request::Pass);
        assert_eq!(
            request(&mut scope, front),
            Request::Refuse(r#"{"id":6,"result":{},"sessionId":"P1"}"#.to_owned())
        );

        // Once that chat's turn ends, the page is the agent's to act on.
        known.locked.clear();
        assert_eq!(scope.learn(&known, || 0), []);
        assert_eq!(request(&mut scope, click), Request::Pass);
        assert_eq!(request(&mut scope, front), Request::Pass);
    }

    #[test]
    fn the_agent_sees_the_pages_its_chat_opens_later() {
        let mut scope = page_scope();
        let mut known = Known {
            pages: vec![known("PAGE", "TAB", false)],
            ..Known::default()
        };
        let created = r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"NEW-TAB","type":"tab"}}}"#;
        assert_eq!(scope.reply(created), Reply::Drop);

        known
            .pages
            .push(super::tests::known("NEW", "NEW-TAB", false));
        // The browser tells of a page that opens: the scope says nothing.
        assert_eq!(scope.learn(&known, || 0), []);
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
    fn a_session_is_in_the_page_it_is_attached_to_its_tab_or_within() {
        let mut scope = page_scope();
        for attached in [
            r#"{"method":"Target.attachedToTarget","params":{"sessionId":"T1","targetInfo":{"targetId":"TAB","type":"tab"},"waitingForDebugger":false}}"#,
            r#"{"method":"Target.attachedToTarget","sessionId":"T1","params":{"sessionId":"P1","targetInfo":{"targetId":"PAGE","type":"page"},"waitingForDebugger":false}}"#,
            r#"{"method":"Target.attachedToTarget","sessionId":"P1","params":{"sessionId":"F1","targetInfo":{"targetId":"FRAME","type":"iframe"},"waitingForDebugger":false}}"#,
        ] {
            assert_eq!(scope.reply(attached), Reply::Pass);
        }
        for session in ["T1", "P1", "F1"] {
            assert_eq!(scope.page_of(session), Some("PAGE".to_owned()));
        }
    }

    /// The agent asked to hear of every target and to be attached to them,
    /// while the browser had the page `OTHER`, in `OTHER-TAB`, of no chat:
    /// the agent heard of neither, and the relay let go of the tab.
    fn scope_with_other_page() -> Scope {
        let mut scope = page_scope();
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":1,"method":"Target.setDiscoverTargets","params":{"discover":true}}"#
            ),
            Request::Pass
        );
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":2,"method":"Target.setAutoAttach","params":{"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}}"#
            ),
            Request::Pass
        );
        for created in [
            r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"OTHER-TAB","type":"tab","url":"https://a.example/"}}}"#,
            r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"OTHER","type":"page","url":"https://a.example/"}}}"#,
        ] {
            assert_eq!(scope.reply(created), Reply::Drop);
        }
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","params":{"sessionId":"OT1","targetInfo":{"targetId":"OTHER-TAB","type":"tab","url":"https://a.example/"},"waitingForDebugger":false}}"#
            ),
            Reply::Hide {
                session: "OT1".to_owned(),
                waiting: false
            }
        );
        scope
    }

    #[test]
    fn the_agent_hears_of_a_page_handed_to_its_chat_and_is_attached_to_it() {
        let mut scope = scope_with_other_page();
        let said = scope.learn(
            &Known {
                pages: vec![
                    known("PAGE", "TAB", false),
                    known("OTHER", "OTHER-TAB", true),
                ],
                ..Known::default()
            },
            || 9,
        );
        assert_eq!(
            to_agent(&said),
            [
                r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"OTHER-TAB","type":"tab","url":"https://a.example/"}}}"#,
                r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"OTHER","type":"page","url":"https://a.example/"}}}"#,
            ]
        );
        assert_eq!(
            to_browser(&said),
            [
                r#"{"id":9,"method":"Target.setAutoAttach","params":{"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}}"#
            ]
        );
        // The browser attaches the agent to the tab again, which it sees
        // now, and to the page within it.
        for attached in [
            r#"{"method":"Target.attachedToTarget","params":{"sessionId":"OT2","targetInfo":{"targetId":"OTHER-TAB","type":"tab"},"waitingForDebugger":false}}"#,
            r#"{"method":"Target.attachedToTarget","sessionId":"OT2","params":{"sessionId":"OP2","targetInfo":{"targetId":"OTHER","type":"page"},"waitingForDebugger":false}}"#,
        ] {
            assert_eq!(scope.reply(attached), Reply::Pass);
        }
        assert_eq!(scope.page_of("OP2"), Some("OTHER".to_owned()));
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":3,"method":"Target.closeTarget","params":{"targetId":"OTHER"}}"#
            ),
            Request::Ask(Ask::Close {
                page: "OTHER".to_owned()
            })
        );
    }

    #[test]
    fn the_agent_hears_a_page_taken_back_is_gone_and_the_relay_lets_go_of_it() {
        let mut scope = page_scope();
        for attached in [
            r#"{"method":"Target.attachedToTarget","params":{"sessionId":"T1","targetInfo":{"targetId":"TAB","type":"tab"},"waitingForDebugger":false}}"#,
            r#"{"method":"Target.attachedToTarget","sessionId":"T1","params":{"sessionId":"P1","targetInfo":{"targetId":"PAGE","type":"page"},"waitingForDebugger":false}}"#,
            r#"{"method":"Target.attachedToTarget","sessionId":"P1","params":{"sessionId":"F1","targetInfo":{"targetId":"FRAME","type":"iframe"},"waitingForDebugger":false}}"#,
        ] {
            assert_eq!(scope.reply(attached), Reply::Pass);
        }
        let mut id = 0;
        let said = scope.learn(&Known::default(), || {
            id += 1;
            id
        });
        assert_eq!(
            to_agent(&said),
            [
                r#"{"method":"Target.detachedFromTarget","params":{"sessionId":"F1"},"sessionId":"P1"}"#,
                r#"{"method":"Target.detachedFromTarget","params":{"sessionId":"P1"},"sessionId":"T1"}"#,
                r#"{"method":"Target.detachedFromTarget","params":{"sessionId":"T1"}}"#,
                r#"{"method":"Target.targetDestroyed","params":{"targetId":"FRAME"}}"#,
                r#"{"method":"Target.targetDestroyed","params":{"targetId":"PAGE"}}"#,
                r#"{"method":"Target.targetDestroyed","params":{"targetId":"TAB"}}"#,
            ]
        );
        assert_eq!(
            to_browser(&said),
            [r#"{"id":1,"method":"Target.detachFromTarget","params":{"sessionId":"T1"}}"#]
        );
        // What the browser says of the page from now on, the agent does
        // not hear, and what it asks of it is refused.
        assert_eq!(
            scope.reply(r#"{"method":"Page.loadEventFired","params":{},"sessionId":"P1"}"#),
            Reply::Drop
        );
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.targetInfoChanged","params":{"targetInfo":{"targetId":"PAGE","type":"page"}}}"#
            ),
            Reply::Drop
        );
        assert!(matches!(
            request(
                &mut scope,
                r#"{"id":4,"method":"Target.attachToTarget","params":{"targetId":"PAGE","flatten":true}}"#
            ),
            Request::Refuse(_)
        ));
        assert!(matches!(
            request(
                &mut scope,
                r#"{"id":5,"sessionId":"P1","method":"Runtime.evaluate","params":{"expression":"1"}}"#
            ),
            Request::Refuse(_)
        ));
    }
}
