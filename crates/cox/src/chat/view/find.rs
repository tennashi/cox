//! Vim's keys going through the conversation: `/`, `?`, `n` and `N`,
//! to the entries whose text has what is searched for, and `]]` and `[[`,
//! from entry to entry. Each entry is scrolled to until one, laid out,
//! shows what is looked for, which is selected and brought into sight.

use std::collections::VecDeque;

use cox_core::entry::Entry;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ListOffset, Window, actions, canvas, px,
};

use super::ChatView;
use crate::vim::search::{self, Direction, Find};

actions!(
    chat,
    [
        /// Selects the next entry of the conversation (`]]`), or block of
        /// a document.
        NextEntry,
        /// Selects the entry, or block, before (`[[`).
        PreviousEntry,
    ]
);

/// What a search through the conversation looks for in an entry.
enum Looked {
    /// The next match of `query`, as `direction` goes.
    Text { query: String, direction: Direction },
    /// The whole entry, if it shows text.
    Whole,
}

/// A search going through the conversation.
pub(super) struct Finding {
    looked: Looked,
    /// The entries left to look in, each with whether to look after what
    /// is selected in it: the one it starts in, the first time.
    left: VecDeque<(usize, bool)>,
    /// The entry the conversation was scrolled to, to be laid out.
    scrolled: Option<usize>,
}

impl ChatView {
    /// The entry a search starts from: the one with what is selected, or
    /// the first in sight.
    fn searched_from(&self) -> Option<usize> {
        self.reading
            .selected_entry()
            .or_else(|| {
                let top = self.list.logical_scroll_top().item_ix;
                self.places.get(top).map(super::entries::Place::first)
            })
            .filter(|&start| start < self.conversation.entries().len())
    }

    /// Starts going through the conversation; the frame that lays out the
    /// next entry to look in finds in it (see [`Self::go_on_finding`]).
    fn look(&mut self, looked: Looked, left: VecDeque<(usize, bool)>, cx: &mut Context<Self>) {
        self.finding = Some(Finding {
            looked,
            left,
            scrolled: None,
        });
        cx.notify();
    }

    /// Starts a search through the conversation, from what is selected,
    /// or the first entry in sight, round to it again.
    pub(super) fn find_text(&mut self, find: &Find, _: &mut Window, cx: &mut Context<Self>) {
        let Some(start) = self.searched_from() else {
            return;
        };
        let entries = self.conversation.entries();
        let has =
            |entry: usize| !search::matches(&find.text, &searched(&entries[entry])).is_empty();
        let left = order(start, entries.len(), find.direction)
            .filter(|&(entry, _)| has(entry))
            .collect();
        let looked = Looked::Text {
            query: find.text.clone(),
            direction: find.direction,
        };
        self.look(looked, left, cx);
    }

    /// Selects the entry after the one selected, or with none, the first
    /// in sight: the next that shows text, to the conversation's end.
    pub(super) fn next_entry(&mut self, _: &NextEntry, _: &mut Window, cx: &mut Context<Self>) {
        let Some(start) = self.searched_from() else {
            return;
        };
        let first = if self.reading.selected_entry().is_some() {
            start + 1
        } else {
            start
        };
        let left = (first..self.conversation.entries().len())
            .map(|entry| (entry, false))
            .collect();
        self.look(Looked::Whole, left, cx);
    }

    /// Selects the entry before the one selected, or with none, the first
    /// in sight: the one before that shows text, to the conversation's
    /// start.
    pub(super) fn previous_entry(
        &mut self,
        _: &PreviousEntry,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(start) = self.searched_from() else {
            return;
        };
        let last = if self.reading.selected_entry().is_some() {
            start
        } else {
            start + 1
        };
        let left = (0..last).rev().map(|entry| (entry, false)).collect();
        self.look(Looked::Whole, left, cx);
    }

