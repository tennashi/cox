//! Plugins of the stage: pages, in HTML and JavaScript, that draw what Cox
//! knows, as a tab Cox draws would, for whoever wants a view of their own
//! on the stage, the agent included. A plugin is a folder with an
//! `index.html`, and a `plugin.json` naming its `title` if it has one: the
//! ones built into Cox, in `crates/cox/plugins`, and the project's, in
//! `.cox/plugins/<name>` under its folder. The stage shows a plugin as a
//! page at `cox://<name>/`, which it [serves](Plugins::serve) itself (see
//! [`cox_browser::scheme`]), and a button on its tab bar for each.
//!
//! What Cox knows, a plugin reads as JSON from `cox://api/<what>`:
//!
//! - `cox://api/parts`: the parts the chat runs with, as the Parts tab
//!   shows them (see [`crate::stage::parts`]), with the file and line of
//!   the code that starts each.
//!
//! A plugin opens a file on a tab of the stage by going to
//! `cox://open/<path>#L<line>`, as the Parts plugin does for a part's code.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use cox_browser::scheme::{Resource, Serve};

use crate::stage::parts::{self, Started};

/// Where a project keeps its plugins, under its folder.
const PROJECT_PLUGINS: &str = ".cox/plugins";

/// The files of a plugin, by their paths within it.
type Files = &'static [(&'static str, &'static [u8])];

/// The plugins built into Cox: each by its name, with its files.
const BUILT_IN: &[(&str, Files)] = &[(
    "parts",
    &[
        (
            "plugin.json",
            include_bytes!("../../plugins/parts/plugin.json"),
        ),
        (
            "index.html",
            include_bytes!("../../plugins/parts/index.html"),
        ),
        ("app.js", include_bytes!("../../plugins/parts/app.js")),
        (
            "vendor/preact.mjs",
            include_bytes!("../../plugins/parts/vendor/preact.mjs"),
        ),
        (
            "vendor/hooks.mjs",
            include_bytes!("../../plugins/parts/vendor/hooks.mjs"),
        ),
        (
            "vendor/htm.mjs",
            include_bytes!("../../plugins/parts/vendor/htm.mjs"),
        ),
    ],
)];

/// A plugin the stage can show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    /// Its folder's name, which its address goes by.
    pub name: String,
    /// What its button says: its `plugin.json`'s `title`, or its name.
    pub title: String,
}

impl Plugin {
    /// The address of its page.
    #[must_use]
    pub fn url(&self) -> String {
        format!("cox://{}/", self.name)
    }
}

/// The plugins of one chat's stage, and what they read of the chat.
#[derive(Clone)]
pub struct Plugins {
    /// The project's folder.
    root: PathBuf,
    /// What the chat runs with, once its agent runs.
    started: Arc<Mutex<Option<Started>>>,
}

impl Plugins {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            started: Arc::new(Mutex::new(None)),
        }
    }

    /// The chat's agent runs, with `started`.
    pub fn runs(&self, started: Started) {
        *self
            .started
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(started);
    }

    /// The plugins there are: the built-in ones, then the project's, by
    /// name.
    #[must_use]
    pub fn list(&self) -> Vec<Plugin> {
        let mut plugins: Vec<Plugin> = BUILT_IN
            .iter()
            .map(|(name, files)| {
                let manifest = files
                    .iter()
                    .find(|(path, _)| *path == "plugin.json")
                    .map(|(_, bytes)| bytes.to_vec());
                plugin(name, manifest.as_deref())
            })
            .collect();
        let mut project: Vec<Plugin> = std::fs::read_dir(self.root.join(PROJECT_PLUGINS))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("index.html").is_file())
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                if BUILT_IN.iter().any(|(built, _)| *built == name) || name == "api" {
                    return None;
                }
                let manifest = std::fs::read(entry.path().join("plugin.json")).ok();
                Some(plugin(&name, manifest.as_deref()))
            })
            .collect();
        project.sort_by(|a, b| a.name.cmp(&b.name));
        plugins.extend(project);
        plugins
    }

    /// What the stage's pages get at `cox:` addresses.
    #[must_use]
    pub fn serve(&self) -> Serve {
        let plugins = self.clone();
        Arc::new(move |url| plugins.resource(url))
    }

    /// What is at `url`, if anything.
    fn resource(&self, url: &str) -> Option<Resource> {
        let (host, path) = split(url)?;
        if host == "api" {
            return self.api(path);
        }
        let path = if path.is_empty() || path.ends_with('/') {
            format!("{path}index.html")
        } else {
            path.to_owned()
        };
        let body = Self::built_in(host, &path).or_else(|| self.of_project(host, &path))?;
        Some(Resource {
            mime: mime(&path).to_owned(),
            body,
        })
    }

    /// What Cox knows, as JSON, at `cox://api/<what>`.
    fn api(&self, what: &str) -> Option<Resource> {
        let json = match what {
            "parts" => {
                let started = self
                    .started
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                parts::told(&self.root, started.as_ref())
            }
            _ => return None,
        };
        Some(Resource {
            mime: "application/json".to_owned(),
            body: serde_json::to_vec(&json).ok()?,
        })
    }

    /// The file `path` of the built-in plugin `name`, if there is one.
    fn built_in(name: &str, path: &str) -> Option<Vec<u8>> {
        let (_, files) = BUILT_IN.iter().find(|(built, _)| *built == name)?;
        files
            .iter()
            .find(|(file, _)| *file == path)
            .map(|(_, bytes)| bytes.to_vec())
    }

    /// The file `path` of the project's plugin `name`, if there is one:
    /// within the plugin's folder only.
    fn of_project(&self, name: &str, path: &str) -> Option<Vec<u8>> {
        let within = |part: &str| {
            Path::new(part)
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        };
        if !within(name) || !within(path) {
            return None;
        }
        std::fs::read(self.root.join(PROJECT_PLUGINS).join(name).join(path)).ok()
    }
}

