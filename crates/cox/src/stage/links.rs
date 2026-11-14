//! What opens on a tab, and which: a link the human clicked, on the tab
//! showing its page already or a new one; a file, as Markdown or as it is
//! written, on the tab showing it already or a new one, from the link's
//! own folder, as one does from what changed in the project's files, and
//! from the map of the project; and a file chosen on the tree of the
//! project's files, on the tab of a file shown, in place of what it
//! showed, or on a tab of its own.

use std::path::{Path, PathBuf};

use cox_core::tabs::Lines;
use gpui::{Context, EventEmitter, Focusable as _, Subscription, Window, prelude::*};

use crate::stage::Stage;
use crate::stage::changes::{self, Changes, ShowTurn};
use crate::stage::code::Code;
use crate::stage::document::Document;
use crate::stage::flow::{self, Flow};
use crate::stage::tab::{ShowsFile, Tab, TabContent};
use crate::stage::tree::{self, Chosen, Unchosen};
use crate::ui::LinkClicked;
use crate::ui::markdown::FileLink;
use coxd::home::{expanded_in, home};

impl EventEmitter<ShowTurn> for Stage {}

impl Stage {
    /// Shows the file at `path`, which says `text`, as a `T` shows it, at
    /// `lines` if any: on the tab that shows it already, or a new one.
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
            if self.in_sight() {
                self.tabs.show(index);
            }
            self.changed(cx);
        } else {
            self.push_file::<T>(path, text, lines, true, window, cx);
        }
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
        let (content, changes, opens) = self.file_tab::<T>(path, text, lines, window, cx);
        self.push(content, changes, Some(opens), show, cx);
    }

    /// What a tab shows of the file at `path`, which says `text`, as a `T`
    /// shows it, at `lines` if any, with what redraws the tab as it
    /// changes and opens the links clicked in it.
    fn file_tab<T: ShowsFile>(
        &self,
        path: PathBuf,
        text: &str,
        lines: Option<Lines>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (TabContent, Subscription, Subscription) {
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
        (T::content(shown), changes, opens)
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
        let path = linked(link.path, from, home().as_deref());
        if let Err(error) = self.open_path(&path, link.lines, window, cx) {
            tracing::warn!("cannot open {}: {error}", path.display());
        }
    }

    /// Opens the file at `path`, at `lines` if any, with the keyboard in
    /// its tab: as Markdown if it is a Markdown file and no lines are
    /// given, and as it is written otherwise. Opens nothing if the file
    /// cannot be read.
    pub(super) fn open_path(
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

    /// Opens the tab of a file not chosen yet, with the tree of the
    /// project's files beside it, unfolded, to choose one there.
    pub fn new_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        tree::fold(false, cx);
        self.push_files(true, cx);
        self.focus_shown(window, cx);
    }

    /// Puts the tab of a file not chosen yet after the others, shown if
    /// `show` is.
    pub(super) fn push_files(&mut self, show: bool, cx: &mut Context<Self>) {
        let unchosen = cx.new(Unchosen::new);
        let changes = cx.observe(&unchosen, |stage, _, cx| stage.changed(cx));
        self.push(TabContent::Files(unchosen), changes, None, show, cx);
    }

    /// Shows the file the human chose on the tree: on the tab of a file
    /// shown, unless they asked for a tab of its own, or there is none.
    pub(super) fn open_chosen(
        &mut self,
        chosen: &Chosen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from_files = self.files.focus_handle(cx).contains_focused(window, cx);
        let shown = self
            .tabs
            .shown_index()
            .filter(|&index| self.tabs[index].content.is_file());
        match shown {
            Some(index) if !chosen.apart => self.show_in(index, &chosen.path, window, cx),
            // A file that cannot be read gets a tab all the same, which
            // says why.
            _ => {
                if self.open_path(&chosen.path, None, window, cx).is_err() {
                    self.show_file::<Code>(chosen.path.clone(), "", None, window, cx);
                    self.focus_shown(window, cx);
                }
            }
        }
        // As NERDTree's `o` does, the keyboard goes on to the file chosen.
        if from_files {
            self.focus_shown(window, cx);
        }
    }

    /// Shows the file at `path` on the tab of a file at `index`, in place
    /// of what it showed: as it is written on a tab of code, which back
    /// goes from to what it showed before, and on a tab of its own kind
    /// otherwise, as Markdown for a Markdown file.
    fn show_in(&mut self, index: usize, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        match (FileTab::of(&path, None), &self.tabs[index].content) {
            (FileTab::Code(_), TabContent::Code(code)) => {
                code.update(cx, |code, cx| code.go_to_file(&path, cx));
            }
            (kind, _) => {
                // A file that cannot be read is shown all the same, which
                // its tab says.
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                match kind {
                    FileTab::Document => self.put_file::<Document>(index, path, &text, window, cx),
                    FileTab::Code(_) => self.put_file::<Code>(index, path, &text, window, cx),
                }
            }
        }
        self.changed(cx);
        self.focus_shown(window, cx);
    }

    /// Puts a tab showing the file at `path`, which says `text`, as a `T`
    /// shows it, in place of the tab at `index`, of the same chat.
    fn put_file<T: ShowsFile>(
        &mut self,
        index: usize,
        path: PathBuf,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (content, changes, opens) = self.file_tab::<T>(path, text, None, window, cx);
        self.tabs[index] = Tab {
            content,
            owner: self.tabs[index].owner,
            _changes: changes,
            _opens: Some(opens),
        };
    }

    /// Shows what changed in the files of the repositories of `folder`, an
    /// absolute path (see [`crate::repositories::of`]): on its tab,
    /// or a new one, with the keyboard in it.
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

    /// Shows what the commit `id` changed, of the repository `name` of
    /// `folder` (see [`crate::repositories::of`]), on the tab of what
    /// changed in `folder`, or a new one.
    pub fn show_commit(
        &mut self,
        folder: &Path,
        name: Option<String>,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_changes(folder.to_owned(), window, cx);
        let shown = self.tabs.iter().find_map(|tab| match &tab.content {
            TabContent::Changes(changes) if changes.read(cx).folder() == folder => {
                Some(changes.clone())
            }
            _ => None,
        });
        if let Some(changes) = shown {
            changes.update(cx, |changes, cx| changes.show_commit(name, id, cx));
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
        // The turn a commit was made in shows in its chat, which the window
        // shows.
        let turns = cx.subscribe(&shown, |_, _, &ShowTurn { chat, entry }, cx| {
            cx.emit(ShowTurn { chat, entry });
        });
        let opens = Subscription::join(opens, turns);
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

    /// Shows the map of the project named `map`, or its first unless
    /// named, open `at`, a tier and a box in it by their ids, or at its
    /// top, as the agent asks: on the Map page's tab, or a new one, with
    /// the keyboard in it.
    pub(super) fn show_map_at(
        &mut self,
        map: Option<&str>,
        at: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_plugin("map", &map_fragment(map, at), window, cx);
    }
}

/// The address of the Activity page, which a stage shows first, unless
/// `--stage` says another.
pub const ACTIVITY_URL: &str = "cox://activity/";

/// The hash of the Map page open at the map `map`, or the project's
/// first, `at`, a tier and a box in it by their ids: `#cox/stage`, or
/// `#deploy:cox/stage` for the map `deploy`, and `#` alone at the top of
/// the first.
fn map_fragment(map: Option<&str>, at: &[String]) -> String {
    let at = at.join("/");
    match map {
        Some(map) => format!("#{}:{at}", percent_encoded(map)),
        None => format!("#{at}"),
    }
}

/// `name` as a hash has it, its characters past ASCII's letters, digits
/// and `-_.` written as `%` and their bytes, as the page decodes them.
fn percent_encoded(name: &str) -> String {
    name.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
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

/// The file a link to `path` is to, from `from`, where the link is: `~`
/// stands for `home`, when there is one.
fn linked(path: &str, from: &Path, home: Option<&Path>) -> PathBuf {
    from.join(expanded_in(path, home).unwrap_or_else(|| PathBuf::from(path)))
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

    use super::{FileTab, linked, map_fragment};

    #[test]
    fn the_map_page_is_opened_at_a_map_by_its_name() {
        let at = ["cox".to_owned(), "stage".to_owned()];
        assert_eq!(map_fragment(None, &[]), "#");
        assert_eq!(map_fragment(None, &at), "#cox/stage");
        assert_eq!(map_fragment(Some("deploy"), &at), "#deploy:cox/stage");
        assert_eq!(map_fragment(Some("my map"), &[]), "#my%20map:");
    }

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
            linked("~", from, Some(Path::new("/home/me"))),
            Path::new("/home/me")
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
