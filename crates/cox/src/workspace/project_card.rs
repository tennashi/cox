//! The card where the human edits a project, over the window, which
//! "Edit project…" in the menu of the right button on the project, on
//! the rail, opens: the project's name, its folder's, which Enter
//! changes, as the card does as it closes, renaming the folder; the
//! project's directories, in `directories` in its folder, which Enter
//! makes, or clones a repository into, each of which can become a Git
//! repository and be linked to a remote; the project's folders, its own
//! first,
//! which its chats work in, and the others, which their agents work in
//! too, each of which can become the project's or be taken out; and an
//! input, where Enter adds the folder typed, whole or from `~`, and
//! "Choose…" asks for one. A project whose folder is not there says so,
//! and the input moves the project to where the folder went instead; one
//! whose folder is there does not move, since another folder is another
//! project. Escape, the ×, or a click beside the card closes it.

mod directory;

use directory::Directory;

use std::path::{Path, PathBuf};

use cox_core::store::ProjectId;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, PathPromptOptions, SharedString, Subscription, Window, actions, div, prelude::*,
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

/// How tall a row of the folders is.
const ROW_HEIGHT: f32 = 34.;

/// Binds the key that closes the card.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        CloseProjectCard,
        Some(KEY_CONTEXT),
    )]);
}

/// What the human asked of the project's folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Move the project to this folder, where its own went.
    MoveTo(PathBuf),
    /// Have its chats work in this folder too.
    Add(PathBuf),
    /// Have its chats work in this folder, one beside its own, no more.
    Remove(PathBuf),
    /// Make this folder, one beside its own, the project's.
    MakePrimary(PathBuf),
}

impl Edit {
    /// Whether the project stands as the edit leaves it.
    fn done(&self, standing: &Standing) -> bool {
        match self {
            Self::MoveTo(folder) | Self::MakePrimary(folder) => standing.folder == *folder,
            Self::Add(folder) => standing.also.contains(folder),
            Self::Remove(folder) => !standing.also.contains(folder),
        }
    }
}

/// The human renamed the project, and so its folder.
pub struct Rename(pub String);

/// The human closed the card.
pub struct Dismissed;

/// What the window knows of the project and the others, for the card to
/// say what can be done with its folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    /// The folder its chats work in.
    pub folder: PathBuf,
    /// Whether its folder is not there.
    pub missing: bool,
    /// The other folders its chats work in too.
    pub also: Vec<PathBuf>,
    /// Whether one of its chats is at work, while which it does not move.
    pub at_work: bool,
    /// The folders of the other projects, and Cox's home.
    pub taken: Vec<PathBuf>,
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

