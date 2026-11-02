//! What opens on a tab, and which: a link the human clicked, on the tab
//! showing its page already or a new one; a file, as Markdown or as it is
//! written, on the tab showing it already or a new one, from the link's
//! own folder; and the tree of the project's files, from which a click
//! opens a file the same way, as one does from what changed in them, and
//! from the map of the project.

use std::path::{Path, PathBuf};

use cox_core::tabs::Lines;
use gpui::{Context, Window, prelude::*};

use crate::stage::Stage;
use crate::stage::changes::{self, Changes};
use crate::stage::code::Code;
use crate::stage::document::Document;
use crate::stage::flow::{self, Flow};
use crate::stage::tab::{ShowsFile, TabContent};
use crate::stage::tree::{Chosen, Tree};
use crate::ui::LinkClicked;
use crate::ui::markdown::FileLink;

impl Stage {
    /// Shows the file at `path`, which says `text`, as a `T` shows it, at
    /// `lines` if any: on the tab that shows it already, or a new one. The
    /// stage opens if it was closed.
    pub(super) fn show_file<T: ShowsFile>(
        &mut self,
        path: PathBuf,
        text: &str,
        lines: Option<Lines>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let showing = self
            .tabs
            .iter()
            .position(|tab| T::of(&tab.content).is_some_and(|shown| shown.read(cx).path() == path));
        if let Some(index) = showing {
            if let Some(shown) = T::of(&self.tabs[index].content) {
                shown.update(cx, |shown, cx| shown.show(text, lines, cx));
            }
            self.tabs.show(index);
            self.changed(cx);
        } else {
            self.push_file::<T>(path, text, lines, true, window, cx);
        }
        self.show_panel(cx);
    }

    /// Puts a tab showing the file at `path`, which says `text`, as a `T`
    /// shows it, at `lines` if any, after the others, shown if `show` is.
    pub(super) fn push_file<T: ShowsFile>(
        &mut self,
        path: PathBuf,
        text: &str,
        lines: Option<Lines>,
        show: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = cx.new(|cx| T::open(&self.cwd, path, text, lines, cx));
        let changes = cx.observe(&shown, |stage, _, cx| stage.changed(cx));
        let opens = cx.subscribe_in(
            &shown,
            window,
            |stage, shown, LinkClicked(url), window, cx| {
                // A link to a file is from where the file is.
                let path = shown.read(cx).path();
                let from = path.parent().unwrap_or(path).to_owned();
                stage.open_from(url, &from, window, cx);
            },
        );
        self.push(T::content(shown), changes, Some(opens), show, cx);
    }

    /// Opens `url`, a link the human clicked: on the tab that shows it
    /// already, or a new one. A link to a file is from where the chat
    /// works.
    pub fn open(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.cwd.clone();
        self.open_from(url, &cwd, window, cx);
    }

    /// Opens `url`, a link the human clicked in what is in `from`, which a
    /// link to a file is from.
    fn open_from(&mut self, url: &str, from: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(call) = flow::linked(url) {
            self.show_flow(call.to_owned(), window, cx);
            return;
        }
        if let Some(link) = crate::ui::markdown::file_link(url) {
            self.open_file(link, from, window, cx);
            return;
        }
        let showing = self.tabs.iter().position(|tab| {
            tab.content
                .page()
                .is_some_and(|page| page.read(cx).navigation().url == url)
        });
        if let Some(index) = showing {
            self.show(index, window, cx);
        } else {
            self.open_tab(url, true, window, cx);
            self.focus_shown(window, cx);
        }
    }

    /// Opens the file `link` links to, from `from`, with the keyboard in
    /// its tab.
    fn open_file(
        &mut self,
        link: FileLink<'_>,
        from: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let path = linked(link.path, from, home.as_deref());
        if let Err(error) = self.open_path(&path, link.lines, window, cx) {
            tracing::warn!("cannot open {}: {error}", path.display());
        }
    }

    /// Opens the file at `path`, at `lines` if any, with the keyboard in
    /// its tab: as Markdown if it is a Markdown file and no lines are
    /// given, and as it is written otherwise. Opens nothing if the file
    /// cannot be read.
    fn open_path(
        &mut self,
        path: &Path,
        lines: Option<Lines>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> std::io::Result<()> {
        let path = std::fs::canonicalize(path)?;
        let text = std::fs::read_to_string(&path)?;
        match FileTab::of(&path, lines) {
            FileTab::Document => self.show_file::<Document>(path, &text, None, window, cx),
            FileTab::Code(lines) => self.show_file::<Code>(path, &text, lines, window, cx),
        }
        self.focus_shown(window, cx);
        Ok(())
    }

    /// Shows the tree of the project's files: on its tab, or a new one,
    /// with the keyboard in it.
    pub(super) fn show_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let showing = self
            .tabs
            .iter()
            .position(|tab| matches!(tab.content, TabContent::Tree(_)));
        if let Some(index) = showing {
            self.show(index, window, cx);
        } else {
            self.push_tree(Vec::new(), true, window, cx);
            self.focus_shown(window, cx);
        }
    }