/// A plugin `name`, titled as its manifest says, when it has one that
/// does.
fn plugin(name: &str, manifest: Option<&[u8]>) -> Plugin {
    let title = manifest
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
        .and_then(|manifest| manifest.get("title")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| name.to_owned());
    Plugin {
        name: name.to_owned(),
        title,
    }
}

/// The host and the path of a `cox:` address, without its query and
/// fragment: `cox://parts/app.js` is `("parts", "app.js")`.
fn split(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("cox://")?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    if host.is_empty() {
        return None;
    }
    Some((host, path))
}

/// The media type of a file, by its name.
fn mime(path: &str) -> &'static str {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    match extension {
        "html" | "htm" => "text/html",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "woff2" => "font/woff2",
        "txt" | "md" => "text/plain",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("cox-plugins-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn addresses_split_into_host_and_path() {
        assert_eq!(split("cox://parts/"), Some(("parts", "")));
        assert_eq!(
            split("cox://parts/app.js?x=1#top"),
            Some(("parts", "app.js"))
        );
        assert_eq!(split("cox://api/parts"), Some(("api", "parts")));
        assert_eq!(split("cox://parts"), Some(("parts", "")));
        assert_eq!(split("cox:///x"), None);
        assert_eq!(split("https://parts/"), None);
    }

    #[test]
    fn the_built_in_plugin_is_served_with_its_files() {
        let plugins = Plugins::new(scratch("built-in"));

        let page = plugins.resource("cox://parts/").unwrap();
        assert_eq!(page.mime, "text/html");
        assert!(String::from_utf8(page.body).unwrap().contains("app.js"));
        assert_eq!(
            plugins.resource("cox://parts/app.js").unwrap().mime,
            "text/javascript"
        );
        assert!(plugins.resource("cox://parts/vendor/preact.mjs").is_some());
        assert_eq!(plugins.resource("cox://parts/missing.js"), None);
        assert_eq!(plugins.resource("cox://nothing/"), None);
    }

    #[test]
    fn the_parts_are_told_as_json() {
        let plugins = Plugins::new(scratch("api"));

        let answer = plugins.resource("cox://api/parts").unwrap();
        assert_eq!(answer.mime, "application/json");
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(json["running"], false);
        assert_eq!(json["rows"][0]["parts"][0]["name"], "the agent");
        assert_eq!(plugins.resource("cox://api/weather"), None);
    }

    #[test]
    fn the_projects_plugins_are_listed_and_served_within_their_folders() {
        let root = scratch("project");
        let folder = root.join(PROJECT_PLUGINS).join("tour");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("index.html"), "<h1>Tour</h1>").unwrap();
        std::fs::write(folder.join("plugin.json"), r#"{"title": "A tour"}"#).unwrap();
        std::fs::write(root.join("secret.txt"), "no").unwrap();
        let plugins = Plugins::new(root);

        let listed = plugins.list();
        assert_eq!(listed[0].name, "parts");
        assert_eq!(listed[0].title, "Parts (web)");
        assert_eq!(listed[1].name, "tour");
        assert_eq!(listed[1].title, "A tour");
        assert_eq!(listed[1].url(), "cox://tour/");
        assert_eq!(
            plugins.resource("cox://tour/").unwrap().body,
            b"<h1>Tour</h1>"
        );
        assert_eq!(plugins.resource("cox://tour/../../secret.txt"), None);
        assert_eq!(plugins.resource("cox://../secret.txt"), None);
    }
}
