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
//! - `cox://api/map/<name>`: the map of the project named `<name>`, as
//!   its file in `.cox/maps` has it, with what changed and what is stale
//!   laid over its boxes, and the names of the project's other maps (see
//!   [`crate::stage::map`]); `cox://api/map`, its first map.
//! - `cox://api/file/<path>`: what the project's language server says of
//!   the file at `<path>`, an absolute path in the project: the symbols
//!   it defines and what uses them, once the server has answered, which
//!   the page reads again for (see [`crate::stage::outline`]).
//! - `cox://api/call/<path>:<line>`: what the server says a line of the
//!   file at `<path>` calls, for a step of a flow made there: the
//!   function the line is in and the functions it calls, once the server
//!   has answered (see [`crate::stage::calls`]).
//! - `cox://api/lines/<path>:<line>`: the lines of the file at `<path>`
//!   around `<line>`, each as its stretches with what the syntax says
//!   each is, for a page to show the code a step is made at, colored as
//!   the code tabs color it, and before them the lines that open what
//!   the line is within, as its function's `fn` line and its impl's
//!   (see [`crate::stage::lines`]).
//!
//! - `cox://api/wiki`: the human's wiki, its pages, each with what it
//!   links to, what links to it, where it was used, and what is wrong
//!   with it; and `cox://api/wiki/<page>`, a page, with its text as HTML,
//!   and, when the human reads the wiki in a language of their own, as
//!   HTML in that language too, as far as it is translated (see
//!   [`crate::wiki::told`] and [`crate::translate`]).
//! - `cox://api/project-wiki`: the project's own wiki, in `.cox/wiki` in
//!   its folder, as the human's is told, and whether the folder is the
//!   home, which has none; and `cox://api/project-wiki/<page>`, a page.
//! - `cox://api/raw`: the sources the agents kept of what they read (see
//!   [`crate::raw::told`]), the last kept first; and
//!   `cox://api/raw/<name>`, a source, as a page of the wiki is told,
//!   with where it was read.
//! - `cox://api/project-raw`: the project's own sources, in `raw` in
//!   its folder, as the human's are told, and whether the folder is the
//!   home, which has none; and `cox://api/project-raw/<name>`, a source.
//!
//! - `cox://api/activity`: what goes on in the folder, what waits for
//!   the human and what happened without them (see
//!   [`crate::stage::activity`]).
//! - `cox://api/triggers`: the triggers of the folder, in the order they
//!   run in there, each with whether it runs there and how it last ran,
//!   those on a schedule with when they run next, and in the home, those
//!   of each project too (see
//!   [`crate::trigger::told`]); `cox://api/triggers/set/<change>` turns
//!   one on or off, or orders them, as the JSON of `<change>` says (see
//!   [`crate::trigger::Change`]), and answers them so;
//!   `cox://api/triggers/save/<save>` writes a trigger's card to its file
//!   (see [`crate::trigger::card::Save`]), `triggers/discuss/<line>` has
//!   an agent talk the card through with the human, and
//!   `triggers/discussion/<card>` tells how that stands, and
//!   `triggers/try/<try>` tries the card on the last turn of a chat in
//!   the folder, and `triggers/tried/<card>` tells how it went (see
//!   [`crate::trigger::discuss`]).
//!
//! A plugin draws in the window's colors by linking the stylesheet at
//! `cox://api/theme.css`, whose custom properties on `:root` are the
//! colors of [`crate::ui::theme`], named after them, as `--asking-ground`.
//!
//! A plugin shows a chat of the window by going to `cox://chat/<key>`,
//! the number the Activity page is told it by.
//!
//! A plugin opens a file on a tab of the stage by going to
//! `cox://open/<path>#L<line>`, as the Map plugin does for a box's code.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use cox_browser::scheme::{Resource, Serve, percent_decoded};

use crate::stage::calls::Calls;
use crate::stage::lines;
use crate::stage::map;
use crate::stage::outline::Outlines;
use crate::translate;
use crate::wiki::told as wiki;

/// Where a project keeps its plugins, under its folder.
const PROJECT_PLUGINS: &str = ".cox/plugins";

/// The files of a plugin, by their paths within it.
type Files = &'static [(&'static str, &'static [u8])];

