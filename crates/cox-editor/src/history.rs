//! Where a tab has been, to go back and forward between, as a browser
//! does: the places before where it is, and the places it went back
//! from, which going somewhere new leaves behind.

/// The places a tab has been, before and after where it is now.
#[derive(Clone, Debug)]
pub struct History<T> {
    /// Where the tab was before where it is, the latest last.
    back: Vec<T>,
    /// Where the tab went back from, the latest last.
    forward: Vec<T>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> History<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
        }
    }

    /// The tab leaves `here`, where it is, for somewhere new: back goes
    /// to `here`, and there is nowhere to go forward to any more.
    pub fn leave(&mut self, here: T) {
        self.back.push(here);
        self.forward.clear();
    }

    /// Where to go back to, if anywhere, taken out of the history: once
    /// the tab is there, [`went_back`](Self::went_back) keeps where it was.
    pub fn back(&mut self) -> Option<T> {
        self.back.pop()
    }

    /// Where to go forward to, if anywhere, taken out of the history:
    /// once the tab is there, [`went_forward`](Self::went_forward) keeps
    /// where it was.
    pub fn forward(&mut self) -> Option<T> {
        self.forward.pop()
    }

    /// The tab went back from `here`: forward goes to it.
    pub fn went_back(&mut self, here: T) {
        self.forward.push(here);
    }

    /// The tab went forward from `here`: back goes to it.
    pub fn went_forward(&mut self, here: T) {
        self.back.push(here);
    }

    #[must_use]
    pub fn has_back(&self) -> bool {
        !self.back.is_empty()
    }

    #[must_use]
    pub fn has_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::History;

    #[test]
    fn goes_back_and_forward_between_the_places_left() {
        let mut history = History::new();
        history.leave("a");
        history.leave("b");
        assert!(history.has_back() && !history.has_forward());

        assert_eq!(history.back(), Some("b"));
        history.went_back("c");
        assert_eq!(history.back(), Some("a"));
        history.went_back("b");
        assert!(!history.has_back() && history.has_forward());
        assert_eq!(history.back(), None);

        assert_eq!(history.forward(), Some("b"));
        history.went_forward("a");
        assert_eq!(history.forward(), Some("c"));
        history.went_forward("b");
        assert_eq!(history.forward(), None);
    }

    #[test]
    fn leaving_for_somewhere_new_forgets_where_it_went_back_from() {
        let mut history = History::new();
        history.leave("a");
        assert_eq!(history.back(), Some("a"));
        history.went_back("b");
        assert!(history.has_forward());

        history.leave("a");
        assert!(!history.has_forward());
        assert_eq!(history.back(), Some("a"));
    }
}
