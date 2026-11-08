//! What Cox keeps of a folder that moved, as a project's folder the
//! human moved or renamed: each path kept in or under the folder, and
//! each `file:` URL of one, made to name the same file where the folder
//! is now.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// A folder that moved, from `from` to `to`.
#[derive(Clone, Copy, Debug)]
pub struct Moved<'a> {
    pub from: &'a Path,
    pub to: &'a Path,
}

impl Moved<'_> {
    /// Where `path` is now, when it is the folder or under it.
    #[must_use]
    pub fn path(&self, path: &Path) -> Option<PathBuf> {
        let rest = path.strip_prefix(self.from).ok()?;
        Some(if rest.as_os_str().is_empty() {
            self.to.to_owned()
        } else {
            self.to.join(rest)
        })
    }

    /// `text` as it is now, when it is a path in the folder, or a `file:`
    /// URL of one, as `file:///work/src/main.rs#L12`.
    #[must_use]
    pub fn text(&self, text: &str) -> Option<String> {
        let (scheme, path) = match text.strip_prefix("file://") {
            Some(path) => ("file://", path),
            None => ("", text),
        };
        let from = self.from.to_str()?.trim_end_matches('/');
        let to = self.to.to_str()?.trim_end_matches('/');
        let rest = path.strip_prefix(from)?;
        // Past the folder's own name, what follows starts another part
        // of the path, or else the folder is another, as `/work2` is to
        // `/work`.
        if !(rest.is_empty() || rest.starts_with(['/', '#', '?', ':'])) {
            return None;
        }
        Some(format!("{scheme}{to}{rest}"))
    }

    /// Makes each path in `value`, and each key of its objects that is
    /// one, name where it is now; answers whether any did.
    pub fn json(&self, value: &mut Value) -> bool {
        match value {
            Value::String(text) => match self.text(text) {
                Some(now) => {
                    *text = now;
                    true
                }
                None => false,
            },
            Value::Array(values) => values
                .iter_mut()
                .fold(false, |moved, value| self.json(value) | moved),
            Value::Object(object) => {
                let mut moved = false;
                let kept = std::mem::take(object);
                for (key, mut value) in kept {
                    moved |= self.json(&mut value);
                    let key = match self.text(&key) {
                        Some(now) => {
                            moved = true;
                            now
                        }
                        None => key,
                    };
                    object.insert(key, value);
                }
                moved
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::Moved;

    fn moved() -> Moved<'static> {
        Moved {
            from: Path::new("/work/app"),
            to: Path::new("/src/app-2"),
        }
    }

    #[test]
    fn a_path_in_the_folder_moves_with_it() {
        assert_eq!(
            moved().path(Path::new("/work/app/src/main.rs")).as_deref(),
            Some(Path::new("/src/app-2/src/main.rs"))
        );
        assert_eq!(
            moved().path(Path::new("/work/app")).as_deref(),
            Some(Path::new("/src/app-2"))
        );
        assert_eq!(moved().path(Path::new("/work/application")), None);
        assert_eq!(moved().path(Path::new("/work")), None);
    }

    #[test]
    fn a_path_or_a_file_url_in_text_moves_and_the_rest_stays() {
        let text = |text| moved().text(text);
        assert_eq!(text("/work/app").as_deref(), Some("/src/app-2"));
        assert_eq!(
            text("/work/app/src/main.rs").as_deref(),
            Some("/src/app-2/src/main.rs")
        );
        assert_eq!(
            text("file:///work/app/README.md#L5-7").as_deref(),
            Some("file:///src/app-2/README.md#L5-7")
        );
        assert_eq!(text("/work/application/x"), None);
        assert_eq!(text("https://example.com/work/app"), None);
        assert_eq!(text("the folder /work/app"), None);
    }

    #[test]
    fn json_moves_the_paths_it_holds_and_its_keys_that_are_paths() {
        let mut value = json!({
            "/work/app": {
                "off": ["/work/app/.cox/triggers/ci.md", "/home/me/workflow/triggers/x.md"],
                "order": [],
            },
            "/work/other": {"off": ["/work/other/.cox/triggers/ci.md"]},
            "tabs": [{"page": "file:///work/app/index.html", "zoom": 1.5}],
        });
        assert!(moved().json(&mut value));
        assert_eq!(
            value,
            json!({
                "/src/app-2": {
                    "off": ["/src/app-2/.cox/triggers/ci.md", "/home/me/workflow/triggers/x.md"],
                    "order": [],
                },
                "/work/other": {"off": ["/work/other/.cox/triggers/ci.md"]},
                "tabs": [{"page": "file:///src/app-2/index.html", "zoom": 1.5}],
            })
        );
        let mut other = json!({"/work/other": ["/work/other/a"]});
        assert!(!moved().json(&mut other));
    }
}
