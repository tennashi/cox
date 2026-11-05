//! Finding a path by a few of its letters, as the human types them after
//! `@`: `chv` finds `crates/cox/src/chat/view.rs`. The letters have to
//! come in the path in their order, whatever their case, and the paths
//! where they start words, follow each other, or fall in the file's own
//! name come first.

/// What a letter scores where it is found.
const FOUND: i32 = 1;
/// More, for a letter that starts a word: the first of the path, or one
/// after `/`, `_`, `-`, `.` or a space, or an upper case letter after a
/// lower case one.
const STARTS_WORD: i32 = 8;
/// More, for a letter right after the one found before it.
const FOLLOWS: i32 = 5;
/// More, for a letter in the file's own name, after the last `/`.
const IN_NAME: i32 = 2;

/// How well `query` finds `path`, higher for a better fit, or `None`
/// when its letters are not all in the path in their order. Every path
/// fits an empty query, as well as any other.
#[must_use]
pub fn score(query: &str, path: &str) -> Option<i32> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some(0);
    }
    let letters: Vec<char> = path.chars().collect();
    let lower: Vec<char> = letters
        .iter()
        .map(|letter| letter.to_lowercase().next().unwrap_or(*letter))
        .collect();
    let name_starts = letters
        .iter()
        .rposition(|letter| *letter == '/')
        .map_or(0, |slash| slash + 1);
    // What each letter scores found there, apart from what follows it.
    let worth: Vec<i32> = (0..letters.len())
        .map(|at| {
            let starts = match at.checked_sub(1).map(|before| letters[before]) {
                None | Some('/' | '_' | '-' | '.' | ' ') => true,
                Some(before) => before.is_lowercase() && letters[at].is_uppercase(),
            };
            FOUND
                + if starts { STARTS_WORD } else { 0 }
                + if at >= name_starts { IN_NAME } else { 0 }
        })
        .collect();
    // The best score of the query so far with its last letter found at
    // each place of the path, row by row of the query.
    let mut best: Vec<Option<i32>> = (0..letters.len())
        .map(|at| (lower[at] == query[0]).then_some(worth[at]))
        .collect();
    for wanted in &query[1..] {
        let mut next = vec![None; letters.len()];
        // The best of the row before, at any place before the one before
        // this one.
        let mut before: Option<i32> = None;
        for at in 1..letters.len() {
            if lower[at] == *wanted {
                let after_gap = before;
                let right_after = best[at - 1].map(|score| score + FOLLOWS);
                next[at] = after_gap.max(right_after).map(|score| score + worth[at]);
            }
            before = before.max(best[at - 1]);
        }
        best = next;
    }
    best.into_iter().flatten().max()
}

/// The paths of `paths` that `query` finds, the best first, at most
/// `limit` of them: among those that fit as well, the shorter first, and
/// else in their order in `paths`.
#[must_use]
pub fn find<'p>(query: &str, paths: &'p [String], limit: usize) -> Vec<&'p str> {
    let mut found: Vec<(i32, &str)> = paths
        .iter()
        .filter_map(|path| score(query, path).map(|score| (score, path.as_str())))
        .collect();
    if !query.is_empty() {
        // The sort is stable: the order of `paths` stays among equals.
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));
    }
    found.truncate(limit);
    found.into_iter().map(|(_, path)| path).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(written: &[&str]) -> Vec<String> {
        written.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn the_letters_have_to_come_in_order() {
        assert!(score("chv", "crates/cox/src/chat/view.rs").is_some());
        assert!(score("CHV", "crates/cox/src/chat/view.rs").is_some());
        assert!(score("vhc", "crates/cox/src/chat/view.rs").is_none());
        assert!(score("z", "crates/cox/src/chat/view.rs").is_none());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn the_file_s_own_name_comes_first() {
        let all = paths(&[
            "crates/cox/src/input/view.rs",
            "crates/cox/src/ui/input.rs",
            "docs/inputs-and-outputs.md",
        ]);
        assert_eq!(
            find("input", &all, 10),
            [
                "crates/cox/src/ui/input.rs",
                "docs/inputs-and-outputs.md",
                "crates/cox/src/input/view.rs",
            ]
        );
    }

    #[test]
    fn letters_that_start_words_beat_letters_inside_them() {
        let all = paths(&["src/archive.rs", "src/chat/view.rs"]);
        assert_eq!(find("cv", &all, 10), ["src/chat/view.rs", "src/archive.rs"]);
    }

    #[test]
    fn an_empty_query_keeps_the_order_up_to_the_limit() {
        let all = paths(&["b.rs", "a.rs", "c.rs"]);
        assert_eq!(find("", &all, 2), ["b.rs", "a.rs"]);
    }
}