/// What is built into Cox at `cox://<name>/`: the Activity, Map,
/// Knowledge and Triggers plugins, with their files.
const BUILT_IN: &[(&str, Files)] = &[
    (
        "activity",
        &[
            (
                "plugin.json",
                include_bytes!("../../plugins/activity/plugin.json"),
            ),
            (
                "index.html",
                include_bytes!("../../plugins/activity/index.html"),
            ),
            ("app.js", include_bytes!("../../plugins/activity/app.js")),
        ],
    ),
    (
        "map",
        &[
            (
                "plugin.json",
                include_bytes!("../../plugins/map/plugin.json"),
            ),
            ("index.html", include_bytes!("../../plugins/map/index.html")),
            ("app.js", include_bytes!("../../plugins/map/app.js")),
        ],
    ),
    (
        "wiki",
        &[
            (
                "plugin.json",
                include_bytes!("../../plugins/wiki/plugin.json"),
            ),
            (
                "index.html",
                include_bytes!("../../plugins/wiki/index.html"),
            ),
            ("app.js", include_bytes!("../../plugins/wiki/app.js")),
        ],
    ),
    (
        "triggers",
        &[
            (
                "plugin.json",
                include_bytes!("../../plugins/triggers/plugin.json"),
            ),
            (
                "index.html",
                include_bytes!("../../plugins/triggers/index.html"),
            ),
            ("app.js", include_bytes!("../../plugins/triggers/app.js")),
        ],
    ),
];

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
    /// What the project's language server says of its files.
    outlines: Outlines,
    calls: Calls,
}

impl Plugins {
    /// The plugins of a chat working in `root`, whose pages ask
    /// `outlines` what the project's files define.
    #[must_use]
    pub fn new(root: PathBuf, outlines: Outlines, calls: Calls) -> Self {
        Self {
            root,
            outlines,
            calls,
        }
    }

