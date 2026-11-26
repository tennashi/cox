//! The chooser of the repositories of a project's folder and of their
//! checkouts (see [`coxd::repositories`]), one kind wherever it is drawn:
//! under the heading of the tab of what changed (see
//! [`crate::stage::changes`]), and above the tree of the files (see
//! [`crate::stage::tree`]). The repositories are side by side, each by
//! its name, with what its checkout shown goes by, the one shown marked;
//! under them, a button saying what the checkout shown goes by opens the
//! menu of its checkouts, its own first, each with where it is, marked
//! as Cox's, made by hand, or outside the folder, and how many files
//! changed in it, once counted.
//!
//! A view that draws a chooser keeps the menus open under it in
//! [`Menus`], and answers a press on a menu's button as a [`Chooser`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    AnchoredPositionMode, Context, Div, FontWeight, MouseDownEvent, Pixels, Point, Stateful,
    anchored, deferred, div, point, prelude::*, px, rgb,
};

use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::vim::hints::Hinted as _;
use coxd::paths::shortened;
use coxd::repositories::Checkout;
use coxd::theme::{
    ACCENT, ASKING, BORDER, CODE_FONT, DIM, FAINT, FOREGROUND, HOVER, RAISED, SELECTED, UI_FONT,
};

/// How tall a button that opens a menu under the repositories is.
pub(crate) const MENU_BUTTON_HEIGHT: f32 = 26.;

/// The menus of kind `M` a view opens under its chooser: the one open, if
/// one is, and the one a press last closed, and where the press was,
/// which on its button leaves it closed.
pub(crate) struct Menus<M> {
    open: Option<M>,
    closed: Option<(M, Point<Pixels>)>,
}

impl<M> Default for Menus<M> {
    fn default() -> Self {
        Self {
            open: None,
            closed: None,
        }
    }
}

impl<M: Copy + PartialEq> Menus<M> {
    /// Whether `menu` is open.
    pub(crate) fn is_open(&self, menu: M) -> bool {
        self.open == Some(menu)
    }

    /// Closes the menu open, if one is.
    pub(crate) fn close(&mut self) {
        self.open = None;
    }

    /// Opens `menu`, or closes it if it is open: answers whether it
    /// opened.
    pub(crate) fn toggle(&mut self, menu: M) -> bool {
        let open = self.open != Some(menu);
        self.open = open.then_some(menu);
        open
    }
}

/// A view that draws a chooser, and opens menus under it.
pub(crate) trait Chooser: Sized + 'static {
    /// The menus it opens.
    type Menu: Copy + PartialEq + 'static;

    /// The menus open under its chooser.
    fn menus(&mut self) -> &mut Menus<Self::Menu>;

    /// Opens `menu`, reading what it lists, or closes it, if it is open,
    /// as a press on its button asks.
    fn toggle_menu(&mut self, menu: Self::Menu, cx: &mut Context<Self>);
}

/// A repository of the chooser: its `name`, what its checkout shown goes
/// by, and whether that is a worktree, if it says, a `mark` after them in
/// its color, if any, and whether it is the one shown.
pub(crate) struct Chip {
    pub name: String,
    pub checkout: Option<(String, bool)>,
    pub mark: Option<(String, u32)>,
    pub chosen: bool,
}

/// The repositories `chips` side by side, each found as `{id}-{index}`,
/// a click on one picking it, by its index, with `pick`.
pub(crate) fn chips<T: 'static>(
    id: &'static str,
    chips: Vec<Chip>,
    pick: impl Fn(&mut T, usize, &mut Context<T>) + 'static,
    cx: &mut Context<T>,
) -> Div {
    let pick = Rc::new(pick);
    let chips = chips.into_iter().enumerate().map(|(index, chip)| {
        let pick = Rc::clone(&pick);
        chip_of(chip)
            .id((id, index))
            .debug_selector(move || format!("{id}-{index}"))
            .on_hint_click(cx.listener(move |view, _, _, cx| pick(view, index, cx)))
    });
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(2.))
        .p(px(3.))
        .rounded(px(11.))
        .bg(rgb(RAISED))
        .children(chips)
}

