//! Chats marked done: the human marks one from its menu, and Cox marks
//! one whose work came to its end, as a subagent whose report reached the
//! agent that opened it, or a chat whose merge ended. A chat marked done
//! stays in the list, folded away at its end; it is not unread, nor
//! counted among what waits for the human, and its agent closes, its
//! turn stopped first if one runs. A line sent to it reopens it, and
//! starts its agent again, going on with its session. Its report asked
//! again does not, as nothing is sent to it, nor does CI that ends on a
//! pull request it opened: its project holds that (see
//! [`super::pulls`]).

use std::rc::Rc;
use std::time::Instant;

use cox_core::store::{Resolved, ResolvedBy};

use super::{Daemon, Slot, State, at_work, working};
use crate::chat::conversation::Conversation;

/// How a chat's turn bears on whether it is done, as its conversation
/// stands now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Turned {
    /// A turn that merged what its agent made ended: its work is done.
    Merged,
    /// Its agent went back to work by itself, while it was done.
    Back,
}

impl Turned {
    /// How the turn of the chat of `slot`, whose `conversation` stands as
    /// it does now, bears on whether it is done, if it does.
    pub(super) fn of(slot: &Slot, conversation: &Conversation) -> Option<Self> {
        let state = conversation.state();
        match slot.resolved {
            None if working(slot.state) && state == State::Idle && conversation.merging() => {
                Some(Self::Merged)
            }
            Some(_) if !at_work(slot.state) && at_work(state) => Some(Self::Back),
            _ => None,
        }
    }
}

impl Daemon {
    /// Marks the chat `key` done, by `by`, unless it is: its agent stops,
    /// and closes once its turn is not running (see
    /// [`Self::rest_resolved`]). The chats its agent opened that are
    /// neither at work nor asking the human are marked done with it.
    pub(super) fn resolve(&mut self, key: u64, by: ResolvedBy) {
        let store = Rc::clone(&self.store);
        let Some(slot) = self.slot_mut(key) else {
            return;
        };
        if slot.resolved.is_some() {
            return;
        }
        let resolved = Resolved::now(by);
        slot.resolved = Some(resolved);
        slot.unread = false;
        slot.pulls.seen();
        // Its agent no longer hears of CI that ended in a line of its own.
        if slot.whose.waits() {
            slot.pulls.told.clear();
        }
        if let Some(kept) = slot.kept
            && let Err(error) = store.resolve_chat(kept, Some(resolved))
        {
            tracing::error!("that the chat is done is not kept: {error:#}");
        }
        if let Some(chat) = &mut slot.chat {
            if working(chat.conversation().state()) {
                chat.stop();
            } else {
                chat.rest();
            }
        }
        let now = Instant::now();
        let children: Vec<u64> = self
            .chats
            .iter()
            .chain(&self.archived)
            .filter(|child| {
                child.parent == Some(key)
                    && child.resolved.is_none()
                    && !child.at_work(now)
                    && child.state != State::Asking
            })
            .map(|child| child.key)
            .collect();
        for child in children {
            self.resolve(child, by);
        }
        // The worktrees made for it may go now.
        self.tidy_worktrees(Instant::now(), true);
    }

    /// Marks the chat kept as `number`, of the project of the chat `key`,
    /// done, as its agent asked, the way the human's Resolve does (see
    /// [`Self::resolve`]), and answers what was marked, or why it cannot
    /// be: not the chat `key` itself, nor one at work or asking the
    /// human. One done already stays so.
    pub(super) fn resolve_chat(&mut self, key: u64, number: i64) -> Result<String, String> {
        let now = Instant::now();
        let slot = self
            .seen_by(key)
            .ok_or("the chat has closed")?
            .find(|slot| slot.kept.is_some_and(|kept| kept.number() == number))
            .ok_or_else(|| format!("This project has no chat {number}."))?;
        let (target, name) = (slot.key, slot.name.clone());
        let chat = format!("Chat {number}, \"{name}\",");
        if target == key {
            return Err(format!(
                "{chat} is this chat: the human marks it done, or Cox does once its work ends."
            ));
        }
        if slot.resolved.is_some() {
            return Ok(format!("{chat} is marked done already."));
        }
        if slot.state == State::Asking {
            return Err(format!(
                "{chat} waits for the human to answer it, so it is not marked done."
            ));
        }
        if slot.at_work(now) {
            return Err(format!(
                "{chat} is at work, so it is not marked done. Mark it once its work ends."
            ));
        }
        let open: Vec<u64> = self
            .seen_by(key)
            .into_iter()
            .flatten()
            .filter(|slot| slot.resolved.is_none() && slot.key != target)
            .map(|slot| slot.key)
            .collect();
        self.resolve(target, ResolvedBy::Agent);
        let with: Vec<String> = open
            .into_iter()
            .filter_map(|key| self.slot(key))
            .filter(|slot| slot.resolved.is_some())
            .filter_map(|slot| slot.kept)
            .map(|kept| format!("chat {}", kept.number()))
            .collect();
        let mut done = format!("Marked chat {number}, \"{name}\", done");
        if !with.is_empty() {
            done.push_str(", with the chats it opened: ");
            done.push_str(&with.join(", "));
        }
        done.push_str(". Anything that arrives in it reopens it.");
        Ok(done)
    }

    /// Reopens the chat `key`, if it was marked done.
    pub(super) fn reopen(&mut self, key: u64) {
        let store = Rc::clone(&self.store);
        let Some(slot) = self.slot_mut(key) else {
            return;
        };
        if slot.resolved.take().is_none() {
            return;
        }
        if let Some(kept) = slot.kept
            && let Err(error) = store.resolve_chat(kept, None)
        {
            tracing::error!("that the chat is reopened is not kept: {error:#}");
        }
    }

    /// Marks done each chat of `turns` whose merge ended, reopens each
    /// whose agent went back to work by itself, and closes the agents of
    /// those done whose turn stopped since (see [`Self::rest_resolved`]).
    pub(super) fn turned(&mut self, turns: Vec<(u64, Turned)>) {
        self.rest_resolved();
        for (key, turned) in turns {
            match turned {
                Turned::Merged => self.resolve(key, ResolvedBy::Agent),
                Turned::Back => self.reopen(key),
            }
        }
    }

    /// Closes the agent of each chat marked done whose turn is not
    /// running: one stopped as it was marked closes once it stopped.
    pub(super) fn rest_resolved(&mut self) {
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            if slot.resolved.is_none() {
                continue;
            }
            if let Some(chat) = &mut slot.chat
                && chat.is_open()
                && !working(chat.conversation().state())
            {
                chat.rest();
            }
        }
    }
}
