//! The phone on the settings screen: where a phone follows the chats,
//! and the code it pairs with, so that the human pairs one without a
//! terminal: a phone scans the code, which opens the link that pairs it
//! (see [`crate::daemon::phone::pairing`]). The code holds the secret, so
//! it shows only once the human asks for it, and goes as they hide it,
//! or as the screen closes.
//!
//! A phone lost, or a code seen by someone else, the human unpairs every
//! phone: coxd makes the secret again, and lets in no phone paired with
//! the one before (see [`crate::daemon::wire::ToDaemon::UnpairPhones`]).
//! The code, if it showed, shows again, with the new secret.

use gpui::{
    Bounds, ClipboardItem, Context, Div, EventEmitter, Window, canvas, div, fill, point,
    prelude::*, px, rgb, size,
};
use qrcodegen::{QrCode, QrCodeEcc};

use super::words::{Part, Words};
use crate::daemon::phone::{Pairing, pairing};
use crate::ui::button::{Kind, button};
use crate::ui::theme::{ACCENT, ASKING, BORDER, DIM, FAILED, FAINT, FOREGROUND};
use crate::vim::hints::Hinted as _;

/// How wide the code is drawn, its quiet edge too.
const CODE_SIZE: f32 = 184.;

/// How many modules of quiet edge the code has around it, as readers
/// want.
const QUIET: i32 = 4;

/// What the section shows.
enum Shown {
    /// The button that shows the code.
    Hidden,
    /// Tailscale is asked where the page is.
    Finding,
    /// The code, with where the page is.
    Code { pairing: Pairing, code: QrCode },
    /// Why there is no code.
    Failed(String),
    /// Every phone is being unpaired, the code to show again after, with
    /// the new secret, if `show`.
    Unpairing { show: bool },
}

/// The human asked to unpair every phone.
pub struct UnpairPhones;

/// The phone, to pair.
pub struct PhoneSection {
    shown: Shown,
    /// Whether the link was copied, since the code showed.
    copied: bool,
    /// Whether every phone was unpaired, since the screen opened.
    unpaired: bool,
    words: Words,
}

impl PhoneSection {
    /// The section, with the code hidden, writing what it says in
    /// `words`.
    pub fn new(words: Words) -> Self {
        Self {
            shown: Shown::Hidden,
            copied: false,
            unpaired: false,
            words,
        }
    }

    /// What the section shows, for `cox --inspect state`: never the
    /// secret.
    #[must_use]
    pub fn described(&self) -> String {
        let shown = match &self.shown {
            Shown::Hidden => "Phone: the pairing code hidden".to_owned(),
            Shown::Finding => "Phone: asking Tailscale where the page is".to_owned(),
            Shown::Code { pairing, .. } => {
                format!("Phone: the pairing code shown, for {}", pairing.page)
            }
            Shown::Failed(why) => format!("Phone: no pairing code — {why}"),
            Shown::Unpairing { .. } => "Phone: unpairing every phone".to_owned(),
        };
        if self.unpaired {
            shown + "; every phone unpaired"
        } else {
            shown
        }
    }

