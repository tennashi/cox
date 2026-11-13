//! The card where the human edits a project, over the window, which
//! "Edit project…" in the menu of the right button on the project, on
//! the rail, opens: the project's name, its folder's, which Enter
//! changes, as the card does as it closes, renaming the folder; what the
//! project is, which each chat's agent is told first, set the same way; its
//! icon, which the human chooses, has an agent draw, or takes away; and
//! the project's directories, in `directories` in its folder, which Enter
//! makes, named as the project unless named otherwise, or clones a
//! repository into, each of which can be renamed, become a Git
//! repository and be linked to a remote. A project whose folder is not
//! there says so. Escape, the ×, or a click beside the card closes it.

mod directory;

use directory::Directory;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cox_core::store::ProjectId;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Image, KeyBinding,
    MouseButton, ScrollHandle, SharedString, Subscription, Window, actions, div, img, prelude::*,
    px, rgb,
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
use crate::vim::Move;
use crate::vim::hints::Hinted as _;

actions!(
    cox,
    [
        /// Closes the card of the project.
        CloseProjectCard,
    ]
);

/// The key context of the card.
pub(crate) const KEY_CONTEXT: &str = "ProjectCard";

/// How wide the card is.
const WIDTH: f32 = 560.;

/// How tall a row of the directories is.
const ROW_HEIGHT: f32 = 34.;

/// How big the square of the project's icon is.
const ICON_SIZE: f32 = 44.;

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

/// The human set what the project is, or, blank, that it says nothing of
/// what it is.
pub struct SetAbout(pub String);

/// The human closed the card.
pub struct Dismissed;

/// What the human asks of the project's icon (see
/// [`crate::project_icon`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToIcon {
    /// A picture they choose, as its icon.
    Choose,
    /// An agent to read the project and draw it one.
    Generate,
    /// Its icon taken away, the rail drawing a folder again.
    Remove,
}

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
    /// The icon the human gave it, if they did.
    pub icon: Option<Arc<Image>>,
    /// Whether an agent draws it one.
    pub drawing: Drawing,
    /// What it is, as each chat's agent is told, if the human set it.
    pub about: Option<String>,
}

/// Whether an agent draws a project an icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drawing {
    /// None can, as while Cox does not know its agents.
    Unable,
    /// One can, when the human asks.
    Ready,
    /// One draws it now.
    Now,
    /// The one asked last drew none, for this reason; another can.
    NotDrawn(String),
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

/// What the human gives of a directory in the input under its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gives {
    /// The URL of its remote.
    Remote,
    /// Its new name.
    Name,
}

/// The directory of which the human gives what `gives` says, and the
/// input of it.
struct Editing {
    name: String,
    gives: Gives,
    input: Entity<LineInput>,
    _submitted: Subscription,
}