    /// The plugins there are: the built-in ones, then the project's, by
    /// name.
    #[must_use]
    pub fn list(&self) -> Vec<Plugin> {
        let mut plugins: Vec<Plugin> = BUILT_IN
            .iter()
            .filter(|(_, files)| files.iter().any(|(path, _)| *path == "index.html"))
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
            if path == "theme.css" {
                return Some(Resource {
                    mime: "text/css".to_owned(),
                    body: crate::ui::theme::css().into_bytes(),
                });
            }
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
        let (what, rest) = what.split_once('/').unwrap_or((what, ""));
        // A picture or a video shown in a conversation, which a click on
        // it opens on the stage.
        if what == "media" {
            let (path, mime) = crate::media::kept(rest)?;
            return Some(Resource {
                mime: mime.to_owned(),
                body: std::fs::read(path).ok()?,
            });
        }
        let json = match what {
            "map" if rest.is_empty() => map::told(&self.root, None),
            "map" if !rest.contains('/') => map::told(&self.root, Some(&percent_decoded(rest))),
            "wiki" => {
                let root = crate::settings::wiki();
                if rest.is_empty() {
                    wiki::told(&root)
                } else {
                    wiki::page_in(&root, &percent_decoded(rest), translate::page)?
                }
            }
            "triggers" if rest.is_empty() => self.triggers(),
            "triggers" => {
                if let Some(json) = self.card(rest) {
                    json
                } else {
                    let change = rest.strip_prefix("set/")?;
                    let change: crate::trigger::Change =
                        serde_json::from_str(&percent_decoded(change)).ok()?;
                    // Only where a trigger shown runs.
                    let shown = self.triggers();
                    let runs_there = shown["folders"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .flat_map(|folder| folder["triggers"].as_array().into_iter().flatten())
                        .any(|trigger| trigger["at"].as_str() == change.folder().to_str());
                    if !runs_there {
                        return None;
                    }
                    let kept = crate::storage::path(crate::storage::Kept::Triggers);
                    if let Err(error) = change.make(kept.as_deref()) {
                        tracing::error!("the triggers are not arranged as asked: {error}");
                    }
                    self.triggers()
                }
            }
            "activity" if rest.is_empty() => crate::stage::activity::told(
                &self.root,
                &crate::settings::wiki(),
                &crate::settings::raw(),
                &self.triggers(),
            ),
            "project-wiki" => {
                let root = self.root.join(crate::wiki::PROJECT_WIKI);
                if rest.is_empty() {
                    let mut told = wiki::told(&root);
                    told["home"] = (self.root == home()).into();
                    told
                } else {
                    wiki::page_in(&root, &percent_decoded(rest), translate::page)?
                }
            }
            "raw" | "project-raw" => {
                let root = if what == "raw" {
                    crate::settings::raw()
                } else {
                    self.root.join(crate::raw::PROJECT_RAW)
                };
                if rest.is_empty() {
                    let mut told = crate::raw::told(&root);
                    told["home"] = (self.root == home()).into();
                    told
                } else {
                    crate::raw::told_source(&root, &percent_decoded(rest), translate::page)?
                }
            }
            "file" if !rest.is_empty() => {
                let path = PathBuf::from(format!("/{}", percent_decoded(rest)));
                self.outlines.told(&self.root, &path)?
            }
            "call" | "lines" if !rest.is_empty() => {
                let spec = percent_decoded(rest);
                let (path, line) = spec.rsplit_once(':')?;
                let line: usize = line.parse().ok()?;
                let path = PathBuf::from(format!("/{path}"));
                if what == "call" {
                    self.calls.told(&self.root, &path, line)?
                } else {
                    lines::told(&self.root, &path, line)?
                }
            }
            _ => return None,
        };
        Some(Resource {
            mime: "application/json".to_owned(),
            body: serde_json::to_vec(&json).ok()?,
        })
    }

    /// What the card of a trigger asks of `cox://api/triggers/<rest>`, if
    /// it is the card's: to save it, to talk it through or try it, and how
    /// those stand, of a folder the tab shows only.
    fn card(&self, rest: &str) -> Option<serde_json::Value> {
        use crate::trigger::{card, discuss};
        let (what, said) = rest.split_once('/')?;
        let said = percent_decoded(said);
        let shown = |folder: &Path| {
            self.trigger_folders()
                .iter()
                .any(|(shown, _)| shown == folder)
        };
        Some(match what {
            "save" => {
                let save: card::Save = serde_json::from_str(&said).ok()?;
                if !shown(&save.folder) {
                    return None;
                }
                let made = save.make(&crate::settings::workflow());
                let mut told = self.triggers();
                match made {
                    Ok(file) => told["saved"] = serde_json::json!(file),
                    Err(why) => told["error"] = why.into(),
                }
                told
            }
            "discuss" => {
                let line: discuss::Line = serde_json::from_str(&said).ok()?;
                if !shown(&line.folder) {
                    return None;
                }
                discuss::say(&line)
            }
            "try" => {
                let tried: discuss::Try = serde_json::from_str(&said).ok()?;
                if !shown(&tried.folder) {
                    return None;
                }
                discuss::try_on(tried, crate::storage::path(crate::storage::Kept::LastTurns))
            }
            "discussion" => discuss::told(&said),
            "tried" => discuss::tried(&said),
            "end" => {
                discuss::end(&said);
                serde_json::json!({})
            }
            _ => return None,
        })
    }

    /// The triggers of the folder, as `cox://api/triggers` tells them
    /// (see [`crate::trigger::told`]).
    fn triggers(&self) -> serde_json::Value {
        use crate::storage::{Kept, path};
        let (arranged, runs) = (path(Kept::Triggers), path(Kept::TriggerRuns));
        crate::trigger::told(
            &self.trigger_folders(),
            &crate::trigger::Kept {
                workflow: &crate::settings::workflow(),
                home: &home(),
                arranged: arranged.as_deref(),
                runs: runs.as_deref(),
                schedules: &crate::schedule::Ran::read(path(Kept::Schedules)),
            },
            &jiff::Zoned::now(),
        )
    }

    /// The folders whose triggers the Triggers tab shows: the stage's,
    /// and in the home, each project's after it, by name.
    fn trigger_folders(&self) -> Vec<(PathBuf, Option<String>)> {
        let mut folders = vec![(self.root.clone(), None)];
        if self.root == home() {
            folders.extend(
                crate::stage::activity::projects()
                    .into_iter()
                    .filter(|(folder, _)| *folder != self.root)
                    .map(|(folder, name)| (folder, Some(name))),
            );
        }
        folders
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

/// The home, the folder of Cox's own chats: as the window tells it, or else
/// as the settings set it.
fn home() -> PathBuf {
    crate::stage::activity::home()
        .unwrap_or_else(|| crate::settings::home_for_chats(crate::settings::Saved::read().home))
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
/// fragment: `cox://map/app.js` is `("map", "app.js")`.
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
        assert_eq!(split("cox://map/"), Some(("map", "")));
        assert_eq!(split("cox://map/app.js?x=1#top"), Some(("map", "app.js")));
        assert_eq!(split("cox://api/map"), Some(("api", "map")));
        assert_eq!(split("cox://map"), Some(("map", "")));
        assert_eq!(split("cox:///x"), None);
        assert_eq!(split("https://map/"), None);
    }

    #[test]
    fn the_built_in_plugin_is_served_with_its_files() {
        let plugins = Plugins::new(scratch("built-in"), Outlines::new().0, Calls::new().0);

        let page = plugins.resource("cox://map/").unwrap();
        assert_eq!(page.mime, "text/html");
        assert!(String::from_utf8(page.body).unwrap().contains("app.js"));
        assert_eq!(
            plugins.resource("cox://map/app.js").unwrap().mime,
            "text/javascript"
        );
        assert_eq!(plugins.resource("cox://map/missing.js"), None);
        assert_eq!(plugins.resource("cox://nothing/"), None);
        assert_eq!(plugins.resource("cox://api/weather"), None);
    }

    #[test]
    fn the_map_is_told_as_json() {
        let root = scratch("map");
        let maps = root.join(cox_editor::map::MAPS_FOLDER);
        std::fs::create_dir_all(&maps).unwrap();
        std::fs::write(
            maps.join("map.json"),
            r#"{ "tiers": [{ "id": "whole", "name": "Whole", "rows": [{ "parts": ["one"] }] }],
                 "parts": { "one": { "name": "One", "holds": ["src"] } } }"#,
        )
        .unwrap();
        let plugins = Plugins::new(root, Outlines::new().0, Calls::new().0);

        let answer = plugins.resource("cox://api/map").unwrap();
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(json["map"]["tiers"][0]["id"], "whole");
        assert_eq!(json["facts"]["one"]["stale"], serde_json::json!(["src"]));
        let answer = plugins.resource("cox://api/map/map").unwrap();
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(json["name"], "map");
        assert_eq!(json["map"]["tiers"][0]["id"], "whole");
        let answer = plugins.resource("cox://api/map/no%20such").unwrap();
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("no map `no such`"));
        assert_eq!(plugins.resource("cox://api/map/one/"), None);
    }

