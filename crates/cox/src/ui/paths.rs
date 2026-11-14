//! Paths shown to the human, with their home directory as `~`.

use std::path::Path;

use crate::home::home;

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

    use super::shortened_from;
    use crate::home::expanded_in;

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
    fn a_shortened_path_expands_back() {
        let home = Some(Path::new("/home/me"));
        for path in ["/home/me", "/home/me/Cox", "/elsewhere"] {
            let shown = shortened_from(Path::new(path), home);
            assert_eq!(expanded_in(&shown, home), Some(PathBuf::from(path)));
        }
    }
}
