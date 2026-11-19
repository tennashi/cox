//! Which chat of a project the window shows as the human chooses the
//! project, or as the window starts: the one shown last, unless it was
//! marked done since, or else the one that moved last, a chat marked done
//! only when the project has no other.

/// A chat of the project, as the window lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Candidate {
    /// Its place in the window's list.
    pub index: usize,
    pub key: u64,
    /// Whether it is marked done.
    pub resolved: bool,
    /// When it last moved, in milliseconds since the epoch.
    pub active: i64,
}

/// The place in the list of the chat to show of `chats`, a project's,
/// the one shown last of it being `last`, if one was: that one, unless it
/// is marked done or gone; or else the one not marked done that moved
/// last; or else, with every chat of it marked done, the one that moved
/// last; or none, with no chat. Of chats that moved at once, the one
/// listed later goes first.
pub(super) fn chat_to_show(
    chats: impl IntoIterator<Item = Candidate>,
    last: Option<u64>,
) -> Option<usize> {
    let chats: Vec<Candidate> = chats.into_iter().collect();
    if let Some(shown) = chats
        .iter()
        .find(|chat| Some(chat.key) == last && !chat.resolved)
    {
        return Some(shown.index);
    }
    chats
        .iter()
        .max_by_key(|chat| (!chat.resolved, chat.active, chat.index))
        .map(|chat| chat.index)
}

#[cfg(test)]
mod tests {
    use super::{Candidate, chat_to_show};

    fn chat(index: usize, resolved: bool, active: i64) -> Candidate {
        Candidate {
            index,
            key: 100 + index as u64,
            resolved,
            active,
        }
    }

    #[test]
    fn the_chat_shown_last_is_shown_again() {
        let chats = [chat(0, false, 10), chat(1, false, 30), chat(2, false, 20)];
        assert_eq!(chat_to_show(chats, Some(100)), Some(0));
    }

    #[test]
    fn with_none_shown_last_the_chat_that_moved_last_is_shown_not_the_oldest() {
        let chats = [chat(0, false, 10), chat(1, false, 30), chat(2, false, 20)];
        assert_eq!(chat_to_show(chats, None), Some(1));
    }

    #[test]
    fn a_chat_shown_last_that_was_marked_done_gives_way_to_one_not_done() {
        let chats = [chat(0, true, 50), chat(1, false, 10), chat(2, false, 20)];
        assert_eq!(chat_to_show(chats, Some(100)), Some(2));
    }

    #[test]
    fn a_chat_shown_last_that_is_gone_gives_way_to_the_one_that_moved_last() {
        let chats = [chat(0, false, 10), chat(1, false, 20)];
        assert_eq!(chat_to_show(chats, Some(999)), Some(1));
    }

    #[test]
    fn a_chat_marked_done_is_shown_only_when_every_chat_is() {
        let chats = [chat(0, true, 10), chat(1, true, 30), chat(2, false, 1)];
        assert_eq!(chat_to_show(chats, None), Some(2));
        let done = [chat(0, true, 10), chat(1, true, 30)];
        assert_eq!(chat_to_show(done, Some(100)), Some(1));
    }

    #[test]
    fn of_chats_that_moved_at_once_the_later_listed_is_shown() {
        let chats = [chat(0, false, 10), chat(1, false, 10)];
        assert_eq!(chat_to_show(chats, None), Some(1));
    }

    #[test]
    fn a_project_with_no_chat_shows_none() {
        assert_eq!(chat_to_show([], Some(100)), None);
    }
}