    #[test]
    fn the_schedules_of_the_folder_are_told_among_its_triggers() {
        let root = scratch("schedules");
        std::fs::create_dir_all(root.join(".cox/triggers")).unwrap();
        std::fs::write(
            root.join(".cox/triggers/triage.md"),
            "---\non: {schedule: {cron: \"0 */6 * * *\"}}\n---\n\nLook at what changed.\n",
        )
        .unwrap();
        let plugins = Plugins::new(root, Outlines::new().0, Calls::new().0);

        let answer = plugins.resource("cox://api/triggers").unwrap();
        assert_eq!(answer.mime, "application/json");
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        let triage = &json["folders"][0]["triggers"][0];
        assert_eq!(triage["name"], "triage");
        assert_eq!(triage["scope"], "project");
        assert_eq!(triage["schedule"]["every"], "Every 6 hours");
    }

    #[test]
    fn the_projects_sources_are_told_as_json() {
        let root = scratch("project-raw");
        std::fs::create_dir_all(root.join("raw/sketches")).unwrap();
        std::fs::write(
            root.join("raw/sketches/2026-11-10-mock.md"),
            "---\nsource: \"chat 3\"\nkept: 2026-11-10\n---\n\n# Mock\n\nWhat it tried.\n",
        )
        .unwrap();
        let plugins = Plugins::new(root, Outlines::new().0, Calls::new().0);

        let answer = plugins.resource("cox://api/project-raw").unwrap();
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(json["sources"][0]["name"], "sketches/2026-11-10-mock");
        assert_eq!(json["sources"][0]["source"], "chat 3");
        assert_eq!(json["home"], false);
        let answer = plugins
            .resource("cox://api/project-raw/sketches/2026-11-10-mock")
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(json["title"], "Mock");
        assert_eq!(
            plugins.resource("cox://api/project-raw/sketches/nothing"),
            None
        );
    }

