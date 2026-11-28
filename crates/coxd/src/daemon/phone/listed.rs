//! The projects and chats, as a phone's page is told them: of the
//! [`Listing`] the window is told, what the page reads, and what it asks
//! coxd again by, and no more.
//!
//! The window's list holds, of each chat, all its agent was set up with,
//! as the context the chat that opened it handed it, and what the agent
//! spent, which changes with each thing it says: most of the list's
//! bytes, and most of its changes, of which the page shows nothing. A
//! phone would wait for them on the tailnet, ahead of the conversation
//! it opened. What the page shows of what is left out here, as what a
//! chat spent, it asks for with what goes on (see [`crate::activity`]).
//!
//! The list goes with a tag of what it says, which the page names as it
//! follows the chats again, `&listed=<tag>` on `/events`, as each time it
//! opens a chat: a page that holds the list as it is, is not told it
//! again.

use std::collections::BTreeMap;
use std::path::Path;

use cox_core::goal::Goal;
use cox_core::store::{ChatId, ProjectId, Resolved, TurnEnd};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use super::super::wire::{Background, ChatRow, Listing, ProjectRow, PullRow};
use crate::chat::conversation::State;
use crate::chat::sketch::Sketch;

/// The list, as the page is told it, under the names the window's goes
/// by.
#[derive(Serialize)]
struct Listed<'a> {
    projects: Vec<Project<'a>>,
    chats: Vec<Chat<'a>>,
    archived: Vec<Chat<'a>>,
    agents: &'a BTreeMap<String, String>,
}

/// A project, as the page lists it.
#[derive(Serialize)]
struct Project<'a> {
    id: Option<ProjectId>,
    name: &'a str,
    folder: &'a Path,
    missing: bool,
    archived: bool,
    deleted: bool,
}

impl<'a> Project<'a> {
    fn of(row: &'a ProjectRow) -> Self {
        Self {
            id: row.id,
            name: &row.name,
            folder: &row.folder,
            missing: row.missing,
            archived: row.archived,
            deleted: row.deleted,
        }
    }
}

/// A chat, as the page lists it. A field the page comes to read of a
/// chat's row is added here, and one whose change the page is to hear
/// of, to ask coxd again, as `ending` is for what the human may do of a
/// sketch.
#[derive(Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "how a chat stands, each apart, as the window's row has them"
)]
struct Chat<'a> {
    key: u64,
    project: usize,
    kept: Option<ChatId>,
    name: &'a str,
    open: bool,
    again: bool,
    state: State,
    unread: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    goal: Option<&'a Goal>,
    parent: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    setup: Option<Apart<'a>>,
    ending: bool,
    agent: &'a str,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    background: &'a [Background],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pulls: &'a [PullRow],
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved: Option<&'a Resolved>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cut_off: Option<&'a TurnEnd>,
}

/// Of how a chat's agent is set up, where it works apart from its
/// project: all the page reads of it.
#[derive(Serialize)]
struct Apart<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    folder: Option<&'a Path>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sketch: Option<Sketch>,
}

impl<'a> Chat<'a> {
    fn of(row: &'a ChatRow) -> Self {
        let setup = row.setup.as_ref().map(|setup| Apart {
            folder: setup.folder.as_deref(),
            sketch: setup.sketch,
        });
        Self {
            key: row.key,
            project: row.project,
            kept: row.kept,
            name: &row.name,
            open: row.open,
            again: row.again,
            state: row.state,
            unread: row.unread,
            goal: row.goal.as_ref(),
            parent: row.parent,
            setup: setup.filter(|apart| apart.folder.is_some() || apart.sketch.is_some()),
            ending: row.ending,
            agent: &row.agent,
            background: &row.background,
            pulls: &row.pulls,
            resolved: row.resolved.as_ref(),
            cut_off: row.cut_off.as_ref(),
        }
    }
}

