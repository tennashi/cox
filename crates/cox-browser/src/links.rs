//! Which of a page's navigations is a link to a file, for the stage to
//! open on a tab of its own instead of in the page, as a link in the
//! conversation opens: one the human clicked, to a `file:` address that
//! is not a page itself.

use cox_core::address::percent_decoded;

/// The file a navigation of a page to `url` is a link to, as a `file:`
/// address with the lines after it, if any, decoded for the stage to
/// open: when the human asked for it, by a click (`user_gesture`), and it
/// is not an HTML page, which the page shows as any other. Nothing
/// otherwise, so that the page goes there.
pub(crate) fn file_linked(url: &str, user_gesture: bool) -> Option<String> {
    if !user_gesture {
        return None;
    }
    let path = url.strip_prefix("file://")?;
    let path = path.split(['#', '?']).next().unwrap_or(path);
    let name = path.rsplit('/').next().unwrap_or(path);
    let is_page = std::path::Path::new(name)
        .extension()
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
        });
    if is_page || name.is_empty() {
        return None;
    }
    let without_query = match url.split_once('?') {
        // A `?…` ahead of the lines, which a file has no use for.
        Some((before, after)) => match after.split_once('#') {
            Some((_, anchor)) => format!("{before}#{anchor}"),
            None => before.to_owned(),
        },
        None => url.to_owned(),
    };
    Some(percent_decoded(&without_query))
}

#[cfg(test)]
mod tests {
    use super::file_linked;

    #[test]
    fn a_click_on_a_file_address_is_a_link_to_the_file() {
        assert_eq!(
            file_linked("file:///home/me/src/main.rs", true).as_deref(),
            Some("file:///home/me/src/main.rs")
        );
        assert_eq!(
            file_linked("file:///home/me/src/main.rs#L12-L20", true).as_deref(),
            Some("file:///home/me/src/main.rs#L12-L20")
        );
        assert_eq!(
            file_linked("file:///home/me/notes.md?x=1#L3", true).as_deref(),
            Some("file:///home/me/notes.md#L3")
        );
    }

    #[test]
    fn a_page_goes_to_what_is_not_a_link_to_a_file() {
        assert_eq!(file_linked("file:///home/me/src/main.rs", false), None);
        assert_eq!(file_linked("file:///home/me/index.html", true), None);
        assert_eq!(file_linked("file:///home/me/page.HTM#top", true), None);
        assert_eq!(file_linked("file:///home/me/", true), None);
        assert_eq!(file_linked("https://example.com/main.rs", true), None);
    }

    #[test]
    fn escapes_are_decoded() {
        assert_eq!(
            file_linked("file:///home/me/my%20file.rs#L1", true).as_deref(),
            Some("file:///home/me/my file.rs#L1")
        );
    }
}