/// What the line under the input says.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Told {
    /// What the input does.
    Hint,
    /// The edit is on its way.
    Asked(Edit),
    /// The project stands as the edit leaves it.
    Done(Edit),
    /// The edit cannot be made.
    Failed(String),
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
    /// The input of a folder.
    input: Entity<LineInput>,
    told: Told,
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
        let directory = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_placeholder("A directory's name, or a Git repository's URL", cx);
            input
        });
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_placeholder("Another folder, whole or from ~", cx);
            input.set_text(&filled(&standing), cx);
            input
        });
        let named = cx.subscribe_in(&name, window, |card, _, Submitted, _, cx| {
            card.submit_name(cx);
        });
        let submitted = cx.subscribe_in(&input, window, |card, _, Submitted, _, cx| {
            card.submit(cx);
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
            input,
            told: Told::Hint,
            _submitted: [named, submitted, made, typed],
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

    /// The input of a folder.
    #[must_use]
    pub fn input(&self) -> &Entity<LineInput> {
        &self.input
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

    /// Follows the project as the window lists it now: once it stands as
    /// the human asked, says so.
    pub fn follow(&mut self, standing: Standing, cx: &mut Context<Self>) {
        if self.standing.folder != standing.folder || self.standing.missing != standing.missing {
            self.fill(&standing, cx);
        }
        if self.standing.folder != standing.folder {
            let text = named(&standing.folder);
            self.name.update(cx, |input, cx| input.set_text(&text, cx));
            self.directories = directory::listed(&standing.folder);
            self.linking = None;
        }
        if let Told::Asked(edit) = &self.told
            && edit.done(&standing)
        {
            if matches!(edit, Edit::Add(_)) {
                self.input.update(cx, LineInput::clear);
            }
            self.told = Told::Done(edit.clone());
        }
        self.standing = standing;
        cx.notify();
    }

    /// Fills the input as the project standing as `standing` has it.
    fn fill(&self, standing: &Standing, cx: &mut Context<Self>) {
        let text = filled(standing);
        self.input.update(cx, |input, cx| input.set_text(&text, cx));
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
            .chain(std::iter::once(format!(
                "Folder {} (primary{})",
                shortened(&self.standing.folder),
                if self.standing.missing {
                    ", not there"
                } else {
                    ""
                }
            )))
            .chain(
                self.standing
                    .also
                    .iter()
                    .map(|folder| format!("Folder {}", shortened(folder))),
            )
            .chain([
                format!("Folder input holding {:?}", self.input.read(cx).text()),
                format!("Under it: {}", self.told().0),
            ])
            .collect()
    }

    /// Does with the folder the input names what the input does, if it
    /// can.
    fn submit(&mut self, cx: &mut Context<Self>) {
        let user = std::env::var_os("HOME").map(PathBuf::from);
        let text = self.input.read(cx).text().to_owned();
        match crate::settings::folder_of(&text, user.as_deref()) {
            Ok(folder) => self.take(&folder, cx),
            Err(why) => self.tell(Told::Failed(why.to_owned()), cx),
        }
    }

    /// Moves the project to `folder`, while its own is not there, or else
    /// adds `folder` to it, if it can.
    fn take(&mut self, folder: &Path, cx: &mut Context<Self>) {
        let folder: PathBuf = folder.components().collect();
        if self.standing.missing {
            self.ask(why_not(&self.standing, &folder), Edit::MoveTo(folder), cx);
        } else {
            self.ask(why_not_add(&self.standing, &folder), Edit::Add(folder), cx);
        }
    }

    /// Asks for `edit`, unless `why_not` says why it cannot be made.
    fn ask(&mut self, why_not: Option<&'static str>, edit: Edit, cx: &mut Context<Self>) {
        if let Some(why) = why_not {
            return self.tell(Told::Failed(why.to_owned()), cx);
        }
        self.tell(Told::Asked(edit.clone()), cx);
        cx.emit(edit);
    }

    fn tell(&mut self, told: Told, cx: &mut Context<Self>) {
        self.told = told;
        cx.notify();
    }

    /// Asks the human for a folder, and does with it what the input does.
    fn choose(window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let folder = match chosen.await {
                Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.swap_remove(0),
                Ok(Err(error)) => {
                    tracing::error!("no folder chosen: {error:#}");
                    return;
                }
                _ => return,
            };
            this.update(cx, |card, cx| card.take(&folder, cx)).ok();
        })
        .detach();
    }

    /// What the line under the input says, and in which color.
    fn told(&self) -> (String, u32) {
        match &self.told {
            Told::Failed(why) => (why.clone(), FAILED),
            Told::Asked(_) => ("…".to_owned(), FAINT),
            Told::Done(edit) => (done(edit), ACCENT),
            Told::Hint if self.standing.missing => (
                "Cox cannot find this folder. Type where it is now, and Enter moves the \
                 project there."
                    .to_owned(),
                ASKING,
            ),
            Told::Hint => (
                "Type another folder, or choose one, and Enter adds it: each chat's agent \
                 works in it too, from the next time it starts."
                    .to_owned(),
                FAINT,
            ),
        }
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

    /// The project's folders, a row each, its own first.
    fn render_folders(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let own = {
            let (tag, color) = if self.standing.missing {
                ("NOT THERE", ASKING)
            } else {
                ("PRIMARY", FAINT)
            };
            folder_row(0, &self.standing.folder).child(
                div()
                    .flex_shrink_0()
                    .px_1()
                    .text_size(px(10.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(color))
                    .child(tag),
            )
        };
        let others = self
            .standing
            .also
            .iter()
            .enumerate()
            .map(|(index, folder)| {
                let number = index + 1;
                let made = folder.clone();
                let removed = folder.clone();
                folder_row(number, folder)
                    .child(
                        button(("make-primary", number), "Make primary", Kind::Plain)
                            .h(px(26.))
                            .text_size(px(12.))
                            .tooltip(tip(
                                "The chats opened from then on work in it; those the project has \
                             go on where they are",
                            ))
                            .on_click(cx.listener(move |card, _, _, cx| {
                                let why = why_not_primary(&card.standing, &made);
                                card.ask(why, Edit::MakePrimary(made.clone()), cx);
                            })),
                    )
                    .child(
                        div()
                            .id(("remove-folder", number))
                            .debug_selector(move || format!("remove-folder-{number}"))
                            .flex_shrink_0()
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(HOVER)))
                            .child(icon(Icon::Close, 13., FAINT))
                            .tooltip(tip("Take it out of the project"))
                            .on_click(cx.listener(move |card, _, _, cx| {
                                card.ask(None, Edit::Remove(removed.clone()), cx);
                            })),
                    )
            });
        div()
            .flex()
            .flex_col()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(RAISED))
            .child(own)
            .children(others)
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
        return Some("The project's folder is not there: say where it went first.");
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

/// What the input holds first for the project that stands as `standing`:
/// its folder while that is not there, for the human to say where it
/// went, or else nothing, for another folder.
fn filled(standing: &Standing) -> String {
    if standing.missing {
        shortened(&standing.folder)
    } else {
        String::new()
    }
}

/// A row of the project's folders, the `number`th, from 0, which shows
/// `folder`, by its name and then its path, which gives way where the
/// row is too narrow, for what it offers after it.
fn folder_row(number: usize, folder: &Path) -> gpui::Div {
    let name: SharedString = named(folder).into();
    div()
        .debug_selector(move || format!("project-folder-{number}"))
        .h(px(ROW_HEIGHT))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .when(number > 0, |row| row.border_t_1().border_color(rgb(BORDER)))
        .child(icon(Icon::Folder, 14., DIM))
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(13.))
                .text_color(rgb(FOREGROUND))
                .child(name),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_size(px(12.))
                .text_color(rgb(FAINT))
                .child(ellipsis(shortened(folder))),
        )
}