    /// Puts a tab showing the tree of the project's files, with the
    /// folders `open` open, after the others, shown if `show` is.
    pub(super) fn push_tree(
        &mut self,
        open: Vec<PathBuf>,
        show: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tree = cx.new(|cx| Tree::new(self.cwd.clone(), open, cx));
        let changes = cx.observe(&tree, |stage, _, cx| stage.changed(cx));
        let opens = cx.subscribe_in(&tree, window, |stage, _, Chosen(path), window, cx| {
            // A file that cannot be read gets a tab all the same, which
            // says why.
            if stage.open_path(path, None, window, cx).is_err() {
                stage.show_file::<Code>(path.clone(), "", None, window, cx);
                stage.focus_shown(window, cx);
            }
        });
        self.push(TabContent::Tree(tree), changes, Some(opens), show, cx);
    }

    /// Shows what changed in the files of the repository of `folder`, an
    /// absolute path: on its tab, or a new one, with the keyboard in it.
    pub(super) fn show_changes(
        &mut self,
        folder: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let showing = self.tabs.iter().position(|tab| match &tab.content {
            TabContent::Changes(changes) => changes.read(cx).folder() == folder,
            _ => false,
        });
        if let Some(index) = showing {
            self.show(index, window, cx);
        } else {
            self.push_changes(folder, true, window, cx);
            self.focus_shown(window, cx);
        }
    }

    /// Puts a tab showing what changed in the files of the repository of
    /// `folder` after the others, shown if `show` is.
    pub(super) fn push_changes(
        &mut self,
        folder: PathBuf,
        show: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = self.cwd.clone();
        let shown = cx.new(|cx| Changes::new(folder, &cwd, cx));
        let changes = cx.observe(&shown, |stage, _, cx| stage.changed(cx));
        let opens = cx.subscribe_in(
            &shown,
            window,
            |stage, _, changes::Chosen { path, line }, window, cx| {
                let lines = line.map(|line| Lines {
                    first: line,
                    last: line,
                });
                if stage.open_path(path, lines, window, cx).is_err() {
                    stage.show_file::<Code>(path.clone(), "", lines, window, cx);
                    stage.focus_shown(window, cx);
                }
            },
        );
        self.push(TabContent::Changes(shown), changes, Some(opens), show, cx);
    }

    /// Shows the page of the plugin `name` (see [`crate::stage::plugins`]),
    /// at `fragment`, `#` and what follows, when one is given: on the tab
    /// that shows the page, wherever it has gone, which goes to the
    /// fragment, or a new one, with the keyboard in it.
    pub(super) fn show_plugin(
        &mut self,
        name: &str,
        fragment: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prefix = format!("cox://{name}/");
        let url = format!("{prefix}{fragment}");
        let showing = self.tabs.iter().position(|tab| {
            tab.content
                .page()
                .is_some_and(|page| page.read(cx).navigation().url.starts_with(&prefix))
        });
        if let Some(index) = showing {
            if !fragment.is_empty()
                && let Some(page) = self.tabs[index].content.page().cloned()
            {
                page.update(cx, |page, cx| page.open(&url, window, cx));
            }
            self.show(index, window, cx);
        } else {
            self.open_tab(&url, true, window, cx);
            self.focus_shown(window, cx);
        }
    }

    /// Shows the map of the project open `at`, a tier and a box in it
    /// by their ids, or at its top, as the agent asks: on the Map page's
    /// tab, or a new one, with the keyboard in it.
    pub(super) fn show_map_at(
        &mut self,
        at: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_plugin("map", &map_fragment(at), window, cx);
    }
}

/// The address of the Parts page.
pub(super) const PARTS_URL: &str = "cox://parts/";

/// The address of the Map page, at its top.
pub(super) const MAP_URL: &str = "cox://map/";

/// The hash of the Map page open `at`, a tier and a box in it by their
/// ids, as `#cox/stage`: `#` alone at the top.
fn map_fragment(at: &[String]) -> String {
    format!("#{}", at.join("/"))
}