/// `listing` as a phone's page is told it, as JSON, and its tag, which
/// the JSON gives too, as `"tag"`: the start of the sum of the rest, so
/// that two lists the page would read alike are tagged alike.
pub(super) fn listed(listing: &Listing) -> serde_json::Result<(String, String)> {
    let json = serde_json::to_string(&Listed {
        projects: listing.projects.iter().map(Project::of).collect(),
        chats: listing.chats.iter().map(Chat::of).collect(),
        archived: listing.archived.iter().map(Chat::of).collect(),
        agents: &listing.agents,
    })?;
    let sum = format!("{:x}", Sha256::digest(&json));
    let tag = sum[..16].to_owned();
    // The list is an object of the fields above, never empty, and a tag
    // is hexadecimal.
    let fields = json.strip_suffix('}').unwrap_or(&json);
    Ok((format!("{fields},\"tag\":\"{tag}\"}}"), tag))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::agent::setup::Setup;
    use crate::chat::spent::Spent;

    fn listing(rows: Vec<ChatRow>) -> Listing {
        Listing {
            projects: vec![ProjectRow {
                id: None,
                name: "Cox".to_owned(),
                folder: PathBuf::from("/home"),
                missing: false,
                archived: false,
                deleted: false,
                about: Some("What the project is.".to_owned()),
                last: Some(7),
            }],
            chats: rows,
            agents: BTreeMap::from([("claude".to_owned(), "Claude".to_owned())]),
            chosen: 0,
            ..Listing::default()
        }
    }

    /// A chat an agent opened, handed a long context.
    fn opened_by_an_agent() -> ChatRow {
        let context = "Keep to this for the whole session. ".repeat(40);
        serde_json::from_value(serde_json::json!({
            "key": 7, "project": 0, "kept": 12, "name": "chat 7", "named": "FirstLine",
            "open": true, "again": true, "state": "Idle", "unread": false, "goal": null,
            "parent": 3, "setup": { "context": context }, "ending": false, "agent": "claude",
        }))
        .unwrap()
    }

    #[test]
    fn tells_a_phone_what_its_page_reads_of_a_chat_and_no_more() {
        let (json, tag) = listed(&listing(vec![opened_by_an_agent()])).unwrap();
        let told: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            told,
            serde_json::json!({
                "projects": [{
                    "id": null, "name": "Cox", "folder": "/home",
                    "missing": false, "archived": false, "deleted": false,
                }],
                "chats": [{
                    "key": 7, "project": 0, "kept": 12, "name": "chat 7",
                    "open": true, "again": true, "state": "Idle", "unread": false,
                    "parent": 3, "ending": false, "agent": "claude",
                }],
                "archived": [],
                "agents": { "claude": "Claude" },
                "tag": tag,
            })
        );
    }

    #[test]
    fn tells_a_phone_where_a_chat_works_apart_from_its_project() {
        let mut row = opened_by_an_agent();
        row.setup = Some(Setup {
            context: Some("Try it.".to_owned()),
            folder: Some(PathBuf::from("/sketches/app")),
            sketch: Some(Sketch::Empty),
            ..Setup::default()
        });
        let (json, _) = listed(&listing(vec![row])).unwrap();
        let told: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            told["chats"][0]["setup"],
            serde_json::json!({ "folder": "/sketches/app", "sketch": "empty" })
        );
    }

    #[test]
    fn tags_alike_two_lists_a_page_reads_alike() {
        let row = opened_by_an_agent();
        let (_, before) = listed(&listing(vec![row.clone()])).unwrap();

        let mut spent = row.clone();
        spent.spent = Some(Spent::default());
        spent.active = 1_800_000_000_000;
        let (_, after_spending) = listed(&listing(vec![spent])).unwrap();
        assert_eq!(after_spending, before);

        let mut busy = row;
        busy.state = State::Busy;
        let (_, at_work) = listed(&listing(vec![busy])).unwrap();
        assert_ne!(at_work, before);
    }
}
