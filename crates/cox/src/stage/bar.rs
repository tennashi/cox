//! The bar along the top of the stage: the views of the project it keeps
//! in sight, what changed in its files, and the pages of the plugins, such
//! as the map, each by its icon but the one shown, as the column beside
//! the stage lists the rest of the tabs (see [`super::column`]); and, at
//! its end, the menu that opens another tab: a page, a terminal, a file,
//! chosen on the tree of the project's files, or an Android emulator of
//! each virtual device the SDK has. The button of what changed counts the
//! files that did, while the stage is open.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    App, Context, Entity, Focusable, FontWeight, SharedString, Subscription, Task, Window, actions,
    deferred, div, prelude::*, px, rgb,
};

use super::Stage;
use super::code::Code;
use super::plugins::Plugin;
use super::tab::{Tab, TabContent};
use super::typed::{Typed, Unmatched, typed};
use crate::terminal::Program;
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{LineInput, Submitted};
use crate::ui::tip;
use crate::vim::hints::Hinted as _;
use crate::vim::keymap::Follow;
use coxd::theme::{
    ACCENT, ASKING, BORDER_STRONG, DIM, FAINT, FOREGROUND, HOVER, RAISED, SELECTED, SURFACE,
};

/// How tall a view's button is, and the +, and how round each is.
const VIEW_HEIGHT: f32 = 28.;
const PLUS_SIZE: f32 = 32.;
const VIEW_RADIUS: f32 = 9.;
const PLUS_RADIUS: f32 = 10.;

/// The keys of the menu's items, as the menu shows them (see
/// [`crate::workspace::bind_keys`]).
#[cfg(target_os = "macos")]
const NEW_PAGE_KEY: &str = "⌘T";
#[cfg(not(target_os = "macos"))]
const NEW_PAGE_KEY: &str = "Ctrl+T";
#[cfg(target_os = "macos")]
const NEW_TERMINAL_KEY: &str = "⌃`";
#[cfg(not(target_os = "macos"))]
const NEW_TERMINAL_KEY: &str = "Ctrl+`";
#[cfg(target_os = "macos")]
const OPEN_FILE_KEY: &str = "⌘P";
#[cfg(not(target_os = "macos"))]
const OPEN_FILE_KEY: &str = "Ctrl+P";

/// How often the bar counts the files that changed, as the tab of what
/// changed looks again.
const COUNT_EVERY: Duration = Duration::from_secs(2);

actions!(
    stage,
    [
        /// Closes the menu of the stage's +.
        CloseMenu,
    ]
);

/// The key context of the menu of the stage's +.
pub(crate) const MENU_CONTEXT: &str = "StageMenu";

/// The menu of the stage's +, while it is open: the line at its top,
/// where the human types a URL, a file, a command, or, where Vim's `t`
/// opened it, words to search for with `engine`.
pub(super) struct Menu {
    input: Entity<LineInput>,
    unmatched: Unmatched,
    engine: coxd::settings::SearchEngine,
    _typed: Subscription,
}

/// A view of the project that the bar keeps in sight, apart from the tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum View {
    Changes,
    Plugin(String),
}

/// The plugin that draws the project's maps.
const MAP: &str = "map";

/// The plugin that shows what goes on in the folder.
const ACTIVITY: &str = "activity";

/// Which of the views a project may not have it has: a map, or a Git
/// repository to show what changed in, its own or among its directories,
/// or a tab of it open already.
#[derive(Clone, Copy)]
struct Present {
    map: bool,
    changes: bool,
}

impl Stage {
    /// The view of the project `tab` shows, if it shows one: what changed
    /// in the project's files, or a plugin's page.
    fn view_of(&self, tab: &Tab, plugins: &[Plugin], cx: &App) -> Option<View> {
        match &tab.content {
            TabContent::Changes(changes) if changes.read(cx).folder() == self.cwd => {
                Some(View::Changes)
            }
            TabContent::Page { page, .. } => {
                let url = &page.read(cx).navigation().url;
                plugins
                    .iter()
                    .find(|plugin| url.starts_with(&format!("cox://{}/", plugin.name)))
                    .map(|plugin| View::Plugin(plugin.name.clone()))
            }
            _ => None,
        }
    }