/// The chip of `chip`.
fn chip_of(chip: Chip) -> Div {
    let checkout = chip.checkout.map(|(label, worktree)| {
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(11.5))
            .text_color(rgb(FAINT))
            .when(worktree, |label| label.child(icon(Icon::Flow, 11., FAINT)))
            .child(label)
    });
    div()
        .flex_shrink_0()
        .h(px(26.))
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .rounded(px(8.))
        .font_family(CODE_FONT)
        .text_size(px(12.5))
        .cursor_pointer()
        .map(|chip_div| {
            if chip.chosen {
                chip_div.bg(rgb(SELECTED)).text_color(rgb(FOREGROUND))
            } else {
                chip_div
                    .text_color(rgb(DIM))
                    .hover(|style| style.bg(rgb(HOVER)))
            }
        })
        .child(chip.name)
        .children(checkout)
        .children(chip.mark.map(|(mark, color)| {
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(color))
                .child(mark)
        }))
}

/// A button under the repositories, found as `id`, which opens `menu`, or
/// closes it, drawn as open while `open`.
pub(crate) fn menu_button<T: Chooser>(
    id: &'static str,
    menu: T::Menu,
    open: bool,
    cx: &mut Context<T>,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .flex_shrink_0()
        .h(px(MENU_BUTTON_HEIGHT))
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(9.))
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(if open { SELECTED } else { RAISED }))
        .font_family(CODE_FONT)
        .text_size(px(12.))
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(HOVER)))
        .on_hint_mouse_down(
            cx.listener(move |view: &mut T, event: &MouseDownEvent, _, cx| {
                // The press that closed the menu, on its button, leaves it
                // closed.
                if view.menus().closed.take() != Some((menu, event.position)) {
                    view.toggle_menu(menu, cx);
                }
                cx.stop_propagation();
            }),
        )
}

/// The button, found as `id`, saying what `checkout` goes by, and
/// whether it is a worktree, which opens `menu`, the menu of the
/// checkouts, drawn as open while `open`; a branch no checkout has is
/// marked `branch_tag`.
pub(crate) fn checkout_button<T: Chooser>(
    id: &'static str,
    menu: T::Menu,
    open: bool,
    checkout: &Checkout,
    branch_tag: &'static str,
    cx: &mut Context<T>,
) -> Stateful<Div> {
    menu_button(id, menu, open, cx)
        .child(icon(checkout_icon(checkout), 12., DIM))
        .child(checkout.label())
        .children(checkout_tags(checkout, branch_tag))
        .child(caret())
}

/// A menu under the repositories, `menu`, found as `id`, headed
/// `heading`, which a press anywhere else closes.
pub(crate) fn menu_list<T: Chooser>(
    id: &'static str,
    menu: T::Menu,
    heading: String,
    cx: &mut Context<T>,
) -> Stateful<Div> {
    crate::ui::menu::menu()
        .id(id)
        .debug_selector(move || id.to_owned())
        .min_w(px(320.))
        .max_w(px(640.))
        .max_h(px(360.))
        .overflow_y_scroll()
        .on_mouse_down_out(
            cx.listener(move |view: &mut T, event: &MouseDownEvent, _, cx| {
                let menus = view.menus();
                menus.open = None;
                menus.closed = Some((menu, event.position));
                cx.notify();
            }),
        )
        .child(
            div()
                .px(px(10.))
                .pt(px(4.))
                .pb(px(2.))
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(FAINT))
                .child(heading),
        )
}

/// The menu `menu` of `checkouts`, the checkouts of the repository
/// `name`, found as `id`, its rows as `{choice}-{index}`: each with where
/// it is, from `folder` when it is in it, what marks it, and how many
/// files changed in it, as `counted` has them; the one at `shown`
/// marked, and a click on one choosing it with `choose`. The branches no
/// checkout has, if any, come last, under a heading of their own, each
/// marked `branch_tag`.
#[expect(
    clippy::too_many_arguments,
    reason = "what the menu lists, and where, as each view has them"
)]
pub(crate) fn checkouts_menu<T: Chooser>(
    id: &'static str,
    choice: &'static str,
    menu: T::Menu,
    name: Option<&str>,
    checkouts: &[Checkout],
    shown: Option<usize>,
    folder: &Path,
    counted: &HashMap<PathBuf, Result<usize, String>>,
    branch_tag: &'static str,
    choose: impl Fn(&mut T, &Checkout, &mut Context<T>) + 'static,
    cx: &mut Context<T>,
) -> Stateful<Div> {
    let heading = match name {
        Some(name) => format!("CHECKOUTS OF {}", name.to_uppercase()),
        None => "CHECKOUTS".to_owned(),
    };
    let choose = Rc::new(choose);
    let first_branch = checkouts
        .iter()
        .position(|checkout| checkout.branch().is_some());
    let rows: Vec<Div> = checkouts
        .iter()
        .enumerate()
        .map(|(index, checkout)| {
            let at = checkout.folder().to_owned();
            let counted = match checkout.branch().map_or_else(|| counted.get(&at), |_| None) {
                Some(Ok(0)) => "clean".to_owned(),
                Some(Ok(1)) => "1 changed".to_owned(),
                Some(Ok(count)) => format!("{count} changed"),
                Some(Err(_)) => "!".to_owned(),
                None => String::new(),
            };
            let label = div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon(checkout_icon(checkout), 12., DIM))
                .child(
                    div()
                        .flex_shrink_0()
                        .font_family(CODE_FONT)
                        .text_size(px(12.5))
                        .child(checkout.label()),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(11.5))
                        .text_color(rgb(FAINT))
                        .child(ellipsis(where_is(&at, folder))),
                )
                .children(checkout_tags(checkout, branch_tag));
            let choose = Rc::clone(&choose);
            let chosen = checkout.clone();
            let row = crate::ui::menu::item((choice, index), label, &counted)
                .when(Some(index) == shown, |row| row.bg(rgb(SELECTED)))
                .on_hint_click(cx.listener(move |view, _, _, cx| choose(view, &chosen, cx)));
            let heading = (Some(index) == first_branch).then(|| {
                div()
                    .mt(px(4.))
                    .px(px(10.))
                    .pt(px(6.))
                    .pb(px(2.))
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(FAINT))
                    .child("BRANCHES NO CHECKOUT HAS")
            });
            div().children(heading).child(row)
        })
        .collect();
    menu_list(id, menu, heading, cx).children(rows)
}

