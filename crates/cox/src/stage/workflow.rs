//! A tab of the stage that shows how the human works with Cox there,
//! "Workflow" on the stage's bar: the files of their workflow, which every
//! chat goes by whatever the project, and those of the stage's folder,
//! which its chats go by, each with what it does and whether it is there,
//! a click opening one that is. The settings are apart, on a card over
//! the window (see [`crate::settings::view`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use cox_core::tabs::KeptTab;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, Task, Window, div, prelude::*,
    px, rgb,
};

use crate::stage::tab::Shows;
use crate::ui::paths::shortened;
use crate::ui::theme::{BORDER, DIM, FAINT, FOREGROUND, HOVER, LINK};

/// How often the tab looks again at which of the files are there.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// How wide what the tab shows grows, at most.
const WIDTH: f32 = 720.;

/// The human chose the file at this path, to open it.
pub struct Chosen(pub PathBuf);

/// A file, or a folder, of a workflow: where it is from the workflow's
/// folder, and what it does.
struct Part {
    path: &'static str,
    about: &'static str,
}

/// The parts of the human's workflow, in its folder.
const YOURS: &[Part] = &[
    Part {
        path: "COX.md",
        about: "What every chat's agent is told first, whatever the project; Cox's own when \
                there is none.",
    },
    Part {
        path: "sketch.json",
        about: "What a sketch's agent does unasked, and the ways a sketch ends.",
    },
    Part {
        path: "sketch/",
        about: "What Cox tells the agent in your place at the turns of a sketch.",
    },
    Part {
        path: "wiki/save.md",
        about: "What \"Save to wiki\" asks the agent.",
    },
    Part {
        path: "schedules/",
        about: "The chats that open on their own, at the times they say.",
    },
];

/// The parts of a folder's own, which its chats go by.
const THE_FOLDERS: &[Part] = &[
    Part {
        path: "COX.md",
        about: "What the agents of the chats here are told after your workflow's.",
    },
    Part {
        path: ".cox/goal.md",
        about: "How a goal in words is judged here.",
    },
    Part {
        path: ".cox/schedules/",
        about: "The schedules of this folder alone.",
    },
    Part {
        path: ".cox/maps/",
        about: "The maps \"Map\" draws.",
    },
    Part {
        path: ".cox/wiki/",
        about: "The wiki of this folder alone.",
    },
    Part {
        path: ".cox/plugins/",
        about: "The pages of this folder's own on the bar.",
    },
    Part {
        path: ".cox/servers.json",
        about: "The language servers the code tabs ask, ahead of Cox's.",
    },
];

pub struct Workflow {
    /// The stage's folder.
    cwd: PathBuf,
    focus_handle: FocusHandle,
    _looking: Task<()>,
}

impl Workflow {
    /// The tab of the stage of `cwd`.
    pub fn new(cwd: PathBuf, cx: &mut Context<Self>) -> Self {
        let looking = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(LOOK_EVERY).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self {
            cwd,
            focus_handle: cx.focus_handle(),
            _looking: looking,
        }
    }

    /// The rows of `parts`, in the folder `root`, with their heading.
    fn render_parts(
        heading: &str,
        root: &Path,
        parts: &[Part],
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let rows = parts.iter().enumerate().map(|(index, part)| {
            let path = root.join(part.path);
            let folder = part.path.ends_with('/');
            let (there, says) = if folder {
                let count = std::fs::read_dir(&path).map(Iterator::count).ok();
                (
                    count.is_some(),
                    match count {
                        Some(1) => "1 file".to_owned(),
                        Some(count) => format!("{count} files"),
                        None => "none".to_owned(),
                    },
                )
            } else if path.is_file() {
                (true, "open".to_owned())
            } else {
                (false, "none".to_owned())
            };
            let opens = there && !folder;
            div()
                .id(gpui::SharedString::from(format!("{heading}-{index}")))
                .debug_selector(move || format!("workflow-{}", part.path))
                .px(px(10.))
                .py(px(8.))
                .flex()
                .gap(px(12.))
                .border_b_1()
                .border_color(rgb(BORDER))
                .when(opens, |row| {
                    let path = path.clone();
                    row.cursor_pointer()
                        .hover(|style| style.bg(rgb(HOVER)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(Chosen(path.clone()));
                        }))
                })
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(150.))
                        .text_size(px(12.5))
                        .text_color(rgb(if there { FOREGROUND } else { FAINT }))
                        .child(part.path),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.5))
                        .text_color(rgb(DIM))
                        .child(part.about),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(rgb(if opens { LINK } else { FAINT }))
                        .child(says),
                )
        });
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .pb(px(6.))
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(DIM))
                            .child(heading.to_owned()),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(FAINT))
                            .child(shortened(root)),
                    ),
            )
            .children(rows)
    }
}

impl EventEmitter<Chosen> for Workflow {}

impl Shows for Workflow {
    fn title(&self) -> String {
        "Workflow".to_owned()
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Workflow
    }
}

impl Focusable for Workflow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workflow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workflow = crate::settings::workflow();
        let yours = Self::render_parts("YOUR WORKFLOW", &workflow, YOURS, cx);
        let the_folders = Self::render_parts("THIS FOLDER", &self.cwd, THE_FOLDERS, cx);
        div()
            .id("workflow")
            .debug_selector(|| "workflow".to_owned())
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(WIDTH))
                    .px(px(22.))
                    .py(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(22.))
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Workflow"),
                    )
                    .child(yours)
                    .child(the_folders),
            )
    }
}