/// What `folder` is called: its name, or its path when it has none.
fn named(folder: &Path) -> String {
    folder
        .file_name()
        .map_or_else(|| folder.to_string_lossy(), |name| name.to_string_lossy())
        .into_owned()
}

/// What the line under the input says once `edit` is made.
fn done(edit: &Edit) -> String {
    match edit {
        Edit::MoveTo(folder) => format!(
            "Moved: its chats work in {} from now on.",
            shortened(folder)
        ),
        Edit::Add(folder) => format!(
            "Added {}: each chat's agent works in it too, from the next time it starts.",
            named(folder)
        ),
        Edit::Remove(folder) => format!(
            "Took out {}: each chat's agent works in it no more, from the next time it starts.",
            named(folder)
        ),
        Edit::MakePrimary(folder) => format!(
            "The chats opened from now on work in {}; those the project has go on where they \
             are.",
            named(folder)
        ),
    }
}

/// Why the project that stands as `standing` cannot move to `folder`, if
/// it cannot.
fn why_not(standing: &Standing, folder: &Path) -> Option<&'static str> {
    if folder == standing.folder {
        return Some(if standing.missing {
            "That is where Cox looked: there is no folder there."
        } else {
            "The project is in that folder already."
        });
    }
    // A project goes only where its folder went: another folder, while
    // its own is there, is another project.
    if !standing.missing {
        return Some("The project's folder is still there, so it has not moved.");
    }
    if !folder.is_dir() {
        return Some("There is no folder there.");
    }
    if standing.taken.iter().any(|taken| taken == folder) {
        return Some("Another project is in that folder, or Cox's own chats are.");
    }
    if standing.at_work {
        return Some("A chat of the project is at work: it moves once the turn ends.");
    }
    None
}

/// Why the project that stands as `standing` cannot work in `folder` too,
/// if it cannot.
fn why_not_add(standing: &Standing, folder: &Path) -> Option<&'static str> {
    if folder == standing.folder {
        return Some("That is the project's folder already.");
    }
    if standing.also.iter().any(|also| also == folder) {
        return Some("The project has that folder already.");
    }
    if !folder.is_dir() {
        return Some("There is no folder there.");
    }
    None
}

/// Why `folder`, one beside the project's own, cannot become the
/// project's, if it cannot.
fn why_not_primary(standing: &Standing, folder: &Path) -> Option<&'static str> {
    if !folder.is_dir() {
        return Some("That folder is not there.");
    }
    // Another project's folder is that project's.
    if standing.taken.iter().any(|taken| taken == folder) {
        return Some("Another project is in that folder, or Cox's own chats are.");
    }
    None
}

impl EventEmitter<Edit> for ProjectCard {}
impl EventEmitter<Rename> for ProjectCard {}
impl EventEmitter<Dismissed> for ProjectCard {}