    /// The tabs that are not views of the project, which the column
    /// beside the stage lists, by their indexes.
    pub(super) fn rest(&self, cx: &App) -> Vec<usize> {
        let plugins = self.plugins.list();
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| self.view_of(tab, &plugins, cx).is_none())
            .map(|(index, _)| index)
            .collect()
    }

    pub(super) fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let plugins = self.plugins.list();
        let shown = self
            .tabs
            .shown()
            .and_then(|tab| self.view_of(tab, &plugins, cx));
        let views: Vec<View> = self
            .tabs
            .iter()
            .filter_map(|tab| self.view_of(tab, &plugins, cx))
            .collect();
        div()
            .relative()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .pt(px(10.))
            .pb(px(8.))
            .pl(px(12.))
            .pr(px(10.))
            .child(Self::render_views(
                shown.as_ref(),
                plugins,
                Present {
                    map: self.mapped || views.contains(&View::Plugin(MAP.to_owned())),
                    changes: self.changed.is_some() || views.contains(&View::Changes),
                },
                self.changed,
                self.waiting,
                cx,
            ))
            .child(div().flex_1())
            .child(
                div()
                    .id("stage-menu")
                    .debug_selector(|| "stage-menu".to_owned())
                    .flex_shrink_0()
                    .size(px(PLUS_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(PLUS_RADIUS))
                    .text_color(rgb(FOREGROUND))
                    .cursor_pointer()
                    .when(self.menu.is_some(), |button| button.bg(rgb(SELECTED)))
                    .hover(|style| style.bg(rgb(HOVER)))
                    .child(icon(Icon::Plus, 15., FOREGROUND))
                    // The menu closes as the mouse goes down outside it,
                    // here too, so the button opens it as it comes up.
                    .on_hint_click(cx.listener(|stage, _, window, cx| {
                        stage.open_menu(Unmatched::Command, window, cx);
                    })),
            )
            .children(
                self.menu
                    .as_ref()
                    .map(|menu| Self::render_menu(menu, &self.cwd, cx)),
            )
            .children(self.render_tab_menu(cx))
    }

    /// The buttons of the views of the project, side by side in one
    /// ground, each by its icon, named where the mouse rests, but the one
    /// shown, which is marked and named: "Activity", counting the chats of
    /// the folder that wait for the human, "Map" when the project has one,
    /// then "Changes" in a Git repository, or a project with one
    /// among its directories, then the other plugins, "Triggers" last.
    fn render_views(
        shown: Option<&View>,
        plugins: Vec<Plugin>,
        present: Present,
        changed: Option<usize>,
        waiting: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let button = |id: SharedString, view: View, label: SharedString, glyph: Icon| {
            view_button(shown == Some(&view), id, label, glyph)
        };
        let mut plugin = |plugin: Plugin| {
            let glyph = match plugin.name.as_str() {
                ACTIVITY => Icon::Inbox,
                MAP => Icon::Map,
                "wiki" => Icon::Book,
                "triggers" => Icon::Bolt,
                _ => Icon::Plug,
            };
            let name = plugin.name.clone();
            let counted = (plugin.name == ACTIVITY && waiting > 0).then(|| {
                div()
                    .debug_selector(|| "waiting-count".to_owned())
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(ASKING))
                    .child(waiting.to_string())
            });
            button(
                format!("plugin-{}", plugin.name).into(),
                View::Plugin(plugin.name),
                plugin.title.into(),
                glyph,
            )
            .children(counted)
            .on_hint_click(cx.listener(move |stage, _, window, cx| {
                stage.show_plugin(&name, "", window, cx);
            }))
        };
        let (ahead, rest): (Vec<Plugin>, Vec<Plugin>) = plugins
            .into_iter()
            .partition(|plugin| plugin.name == ACTIVITY || plugin.name == MAP);
        let ahead: Vec<_> = ahead
            .into_iter()
            .filter(|plugin| plugin.name != MAP || present.map)
            .map(&mut plugin)
            .collect();
        let rest: Vec<_> = rest.into_iter().map(&mut plugin).collect();
        div()
            .flex_shrink_0()
            .flex()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(12.))
            .bg(rgb(RAISED))
            .children(ahead)
            .when(present.changes, |views| {
                views.child(
                    button(
                        "show-changes".into(),
                        View::Changes,
                        "Changes".into(),
                        Icon::Changes,
                    )
                    .children(changed.filter(|&count| count > 0).map(|count| {
                        div()
                            .debug_selector(|| "changed-count".to_owned())
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(ACCENT))
                            .child(count.to_string())
                    }))
                    .on_hint_click(cx.listener(|stage, _, window, cx| {
                        let cwd = stage.cwd.clone();
                        stage.show_changes(cwd, window, cx);
                    })),
                )
            })
            .children(rest)
    }

    /// Opens the menu of the stage's +, with the keyboard in the line at
    /// its top, where a line that is no URL, file or path opens what
    /// `unmatched` says.
    pub(super) fn open_menu(
        &mut self,
        unmatched: Unmatched,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(LineInput::new);
        let engine = coxd::settings::search();
        let searched = engine.clone();
        let typed = cx.subscribe_in(
            &input,
            window,
            move |stage, input, Submitted, window, cx| {
                let line = input.read(cx).text().to_owned();
                stage.close_menu(cx);
                let typed = typed(&line, &stage.cwd, unmatched);
                stage.open_typed(typed, &searched, Follow::NewTab, window, cx);
            },
        );
        window.focus(&input.focus_handle(cx));
        self.menu = Some(Menu {
            input,
            unmatched,
            engine,
            _typed: typed,
        });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        cx.notify();
    }

    /// Opens what the line of Vim's `:open` or `:tabopen` says, sorted as
    /// the menu Vim's `t` opens sorts what is typed in it.
    pub(super) fn open_line(
        &mut self,
        line: &str,
        follow: Follow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let typed = typed(line, &self.cwd, Unmatched::Search);
        self.open_typed(typed, &coxd::settings::search(), follow, window, cx);
    }

    /// Opens `typed`: a page, or the results of a search with `engine`, in the page shown when `follow` says here and a
    /// page is shown, or else on a tab of its own; a file; or a command in
    /// a terminal.
    fn open_typed(
        &mut self,
        typed: Option<Typed>,
        engine: &coxd::settings::SearchEngine,
        follow: Follow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let url = match typed {
            Some(Typed::Web(address)) => address.url(engine),
            Some(Typed::File(path)) => return self.open_typed_file(path, window, cx),
            Some(Typed::Command(command)) => {
                return self.start_terminal(Program::Command(command), window, cx);
            }
            None => return,
        };
        let shown = self
            .tabs
            .shown()
            .and_then(|tab| tab.content.page())
            .cloned();
        match (follow, shown) {
            (Follow::Here, Some(page)) => page.read(cx).load(&url),
            _ => {
                self.open_tab(&url, true, window, cx);
            }
        }
        self.focus_shown(window, cx);
    }

    /// Opens the file at `path` with the keyboard in its tab. A file that
    /// cannot be read gets a tab all the same, which says why.
    fn open_typed_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_path(&path, None, window, cx).is_err() {
            self.show_file::<Code>(path, "", None, window, cx);
            self.focus_shown(window, cx);
        }
    }

    /// The line at the top of the menu, where the human types a URL, a
    /// file, a command, or words to search for, which says so while it is
    /// empty, and, where Vim's `t` opened the menu, what Enter does with
    /// what is typed.
    fn render_menu_line(menu: &Menu, cwd: &std::path::Path, cx: &App) -> impl IntoElement + use<> {
        let input = &menu.input;
        let text = input.read(cx).text();
        let empty = text.is_empty();
        let (placeholder, said) = match menu.unmatched {
            Unmatched::Command => ("URL, file or command", None),
            Unmatched::Search => (
                "URL, file, search or !command",
                typed(text, cwd, Unmatched::Search).map(|typed| typed.said(&menu.engine)),
            ),
        };
        div()
            .px(px(4.))
            .pt(px(4.))
            .pb(px(6.))
            .child(
                div()
                    .debug_selector(|| "stage-menu-line".to_owned())
                    .h(px(32.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(VIEW_RADIUS))
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(ACCENT))
                    .text_size(px(13.))
                    .child(icon(Icon::Search, 13., FAINT))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .child(input.clone())
                            .when(empty, |line| {
                                line.child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left_0()
                                        .text_color(rgb(FAINT))
                                        .child(placeholder),
                                )
                            }),
                    ),
            )
            .children(said.map(|said| {
                div()
                    .debug_selector(|| "stage-menu-says".to_owned())
                    .px(px(10.))
                    .pt(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(FAINT))
                    .truncate()
                    .child(said)
            }))
    }

    /// The menu of what the human opens on the stage: the line where they
    /// type a URL, a file, a command or words to search for, then a page, a terminal, a file,
    /// the Android emulator's devices, or, on macOS, other apps' windows.
    fn render_menu(
        menu: &Menu,
        cwd: &std::path::Path,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let line = Self::render_menu_line(menu, cwd, cx);
        let item = |id: &'static str, label: &'static str, glyph: Icon, key: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .h(px(34.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(VIEW_RADIUS))
                .text_size(px(13.))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(SELECTED)))
                .child(icon(glyph, 15., DIM))
                .child(div().flex_1().child(label))
                .child(div().text_size(px(12.)).text_color(rgb(FAINT)).child(key))
        };
        let close = |stage: &mut Self, cx: &mut Context<Self>| stage.close_menu(cx);
        deferred(
            div()
                .id("stage-menu-items")
                .key_context(MENU_CONTEXT)
                .child(crate::vim::hints::cover())
                .occlude()
                .absolute()
                .top(px(48.))
                .right(px(10.))
                .w(px(260.))
                .p(px(6.))
                .flex()
                .flex_col()
                .bg(rgb(RAISED))
                .border_1()
                .border_color(rgb(BORDER_STRONG))
                .rounded(px(14.))
                .shadow_lg()
                .text_color(rgb(FOREGROUND))
                .on_mouse_down_out(cx.listener(move |stage, _, _, cx| close(stage, cx)))
                .on_action(cx.listener(move |stage, _: &CloseMenu, window, cx| {
                    close(stage, cx);
                    stage.focus_shown(window, cx);
                }))
                .child(line)
                .child(
                    item("new-tab", "Web page", Icon::Globe, NEW_PAGE_KEY).on_hint_click(
                        cx.listener(move |stage, _, window, cx| {
                            close(stage, cx);
                            stage.new_tab(window, cx);
                        }),
                    ),
                )
                .child(
                    item("new-terminal", "Terminal", Icon::Terminal, NEW_TERMINAL_KEY)
                        .on_hint_click(cx.listener(move |stage, _, window, cx| {
                            close(stage, cx);
                            stage.new_terminal(window, cx);
                        })),
                )
                .child(
                    item("open-files", "Files", Icon::Files, OPEN_FILE_KEY).on_hint_click(
                        cx.listener(move |stage, _, window, cx| {
                            close(stage, cx);
                            stage.new_files(window, cx);
                        }),
                    ),
                )
                .child(
                    item("android-devices", "Android devices", Icon::Phone, "").on_hint_click(
                        cx.listener(move |stage, _, window, cx| {
                            close(stage, cx);
                            stage.show_devices(window, cx);
                        }),
                    ),
                )
                // Other apps' windows are shown on macOS only.
                .when(cfg!(target_os = "macos"), |menu| {
                    menu.child(
                        item("mac-windows", "Mac windows", Icon::Window, "").on_hint_click(
                            cx.listener(move |stage, _, window, cx| {
                                close(stage, cx);
                                stage.show_windows(window, cx);
                            }),
                        ),
                    )
                }),
        )
        .with_priority(2)
    }
}

