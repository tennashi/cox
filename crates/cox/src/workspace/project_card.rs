//! The card where the human edits a project, over the window, which
//! "Edit project…" in the menu of the right button on the project, on
//! the rail, opens: the project's name, its folder's, which Enter
//! changes, as the card does as it closes, renaming the folder; and the
//! project's directories, in `directories` in its folder, which Enter
//! makes, or clones a repository into, each of which can become a Git
//! repository and be linked to a remote. A project whose folder is not
//! there says so. Escape, the ×, or a click beside the card closes it.

mod directory;

use directory::Directory;

use std::path::{Path, PathBuf};

use cox_core::store::ProjectId;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, SharedString, Subscription, Window, actions, div, prelude::*, px, rgb,
};

use crate::ui::button::{INPUT_HEIGHT, Kind, button};
use crate::ui::card;
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{LineInput, Submitted};
use crate::ui::paths::shortened;
use crate::ui::theme::{
    ACCENT, ASKING, BORDER, DIM, FAILED, FAINT, FOREGROUND, HOVER, LINK, RAISED,
};
use crate::ui::tip;

actions!(
    cox,
    [
        /// Closes the card of the project.
        CloseProjectCard,
    ]
);

/// The key context of the card.
const KEY_CONTEXT: &str = "ProjectCard";

/// How wide the card is.
const WIDTH: f32 = 560.;

/// How tall a row of the directories is.
const ROW_HEIGHT: f32 = 34.;

/// Binds the key that closes the card.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        CloseProjectCard,
        Some(KEY_CONTEXT),
    )]);
}

/// The human renamed the project, and so its folder.
pub struct Rename(pub String);

/// The human closed the card.
pub struct Dismissed;

/// What the window knows of the project, for the card to say what can be
/// done with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    /// The folder its chats work in.
    pub folder: PathBuf,
    /// Whether its folder is not there.
    pub missing: bool,
    /// Whether one of its chats is at work, while which it is not
    /// renamed.
    pub at_work: bool,
}

/// The box of `input`, as the card draws its inputs, named `selector`.
fn input_box(selector: &'static str, input: &Entity<LineInput>) -> gpui::Div {
    div()
        .debug_selector(move || selector.to_owned())
        .h(px(INPUT_HEIGHT))
        .px_2()
        .flex()
        .items_center()
        .border_1()
        .border_color(rgb(LINK))
        .rounded(px(10.))
        .text_size(px(13.))
        .text_color(rgb(FOREGROUND))
        .child(input.clone())
}

/// What the line under the input of a directory says.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Making {
    /// What the input does.
    Hint,
    /// A directory is made, cloned or linked, as this says.
    Going(String),
    /// A directory was made, cloned or linked, as this says.
    Done(String),
    /// It was not, for this reason.
    Failed(String),
}

/// The directory whose remote the human gives, and the input of it.
struct Linking {
    name: String,
    input: Entity<LineInput>,
    _submitted: Subscription,
}

pub struct ProjectCard {
    project: ProjectId,
    standing: Standing,
    /// The input of the project's name.
    name: Entity<LineInput>,
    /// The project's directories, as they stand.
    directories: Vec<Directory>,
    /// The input of a directory's name, or a Git repository's URL.
    directory: Entity<LineInput>,
    /// What the line under it says.
    making: Making,
    /// The directory whose remote the human gives, if they do.
    linking: Option<Linking>,
    _submitted: [Subscription; 3],
}

