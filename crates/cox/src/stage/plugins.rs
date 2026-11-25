//! Plugins of the stage: pages, in HTML and JavaScript, that draw what Cox
//! knows, as a tab Cox draws would, for whoever wants a view of their own
//! on the stage, the agent included. A plugin is a folder with an
//! `index.html`, and a `plugin.json` naming its `title` if it has one: the
//! ones built into Cox, in `crates/cox/plugins`, and the project's, in
//! `plugins/<name>` under its folder. The stage shows a plugin as a
//! page at `cox://<name>/`, which it [serves](Plugins::serve) itself (see
//! [`cox_browser::scheme`]), and a button on its tab bar for each.
//!
//! What Cox knows, a plugin reads as JSON from `cox://api/<what>`:
//!
//! - `cox://api/map/<name>`: the map of the project named `<name>`, as
//!   coxd draws it from its vocabulary in `maps`, the project's or one
//!   of its repositories' (as `app/use`), and the indexes of the
//!   project's code, which asking for it has coxd make when they are not
//!   made yet, with how each stands, what changed in the code of its
//!   boxes, and the names of the project's other maps (see
//!   [`crate::stage::map::told`]); `cox://api/map`, its first map; and
//!   `cox://api/map/again/<name>`, the map, making again the indexes that
//!   failed, which only a plugin's own `fetch` has done.
//! - `cox://api/wiki`: the human's wiki, its pages, each with what it
//!   links to, what links to it, where it was used, and what is wrong
//!   with it; and `cox://api/wiki/<page>`, a page, with its text as HTML,
//!   and, when the human reads the wiki in a language of their own, as
//!   HTML in that language too, as far as it is translated (see
//!   [`coxd::wiki::told`] and [`coxd::translate`]).
//! - `cox://api/project-wiki`: the project's own wiki, in `wiki` in
//!   its folder, as the human's is told, and whether the folder is the
//!   home, which has none; and `cox://api/project-wiki/<page>`, a page.
//! - `cox://api/raw`: the sources the agents kept of what they read (see
//!   [`coxd::raw::told`]), the last kept first; and
//!   `cox://api/raw/<name>`, a source, as a page of the wiki is told,
//!   with where it was read.
//! - `cox://api/project-raw`: the project's own sources, in `raw` in
//!   its folder, as the human's are told, and whether the folder is the
//!   home, which has none; and `cox://api/project-raw/<name>`, a source.
//!
//! - `cox://api/activity`: what goes on in the folder, the human's
//!   inbox: what is asked of them, what is at work and what each chat
//!   there does now, and how many steps got done lately (see
//!   [`crate::stage::activity`]);
//!   `cox://api/activity/dismiss/<plans>` hides the plans the JSON of
//!   `<plans>` names until their chats' agents send others, and answers
//!   what goes on then. Only a plugin's own `fetch` has them hidden.
//!   `cox://api/activity/fix/{"key":<key>,"repo":<repo>,"number":<n>}`
//!   asks the chat `<key>` to fix the CI that failed on the pull request
//!   its agent opened (see [`crate::stage::activity::fix`]), and answers
//!   what goes on then.
//!   `cox://api/activity/timeline/{"chat":<chat>}` tells, alone, the story
//!   of the chat coxd keeps as `<chat>`: when it started, what it did
//!   when, and when and how it ended (see
//!   [`crate::stage::activity::timeline`]); and
//!   `cox://api/activity/command/{"chat":<chat>,"number":<entry>}`, a
//!   command of the story, laid out, with what it printed.
//! - `cox://api/triggers`: the triggers of the folder, in the order they
//!   run in there, each with whether it runs there and how it last ran,
//!   those on a schedule with when they run next, and in the home, those
//!   of each project too (see
//!   [`coxd::trigger::told`]); `cox://api/triggers/set/<change>` turns
//!   one on or off, or orders them, as the JSON of `<change>` says (see
//!   [`coxd::trigger::Change`]), and answers them so;
//!   `cox://api/triggers/save/<save>` writes a trigger's card to its file
//!   (see [`coxd::trigger::card::Save`]), `triggers/discuss/<line>` has
//!   an agent talk the card through with the human, and
//!   `triggers/discussion/<card>` tells how that stands, and
//!   `triggers/try/<try>` tries the card on the last turn of a chat in
//!   the folder, and `triggers/tried/<card>` tells how it went (see
//!   [`coxd::trigger::discuss`]). Only a plugin's own `fetch` has the
//!   triggers set, or a card saved, talked through, tried or ended
//!   (`triggers/end/<card>`): a tab sent to one of those, by a link or a
//!   page elsewhere, gets nothing (see [`Asker`]).
//!
//! A plugin draws in the window's colors by linking the stylesheet at
//! `cox://api/theme.css`, or `theme.css` beside its page, whose custom
//! properties on `:root` are the colors of [`coxd::theme`], named after
//! them, as `--asking-ground`.
//!
//! A plugin shows a chat of the window by going to `cox://chat/<key>`,
//! the number the Activity page is told it by.
//!
//! A plugin opens a file on a tab of the stage by going to
//! `cox://open/<path>#L<line>`, as the Map plugin does for a box's code.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use cox_browser::scheme::{Asker, Resource, Serve};
use cox_core::address::percent_decoded;