impl Focusable for ProjectCard {
    /// The input that has the keyboard as the card opens: the folder's,
    /// while the project's is not there, for the human to say where it
    /// went, or else the name's.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.standing.missing {
            self.input.focus_handle(cx)
        } else {
            self.name.focus_handle(cx)
        }
    }
}

impl Render for ProjectCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (told, color) = self.told();
        let edge = if self.standing.missing { ASKING } else { LINK };
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
                    .child(self.render_name(cx))
                    .child(self.render_directories(cx))
                    .child(label("FOLDERS"))
                    .child(div().text_size(px(13.)).text_color(rgb(DIM)).child(
                        "The project's chats work in its primary folder, and their agents in \
                         the others too. Once you move the primary elsewhere yourself, say where \
                         it is now: the chats, the tabs of the stage, the icon and how the \
                         triggers are arranged go with it. Saying so moves no file.",
                    ))
                    .child(self.render_folders(cx))
                    .child(
                        div()
                            .mt_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .debug_selector(|| "project-folder-input".to_owned())
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(INPUT_HEIGHT))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .border_1()
                                    .border_color(rgb(edge))
                                    .rounded(px(10.))
                                    .text_size(px(13.))
                                    .text_color(rgb(FOREGROUND))
                                    .child(self.input.clone()),
                            )
                            .child(
                                button(
                                    "choose-project-folder",
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(icon(Icon::Folder, 14., DIM))
                                        .child("Choose…"),
                                    Kind::Plain,
                                )
                                .h(px(INPUT_HEIGHT))
                                .on_click(cx.listener(|_, _, window, cx| Self::choose(window, cx))),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "project-folder-told".to_owned())
                            .text_size(px(12.))
                            .text_color(rgb(color))
                            .child(told),
                    ),
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
    use std::path::PathBuf;

    use super::{Edit, Standing, why_not, why_not_add, why_not_primary};

    #[test]
    fn a_project_moves_only_to_a_folder_that_is_there_and_no_one_elses() {
        let root = std::env::temp_dir().join(format!("cox-project-card-{}", std::process::id()));
        std::fs::create_dir_all(root.join("app-2")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        let standing = Standing {
            folder: root.join("app"),
            missing: true,
            also: Vec::new(),
            at_work: false,
            taken: vec![root.join("other")],
        };
        assert_eq!(why_not(&standing, &root.join("app-2")), None);
        assert!(why_not(&standing, &root.join("app")).is_some());
        assert!(why_not(&standing, &root.join("nowhere")).is_some());
        assert!(why_not(&standing, &root.join("other")).is_some());
        let busy = Standing {
            at_work: true,
            ..standing
        };
        assert!(why_not(&busy, &root.join("app-2")).is_some());
        // While its folder is there, it does not move.
        let there = Standing {
            missing: false,
            at_work: false,
            ..busy
        };
        assert!(why_not(&there, &root.join("app-2")).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_project_works_in_folders_that_are_there_and_makes_one_its_own_unless_taken() {
        let root = std::env::temp_dir().join(format!("cox-project-folders-{}", std::process::id()));
        for folder in ["app", "lib", "other", "docs"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        let standing = Standing {
            folder: root.join("app"),
            missing: false,
            also: vec![root.join("lib"), root.join("other")],
            at_work: true,
            taken: vec![root.join("other")],
        };
        // Another project's folder is one a project works in too.
        assert_eq!(why_not_add(&standing, &root.join("docs")), None);
        assert!(why_not_add(&standing, &root.join("app")).is_some());
        assert!(why_not_add(&standing, &root.join("lib")).is_some());
        assert!(why_not_add(&standing, &root.join("nowhere")).is_some());
        // A chat at work goes on where it works.
        assert_eq!(why_not_primary(&standing, &root.join("lib")), None);
        assert!(why_not_primary(&standing, &root.join("other")).is_some());
        assert!(why_not_primary(&standing, &root.join("nowhere")).is_some());

        assert!(Edit::Add(root.join("lib")).done(&standing));
        assert!(!Edit::Remove(root.join("lib")).done(&standing));
        assert!(!Edit::MakePrimary(root.join("lib")).done(&standing));
        let made = Standing {
            folder: root.join("lib"),
            also: vec![root.join("app"), root.join("other")],
            ..standing
        };
        assert!(Edit::MakePrimary(root.join("lib")).done(&made));
        assert!(Edit::Remove(PathBuf::from("/elsewhere")).done(&made));
        let _ = std::fs::remove_dir_all(&root);
    }
}
