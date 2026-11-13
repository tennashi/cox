//! Vim's `/`, `?`, `n` and `N` in the conversation: the entries whose
//! text has what is searched for, in the order the search goes through
//! them, each scrolled to until one, laid out, shows a match, which is
//! selected and brought into sight.

use std::collections::VecDeque;

use cox_core::entry::Entry;
use gpui::{Context, IntoElement, ListOffset, Window, canvas, px};

use super::ChatView;
use crate::vim::search::{self, Direction, Find};

/// A search going through the conversation.
pub(super) struct Finding {
    query: String,
    direction: Direction,
    /// The entries left to look in, each with whether to look after what
    /// is selected in it: the one it starts in, the first time.
    left: VecDeque<(usize, bool)>,
    /// The entry the conversation was scrolled to, to be laid out.
    scrolled: Option<usize>,
}

impl ChatView {
    /// Starts a search through the conversation, from what is selected,
    /// or the first entry in sight; the frame that lays the next entry
    /// with a match out finds it (see [`Self::go_on_finding`]).
    pub(super) fn find_text(&mut self, find: &Find, _: &mut Window, cx: &mut Context<Self>) {
        let entries = self.conversation.entries();
        let start = self.reading.selected_entry().or_else(|| {
            let top = self.list.logical_scroll_top().item_ix;
            self.places.get(top).map(super::entries::Place::first)
        });
        let Some(start) = start.filter(|&start| start < entries.len()) else {
            return;
        };
        let has =
            |entry: usize| !search::matches(&find.text, &searched(&entries[entry])).is_empty();
        let left = order(start, entries.len(), find.direction)
            .filter(|&(entry, _)| has(entry))
            .collect();
        self.finding = Some(Finding {
            query: find.text.clone(),
            direction: find.direction,
            left,
            scrolled: None,
        });
        cx.notify();
    }

    /// Goes on with the search, once a frame laid the conversation out:
    /// selects the match in the next entry that has one as laid out, and
    /// brings it into sight, a third of the way down; or scrolls to the
    /// entry, for the next frame to lay it out.
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
            if let Some(at) = self
                .reading
                .find_in(entry, &finding.query, finding.direction, after)
            {
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
