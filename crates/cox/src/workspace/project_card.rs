//! The card where the human edits a project, over the window, which
//! "Edit project…" in the menu of the right button on the project, on
//! the rail, opens: the folder its chats work in, in an input, where
//! Enter moves the project to the folder typed, whole or from `~`, as
//! when the human moved or renamed its folder, and "Choose…" asks for a
//! folder and moves it there. A project whose folder is not there says
//! so; one whose folder is there does not move, since another folder is
//! another project. Escape, the ×, or a click beside the card closes it.

use std::path::{Path, PathBuf};

use cox_core::store::ProjectId;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, PathPromptOptions, Subscription, Window, actions, div, prelude::*, px, rgb,
};

use crate::ui::button::{INPUT_HEIGHT, Kind, button};
use crate::ui::card;
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{LineInput, Submitted};
use crate::ui::paths::shortened;
use crate::ui::theme::{ACCENT, ASKING, BORDER, DIM, FAILED, FAINT, FOREGROUND, HOVER, LINK};

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

/// Binds the key that closes the card.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        CloseProjectCard,
        Some(KEY_CONTEXT),
    )]);
}

/// The human asked to move the project to this folder.
pub struct MoveTo(pub PathBuf);

/// The human closed the card.
pub struct Dismissed;

/// What the window knows of the project and the others, for the card to
/// say whether the project can move to a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    /// The folder its chats work in.
    pub folder: PathBuf,
    /// Whether its folder is not there.
    pub missing: bool,
    /// Whether one of its chats is at work, while which it does not move.
    pub at_work: bool,
    /// The folders of the other projects, and Cox's home.
    pub taken: Vec<PathBuf>,
}

/// What the line under the input says.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Told {
    /// How to move the project.
    Hint,
    /// The project is on its way to this folder.
    Moving(PathBuf),
    /// The project moved to this folder.
    Moved(PathBuf),
    /// The project cannot move to what the human typed.
    Failed(String),
}

pub struct ProjectCard {
    project: ProjectId,
    name: String,
    standing: Standing,
    input: Entity<LineInput>,
    told: Told,
    _submitted: Subscription,
}

impl ProjectCard {
    /// The card of the project kept as `project`, named `name`, which
    /// stands as `standing` says.
    pub fn new(
        project: ProjectId,
        name: String,
        standing: Standing,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_text(&shortened(&standing.folder), cx);
            input
        });
        let submitted = cx.subscribe_in(&input, window, |card, _, Submitted, _, cx| {
            card.submit(cx);
        });
        Self {
            project,
            name,
            standing,
            input,
            told: Told::Hint,
            _submitted: submitted,
        }
    }

    /// The project the card edits.
    #[must_use]
    pub fn project(&self) -> ProjectId {
        self.project
    }

    /// The input of the folder.
    #[must_use]
    pub fn input(&self) -> &Entity<LineInput> {
        &self.input
    }

    /// Follows the project as the window lists it now, named `name`: once
    /// it is where the human moved it, says so.
    pub fn follow(&mut self, name: String, standing: Standing, cx: &mut Context<Self>) {
        if self.standing.folder != standing.folder {
            let shown = shortened(&standing.folder);
            self.input
                .update(cx, |input, cx| input.set_text(&shown, cx));
        }
        if matches!(&self.told, Told::Moving(to) if *to == standing.folder) {
            self.told = Told::Moved(standing.folder.clone());
        }
        self.name = name;
        self.standing = standing;
        cx.notify();
    }

    /// What the card shows, a line each, for `cox --inspect state`.
    #[must_use]
    pub fn described(&self, cx: &App) -> Vec<String> {
        vec![
            format!("Project card of {}", self.name),
            format!("Folder input holding {:?}", self.input.read(cx).text()),
            format!("Under it: {}", self.told().0),
        ]
    }

    /// Moves the project to the folder the input names, if it can.
    fn submit(&mut self, cx: &mut Context<Self>) {
        let user = std::env::var_os("HOME").map(PathBuf::from);
        let text = self.input.read(cx).text().to_owned();
        match crate::settings::folder_of(&text, user.as_deref()) {
            Ok(folder) => self.move_to(&folder, cx),
            Err(why) => self.tell(Told::Failed(why.to_owned()), cx),
        }
    }

    /// Moves the project to `folder`, if it can.
    fn move_to(&mut self, folder: &Path, cx: &mut Context<Self>) {
        let folder: PathBuf = folder.components().collect();
        if let Some(why) = why_not(&self.standing, &folder) {
            return self.tell(Told::Failed(why.to_owned()), cx);
        }
        self.tell(Told::Moving(folder.clone()), cx);
        cx.emit(MoveTo(folder));
    }

    fn tell(&mut self, told: Told, cx: &mut Context<Self>) {
        self.told = told;
        cx.notify();
    }

    /// Asks the human for a folder, and moves the project to it.
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
            this.update(cx, |card, cx| card.move_to(&folder, cx)).ok();
        })
        .detach();
    }

    /// What the line under the input says, and in which color.
    fn told(&self) -> (String, u32) {
        match &self.told {
            Told::Failed(why) => (why.clone(), FAILED),
            Told::Moving(_) => ("Moving…".to_owned(), FAINT),
            Told::Moved(folder) => (
                format!(
                    "Moved: its chats work in {} from now on.",
                    shortened(folder)
                ),
                ACCENT,
            ),
            Told::Hint if self.standing.missing => (
                "Cox cannot find this folder. Type where it is now, and Enter moves the \
                 project there."
                    .to_owned(),
                ASKING,
            ),
            Told::Hint => (
                "Once you move or rename the folder, type where it is now, and Enter moves the \
                 project there; Escape closes."
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
                    .child(format!("Edit {}", self.name)),
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
        return Some(
            "The project's folder is still there, so it has not moved. To work in another \
             folder, open it with Open folder….",
        );
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

impl EventEmitter<MoveTo> for ProjectCard {}
impl EventEmitter<Dismissed> for ProjectCard {}

impl Focusable for ProjectCard {
    /// The input, which has the keyboard while the card is open.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
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
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(FAINT))
                            .child("FOLDER"),
                    )
                    .child(div().text_size(px(13.)).text_color(rgb(DIM)).child(
                        "The folder the project's chats work in. Once you move or rename \
                             it, say where it is now: the chats, the tabs of the stage, the \
                             icon and how the triggers are arranged go with it, and each \
                             chat's agent starts again there, told the conversation so far. \
                             No file moves.",
                    ))
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

#[cfg(test)]
mod tests {
    use super::{Standing, why_not};

    #[test]
    fn a_project_moves_only_to_a_folder_that_is_there_and_no_one_elses() {
        let root = std::env::temp_dir().join(format!("cox-project-card-{}", std::process::id()));
        std::fs::create_dir_all(root.join("app-2")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        let standing = Standing {
            folder: root.join("app"),
            missing: true,
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
}