impl ProjectCard {
    /// The card of the project kept as `project`, which stands as
    /// `standing` says.
    pub fn new(
        project: ProjectId,
        standing: Standing,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_placeholder("The project's name", cx);
            input.set_text(&named(&standing.folder), cx);
            input
        });
        let directory = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_placeholder("A directory's name, or a Git repository's URL", cx);
            input
        });
        let named = cx.subscribe_in(&name, window, |card, _, Submitted, _, cx| {
            card.submit_name(cx);
        });
        let made = cx.subscribe_in(&directory, window, |card, _, Submitted, _, cx| {
            card.make_directory(cx);
        });
        // What the card says of the name follows it as it is typed.
        let typed = cx.observe(&name, |_, _, cx| cx.notify());
        Self {
            project,
            directories: directory::listed(&standing.folder),
            standing,
            name,
            directory,
            making: Making::Hint,
            linking: None,
            _submitted: [named, made, typed],
        }
    }

    /// The project the card edits.
    #[must_use]
    pub fn project(&self) -> ProjectId {
        self.project
    }

    /// The input of the project's name.
    #[must_use]
    pub fn name_input(&self) -> &Entity<LineInput> {
        &self.name
    }

    /// The input of a directory's name, or a Git repository's URL.
    #[must_use]
    pub fn directory_input(&self) -> &Entity<LineInput> {
        &self.directory
    }

    /// The input of the remote of the directory being linked, if one is.
    #[must_use]
    pub fn link_input(&self) -> Option<&Entity<LineInput>> {
        self.linking.as_ref().map(|linking| &linking.input)
    }

    /// Makes the directory whose name the input holds, or clones the
    /// repository whose URL it holds into one named after it, in the
    /// background, if it can.
    fn make_directory(&mut self, cx: &mut Context<Self>) {
        let typed = self.directory.read(cx).text().trim().to_owned();
        let url = directory::is_url(&typed);
        let name = if url {
            match directory::name_of(&typed) {
                Some(name) => name,
                None => {
                    return self.say(
                        Making::Failed(
                            "Cox cannot tell the repository's name from that.".to_owned(),
                        ),
                        cx,
                    );
                }
            }
        } else {
            if let Some(why) = directory::why_not_named(&typed) {
                return self.say(Making::Failed(why.to_owned()), cx);
            }
            typed.clone()
        };
        let Some(into) = self.free(&name, cx) else {
            return;
        };
        if url {
            self.go(
                format!("Cloning into {name}…"),
                move || {
                    directory::clone(&typed, &into)?;
                    Ok(format!("Cloned into {name}."))
                },
                cx,
            );
        } else {
            self.go(
                format!("Making {name}…"),
                move || {
                    directory::make(&into)?;
                    Ok(format!("Made {name}."))
                },
                cx,
            );
        }
    }

    /// The folder of the directory `name`, if the project can have one
    /// there; or else says why not.
    fn free(&mut self, name: &str, cx: &mut Context<Self>) -> Option<PathBuf> {
        if self.standing.missing {
            self.say(
                Making::Failed("The project's folder is not there.".to_owned()),
                cx,
            );
            return None;
        }
        let into = directory::folder_of(&self.standing.folder, name);
        if into.exists() {
            self.say(
                Making::Failed(format!("The project has a directory {name} already.")),
                cx,
            );
            return None;
        }
        Some(into)
    }

    /// Makes the directory `name` a Git repository, in the background.
    fn make_repository(&mut self, name: String, cx: &mut Context<Self>) {
        let folder = directory::folder_of(&self.standing.folder, &name);
        self.go(
            format!("Making {name} a repository…"),
            move || {
                directory::init(&folder)?;
                Ok(format!("Made {name} a Git repository, with no remote yet."))
            },
            cx,
        );
    }

    /// Opens the input of the remote of the repository `name`, under its
    /// row, holding the remote it has; or closes it, if it is open.
    fn start_linking(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .linking
            .take()
            .is_some_and(|linking| linking.name == name)
        {
            cx.notify();
            return;
        }
        let remote = self
            .directories
            .iter()
            .find(|directory| directory.name == name)
            .and_then(|directory| directory.remote.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_placeholder("The URL of its remote", cx);
            input.set_text(&remote, cx);
            input
        });
        let submitted = cx.subscribe_in(&input, window, |card, _, Submitted, _, cx| {
            card.link(cx);
        });
        window.focus(&input.focus_handle(cx));
        self.linking = Some(Linking {
            name,
            input,
            _submitted: submitted,
        });
        cx.notify();
    }

    /// Links the repository being linked to the remote its input holds,
    /// in the background.
    fn link(&mut self, cx: &mut Context<Self>) {
        let Some(linking) = &self.linking else {
            return;
        };
        let url = linking.input.read(cx).text().trim().to_owned();
        if url.is_empty() {
            return self.say(Making::Failed("A remote needs a URL.".to_owned()), cx);
        }
        let name = linking.name.clone();
        let folder = directory::folder_of(&self.standing.folder, &name);
        self.go(
            format!("Linking {name}…"),
            move || {
                Ok(match directory::link(&folder, &url)? {
                    directory::Linked::Added => format!("Linked {name} to its remote."),
                    directory::Linked::CheckedOut(branch) => {
                        format!("Linked {name} to its remote, and took its {branch}.")
                    }
                })
            },
            cx,
        );
    }

    /// Says `going`, and does `work` in the background, then says what
    /// it did, or why it failed, and lists the directories again.
    fn go(
        &mut self,
        going: String,
        work: impl FnOnce() -> Result<String, String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.making, Making::Going(_)) {
            return;
        }
        self.say(Making::Going(going), cx);
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move {
                    let done = work();
                    if let Err(why) = &done {
                        tracing::warn!("the directory is not made or linked: {why}");
                    }
                    done
                })
                .await;
            this.update(cx, |card, cx| {
                card.directories = directory::listed(&card.standing.folder);
                match done {
                    Ok(done) => {
                        card.directory.update(cx, LineInput::clear);
                        card.linking = None;
                        card.say(Making::Done(done), cx);
                    }
                    Err(why) => card.say(Making::Failed(why), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn say(&mut self, making: Making, cx: &mut Context<Self>) {
        self.making = making;
        cx.notify();
    }

    /// What the line under the input of a directory says, and in which
    /// color.
    fn said_of_making(&self) -> (String, u32) {
        match &self.making {
            Making::Hint => (
                "Enter makes a directory of a name, where the project's agents work, or \
                 clones the Git repository a URL names into one named after it."
                    .to_owned(),
                FAINT,
            ),
            Making::Going(said) => (said.clone(), FAINT),
            Making::Done(said) => (said.clone(), ACCENT),
            Making::Failed(why) => (why.clone(), FAILED),
        }
    }

    /// The name the input holds.
    fn typed_name<'a>(&self, cx: &'a App) -> &'a str {
        self.name.read(cx).text().trim()
    }

    /// The name the human typed for the project the card edits, while it
    /// is not what the project is named, and the project can take it.
    #[must_use]
    pub fn renamed(&self, cx: &App) -> Option<Rename> {
        let typed = self.typed_name(cx);
        (typed != named(&self.standing.folder) && why_not_renamed(&self.standing, typed).is_none())
            .then(|| Rename(typed.to_owned()))
    }

    /// Keeps the name typed.
    fn submit_name(&mut self, cx: &mut Context<Self>) {
        if let Some(rename) = self.renamed(cx) {
            cx.emit(rename);
        }
    }

    /// Follows the project as the window lists it now, as once it is
    /// renamed.
    pub fn follow(&mut self, standing: Standing, cx: &mut Context<Self>) {
        if self.standing.folder != standing.folder {
            let text = named(&standing.folder);
            self.name.update(cx, |input, cx| input.set_text(&text, cx));
            self.directories = directory::listed(&standing.folder);
            self.linking = None;
        }
        self.standing = standing;
        cx.notify();
    }

    /// What the card shows, a line each, for `cox --inspect state`.
    #[must_use]
    pub fn described(&self, cx: &App) -> Vec<String> {
        std::iter::once(format!("Project card of {}", named(&self.standing.folder)))
            .chain(std::iter::once(format!(
                "Name input holding {:?}{}",
                self.name.read(cx).text(),
                why_not_renamed(&self.standing, self.typed_name(cx))
                    .map(|why| format!(": {why}"))
                    .unwrap_or_default()
            )))
            .chain(self.directories.iter().map(|directory| {
                format!(
                    "Directory {}{}",
                    directory.name,
                    match (directory.git, &directory.remote) {
                        (false, _) => String::new(),
                        (true, None) => " (Git, no remote)".to_owned(),
                        (true, Some(remote)) => format!(" (Git, remote {remote})"),
                    }
                )
            }))
            .chain(self.linking.as_ref().map(|linking| {
                format!(
                    "Remote input of {} holding {:?}",
                    linking.name,
                    linking.input.read(cx).text()
                )
            }))
            .chain([
                format!(
                    "Directory input holding {:?}",
                    self.directory.read(cx).text()
                ),
                format!("Under it: {}", self.said_of_making().0),
            ])
            .chain(
                self.standing
                    .missing
                    .then(|| missing(&self.standing.folder)),
            )
            .collect()
    }

    /// The card's heading, with the × that closes it.
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .px(px(22.))
            .py(px(16.))
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Edit {}", named(&self.standing.folder))),
            )
            .child(
                div()
                    .id("close-project-card")
                    .debug_selector(|| "close-project-card".to_owned())
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)))
                    .child(icon(Icon::Close, 14., FAINT))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Dismissed))),
            )
    }

    /// The input of the project's name, under its heading, with what it
    /// does.
    fn render_name(&self, cx: &App) -> impl IntoElement + use<> {
        let typed = self.typed_name(cx);
        let (said, color) = match why_not_renamed(&self.standing, typed) {
            Some(why) if typed != named(&self.standing.folder) => (why, FAILED),
            _ => (
                "The project's folder is named so too. Enter renames both, as closing the card \
                 does; its chats stop, and a line sent starts each again in the folder \
                 renamed.",
                FAINT,
            ),
        };
        div()
            .mb_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(label("NAME"))
            .child(
                div()
                    .debug_selector(|| "project-name-input".to_owned())
                    .h(px(INPUT_HEIGHT))
                    .px_2()
                    .flex()
                    .items_center()
                    .border_1()
                    .border_color(rgb(LINK))
                    .rounded(px(10.))
                    .text_size(px(13.))
                    .text_color(rgb(FOREGROUND))
                    .child(self.name.clone()),
            )
            .child(div().text_size(px(12.)).text_color(rgb(color)).child(said))
    }

    /// The project's directories, a row each, under their heading, each
    /// with what can be done with it, and the input of another's name, or
    /// a repository's URL, with what it does.
    fn render_directories(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (said, color) = self.said_of_making();
        let rows = self
            .directories
            .iter()
            .enumerate()
            .map(|(index, directory)| {
                let name = directory.name.clone();
                let (what, action): (SharedString, _) = match (&directory.git, &directory.remote) {
                    (false, _) => (SharedString::default(), "Make repository"),
                    (true, None) => ("Git, no remote".into(), "Link…"),
                    (true, Some(remote)) => (remote.clone().into(), "Link…"),
                };
                let git = directory.git;
                let row = div()
                    .debug_selector(move || format!("project-directory-{index}"))
                    .h(px(ROW_HEIGHT))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(Icon::Folder, 14., DIM))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(13.))
                            .text_color(rgb(FOREGROUND))
                            .child(SharedString::from(directory.name.clone())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(rgb(FAINT))
                            .child(ellipsis(what)),
                    )
                    .child(
                        button(("directory-action", index), action, Kind::Plain)
                            .debug_selector(move || format!("project-directory-action-{index}"))
                            .h(px(26.))
                            .text_size(px(12.))
                            .tooltip(tip(if git {
                                "Link it to a remote, as its origin"
                            } else {
                                "Make it a Git repository, with no remote yet"
                            }))
                            .on_click(cx.listener(move |card, _, window, cx| {
                                if git {
                                    card.start_linking(name.clone(), window, cx);
                                } else {
                                    card.make_repository(name.clone(), cx);
                                }
                            })),
                    );
                let linking = self
                    .linking
                    .as_ref()
                    .filter(|linking| linking.name == directory.name)
                    .map(|linking| {
                        div()
                            .px_2()
                            .pb_2()
                            .child(input_box("project-directory-remote", &linking.input))
                    });
                div()
                    .flex()
                    .flex_col()
                    .when(index > 0, |row| row.border_t_1().border_color(rgb(BORDER)))
                    .child(row)
                    .children(linking)
            });
        div()
            .mb_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(label("DIRECTORIES"))
            .when(!self.directories.is_empty(), |section| {
                section.child(
                    div()
                        .flex()
                        .flex_col()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(RAISED))
                        .children(rows),
                )
            })
            .child(input_box("project-directory-input", &self.directory))
            .child(
                div()
                    .debug_selector(|| "project-directory-told".to_owned())
                    .text_size(px(12.))
                    .text_color(rgb(color))
                    .child(said),
            )
    }
}