    /// Asks Tailscale where the page is, apart from the window, which it
    /// may take seconds to answer, and shows the code once it does.
    fn show(&mut self, cx: &mut Context<Self>) {
        if matches!(self.shown, Shown::Finding | Shown::Unpairing { .. }) {
            return;
        }
        self.shown = Shown::Finding;
        self.copied = false;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async move { pairing() }).await;
            this.update(cx, |section, cx| {
                // Hidden meanwhile, it stays hidden.
                if !matches!(section.shown, Shown::Finding) {
                    return;
                }
                section.shown = match found {
                    Ok(pairing) => match QrCode::encode_text(&pairing.pair, QrCodeEcc::Medium) {
                        Ok(code) => Shown::Code { pairing, code },
                        Err(error) => Shown::Failed(format!("Cox cannot make the code: {error}")),
                    },
                    Err(error) => Shown::Failed(format!("{error:#}")),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What the section says beside the code `pairing` is drawn as.
    fn told(&self, pairing: &Pairing, words: &mut Part, cx: &mut Context<Self>) -> Div {
        let opens = match &pairing.login {
            Some(login) => format!(
                "A phone logged in to Tailscale as {login} scans the code once, keeps \
                 what it pairs with, and opens {} from then on.",
                pairing.page
            ),
            None => format!(
                "Tailscale does not say this Mac's name in a tailnet, so the code \
                 opens the page on this Mac alone, at {}.",
                pairing.page
            ),
        };
        let mut told = div().flex_1().min_w_0().flex().flex_col().gap_2().child(
            div()
                .text_size(px(13.))
                .text_color(rgb(FOREGROUND))
                .child(words.about(&opens)),
        );
        if pairing.login.is_some() && !pairing.magic_dns {
            told = told.child(words.note(
                "MagicDNS is off in the tailnet: turn it on for the phone to find the \
                 Mac by its name.",
                ASKING,
            ));
        }
        told = told
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button(
                            "copy-pairing",
                            if self.copied { "Copied" } else { "Copy link" },
                            Kind::Plain,
                        )
                        .on_hint_click(cx.listener(|section, _, _, cx| section.copy(cx))),
                    )
                    .child(
                        button("hide-pairing", "Hide", Kind::Quiet)
                            .on_hint_click(cx.listener(|section, _, _, cx| section.hide(cx))),
                    )
                    .child(unpair_button(cx)),
            )
            .when(self.copied, |told| {
                told.child(words.note(
                    "Copied: paste it into the page a home-screen app asks for it on.",
                    ACCENT,
                ))
            });
        told
    }

    /// Asks coxd to unpair every phone, showing the code again after, if
    /// it shows now.
    fn unpair(&mut self, cx: &mut Context<Self>) {
        if matches!(self.shown, Shown::Finding | Shown::Unpairing { .. }) {
            return;
        }
        let show = matches!(self.shown, Shown::Code { .. });
        self.shown = Shown::Unpairing { show };
        self.copied = false;
        cx.emit(UnpairPhones);
        cx.notify();
    }

    /// Every phone is unpaired, unless `failed` says why not.
    pub fn unpaired(&mut self, failed: Option<String>, cx: &mut Context<Self>) {
        let Shown::Unpairing { show } = self.shown else {
            return;
        };
        if let Some(why) = failed {
            self.shown = Shown::Failed(format!("The phones are still paired: {why}"));
            cx.notify();
            return;
        }
        self.unpaired = true;
        self.shown = Shown::Hidden;
        if show {
            self.show(cx);
        }
        cx.notify();
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        self.shown = Shown::Hidden;
        self.copied = false;
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Shown::Code { pairing, .. } = &self.shown {
            cx.write_to_clipboard(ClipboardItem::new_string(pairing.pair.clone()));
            self.copied = true;
            cx.notify();
        }
    }
}

/// The button that unpairs every phone.
fn unpair_button(cx: &mut Context<PhoneSection>) -> impl IntoElement + use<> {
    button("unpair-phones", "Unpair every phone", Kind::Quiet)
        .on_hint_click(cx.listener(|section, _, _, cx| section.unpair(cx)))
}

impl EventEmitter<UnpairPhones> for PhoneSection {}

/// The code `code`, dark on light, as readers read it best, with its
/// quiet edge, `CODE_SIZE` wide.
#[expect(
    clippy::cast_precision_loss,
    reason = "a code is at most 177 modules across"
)]
fn drawn(code: &QrCode) -> impl IntoElement + use<> {
    let modules = code.size();
    let dark: Vec<(i32, i32)> = (0..modules)
        .flat_map(|y| (0..modules).map(move |x| (x, y)))
        .filter(|&(x, y)| code.get_module(x, y))
        .collect();
    let across = modules + 2 * QUIET;
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            window.paint_quad(fill(bounds, rgb(0xff_ffff)));
            let module = bounds.size.width / across as f32;
            for &(x, y) in &dark {
                let at = point(
                    bounds.origin.x + module * (x + QUIET) as f32,
                    bounds.origin.y + module * (y + QUIET) as f32,
                );
                window.paint_quad(fill(Bounds::new(at, size(module, module)), rgb(0)));
            }
        },
    )
    .flex_shrink_0()
    .size(px(CODE_SIZE))
}

impl Render for PhoneSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut words = self.words.part();
        let section = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(words.heading("PHONE"))
            .child(words.about(
                "Follow the chats from your phone, through Tailscale: a phone pairs once by \
                 scanning the code.",
            ));
        let section = match &self.shown {
            Shown::Hidden => section
                .child(
                    div()
                        .mt_1()
                        .flex()
                        .gap_2()
                        .child(
                            button("show-pairing", "Show pairing code", Kind::Plain)
                                .on_hint_click(cx.listener(|section, _, _, cx| section.show(cx))),
                        )
                        .child(unpair_button(cx)),
                )
                .child(words.note(
                    "The code holds the secret a phone pairs with: show it where no one else \
                     sees the screen.",
                    FAINT,
                )),
            Shown::Finding => {
                section.child(words.note("Asking Tailscale where the page is…", FAINT))
            }
            Shown::Unpairing { .. } => section.child(words.note("Unpairing every phone…", FAINT)),
            Shown::Failed(why) => section.child(words.note(why, FAILED)).child(
                div().flex().child(
                    button("show-pairing", "Try again", Kind::Plain)
                        .on_hint_click(cx.listener(|section, _, _, cx| section.show(cx))),
                ),
            ),
            Shown::Code { pairing, code } => {
                let told = self.told(pairing, &mut words, cx);
                section.child(
                    div()
                        .mt_1()
                        .flex()
                        .items_start()
                        .gap_4()
                        .child(
                            div()
                                .debug_selector(|| "pairing-code".to_owned())
                                .rounded(px(10.))
                                .overflow_hidden()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .child(drawn(code)),
                        )
                        .child(told.text_color(rgb(DIM))),
                )
            }
        };
        section.when(self.unpaired, |section| {
            section.child(words.note(
                "Every phone is unpaired: each pairs again with the pairing code.",
                ACCENT,
            ))
        })
    }
}