    /// Goes on going through the conversation, once a frame laid it out:
    /// selects what is looked for in the next entry that shows it as laid
    /// out, and brings it into sight, a third of the way down; or scrolls
    /// to the entry, for the next frame to lay it out.
    pub(super) fn go_on_finding(&mut self, cx: &mut Context<Self>) {
        let Some(mut finding) = self.finding.take() else {
            return;
        };
        while let Some(&(entry, after)) = finding.left.front() {
            if !self.reading.laid_out(entry) {
                if finding.scrolled == Some(entry) {
                    finding.left.pop_front();
                    continue;
                }
                let Some(place) = self.places.iter().rposition(|place| place.first() <= entry)
                else {
                    finding.left.pop_front();
                    continue;
                };
                self.list.scroll_to(ListOffset {
                    item_ix: place,
                    offset_in_item: px(0.),
                });
                finding.scrolled = Some(entry);
                self.finding = Some(finding);
                cx.notify();
                return;
            }
            let found = match &finding.looked {
                Looked::Text { query, direction } => {
                    self.reading.find_in(entry, query, *direction, after)
                }
                Looked::Whole => self.reading.select_entry(entry),
            };
            if let Some(at) = found {
                self.reading.cursor_to_selection();
                let view = self.list.viewport_bounds();
                self.list
                    .scroll_by(at.y - view.origin.y - view.size.height / 3.);
                cx.notify();
                return;
            }
            finding.left.pop_front();
        }
    }
}

impl ChatView {
    /// Takes Vim's keys that go through the conversation on `element`.
    pub(super) fn go_through_by_keys(element: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        element
            .on_action(cx.listener(Self::find_text))
            .on_action(cx.listener(Self::next_entry))
            .on_action(cx.listener(Self::previous_entry))
    }

    /// What finds in the conversation once it is painted, while a search
    /// goes on: right after the frame, which cannot ask for another.
    pub(super) fn render_finding(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        self.finding.as_ref()?;
        let view = cx.entity();
        Some(canvas(
            |_, _, _| {},
            move |_, (), window, cx| {
                window.defer(cx, move |_, cx| {
                    view.update(cx, ChatView::go_on_finding);
                });
            },
        ))
    }
}

/// The entries a search goes through from `start`, of `count`, as
/// `direction` goes, round to `start` again: in it, after what is
/// selected the first time, and the whole of it the last.
fn order(start: usize, count: usize, direction: Direction) -> impl Iterator<Item = (usize, bool)> {
    let others = (1..count).map(move |step| match direction {
        Direction::Forward => (start + step) % count,
        Direction::Backward => (start + count - step) % count,
    });
    std::iter::once((start, true))
        .chain(others.map(|entry| (entry, false)))
        .chain(std::iter::once((start, false)))
}

/// The text of `entry` a search looks through: every text it holds, more
/// than the conversation shows of it, which the search then looks in.
fn searched(entry: &Entry) -> String {
    fn gather(value: &serde_json::Value, into: &mut String) {
        match value {
            serde_json::Value::String(text) => {
                into.push_str(text);
                into.push('\n');
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    gather(value, into);
                }
            }
            serde_json::Value::Object(fields) => {
                for value in fields.values() {
                    gather(value, into);
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    if let Ok(value) = serde_json::to_value(entry) {
        gather(&value, &mut text);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_goes_round_the_entries_from_where_it_starts() {
        let forward: Vec<_> = order(2, 4, Direction::Forward).collect();
        assert_eq!(
            forward,
            [(2, true), (3, false), (0, false), (1, false), (2, false)]
        );
        let back: Vec<_> = order(2, 4, Direction::Backward).collect();
        assert_eq!(
            back,
            [(2, true), (1, false), (0, false), (3, false), (2, false)]
        );
    }

    #[test]
    fn every_text_an_entry_holds_is_searched() {
        let text = searched(&Entry::Human("find the needle".to_owned()));
        assert!(text.contains("needle"));
    }
}
