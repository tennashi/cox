//! What the agents of the chats run, as the window's pages read it: what
//! each chat's agent does now, as the Activity page tells it on the
//! chat's line (see [`now`]), read as coxd reads it for the phone's (see
//! [`coxd::commands`]).
//!
//! The window tells this module of a chat's conversation as it changes
//! (see [`tell`]), since the stage's pages are served apart from it.

use std::collections::BTreeMap;
use std::sync::Mutex;

pub use coxd::commands::{Chat, Now, called};

/// What the window told, by chat.
static TOLD: Mutex<BTreeMap<u64, Chat>> = Mutex::new(BTreeMap::new());

/// Keeps `chat` as it is now, for the pages to read.
pub fn tell(chat: Chat) {
    if let Ok(mut told) = TOLD.lock() {
        told.insert(chat.key, chat);
    }
}

/// Forgets the chats the window no longer has, all but `keys`.
pub fn keep(keys: &[u64]) {
    if let Ok(mut told) = TOLD.lock() {
        told.retain(|key, _| keys.contains(key));
    }
}

/// What the agent of the chat `key` does now, and what it asks, if the
/// window told of its conversation.
#[must_use]
pub fn now(key: u64) -> Option<Now> {
    TOLD.lock().ok()?.get(&key).map(Chat::now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cox_core::entry::{Content, Entry};

    #[test]
    fn the_window_tells_how_a_chat_does_until_it_no_longer_has_it() {
        // Keys of their own, apart from the other tests'.
        let entries = [Entry::ToolCall {
            id: "1".to_owned(),
            title: "make".to_owned(),
            command: true,
            about: None,
            content: vec![Content::Text(String::new())],
            result: None,
        }];
        tell(Chat::of(9001, &entries));
        tell(Chat::of(9002, &entries));

        keep(&[9002]);

        assert_eq!(now(9001), None);
        assert_eq!(
            now(9002).and_then(|now| now.running),
            Some(("make".to_owned(), "make".to_owned()))
        );
        keep(&[]);
    }
}