impl Stage {
    /// Shows the flow of the tool call `call` through Cox (see
    /// [`cox_core::tabs::KeptTab::Flow`]): on its tab, or a new one, with
    /// the keyboard in it.
    pub(super) fn show_flow(&mut self, call: String, window: &mut Window, cx: &mut Context<Self>) {
        let showing = self
            .tabs
            .iter()
            .position(|tab| matches!(tab.content, TabContent::Flow(_)));
        if let Some(index) = showing {
            if let TabContent::Flow(flow) = &self.tabs[index].content {
                flow.update(cx, |flow, cx| flow.open(call, cx));
            }
            self.show(index, window, cx);
        } else {
            self.push_flow(call, true, window, cx);
            self.focus_shown(window, cx);
        }
    }

    /// Puts a tab showing the flow of the tool call `call` after the
    /// others, shown if `show` is. A click on a step opens the file it is
    /// taken in, at its line.
    pub(super) fn push_flow(
        &mut self,
        call: String,
        show: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let logs = crate::logs::directory().map(Path::to_owned);
        let shown = cx.new(|cx| Flow::new(self.cwd.clone(), logs, call, cx));
        let changes = cx.observe(&shown, |stage, _, cx| stage.changed(cx));
        let opens = cx.subscribe_in(
            &shown,
            window,
            |stage, _, opened: &flow::Opened, window, cx| {
                let line = usize::try_from(opened.line).unwrap_or(1).max(1);
                let lines = Some(Lines {
                    first: line,
                    last: line,
                });
                if stage.open_path(&opened.path, lines, window, cx).is_err() {
                    stage.show_file::<Code>(opened.path.clone(), "", lines, window, cx);
                    stage.focus_shown(window, cx);
                }
            },
        );
        self.push(TabContent::Flow(shown), changes, Some(opens), show, cx);
    }
}

/// The file a link to `path` is to, from `from`, where the link is: `~/`
/// stands for `home`, when there is one.
fn linked(path: &str, from: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => from.join(path),
    }
}

/// Which tab a file opens on, from a link to it or from the tree.
#[derive(Debug, PartialEq, Eq)]
enum FileTab {
    /// A tab showing it as Markdown, drawn.
    Document,
    /// A tab showing it as it is written, at these lines if any.
    Code(Option<Lines>),
}

impl FileTab {
    /// Which tab the file at `path` opens on, at `lines` if the link names
    /// any: as Markdown if it is a Markdown file and no lines are named,
    /// and as it is written otherwise.
    fn of(path: &Path, lines: Option<Lines>) -> Self {
        let is_markdown = path
            .extension()
            .is_some_and(|extension| extension == "md" || extension == "markdown");
        if is_markdown && lines.is_none() {
            Self::Document
        } else {
            Self::Code(lines)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use cox_core::tabs::Lines;

    use super::{FileTab, linked};

    #[test]
    fn a_link_to_a_file_is_from_where_the_link_is() {
        let from = Path::new("/project/docs");
        let home = Some(Path::new("/home/me"));
        assert_eq!(
            linked("notes.md", from, home),
            Path::new("/project/docs/notes.md")
        );
        assert_eq!(
            linked("../src/main.rs", from, home),
            Path::new("/project/docs/../src/main.rs")
        );
        assert_eq!(linked("/etc/hosts", from, home), Path::new("/etc/hosts"));
    }

    #[test]
    fn a_tilde_stands_for_home_when_there_is_one() {
        let from = Path::new("/project");
        assert_eq!(
            linked("~/notes.md", from, Some(Path::new("/home/me"))),
            Path::new("/home/me/notes.md")
        );
        assert_eq!(
            linked("~/notes.md", from, None),
            Path::new("/project/~/notes.md")
        );
    }

    #[test]
    fn a_markdown_file_opens_drawn_unless_lines_are_named() {
        let lines = Some(Lines { first: 3, last: 4 });
        assert_eq!(
            FileTab::of(Path::new("/a/README.md"), None),
            FileTab::Document
        );
        assert_eq!(
            FileTab::of(Path::new("/a/notes.markdown"), None),
            FileTab::Document
        );
        assert_eq!(
            FileTab::of(Path::new("/a/README.md"), lines),
            FileTab::Code(lines)
        );
        assert_eq!(
            FileTab::of(Path::new("/a/main.rs"), None),
            FileTab::Code(None)
        );
        assert_eq!(
            FileTab::of(Path::new("/a/main.rs"), lines),
            FileTab::Code(lines)
        );
    }
}
