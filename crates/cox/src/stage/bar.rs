//! The bar along the top of the stage: the views of the project it keeps
//! in sight, the project's files, what changed in them, and the pages of
//! the plugins, such as the map; then the tabs of the rest, each with an
//! icon of what it shows; and, at its end, the menu that opens another,
//! among them an Android emulator of each virtual device the SDK has.
//! The button of what changed counts the files that did, while the stage
//! is open.

use std::cell::Cell;
use std::path::PathBuf;
use std::time::Duration;

use cox_editor::changes;
use gpui::{
    App, Context, Div, Entity, Focusable, FontWeight, PathPromptOptions, ScrollHandle,
    SharedString, Stateful, Subscription, Task, Window, actions, deferred, div, prelude::*, px,
    rgb,
};

use super::code::Code;
use super::plugins::Plugin;
use super::tab::{Tab, TabContent};
use super::terminal::Program;
use super::typed::{Typed, typed};
use super::{Stage, TAB_WIDTH, TITLE_LENGTH};
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{LineInput, Submitted};
use crate::ui::theme::{
    ACCENT, BORDER, BORDER_STRONG, BUTTON_HOVER, DIM, FAINT, FOREGROUND, HOVER, RAISED, SELECTED,
    SURFACE,
};

/// Where the tabs scroll along the bar, when there are more than it has
/// room for, which keeps the tab shown in sight as another is shown.
#[derive(Default)]
pub(super) struct Strip {
    scroll: ScrollHandle,
    /// The tab the strip last scrolled to, so that it scrolls once each
    /// time another is shown, and the human can scroll it after.
    shown: Cell<Option<usize>>,
}

impl Strip {
    /// Scrolls to the tab shown, `shown`, at `place` among the tabs the
    /// strip has, unless it scrolled to it already.
    fn follow(&self, shown: Option<usize>, place: Option<usize>) {
        if self.shown.replace(shown) == shown {
            return;
        }
        if let Some(place) = place {
            self.scroll.scroll_to_item(place);
        }
    }
}

/// How tall a view's button is, and a tab, and how round each is.
const VIEW_HEIGHT: f32 = 28.;
const TAB_HEIGHT: f32 = 32.;
const VIEW_RADIUS: f32 = 9.;
const TAB_RADIUS: f32 = 10.;

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
pub(super) const MENU_CONTEXT: &str = "StageMenu";

/// The menu of the stage's +, while it is open: the line at its top,
/// where the human types a URL, a file or a command.
pub(super) struct Menu {
    input: Entity<LineInput>,
    _typed: Subscription,
}

/// A view of the project that the bar keeps in sight, apart from the tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum View {
    Files,
    Changes,
    Plugin(String),
}

