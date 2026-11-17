//! Chats marked done: the human marks one from its menu, and Cox marks
//! one whose work came to its end, as a subagent whose report reached the
//! agent that opened it, or a chat whose merge ended. A chat marked done
//! stays in the list, folded away at its end; it is not unread, nor
//! counted among what waits for the human, and its agent closes, its
//! turn stopped first if one runs. Anything that arrives in it reopens
//! it, as a line, a report or CI that ended, and a line starts its agent
//! again, going on with its session.

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
