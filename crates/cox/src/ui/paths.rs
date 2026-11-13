//! Paths as the window writes them.

use std::path::{Path, PathBuf};

/// `path`, with the home directory as `~`.
#[must_use]
pub fn shortened(path: &Path) -> String {
    shortened_from(path, std::env::var_os("HOME").map(PathBuf::from).as_deref())
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
    use std::path::Path;

    use super::shortened_from;

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
}