pub struct ProjectCard {
    project: ProjectId,
    standing: Standing,
    /// The input of the project's name.
    name: Entity<LineInput>,
    /// The input of what the project is.
    about: Entity<LineInput>,
    /// The project's directories, as they stand.
    directories: Vec<Directory>,
    /// The input of a directory's name, or a Git repository's URL.
    directory: Entity<LineInput>,
    /// What the line under it says.
    making: Making,
    /// The directory of which the human gives its remote or its name, if
    /// they do.
    editing: Option<Editing>,
    scroll: ScrollHandle,
    _submitted: [Subscription; 4],
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
        let about = cx.new(|cx| {
            let mut input = LineInput::wrapping(cx);
            input.set_placeholder("What the project is, in a sentence or two", cx);
            input.set_text(standing.about.as_deref().unwrap_or_default(), cx);
            input
        });
        let directory = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_placeholder(
                "A directory's name, the project's unless given, or a Git repository's URL",
                cx,
            );
            input
        });
        let named = cx.subscribe_in(&name, window, |card, _, Submitted, _, cx| {
            card.submit_name(cx);
        });
        let told = cx.subscribe_in(&about, window, |card, _, Submitted, _, cx| {
            card.submit_about(cx);
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
            about,
            directory,
            making: Making::Hint,
            editing: None,
            scroll: ScrollHandle::new(),
            _submitted: [named, told, made, typed],
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

    /// The input of what the project is.
    #[must_use]
    pub fn about_input(&self) -> &Entity<LineInput> {
        &self.about
    }

    /// The input of a directory's name, or a Git repository's URL.
    #[must_use]
    pub fn directory_input(&self) -> &Entity<LineInput> {
        &self.directory
    }

    /// The input under the row of a directory, if one is open, and what
    /// the human gives in it.
    #[must_use]
    pub fn row_input(&self) -> Option<(Gives, &Entity<LineInput>)> {
        self.editing
            .as_ref()
            .map(|editing| (editing.gives, &editing.input))
    }

    /// Makes the directory whose name the input holds, or, when it holds
    /// none, the project's, or clones the repository whose URL it holds
    /// into one named after it, in the background, if it can.
    fn make_directory(&mut self, cx: &mut Context<Self>) {
        let mut typed = self.directory.read(cx).text().trim().to_owned();
        if typed.is_empty() {
            typed = named(&self.standing.folder);
        }
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

    /// Opens the input under the row of the directory `name`, where the
    /// human gives what `gives` says, holding what it has: the remote of
    /// a repository, or its name; or closes it, if it is open.
    fn start_editing(
        &mut self,
        name: String,
        gives: Gives,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editing
            .take()
            .is_some_and(|editing| editing.name == name && editing.gives == gives)
        {
            cx.notify();
            return;
        }
        let (placeholder, text) = match gives {
            Gives::Remote => (
                "The URL of its remote",
                self.directories
                    .iter()
                    .find(|directory| directory.name == name)
                    .and_then(|directory| directory.remote.clone())
                    .unwrap_or_default(),
            ),
            Gives::Name => ("Its new name", name.clone()),
        };
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_placeholder(placeholder, cx);
            input.set_text(&text, cx);
            input
        });
        let submitted = cx.subscribe_in(&input, window, |card, _, Submitted, _, cx| {
            match card.editing.as_ref().map(|editing| editing.gives) {
                Some(Gives::Remote) => card.link(cx),
                Some(Gives::Name) => card.rename_directory(cx),
                None => {}
            }
        });
        window.focus(&input.focus_handle(cx));
        self.editing = Some(Editing {
            name,
            gives,
            input,
            _submitted: submitted,
        });
        cx.notify();
    }

    /// Renames the directory being renamed as its input says, in the
    /// background, unless a chat of the project is at work in it.
    fn rename_directory(&mut self, cx: &mut Context<Self>) {
        let Some(editing) = &self.editing else {
            return;
        };
        let to = editing.input.read(cx).text().trim().to_owned();
        let from = editing.name.clone();
        if to == from {
            self.editing = None;
            return cx.notify();
        }
        if let Some(why) = directory::why_not_named(&to) {
            return self.say(Making::Failed(why.to_owned()), cx);
        }
        if self.standing.at_work {
            return self.say(
                Making::Failed(
                    "A chat of the project is at work: rename it once the turn ends.".to_owned(),
                ),
                cx,
            );
        }
        let project = self.standing.folder.clone();
        self.go(
            format!("Renaming {from}…"),
            move || {
                directory::rename(&project, &from, &to)?;
                Ok(format!("Renamed {from} to {to}."))
            },
            cx,
        );
    }

    /// Links the repository being linked to the remote its input holds,
    /// in the background.
    fn link(&mut self, cx: &mut Context<Self>) {
        let Some(linking) = &self.editing else {
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
                        card.editing = None;
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
                "Enter makes a directory of a name, or of the project's, where the project's \
                 agents work, or clones the Git repository a URL names into one named after it. \
                 A directory named as the project is renamed with it."
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

    /// What the human typed of what the project is, while it is not what
    /// the project is now.
    #[must_use]
    pub fn told(&self, cx: &App) -> Option<SetAbout> {
        let typed = self.about.read(cx).text().trim();
        (typed != self.standing.about.as_deref().unwrap_or_default())
            .then(|| SetAbout(typed.to_owned()))
    }

    /// Keeps what the project is as typed.
    fn submit_about(&mut self, cx: &mut Context<Self>) {
        if let Some(set) = self.told(cx) {
            cx.emit(set);
        }
    }

    /// Follows the project as the window lists it now, as once it is
    /// renamed, or what it is was set elsewhere, as from a chat.
    pub fn follow(&mut self, standing: Standing, cx: &mut Context<Self>) {
        let about = standing.about.as_deref().unwrap_or_default();
        if self.standing.about != standing.about && self.about.read(cx).text().trim() != about {
            self.about.update(cx, |input, cx| input.set_text(about, cx));
        }
        if self.standing.folder != standing.folder {
            let text = named(&standing.folder);
            self.name.update(cx, |input, cx| input.set_text(&text, cx));
            self.directories = directory::listed(&standing.folder);
            self.editing = None;
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
            .chain(std::iter::once(format!(
                "About input holding {:?}",
                self.about.read(cx).text()
            )))
            .chain(std::iter::once(self.described_icon()))
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
            .chain(self.editing.as_ref().map(|editing| {
                format!(
                    "{} input of {} holding {:?}",
                    match editing.gives {
                        Gives::Remote => "Remote",
                        Gives::Name => "Name",
                    },
                    editing.name,
                    editing.input.read(cx).text()
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
                    .on_hint_click(cx.listener(|_, _, _, cx| cx.emit(Dismissed))),
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

    /// The input of what the project is, under its name, with what it
    /// does.
    fn render_about(&self) -> impl IntoElement + use<> {
        div()
            .mb_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(label("ABOUT"))
            .child(
                div()
                    .debug_selector(|| "project-about-input".to_owned())
                    .min_h(px(INPUT_HEIGHT))
                    .px_2()
                    .py(px(7.))
                    .border_1()
                    .border_color(rgb(LINK))
                    .rounded(px(10.))
                    .text_size(px(13.))
                    .text_color(rgb(FOREGROUND))
                    .child(self.about.clone()),
            )
            .child(div().text_size(px(12.)).text_color(rgb(FAINT)).child(
                "Each chat's agent in the project is told it first, as its session \
                     starts. Enter sets it, as closing the card does; Shift+Enter starts a \
                     new line.",
            ))
    }

    /// What the line under the project's icon says, and in which color.
    fn said_of_icon(&self) -> (String, u32) {
        match &self.standing.drawing {
            Drawing::Now => {
                return (
                    "An agent reads the project and draws its icon…".to_owned(),
                    FAINT,
                );
            }
            Drawing::NotDrawn(why) => return (format!("No icon was drawn: {why}"), FAILED),
            Drawing::Unable | Drawing::Ready => {}
        }
        (
            "The rail shows it in place of a folder: a picture, cropped square, or one an \
             agent draws from what it reads of the project. Cox keeps it apart from the \
             project's folder."
                .to_owned(),
            FAINT,
        )
    }

    /// The line `cox --inspect state` says of the project's icon.
    fn described_icon(&self) -> String {
        let standing = &self.standing;
        format!(
            "Icon: {}{}",
            if standing.icon.is_some() {
                "given"
            } else {
                "none, a folder"
            },
            if matches!(standing.drawing, Drawing::Now | Drawing::NotDrawn(_)) {
                format!("; {}", self.said_of_icon().0)
            } else {
                String::new()
            }
        )
    }

    /// The project's icon, in its square, under its heading, with what
    /// can be done with it and what is said of it.
    fn render_icon(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let standing = &self.standing;
        let square = div()
            .debug_selector(|| "project-icon".to_owned())
            .flex_shrink_0()
            .size(px(ICON_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(BORDER))
            .when(standing.drawing == Drawing::Now, |square| {
                square.border_dashed().border_color(rgb(ACCENT))
            });
        let square = match standing.icon.clone() {
            Some(picture) => square.child(img(picture).size(px(ICON_SIZE - 2.)).rounded(px(9.))),
            None => square.bg(rgb(RAISED)).child(icon(Icon::Folder, 20., FAINT)),
        };
        let ask = |to: ToIcon, label: &'static str, said: &'static str, cx: &Context<Self>| {
            let selector = match to {
                ToIcon::Choose => "project-icon-choose",
                ToIcon::Generate => "project-icon-generate",
                ToIcon::Remove => "project-icon-remove",
            };
            button(selector, label, Kind::Plain)
                .debug_selector(move || selector.to_owned())
                .h(px(26.))
                .text_size(px(12.))
                .tooltip(tip(said))
                .on_hint_click(cx.listener(move |_, _, _, cx| cx.emit(to)))
        };
        let (said, color) = self.said_of_icon();
        div()
            .mb_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(label("ICON"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(square)
                    .child(ask(
                        ToIcon::Choose,
                        "Choose…",
                        "A PNG, a JPEG, a WebP or an SVG",
                        cx,
                    ))
                    .when(
                        matches!(standing.drawing, Drawing::Ready | Drawing::NotDrawn(_)),
                        |row| {
                            row.child(ask(
                                ToIcon::Generate,
                                "Generate",
                                "Have an agent read the project and draw one",
                                cx,
                            ))
                        },
                    )
                    .when(standing.icon.is_some(), |row| {
                        row.child(ask(
                            ToIcon::Remove,
                            "Remove",
                            "Take it away, for a folder again",
                            cx,
                        ))
                    }),
            )
            .child(
                div()
                    .debug_selector(|| "project-icon-told".to_owned())
                    .text_size(px(12.))
                    .text_color(rgb(color))
                    .child(said),
            )
    }

    /// The row of `directory`, the `index`th, with what can be done with
    /// it, and the input under it, if the human gives its name or its
    /// remote.
    fn render_directory(
        &self,
        index: usize,
        directory: &Directory,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let name = directory.name.clone();
        let (what, action): (SharedString, _) = match (&directory.git, &directory.remote) {
            (false, _) => (SharedString::default(), "Make repository"),
            (true, None) => ("Git, no remote".into(), "Link…"),
            (true, Some(remote)) => (remote.clone().into(), "Link…"),
        };
        let git = directory.git;
        let renamed = name.clone();
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
                button(("directory-rename", index), "Rename…", Kind::Plain)
                    .debug_selector(move || format!("project-directory-rename-{index}"))
                    .h(px(26.))
                    .text_size(px(12.))
                    .tooltip(tip("Rename it, and its folder"))
                    .on_hint_click(cx.listener(move |card, _, window, cx| {
                        card.start_editing(renamed.clone(), Gives::Name, window, cx);
                    })),
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
                    .on_hint_click(cx.listener(move |card, _, window, cx| {
                        if git {
                            card.start_editing(name.clone(), Gives::Remote, window, cx);
                        } else {
                            card.make_repository(name.clone(), cx);
                        }
                    })),
            );
        let editing = self
            .editing
            .as_ref()
            .filter(|editing| editing.name == directory.name)
            .map(|editing| {
                div().px_2().pb_2().child(input_box(
                    match editing.gives {
                        Gives::Remote => "project-directory-remote",
                        Gives::Name => "project-directory-name",
                    },
                    &editing.input,
                ))
            });
        div()
            .flex()
            .flex_col()
            .when(index > 0, |row| row.border_t_1().border_color(rgb(BORDER)))
            .child(row)
            .children(editing)
    }

    /// The project's directories, a row each, under their heading, each
    /// with what can be done with it, and the input of another's name, or
    /// a repository's URL, with what it does.
    fn render_directories(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (said, color) = self.said_of_making();
        let rows: Vec<gpui::Div> = self
            .directories
            .iter()
            .enumerate()
            .map(|(index, directory)| self.render_directory(index, directory, cx))
            .collect();
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
impl EventEmitter<SetAbout> for ProjectCard {}
impl EventEmitter<Dismissed> for ProjectCard {}
impl EventEmitter<ToIcon> for ProjectCard {}

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
            .on_action(cx.listener(|card, motion: &Move, window, cx| {
                motion.scroll(&card.scroll, window.line_height());
                cx.notify();
            }))
            .child(self.render_header(cx))
            .child(
                div()
                    .id("project-card-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(22.))
                    .pt(px(18.))
                    .pb(px(22.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(missing)
                    .child(self.render_name(cx))
                    .child(self.render_about())
                    .child(self.render_icon(cx))
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
    use super::{Drawing, Standing, why_not_renamed};
    use crate::testing::TempDir;

    /// A project in `root/app`, idle, beside a folder named `taken`.
    fn app_in(root: &TempDir) -> Standing {
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::create_dir_all(root.join("taken")).unwrap();
        Standing {
            folder: root.join("app"),
            missing: false,
            at_work: false,
            icon: None,
            drawing: Drawing::Unable,
            about: None,
        }
    }

    #[test]
    fn a_project_is_renamed_a_free_name_or_its_own() {
        let root = TempDir::new();
        let app = app_in(&root);
        assert_eq!(why_not_renamed(&app, "shop"), None);
        assert_eq!(why_not_renamed(&app, "app"), None);
    }

    #[test]
    fn a_project_is_not_renamed_after_a_folder_already_there() {
        let root = TempDir::new();
        assert_eq!(
            why_not_renamed(&app_in(&root), "taken"),
            Some("There is a folder of that name there already.")
        );
    }

    #[test]
    fn a_name_that_is_no_folder_s_is_refused() {
        let root = TempDir::new();
        let app = app_in(&root);
        assert_eq!(why_not_renamed(&app, ""), Some("A project needs a name."));
        for name in ["a/b", "a\\b", ".hidden", "..", "."] {
            assert_eq!(
                why_not_renamed(&app, name),
                Some("The name is its folder's: no / in it, nor a . first."),
                "{name}"
            );
        }
    }

    #[test]
    fn a_project_whose_folder_is_not_there_keeps_its_name() {
        let root = TempDir::new();
        let gone = Standing {
            folder: root.join("gone"),
            missing: true,
            ..app_in(&root)
        };
        assert_eq!(
            why_not_renamed(&gone, "shop"),
            Some("The project's folder is not there.")
        );
    }

    #[test]
    fn a_project_at_work_keeps_its_name() {
        let root = TempDir::new();
        let busy = Standing {
            at_work: true,
            ..app_in(&root)
        };
        assert_eq!(
            why_not_renamed(&busy, "shop"),
            Some("A chat of the project is at work: rename it once the turn ends.")
        );
    }
}