/// How many files changed in each checkout at `folders`, by its folder,
/// as Git counts them; slow, as it runs Git for each.
pub(crate) fn count(folders: Vec<PathBuf>) -> HashMap<PathBuf, Result<usize, String>> {
    folders
        .into_iter()
        .map(|folder| {
            let count = cox_git::changes::count(&folder);
            (folder, count)
        })
        .collect()
}

/// The icon of `checkout`: a branch's for a repository's own, a
/// worktree's for another.
pub(crate) fn checkout_icon(checkout: &Checkout) -> Icon {
    match checkout {
        Checkout::Own { .. } | Checkout::Branch { .. } => Icon::Changes,
        Checkout::Worktree(_) => Icon::Flow,
    }
}

/// `button`, with `menu` under it, if it is open, kept in the window.
pub(crate) fn under(button: Stateful<Div>, menu: Option<Stateful<Div>>) -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .child(button)
        .children(menu.map(|menu| {
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Local)
                    .position(point(px(0.), px(MENU_BUTTON_HEIGHT + 4.)))
                    .snap_to_window()
                    .child(menu),
            )
            .with_priority(2)
        }))
}

/// The mark at the end of a button that opens a menu.
pub(crate) fn caret() -> Div {
    div().text_size(px(10.)).text_color(rgb(FAINT)).child("▾")
}

/// A word that marks what a button or a row of a menu names, as `said`,
/// in `color`.
pub(crate) fn tag(said: String, color: u32) -> Div {
    div()
        .flex_shrink_0()
        .px(px(5.))
        .rounded(px(4.))
        .border_1()
        .border_color(rgb(BORDER))
        .font_family(UI_FONT)
        .text_size(px(10.5))
        .text_color(rgb(color))
        .child(said)
}

/// What marks `checkout`: for a worktree, the chat Cox made it for, or
/// that it was made by hand, and whether it is outside the folder; for a
/// branch no checkout has, `branch_tag`, which says how it is seen.
pub(crate) fn checkout_tags(checkout: &Checkout, branch_tag: &'static str) -> Vec<Div> {
    let worktree = match checkout {
        Checkout::Worktree(worktree) => worktree,
        Checkout::Branch { .. } => return vec![tag(branch_tag.to_owned(), FAINT)],
        Checkout::Own { .. } => return Vec::new(),
    };
    let made = match worktree.made_for {
        Some(chat) => tag(format!("chat {chat}"), ACCENT),
        None => tag("by hand".to_owned(), FAINT),
    };
    std::iter::once(made)
        .chain(worktree.outside.then(|| tag("outside".to_owned(), ASKING)))
        .collect()
}

/// Where `checkout`, a folder, is: from `folder` when it is in it, or
/// else as people read a path.
fn where_is(checkout: &Path, folder: &Path) -> String {
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_owned());
    canonical(checkout)
        .strip_prefix(canonical(folder))
        .ok()
        .filter(|within| !within.as_os_str().is_empty())
        .map_or_else(
            || shortened(checkout),
            |within| within.display().to_string(),
        )
}

/// The name of `folder`, its last part's.
pub(crate) fn name_of(folder: &Path) -> String {
    folder.file_name().map_or_else(
        || folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
