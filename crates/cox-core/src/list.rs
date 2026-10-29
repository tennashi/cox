//! A list of which one item is shown, as a stage's tabs are, and a
//! window's chats. The items keep their order, and which is shown
//! follows them as they come and go, so that what shows the list only
//! says what comes, goes, or is to be shown.

use std::ops::{Index, IndexMut};

/// A list, and which of its items is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct List<T> {
    items: Vec<T>,
    /// The index of the item shown, if one is.
    shown: Option<usize>,
}

impl<T> Default for List<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            shown: None,
        }
    }
}

impl<T> List<T> {
    /// An empty list, with nothing shown.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.items.iter_mut()
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index)
    }

    /// The index of the item shown, if one is.
    #[must_use]
    pub fn shown_index(&self) -> Option<usize> {
        self.shown
    }

    /// The item shown, if one is.
    #[must_use]
    pub fn shown(&self) -> Option<&T> {
        self.items.get(self.shown?)
    }

    /// Puts `item` after the others, shown if `show` is, and returns its
    /// index.
    pub fn push(&mut self, item: T, show: bool) -> usize {
        self.insert(self.items.len(), item, show)
    }

    /// Puts `item` at `index`, before the one there, shown if `show` is,
    /// and returns its index. The item shown stays shown.
    ///
    /// # Panics
    ///
    /// When `index` is past the end.
    pub fn insert(&mut self, index: usize, item: T, show: bool) -> usize {
        self.items.insert(index, item);
        if show {
            self.shown = Some(index);
        } else if let Some(shown) = self.shown
            && shown >= index
        {
            self.shown = Some(shown + 1);
        }
        index
    }

    /// Shows the item at `index`, and says whether there is one.
    pub fn show(&mut self, index: usize) -> bool {
        if index < self.items.len() {
            self.shown = Some(index);
            true
        } else {
            false
        }
    }

    /// The index of the item `step` places along from the one shown,
    /// going round, if one is shown.
    #[must_use]
    pub fn along(&self, step: isize) -> Option<usize> {
        let shown = isize::try_from(self.shown?).ok()?;
        let count = isize::try_from(self.items.len()).ok()?;
        usize::try_from((shown + step).rem_euclid(count)).ok()
    }

    /// Takes the item at `index` out, if there is one. The item shown
    /// stays shown; once it is the one taken out, the one after it is
    /// shown, or the one before it if it was the last.
    pub fn remove(&mut self, index: usize) -> Option<T> {
        if index >= self.items.len() {
            return None;
        }
        let item = self.items.remove(index);
        self.shown = self
            .shown
            .and_then(|shown| after_removing(shown, index, self.items.len()));
        Some(item)
    }
}

/// Which item is shown once the one at `removed` has gone, when the one at
/// `shown` was, leaving `left` items: the same item, unless it was the one
/// removed, when it is the one after it, or before it if it was the last.
fn after_removing(shown: usize, removed: usize, left: usize) -> Option<usize> {
    if left == 0 {
        None
    } else if shown > removed {
        Some(shown - 1)
    } else {
        Some(shown.min(left - 1))
    }
}

impl<T> Index<usize> for List<T> {
    type Output = T;

    fn index(&self, index: usize) -> &T {
        &self.items[index]
    }
}

impl<T> IndexMut<usize> for List<T> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        &mut self.items[index]
    }
}

impl<'a, T> IntoIterator for &'a List<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut List<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::List;

    /// A list of `items`, with the one at `shown` shown.
    fn list(items: &[&'static str], shown: usize) -> List<&'static str> {
        let mut list = List::new();
        for item in items {
            list.push(*item, false);
        }
        assert!(list.show(shown));
        list
    }

    #[test]
    fn nothing_is_shown_until_an_item_is() {
        let mut list = List::new();
        assert_eq!(list.shown(), None);
        list.push("a", false);
        assert_eq!(list.shown(), None);
        assert_eq!(list.shown_index(), None);
        list.push("b", true);
        assert_eq!(list.shown(), Some(&"b"));
        assert_eq!(list.shown_index(), Some(1));
    }

    #[test]
    fn an_item_pushed_is_shown_only_when_asked() {
        let mut list = list(&["a", "b"], 0);
        assert_eq!(list.push("c", false), 2);
        assert_eq!(list.shown(), Some(&"a"));
        assert_eq!(list.push("d", true), 3);
        assert_eq!(list.shown(), Some(&"d"));
    }

    #[test]
    fn an_item_inserted_leaves_the_one_shown_shown() {
        let mut list = list(&["a", "b", "c"], 1);
        assert_eq!(list.insert(0, "z", false), 0);
        assert_eq!(
            list.iter().copied().collect::<Vec<_>>(),
            ["z", "a", "b", "c"]
        );
        assert_eq!(list.shown(), Some(&"b"));
        list.insert(4, "y", false);
        assert_eq!(list.shown(), Some(&"b"));
        list.insert(1, "x", true);
        assert_eq!(list.shown(), Some(&"x"));
    }

    #[test]
    fn only_an_item_there_can_be_shown() {
        let mut list = list(&["a", "b"], 0);
        assert!(list.show(1));
        assert_eq!(list.shown(), Some(&"b"));
        assert!(!list.show(2));
        assert_eq!(list.shown(), Some(&"b"));
    }

    #[test]
    fn stepping_along_the_list_goes_round() {
        let list = list(&["a", "b", "c"], 2);
        assert_eq!(list.along(1), Some(0));
        assert_eq!(list.along(-1), Some(1));
        assert_eq!(list.along(-3), Some(2));
        assert_eq!(list.along(4), Some(0));
        assert_eq!(List::<&str>::new().along(1), None);
    }

    #[test]
    fn removing_another_item_keeps_the_one_shown() {
        let mut list = list(&["a", "b", "c"], 1);
        assert_eq!(list.remove(0), Some("a"));
        assert_eq!(list.shown(), Some(&"b"));
        assert_eq!(list.shown_index(), Some(0));
        assert_eq!(list.remove(1), Some("c"));
        assert_eq!(list.shown(), Some(&"b"));
        assert_eq!(list.remove(1), None);
    }

    #[test]
    fn removing_the_item_shown_shows_the_next_or_else_the_one_before() {
        let mut list = list(&["a", "b", "c"], 1);
        assert_eq!(list.remove(1), Some("b"));
        assert_eq!(list.shown(), Some(&"c"));
        list.remove(1);
        assert_eq!(list.shown(), Some(&"a"));
        list.remove(0);
        assert_eq!(list.shown(), None);
        assert!(list.is_empty());
    }
}
