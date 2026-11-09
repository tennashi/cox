//! The phone on the settings screen: where a phone follows the chats,
//! and the code it pairs with, so that the human pairs one without a
//! terminal. A phone logged in to Tailscale as the Mac is opens the page
//! as it is; another scans the code, which opens the link that pairs it
//! (see [`crate::daemon::phone::pairing`]). The code holds the secret, so
//! it shows only once the human asks for it, and goes as they hide it,
//! or as the screen closes.

use gpui::{
    Bounds, ClipboardItem, Context, Div, Window, canvas, div, fill, point, prelude::*, px, rgb,
    size,
};
use qrcodegen::{QrCode, QrCodeEcc};

use super::words::{Part, Words};
use crate::daemon::phone::{Pairing, pairing};
use crate::ui::button::{Kind, button};
use crate::ui::theme::{ACCENT, ASKING, BORDER, DIM, FAILED, FAINT, FOREGROUND};

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
}

/// The phone, to pair.
pub struct PhoneSection {
    shown: Shown,
    /// Whether the link was copied, since the code showed.
    copied: bool,
    words: Words,
}

impl PhoneSection {
    /// The section, with the code hidden, writing what it says in
    /// `words`.
    pub fn new(words: Words) -> Self {
        Self {
            shown: Shown::Hidden,
            copied: false,
            words,
        }
    }

    /// What the section shows, for `cox --inspect state`: never the
    /// secret.
    #[must_use]
    pub fn described(&self) -> String {
        match &self.shown {
            Shown::Hidden => "Phone: the pairing code hidden".to_owned(),
            Shown::Finding => "Phone: asking Tailscale where the page is".to_owned(),
            Shown::Code { pairing, .. } => {
                format!("Phone: the pairing code shown, for {}", pairing.page)
            }
            Shown::Failed(why) => format!("Phone: no pairing code — {why}"),
        }
    }

    /// Asks Tailscale where the page is, apart from the window, which it
    /// may take seconds to answer, and shows the code once it does.
    fn show(&mut self, cx: &mut Context<Self>) {
        if matches!(self.shown, Shown::Finding) {
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
                "A phone logged in to Tailscale as {login} opens {} as it is. Another \
                 scans the code once, and keeps what it pairs with.",
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
                        .on_click(cx.listener(|section, _, _, cx| section.copy(cx))),
                    )
                    .child(
                        button("hide-pairing", "Hide", Kind::Quiet)
                            .on_click(cx.listener(|section, _, _, cx| section.hide(cx))),
                    ),
            )
            .when(self.copied, |told| {
                told.child(words.note(
                    "Copied: paste it into the page a home-screen app asks for it on.",
                    ACCENT,
                ))
            });
        told
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
                "Follow the chats from your phone, through Tailscale: a phone logged in to \
                 Tailscale as this Mac opens the page as it is, and another pairs once by \
                 scanning the code.",
            ));
        match &self.shown {
            Shown::Hidden => section
                .child(
                    div().mt_1().flex().child(
                        button("show-pairing", "Show pairing code", Kind::Plain)
                            .on_click(cx.listener(|section, _, _, cx| section.show(cx))),
                    ),
                )
                .child(words.note(
                    "The code holds the secret a phone pairs with: show it where no one else \
                     sees the screen.",
                    FAINT,
                )),
            Shown::Finding => {
                section.child(words.note("Asking Tailscale where the page is…", FAINT))
            }
            Shown::Failed(why) => section.child(words.note(why, FAILED)).child(
                div().flex().child(
                    button("show-pairing", "Try again", Kind::Plain)
                        .on_click(cx.listener(|section, _, _, cx| section.show(cx))),
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
        }
    }
}