impl Stage {
    /// The view of the project `tab` shows, if it shows one: the tree of
    /// the project's files, what changed in them, or a plugin's page.
    fn view_of(&self, tab: &Tab, plugins: &[Plugin], cx: &App) -> Option<View> {
        match &tab.content {
            TabContent::Tree(_) => Some(View::Files),
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

    pub(super) fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let plugins = self.plugins.list();
        let shown = self
            .tabs
            .shown()
            .and_then(|tab| self.view_of(tab, &plugins, cx));
        let rest: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| self.view_of(tab, &plugins, cx).is_none())
            .map(|(index, _)| index)
            .collect();
        let shown_index = self.tabs.shown_index();
        self.strip.follow(
            shown_index,
            rest.iter().position(|&index| Some(index) == shown_index),
        );
        let tabs: Vec<_> = rest
            .into_iter()
            .map(|index| self.render_tab(index, &self.tabs[index], cx))
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
                self.changed,
                cx,
            ))
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(1.))
                    .h(px(18.))
                    .mx(px(6.))
                    .bg(rgb(BORDER)),
            )
            .child(
                div()
                    .id("stage-tabs")
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_x_scroll()
                    .track_scroll(&self.strip.scroll)
                    .children(tabs),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("stage-menu")
                    .debug_selector(|| "stage-menu".to_owned())
                    .flex_shrink_0()
                    .size(px(TAB_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(TAB_RADIUS))
                    .text_color(rgb(FOREGROUND))
                    .cursor_pointer()
                    .when(self.menu.is_some(), |button| button.bg(rgb(SELECTED)))
                    .hover(|style| style.bg(rgb(HOVER)))
                    .child(icon(Icon::Plus, 15., FOREGROUND))
                    // The menu closes as the mouse goes down outside it,
                    // here too, so the button opens it as it comes up.
                    .on_click(cx.listener(|stage, _, window, cx| stage.open_menu(window, cx))),
            )
            .children(
                self.menu
                    .as_ref()
                    .map(|menu| Self::render_menu(&menu.input, cx)),
            )
    }

    /// The buttons of the views of the project, side by side in one
    /// ground, the one shown marked.
    fn render_views(
        shown: Option<&View>,
        plugins: Vec<Plugin>,
        changed: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let button = |id: SharedString, view: View, label: SharedString, glyph: Icon| {
            let chosen = shown == Some(&view);
            div()
                .id(id.clone())
                .debug_selector(move || id.to_string())
                .flex_shrink_0()
                .h(px(VIEW_HEIGHT))
                .px(px(11.))
                .flex()
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
                .child(label)
        };
        let plugins = plugins.into_iter().map(|plugin| {
            let glyph = match plugin.name.as_str() {
                "map" => Icon::Map,
                "wiki" => Icon::Book,
                _ => Icon::Plug,
            };
            let name = plugin.name.clone();
            button(
                format!("plugin-{}", plugin.name).into(),
                View::Plugin(plugin.name),
                plugin.title.into(),
                glyph,
            )
            .on_click(cx.listener(move |stage, _, window, cx| {
                stage.show_plugin(&name, "", window, cx);
            }))
        });
        div()
            .flex_shrink_0()
            .flex()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(12.))
            .bg(rgb(RAISED))
            .child(
                button("show-tree".into(), View::Files, "Files".into(), Icon::Files)
                    .on_click(cx.listener(|stage, _, window, cx| stage.show_tree(window, cx))),
            )
            .child(
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
                .on_click(cx.listener(|stage, _, window, cx| {
                    let cwd = stage.cwd.clone();
                    stage.show_changes(cwd, window, cx);
                })),
            )
            .children(plugins)
    }

    /// The tab at `index`: an icon of what it shows, its title, and the
    /// button that closes it, unless it is the last.
    fn render_tab(&self, index: usize, tab: &Tab, cx: &mut Context<Self>) -> Stateful<Div> {
        let number = index + 1;
        let title = tab.content.title(cx);
        let title: String = title.chars().take(TITLE_LENGTH).collect();
        let glyph = match &tab.content {
            TabContent::Page { .. } => Icon::Globe,
            TabContent::Document(_) => Icon::Document,
            TabContent::Code(_) => Icon::Code,
            TabContent::Tree(_) => Icon::Files,
            TabContent::Changes(_) => Icon::Changes,
            TabContent::Flow(_) => Icon::Flow,
            TabContent::Terminal(_) => Icon::Terminal,
            TabContent::Android(_) | TabContent::Devices(_) => Icon::Phone,
            TabContent::MacWindow(_) | TabContent::Windows(_) => Icon::Window,
        };
        let shown = self.tabs.shown_index() == Some(index);
        // The tabs of the chats not shown are drawn dimmer.
        let others = tab.owner.is_some_and(|owner| Some(owner) != self.showing);
        div()
            .id(("tab", number))
            .debug_selector(move || format!("tab-{number}"))
            .flex_shrink_0()
            .max_w(px(TAB_WIDTH))
            .h(px(TAB_HEIGHT))
            .pl(px(10.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(7.))
            .rounded(px(TAB_RADIUS))
            .text_size(px(13.))
            .cursor_pointer()
            .map(|tab| {
                if shown {
                    tab.bg(rgb(SELECTED)).text_color(rgb(FOREGROUND))
                } else {
                    tab.text_color(rgb(DIM)).hover(|style| style.bg(rgb(HOVER)))
                }
            })
            .when(others, |tab| tab.opacity(0.5))
            .on_click(cx.listener(move |stage, _, window, cx| stage.show(index, window, cx)))
            .child(icon(glyph, 14., if shown { DIM } else { FAINT }))
            .child(div().min_w_0().overflow_hidden().child(ellipsis(title)))
            .when(self.tabs.len() > 1, |tab| {
                tab.child(
                    div()
                        .id(("close-tab", number))
                        .debug_selector(move || format!("close-tab-{number}"))
                        .flex_shrink_0()
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .text_color(rgb(FAINT))
                        .hover(|style| style.bg(rgb(BUTTON_HOVER)).text_color(rgb(FOREGROUND)))
                        .child(icon(Icon::Close, 10., FAINT))
                        .on_click(cx.listener(move |stage, _, window, cx| {
                            cx.stop_propagation();
                            let _ = stage.close_tab(index, window, cx);
                        })),
                )
            })
    }

    /// Opens the menu of the stage's +, with the keyboard in the line at
    /// its top.
    fn open_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(LineInput::new);
        let typed = cx.subscribe_in(&input, window, |stage, input, Submitted, window, cx| {
            let line = input.read(cx).text().to_owned();
            stage.close_menu(cx);
            stage.open_typed(&line, window, cx);
        });
        window.focus(&input.focus_handle(cx));
        self.menu = Some(Menu {
            input,
            _typed: typed,
        });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        cx.notify();
    }

    /// Opens what the human typed in the menu: a file, a page, or a
    /// command in a terminal (see [`typed`]).
    fn open_typed(&mut self, line: &str, window: &mut Window, cx: &mut Context<Self>) {
        match typed(line, &self.cwd) {
            Some(Typed::Page(url)) => {
                self.open_tab(&url, true, window, cx);
                self.focus_shown(window, cx);
            }
            Some(Typed::File(path)) => {
                // A file that cannot be read gets a tab all the same,
                // which says why.
                if self.open_path(&path, None, window, cx).is_err() {
                    self.show_file::<Code>(path, "", None, window, cx);
                    self.focus_shown(window, cx);
                }
            }
            Some(Typed::Command(command)) => {
                self.start_terminal(Program::Command(command), window, cx);
            }
            None => {}
        }
    }

    /// The line at the top of the menu, where the human types a URL, a
    /// file or a command, which says so while it is empty.
    fn render_menu_line(input: &Entity<LineInput>, cx: &App) -> impl IntoElement + use<> {
        let empty = input.read(cx).text().is_empty();
        div().px(px(4.)).pt(px(4.)).pb(px(6.)).child(
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
                                    .child("URL, file or command"),
                            )
                        }),
                ),
        )
    }

    /// The menu of what the human opens on the stage: the line where they
    /// type a URL, a file or a command, then a page, a terminal, a file,
    /// the Android emulator's devices, or, on macOS, other apps' windows.
    fn render_menu(input: &Entity<LineInput>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let line = Self::render_menu_line(input, cx);
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
                    item("new-tab", "Web page", Icon::Globe, NEW_PAGE_KEY).on_click(cx.listener(
                        move |stage, _, window, cx| {
                            close(stage, cx);
                            stage.new_tab(window, cx);
                        },
                    )),
                )
                .child(
                    item("new-terminal", "Terminal", Icon::Terminal, NEW_TERMINAL_KEY).on_click(
                        cx.listener(move |stage, _, window, cx| {
                            close(stage, cx);
                            stage.new_terminal(window, cx);
                        }),
                    ),
                )
                .child(
                    item("open-file", "File…", Icon::Code, OPEN_FILE_KEY).on_click(cx.listener(
                        move |stage, _, window, cx| {
                            close(stage, cx);
                            Self::choose_file(window, cx);
                        },
                    )),
                )
                .child(
                    item("android-devices", "Android devices", Icon::Phone, "").on_click(
                        cx.listener(move |stage, _, window, cx| {
                            close(stage, cx);
                            stage.show_devices(window, cx);
                        }),
                    ),
                )
                // Other apps' windows are shown on macOS only.
                .when(cfg!(target_os = "macos"), |menu| {
                    menu.child(
                        item("mac-windows", "Mac windows", Icon::Window, "").on_click(cx.listener(
                            move |stage, _, window, cx| {
                                close(stage, cx);
                                stage.show_windows(window, cx);
                            },
                        )),
                    )
                }),
        )
        .with_priority(2)
    }

    /// Asks for a file, and opens it on a tab, as a link to it does.
    pub fn choose_file(window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let path: PathBuf = match chosen.await {
                Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.swap_remove(0),
                Ok(Err(error)) => {
                    tracing::error!("no file chosen: {error:#}");
                    return;
                }
                _ => return,
            };
            this.update_in(cx, |stage, window, cx| {
                // A file that cannot be read gets a tab all the same,
                // which says why.
                if stage.open_path(&path, None, window, cx).is_err() {
                    stage.show_file::<Code>(path, "", None, window, cx);
                    stage.focus_shown(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}

/// Counts the files that changed in the project of `cx`'s stage, now and
/// then, while the stage is open, for its button to show.
pub(super) fn count_changes(cx: &mut Context<Stage>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let Ok(open) = this.read_with(cx, |stage, cx| {
                stage.panel.read(cx).is_open().then(|| stage.cwd.clone())
            }) else {
                break;
            };
            if let Some(cwd) = open {
                let changed = cx
                    .background_executor()
                    .spawn(async move { changes::count(&cwd).ok() })
                    .await;
                let counted = this.update(cx, |stage, cx| {
                    if stage.changed != changed {
                        stage.changed = changed;
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