/// Why the project that stands as `standing` cannot be renamed `name`,
/// and its folder with it, if it cannot.
#[must_use]
pub fn why_not_renamed(standing: &Standing, name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("A project needs a name.");
    }
    if name == "." || name == ".." || name.starts_with('.') || name.contains(['/', '\\']) {
        return Some("The name is its folder's: no / in it, nor a . first.");
    }
    if name == named(&standing.folder) {
        return None;
    }
    if standing.missing {
        return Some("The project's folder is not there.");
    }
    if standing.at_work {
        return Some("A chat of the project is at work: rename it once the turn ends.");
    }
    let to = standing.folder.with_file_name(name);
    // A name that differs only in case is the same folder where the disk
    // does not tell cases apart, as on macOS.
    let same = to.canonicalize().ok() == standing.folder.canonicalize().ok();
    if to.exists() && !same {
        return Some("There is a folder of that name there already.");
    }
    None
}

/// What `folder` is called: its name, or its path when it has none.
fn named(folder: &Path) -> String {
    folder
        .file_name()
        .map_or_else(|| folder.to_string_lossy(), |name| name.to_string_lossy())
        .into_owned()
}

impl EventEmitter<Rename> for ProjectCard {}
impl EventEmitter<Dismissed> for ProjectCard {}

