//! Hint mode over a page (see [`crate::vim`]): a label over each thing
//! in sight that can be clicked or typed in, and the labels that still
//! match as the human types one, until one is left.

use cox_browser::{Hint, Kind};

use crate::vim::hints::{Answer, labels};
use crate::vim::keymap::Follow;

/// Where hint mode is over a page.
pub(super) enum Hinting {
    /// The page was asked what can be clicked, for hints to follow as
    /// this says.
    Asked(Follow),
    Shown(Shown),
    /// The window asked what can be clicked, for its hints (see
    /// [`crate::vim::hints`]), to hear of it here.
    AskedByWindow(Answer),
    /// What the page listed for the window's hints.
    Window(Vec<Hint>),
}

/// The hints drawn over a page, by their labels, and what was typed of
/// one so far.
pub(super) struct Shown {
    follow: Follow,
    hints: Vec<(String, Hint)>,
    typed: String,
}

/// What a key typed in hint mode comes to.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Typed {
    /// More than one label still matches: these, by their hints' ids.
    Narrowed(Vec<(u32, String)>),
    /// One is left, to follow as this says.
    Chosen(Hint, Follow),
    /// None matches.
    Missed,
}

impl Shown {
    /// Labels `hints` to follow as `follow` says: the links of them only,
    /// for a tab of their own. Nothing, if none is left to label.
    pub(super) fn new(follow: Follow, hints: Vec<Hint>) -> Option<Self> {
        let hints: Vec<Hint> = hints
            .into_iter()
            .filter(|hint| follow == Follow::Here || matches!(hint.kind, Kind::Link(_)))
            .collect();
        if hints.is_empty() {
            return None;
        }
        Some(Self {
            follow,
            hints: labels(hints.len()).into_iter().zip(hints).collect(),
            typed: String::new(),
        })
    }

    /// The labels that match what was typed, by their hints' ids.
    pub(super) fn labels(&self) -> Vec<(u32, String)> {
        self.hints
            .iter()
            .filter(|(label, _)| label.starts_with(&self.typed))
            .map(|(label, hint)| (hint.id, label.clone()))
            .collect()
    }

    pub(super) fn typed(&mut self, key: char) -> Typed {
        self.typed.push(key);
        let mut matching = self
            .hints
            .iter()
            .filter(|(label, _)| label.starts_with(&self.typed));
        match (matching.next(), matching.next()) {
            (None, _) => Typed::Missed,
            (Some((_, hint)), None) => Typed::Chosen(hint.clone(), self.follow),
            _ => Typed::Narrowed(self.labels()),
        }
    }
}

#[cfg(test)]
mod tests {
    use cox_browser::{Hint, Kind, Rect};

    use super::{Follow, Shown, Typed};

    fn hint(id: u32, kind: Kind) -> Hint {
        Hint {
            id,
            rect: Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            kind,
            text: String::new(),
        }
    }

    fn link(id: u32) -> Hint {
        hint(id, Kind::Link(format!("https://example.com/{id}")))
    }

    #[test]
    fn typing_narrows_the_labels_to_one() {
        let hints = (0..15).map(link).collect();
        let mut shown = Shown::new(Follow::Here, hints).unwrap();
        assert_eq!(shown.labels().len(), 15);
        let Typed::Narrowed(left) = shown.typed('s') else {
            panic!("narrowed");
        };
        assert_eq!(left.len(), 14);
        assert_eq!(left[1], (1, "sa".to_owned()));
        assert_eq!(shown.labels(), left);
        assert_eq!(shown.typed('a'), Typed::Chosen(link(1), Follow::Here));
    }

    #[test]
    fn a_label_alone_in_its_letter_is_chosen_at_once() {
        let mut shown = Shown::new(Follow::Here, (0..15).map(link).collect()).unwrap();
        assert_eq!(shown.typed('a'), Typed::Chosen(link(14), Follow::Here));
    }

    #[test]
    fn a_key_no_label_starts_with_misses() {
        let mut shown = Shown::new(Follow::Here, vec![link(0), link(1)]).unwrap();
        assert_eq!(shown.typed('z'), Typed::Missed);
    }

    #[test]
    fn a_new_tab_takes_links_only() {
        let hints = vec![hint(0, Kind::Field), link(1), hint(2, Kind::Other), link(3)];
        let mut shown = Shown::new(Follow::NewTab, hints).unwrap();
        assert_eq!(shown.labels(), [(1, "s".to_owned()), (3, "a".to_owned())]);
        assert_eq!(shown.typed('a'), Typed::Chosen(link(3), Follow::NewTab));
        assert!(Shown::new(Follow::NewTab, vec![hint(0, Kind::Field)]).is_none());
        assert!(Shown::new(Follow::Here, Vec::new()).is_none());
    }
}