use crate::stage::map;
use coxd::translate;
use coxd::wiki::told as wiki;

/// Where a project keeps its plugins, under its folder.
const PROJECT_PLUGINS: &str = "plugins";

/// The files of a plugin, by their paths within it.
type Files = &'static [(&'static str, &'static [u8])];

/// The pages of the plugins Cox had built in and no longer has: the
/// Commands page, whose commands a chat's story on the Activity page tells
/// now.
const GONE: &[&str] = &["cox://commands/"];

/// Whether `url` is of a page of a plugin Cox had built in and no longer
/// has, which a tab kept of it does not open again.
#[must_use]
pub fn gone(url: &str) -> bool {
    GONE.iter().any(|gone| url.starts_with(gone))
}

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
    // The phone serves the same files (see [`coxd::map::PAGE`]).
    ("map", coxd::map::PAGE),
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
}

impl Plugins {
    /// The plugins of a chat working in `root`.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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

    /// What the stage's pages get at `cox:` addresses: anyone is told what
    /// is there, and only a plugin has the stage do what it asks (see
    /// [`Asker`]).
    #[must_use]
    pub fn serve(&self) -> Serve {
        let plugins = self.clone();
        Arc::new(move |url, asker| {
            if asker == Asker::Anyone && acts(url) {
                return None;
            }
            plugins.resource(url)
        })
    }

