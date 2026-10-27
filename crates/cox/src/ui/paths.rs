//! Paths as the window writes them.

use std::path::{Path, PathBuf};

/// `path`, with the home directory as `~`.
#[must_use]
pub fn shortened(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::shortened;

    #[test]
    fn paths_in_home_are_shortened() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        assert_eq!(shortened(&home), "~");
        assert_eq!(shortened(&home.join("Cox")), "~/Cox");
        assert_eq!(shortened(Path::new("/elsewhere")), "/elsewhere");
    }
}