    #[test]
    fn the_projects_wiki_is_told_as_json() {
        let root = scratch("project-wiki");
        std::fs::create_dir_all(root.join(".cox/wiki")).unwrap();
        std::fs::write(
            root.join(".cox/wiki/people.md"),
            "# People\n\nWho to ask.\n",
        )
        .unwrap();
        let plugins = Plugins::new(root, Outlines::new().0, Calls::new().0);

        let answer = plugins.resource("cox://api/project-wiki").unwrap();
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(json["pages"][0]["name"], "people");
        assert_eq!(json["home"], false);
        let answer = plugins.resource("cox://api/project-wiki/people").unwrap();
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(json["title"], "People");
        assert_eq!(plugins.resource("cox://api/project-wiki/nobody"), None);
    }

    #[test]
    fn a_file_is_told_as_json_within_the_project() {
        let root = scratch("file");
        std::fs::create_dir_all(root.join("my src")).unwrap();
        std::fs::write(root.join("my src/main.rs"), "fn main() {}\n").unwrap();
        let plugins = Plugins::new(root.clone(), Outlines::new().0, Calls::new().0);
        let url = format!("cox://api/file{}/my%20src/main.rs", root.display());
        let answer = plugins.resource(&url).unwrap();
        assert_eq!(answer.mime, "application/json");
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(json["name"], "my src/main.rs");
        assert_eq!(json["lines"], 1);
        assert_eq!(json["pending"], true);
        assert_eq!(plugins.resource("cox://api/file"), None);
        assert_eq!(plugins.resource("cox://api/file/etc/hostname"), None);

        let url = format!("cox://api/call{}/my%20src/main.rs:1", root.display());
        let answer = plugins.resource(&url).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(json["name"], "my src/main.rs");
        assert_eq!(json["line"], 1);
        assert_eq!(json["pending"], true);
        let url = format!("cox://api/call{}/my%20src/main.rs", root.display());
        assert_eq!(plugins.resource(&url), None);
        assert_eq!(plugins.resource("cox://api/call/etc/hostname:1"), None);

        let url = format!("cox://api/lines{}/my%20src/main.rs:1", root.display());
        let answer = plugins.resource(&url).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(json["name"], "my src/main.rs");
        assert_eq!(json["from"], 1);
        let line: String = json["lines"][0]
            .as_array()
            .unwrap()
            .iter()
            .map(|stretch| stretch[0].as_str().unwrap())
            .collect();
        assert_eq!(line, "fn main() {}");
        assert_eq!(json["lines"][0][0], serde_json::json!(["fn", "keyword"]));
        assert_eq!(plugins.resource("cox://api/lines/etc/hostname:1"), None);
    }

    #[test]
    fn the_projects_plugins_are_listed_and_served_within_their_folders() {
        let root = scratch("project");
        let folder = root.join(PROJECT_PLUGINS).join("tour");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("index.html"), "<h1>Tour</h1>").unwrap();
        std::fs::write(folder.join("plugin.json"), r#"{"title": "A tour"}"#).unwrap();
        std::fs::write(root.join("secret.txt"), "no").unwrap();
        let plugins = Plugins::new(root, Outlines::new().0, Calls::new().0);

        let listed = plugins.list();
        let names: Vec<&str> = listed.iter().map(|plugin| plugin.name.as_str()).collect();
        assert_eq!(names, ["activity", "map", "wiki", "triggers", "tour"]);
        assert_eq!(listed[0].title, "Activity");
        assert_eq!(listed[1].title, "Map");
        assert_eq!(listed[2].title, "Knowledge");
        assert_eq!(listed[3].title, "Triggers");
        assert_eq!(listed[4].title, "A tour");
        assert_eq!(listed[4].url(), "cox://tour/");
        assert_eq!(
            plugins.resource("cox://tour/").unwrap().body,
            b"<h1>Tour</h1>"
        );
        assert_eq!(plugins.resource("cox://tour/../../secret.txt"), None);
        assert_eq!(plugins.resource("cox://../secret.txt"), None);
    }
}