    /// What is at `url`, if anything.
    fn resource(&self, url: &str) -> Option<Resource> {
        let (host, path) = split(url)?;
        if host == "api" {
            if path == "theme.css" {
                return Some(Resource {
                    mime: "text/css".to_owned(),
                    body: coxd::theme::css().into_bytes(),
                });
            }
            return self.api(path);
        }
        let path = if path.is_empty() || path.ends_with('/') {
            format!("{path}index.html")
        } else {
            path.to_owned()
        };
        // A plugin's page may link the theme beside itself, as the Map
        // page does, whose files the phone serves with the theme beside
        // them too, unless the plugin has a file of that name.
        let body = Self::built_in(host, &path)
            .or_else(|| self.of_project(host, &path))
            .or_else(|| (path == "theme.css").then(|| coxd::theme::css().into_bytes()))?;
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
            let (path, mime) = coxd::media::kept(rest)?;
            return Some(Resource {
                mime: mime.to_owned(),
                body: std::fs::read(path).ok()?,
            });
        }
        let json = match what {
            "map" => return self.map(rest),
            "wiki" => {
                let root = coxd::settings::wiki();
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
                    let change: coxd::trigger::Change =
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
                    let kept = coxd::storage::path(coxd::storage::Kept::Triggers);
                    if let Err(error) = change.make(kept.as_deref()) {
                        tracing::error!("the triggers are not arranged as asked: {error}");
                    }
                    self.triggers()
                }
            }
            "activity" => self.activity(rest)?,
            "project-wiki" => {
                let root = self.root.join(coxd::wiki::PROJECT_WIKI);
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
                    coxd::settings::raw()
                } else {
                    self.root.join(coxd::raw::PROJECT_RAW)
                };
                if rest.is_empty() {
                    let mut told = coxd::raw::told(&root);
                    told["home"] = (self.root == home()).into();
                    told
                } else {
                    coxd::raw::told_source(&root, &percent_decoded(rest), translate::page)?
                }
            }
            _ => return None,
        };
        Some(Resource {
            mime: "application/json".to_owned(),
            body: serde_json::to_vec(&json).ok()?,
        })
    }

    /// The map `cox://api/map/<rest>` asks for, as coxd draws it (see
    /// [`map::told`]).
    fn map(&self, rest: &str) -> Option<Resource> {
        let (again, name) = match rest.strip_prefix("again/") {
            Some(name) => (true, name),
            None if !rest.contains('/') => (false, rest),
            None => return None,
        };
        let name = (!name.is_empty()).then(|| percent_decoded(name));
        Some(Resource {
            mime: "application/json".to_owned(),
            body: map::told(&self.root, name.as_deref(), again).into_bytes(),
        })
    }

    /// What goes on in the folder, as `cox://api/activity` tells it, once
    /// what `cox://api/activity/<rest>` asks is done, if it asks: to
    /// dismiss plans, to say a line on a step of a plan, to have a chat
    /// fix the CI of a pull request it opened, or to show the thread of a
    /// step, which it tells too.
    fn activity(&self, rest: &str) -> Option<serde_json::Value> {
        use crate::stage::activity;
        let mut open = None;
        if !rest.is_empty() {
            let (what, asked) = rest.split_once('/')?;
            let asked = percent_decoded(asked);
            match what {
                "dismiss" => activity::dismiss(&asked),
                "fix" => activity::fix(&asked),
                "say" => open = Some(activity::say(&asked)?),
                "thread" => open = Some(serde_json::from_str::<activity::Open>(&asked).ok()?),
                // A chat's story, alone, read only for the row the human
                // opened.
                "timeline" => {
                    return Some(activity::timeline(
                        &serde_json::from_str::<activity::Of>(&asked).ok()?,
                    ));
                }
                // A command of a story, laid out, read only once the
                // human opened it.
                "command" => {
                    return Some(activity::command(
                        &serde_json::from_str::<activity::OfCommand>(&asked).ok()?,
                    ));
                }
                _ => return None,
            }
        }
        let mut told = activity::told(&self.root);
        if let Some(open) = open {
            told["thread"] = activity::thread(&open);
        }
        Some(told)
    }

    /// What the card of a trigger asks of `cox://api/triggers/<rest>`, if
    /// it is the card's: to save it, to talk it through or try it, and how
    /// those stand, of a folder the tab shows only.
    fn card(&self, rest: &str) -> Option<serde_json::Value> {
        use coxd::trigger::{card, discuss};
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
                let made = save.make(
                    &coxd::settings::workflow(),
                    coxd::storage::path(coxd::storage::Kept::Triggers).as_deref(),
                );
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
                discuss::try_on(tried, coxd::storage::path(coxd::storage::Kept::LastTurns))
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
    /// (see [`coxd::trigger::told`]).
    fn triggers(&self) -> serde_json::Value {
        use coxd::storage::{Kept, path};
        let (arranged, runs) = (path(Kept::Triggers), path(Kept::TriggerRuns));
        coxd::trigger::told(
            &self.trigger_folders(),
            &coxd::trigger::Kept {
                workflow: &coxd::settings::workflow(),
                home: &home(),
                arranged: arranged.as_deref(),
                runs: runs.as_deref(),
                schedules: &coxd::schedule::Ran::read(path(Kept::Schedules)),
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
        .unwrap_or_else(|| coxd::settings::home_for_chats(coxd::settings::Saved::read().home))
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

/// Whether asking for `url` has the stage do something, not only tell:
/// a new ask of the Triggers tab's does, until it is named here as one
/// that only tells.
fn acts(url: &str) -> bool {
    let Some(("api", path)) = split(url) else {
        return false;
    };
    path.starts_with("activity/dismiss/")
        || path.starts_with("map/again/")
        || path
            .strip_prefix("triggers/")
            .is_some_and(|ask| !ask.starts_with("discussion/") && !ask.starts_with("tried/"))
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
    use coxd::testing::TempDir;

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
        let root = TempDir::new();
        let plugins = Plugins::new(root.to_path_buf());

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
    fn each_built_in_page_draws_with_what_the_theme_or_its_plugin_names() {
        let theme = coxd::theme::css();
        for (name, files) in BUILT_IN {
            // Its own names, in its style or set by its script.
            let own: String = files
                .iter()
                .map(|(_, bytes)| String::from_utf8_lossy(bytes))
                .collect();
            for (file, bytes) in *files {
                let styled = Path::new(file)
                    .extension()
                    .is_some_and(|extension| extension == "html" || extension == "css");
                if !styled {
                    continue;
                }
                let text = String::from_utf8_lossy(bytes);
                for used in text.split("var(--").skip(1) {
                    let name_of = used
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                        .next()
                        .unwrap_or_default();
                    let defined = format!("--{name_of}:");
                    assert!(
                        theme.contains(&defined) || own.contains(&defined),
                        "{name}/{file} draws with --{name_of}, which neither the theme nor its plugin names"
                    );
                }
            }
        }
    }

    #[test]
    fn only_a_plugin_has_the_triggers_tab_do_what_it_asks() {
        let root = TempDir::new();
        let plugins = Plugins::new(root.to_path_buf());
        let serve = plugins.serve();
        let body = |url: &str, asker| serve(url, asker).map(|resource| resource.body);

        assert_eq!(
            body("cox://api/triggers/end/card", Asker::Plugin),
            Some(b"{}".to_vec())
        );
        assert_eq!(body("cox://api/triggers/end/card", Asker::Anyone), None);
        assert_eq!(body("cox://api/activity/dismiss/[]", Asker::Anyone), None);
        assert!(body("cox://api/triggers/tried/card", Asker::Anyone).is_some());
        assert!(body("cox://api/triggers/discussion/card", Asker::Anyone).is_some());
        assert!(body("cox://map/", Asker::Anyone).is_some());
    }

    #[test]
    fn a_chats_story_is_told_alone_for_the_chat_asked() {
        let root = TempDir::new();
        coxd::storage::keep_on_this_thread(Some(root.join("data")));
        let plugins = Plugins::new(root.to_path_buf());
        let serve = plugins.serve();
        let body = |url: &str| serve(url, Asker::Plugin).map(|resource| resource.body);

        // No chat kept by that number: nothing to tell, and not what goes
        // on in the folder either.
        assert_eq!(
            body(r#"cox://api/activity/timeline/{"chat":4242}"#),
            Some(b"null".to_vec())
        );
        assert_eq!(body("cox://api/activity/timeline/not-a-chat"), None);
        coxd::storage::keep_on_this_thread(None);
    }

    #[test]
    fn a_command_of_a_story_is_told_laid_out_once_opened() {
        use cox_core::entry::{Content, Entry, ToolCallResult};
        let root = TempDir::new();
        coxd::storage::keep_on_this_thread(Some(root.join("data")));
        let store =
            cox_core::store::Store::open(&coxd::storage::path(coxd::storage::Kept::Chats).unwrap())
                .unwrap();
        let project = store.project(&root, "app").unwrap();
        let chat = store.add_chat(Some(project), "Chat 1").unwrap();
        store
            .keep_entry(
                chat,
                0,
                &Entry::ToolCall {
                    id: "1".to_owned(),
                    title: "cargo test".to_owned(),
                    command: true,
                    about: None,
                    content: vec![Content::Text("test result: ok".to_owned())],
                    result: Some(ToolCallResult::Completed),
                },
            )
            .unwrap();
        let plugins = Plugins::new(root.to_path_buf());
        let serve = plugins.serve();
        let body = |url: &str| {
            serve(url, Asker::Plugin).map(|resource| {
                serde_json::from_slice::<serde_json::Value>(&resource.body).unwrap()
            })
        };

        let told = body(&format!(
            r#"cox://api/activity/command/{{"chat":{chat},"number":0}}"#
        ))
        .unwrap();

        assert_eq!(told["line"], "cargo test");
        assert_eq!(told["printed"], "test result: ok");
        assert_eq!(
            body(&format!(
                r#"cox://api/activity/command/{{"chat":{chat},"number":1}}"#
            )),
            Some(serde_json::Value::Null)
        );
        assert_eq!(body("cox://api/activity/command/not-a-command"), None);
        coxd::storage::keep_on_this_thread(None);
    }

    #[test]
    fn a_plugin_has_the_theme_beside_its_page_unless_it_has_its_own() {
        let root = TempDir::new();
        let folder = root.join(PROJECT_PLUGINS).join("tour");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("index.html"), "<h1>Tour</h1>").unwrap();
        let plugins = Plugins::new(root.to_path_buf());

        let theme = coxd::theme::css().into_bytes();
        for beside in ["cox://map/theme.css", "cox://tour/theme.css"] {
            let resource = plugins.resource(beside).unwrap();
            assert_eq!(resource.mime, "text/css", "{beside}");
            assert_eq!(resource.body, theme, "{beside}");
        }
        std::fs::write(folder.join("theme.css"), "body{}").unwrap();
        assert_eq!(
            plugins.resource("cox://tour/theme.css").unwrap().body,
            b"body{}"
        );
    }

    #[test]
    fn the_map_page_is_its_own_files_alone_as_the_phone_serves_it() {
        let (_, files) = BUILT_IN.iter().find(|(name, _)| *name == "map").unwrap();
        for (name, body) in *files {
            let text = std::str::from_utf8(body).unwrap();
            // Under `default-src 'self'`, nothing inline runs or styles.
            assert!(!text.contains("<style"), "{name} has a style element");
            assert!(!text.contains("style=\""), "{name} has a style attribute");
            for handler in ["onclick=", "onload=", "onerror=", "href=\"javascript:"] {
                assert!(!text.contains(handler), "{name} has {handler}");
            }
            if mime(name) == "text/html" {
                assert!(!text.contains("<script>"), "{name} has an inline script");
                for script in text.split("<script").skip(1) {
                    assert!(script.contains("src=\"./"), "{name}: {script}");
                }
            }
        }
    }

    #[test]
    fn a_map_is_asked_of_coxd_as_named() {
        let root = TempDir::new();
        let plugins = Plugins::new(root.to_path_buf());
        // In the window's place: answers what it was asked.
        let mut asks = map::listen();
        std::thread::spawn(move || {
            while let Some(ask) = asks.blocking_recv() {
                let told = serde_json::json!({
                    "folder": ask.folder,
                    "name": ask.name,
                    "again": ask.again,
                });
                let _ = ask.answer.send(told.to_string());
            }
        });
        let serve = plugins.serve();
        let asked = |url: &str, asker| {
            serve(url, asker).map(|resource| {
                assert_eq!(resource.mime, "application/json");
                serde_json::from_slice::<serde_json::Value>(&resource.body).unwrap()
            })
        };
        let json = asked("cox://api/map", Asker::Anyone).unwrap();
        assert_eq!(json["folder"], root.display().to_string());
        assert_eq!(json["name"], serde_json::Value::Null);
        assert_eq!(json["again"], false);
        let json = asked("cox://api/map/use%20it", Asker::Anyone).unwrap();
        assert_eq!(json["name"], "use it");
        assert_eq!(json["again"], false);
        // A repository's map, named after it, has its `/` encoded.
        let json = asked("cox://api/map/app%2Fuse", Asker::Anyone).unwrap();
        assert_eq!(json["name"], "app/use");
        // Making a failed index again is a plugin's to ask.
        assert_eq!(asked("cox://api/map/again/use", Asker::Anyone), None);
        let json = asked("cox://api/map/again/use", Asker::Plugin).unwrap();
        assert_eq!(json["name"], "use");
        assert_eq!(json["again"], true);
        assert_eq!(asked("cox://api/map/a/b", Asker::Plugin), None);
    }

    #[test]
    fn the_schedules_of_the_folder_are_told_among_its_triggers() {
        let root = TempDir::new();
        std::fs::create_dir_all(root.join(coxd::trigger::PROJECT_TRIGGERS)).unwrap();
        std::fs::write(
            root.join(coxd::trigger::PROJECT_TRIGGERS).join("triage.md"),
            "---\non: {schedule: {cron: \"0 */6 * * *\"}}\n---\n\nLook at what changed.\n",
        )
        .unwrap();
        let plugins = Plugins::new(root.to_path_buf());

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
        let root = TempDir::new();
        std::fs::create_dir_all(root.join("raw/sketches")).unwrap();
        std::fs::write(
            root.join("raw/sketches/2026-11-10-mock.md"),
            "---\nsource: \"chat 3\"\nkept: 2026-11-10\n---\n\n# Mock\n\nWhat it tried.\n",
        )
        .unwrap();
        let plugins = Plugins::new(root.to_path_buf());

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
        let root = TempDir::new();
        std::fs::create_dir_all(root.join("wiki")).unwrap();
        std::fs::write(root.join("wiki/people.md"), "# People\n\nWho to ask.\n").unwrap();
        let plugins = Plugins::new(root.to_path_buf());

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
    fn the_pages_of_a_plugin_no_longer_built_in_are_gone() {
        let root = TempDir::new();
        let plugins = Plugins::new(root.to_path_buf());

        assert!(gone("cox://commands/"));
        assert_eq!(plugins.resource("cox://commands/"), None);
        assert_eq!(plugins.resource("cox://api/commands"), None);
        assert!(!gone("cox://activity/"));
    }

    #[test]
    fn the_projects_plugins_are_listed_and_served_within_their_folders() {
        let root = TempDir::new();
        let folder = root.join(PROJECT_PLUGINS).join("tour");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("index.html"), "<h1>Tour</h1>").unwrap();
        std::fs::write(folder.join("plugin.json"), r#"{"title": "A tour"}"#).unwrap();
        std::fs::write(root.join("secret.txt"), "no").unwrap();
        let plugins = Plugins::new(root.to_path_buf());

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