/// What the card says of the project whose folder, `folder`, is not
/// there.
fn missing(folder: &Path) -> String {
    format!(
        "Cox cannot find the project's folder, {}: its chats start again once it is back there.",
        shortened(folder)
    )
}

impl Focusable for ProjectCard {
    /// The input that has the keyboard as the card opens: the name's.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.name.focus_handle(cx)
    }
}

impl Render for ProjectCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let missing = self.standing.missing.then(|| {
            div()
                .debug_selector(|| "project-missing".to_owned())
                .mb_3()
                .text_size(px(13.))
                .text_color(rgb(ASKING))
                .child(missing(&self.standing.folder))
        });
        card()
            .id("project-card")
            .debug_selector(|| "project-card".to_owned())
            .key_context(KEY_CONTEXT)
            .occlude()
            .w(px(WIDTH))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .shadow_lg()
            // A click on the card stays there, not closing it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(|_, _: &CloseProjectCard, _, cx| cx.emit(Dismissed)))
            .child(self.render_header(cx))
            .child(
                div()
                    .px(px(22.))
                    .pt(px(18.))
                    .pb(px(22.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(missing)
                    .child(self.render_name(cx))
                    .child(self.render_directories(cx)),
            )
    }
}

/// The heading of a part of the card, as `NAME`.
fn label(text: &'static str) -> gpui::Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(FAINT))
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::{Standing, why_not_renamed};

    #[test]
    fn a_project_whose_folder_is_not_there_keeps_its_name() {
        let root = std::env::temp_dir().join(format!("cox-project-card-{}", std::process::id()));
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::create_dir_all(root.join("taken")).unwrap();
        let standing = Standing {
            folder: root.join("app"),
            missing: false,
            at_work: false,
        };
        assert_eq!(why_not_renamed(&standing, "shop"), None);
        assert_eq!(why_not_renamed(&standing, "app"), None);
        assert!(why_not_renamed(&standing, "taken").is_some());
        assert!(why_not_renamed(&standing, "a/b").is_some());
        let gone = Standing {
            folder: root.join("gone"),
            missing: true,
            at_work: false,
        };
        assert!(why_not_renamed(&gone, "shop").is_some());
        let busy = Standing {
            at_work: true,
            ..standing
        };
        assert!(why_not_renamed(&busy, "shop").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }
}