/// Counts the files that changed in the project of `cx`'s stage, looks
/// whether it has a map, and counts the chats of the folder that wait for
/// the human, now and then, while the stage is open, for the bar to show
/// "Changes" only in a Git repository, or a project with one among its
/// directories (see [`coxd::repositories::of`]), with the count of
/// the files of all of them, "Map" only for a project with a map, and the
/// count on "Activity".
pub(super) fn count_changes(cx: &mut Context<Stage>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let Ok(open) = this.read_with(cx, |stage, cx| {
                stage.panel.read(cx).is_open().then(|| stage.cwd.clone())
            }) else {
                break;
            };
            if let Some(cwd) = open {
                let (changed, mapped, waiting) = cx
                    .background_executor()
                    .spawn(async move {
                        let mapped = !cox_editor::map::list(&cwd).is_empty();
                        let waiting = super::activity::waiting(&cwd);
                        (coxd::repositories::count(&cwd).ok(), mapped, waiting)
                    })
                    .await;
                let counted = this.update(cx, |stage, cx| {
                    if (stage.changed, stage.mapped, stage.waiting) != (changed, mapped, waiting) {
                        stage.changed = changed;
                        stage.mapped = mapped;
                        stage.waiting = waiting;
                        cx.notify();
                    }
                });
                if counted.is_err() {
                    break;
                }
            }
            cx.background_executor().timer(COUNT_EVERY).await;
        }
    })
}

/// A button of a view of the bar, `id`, with its icon, and its `label`
/// beside it when `chosen`, which it is marked as, or else where the
/// mouse rests on it.
fn view_button(
    chosen: bool,
    id: SharedString,
    label: SharedString,
    glyph: Icon,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.clone())
        .debug_selector(move || id.to_string())
        .flex_shrink_0()
        .h(px(VIEW_HEIGHT))
        .min_w(px(VIEW_HEIGHT + 2.))
        .px(px(if chosen { 11. } else { 8. }))
        .flex()
        .justify_center()
        .items_center()
        .gap(px(7.))
        .rounded(px(VIEW_RADIUS))
        .text_size(px(13.))
        .cursor_pointer()
        .map(|button| {
            if chosen {
                button.bg(rgb(SELECTED)).text_color(rgb(FOREGROUND))
            } else {
                button
                    .text_color(rgb(DIM))
                    .hover(|style| style.text_color(rgb(FOREGROUND)))
            }
        })
        .child(icon(glyph, 14., if chosen { FOREGROUND } else { DIM }))
        .map(|button| {
            if chosen {
                button.child(label)
            } else {
                button.tooltip(tip(label))
            }
        })
}
