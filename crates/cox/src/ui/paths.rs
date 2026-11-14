//! The human's home directory, and paths written from it with `~`.

use std::path::{Path, PathBuf};

/// The human's home directory, from `HOME`, or `None` when it is unset
/// or empty.
#[must_use]
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// `text` as a path, with a leading `~` standing for the home directory,
/// or `None` when it starts so and there is none. See [`expanded_in`].
#[must_use]
pub fn expanded(text: &str) -> Option<PathBuf> {
    expanded_in(text, home().as_deref())
}

/// `text` as a path, with `~` or a leading `~/` as `home`, or `None` when
/// it starts so and there is no home. Any other `text`, `~user` among
/// them, is the path as written.
#[must_use]
pub fn expanded_in(text: &str, home: Option<&Path>) -> Option<PathBuf> {
    match text.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            Some(home?.join(rest.trim_start_matches('/')))
        }
        _ => Some(PathBuf::from(text)),
    }
}

/// `path`, with the home directory as `~`.
#[must_use]
pub fn shortened(path: &Path) -> String {
    shortened_from(path, home().as_deref())
}

/// `path`, with `home`, the human's home directory, as `~`.
fn shortened_from(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{expanded_in, shortened_from};

    #[test]
    fn paths_in_home_are_shortened() {
        let home = Some(Path::new("/home/me"));
        assert_eq!(shortened_from(Path::new("/home/me"), home), "~");
        assert_eq!(shortened_from(Path::new("/home/me/Cox"), home), "~/Cox");
        assert_eq!(shortened_from(Path::new("/home/meta"), home), "/home/meta");
        assert_eq!(shortened_from(Path::new("/elsewhere"), home), "/elsewhere");
        assert_eq!(
            shortened_from(Path::new("/home/me/Cox"), None),
            "/home/me/Cox"
        );
    }

    #[test]
    fn a_tilde_alone_or_before_a_slash_stands_for_home() {
        let home = Some(Path::new("/home/me"));
        let expanded = |text| expanded_in(text, home);
        assert_eq!(expanded("~"), Some(PathBuf::from("/home/me")));
        assert_eq!(expanded("~/Work"), Some(PathBuf::from("/home/me/Work")));
        assert_eq!(expanded("~//Work"), Some(PathBuf::from("/home/me/Work")));
        assert_eq!(expanded("~you/Work"), Some(PathBuf::from("~you/Work")));
        assert_eq!(expanded("/etc/hosts"), Some(PathBuf::from("/etc/hosts")));
        assert_eq!(expanded("src/~/x"), Some(PathBuf::from("src/~/x")));
    }

    #[test]
    fn a_tilde_names_nothing_without_a_home() {
        assert_eq!(expanded_in("~", None), None);
        assert_eq!(expanded_in("~/Work", None), None);
        assert_eq!(expanded_in("~you", None), Some(PathBuf::from("~you")));
        assert_eq!(expanded_in("Work", None), Some(PathBuf::from("Work")));
    }

    #[test]
    fn a_shortened_path_expands_back() {
        let home = Some(Path::new("/home/me"));
        for path in ["/home/me", "/home/me/Cox", "/elsewhere"] {
            let shown = shortened_from(Path::new(path), home);
            assert_eq!(expanded_in(&shown, home), Some(PathBuf::from(path)));
        }
    }
}
