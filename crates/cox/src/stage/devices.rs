//! A tab of the stage listing the Android emulator's virtual devices, as
//! Android Studio's Device Manager lists them, for the human to run one
//! on the stage: each with the name they gave it, the phone it is made
//! as, the Android it runs, its display, and whether it runs, which a
//! click opens on a tab of its own (see [`crate::stage::android`]), stops,
//! or starts from nothing (Cold boot) or with its data taken away (Wipe
//! data, which asks first). Making, changing and deleting a device are
//! Android Studio's: one it makes shows here as it is made.

use std::time::Duration;

use cox_android::avd::Avd;
use cox_android::device::Boot;
use cox_android::running::Running;
use cox_core::tabs::KeptTab;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Task, Window, div, prelude::*,
    px, rgb,
};

use crate::stage::heading::{INSET, body, card, heading, note, parted};
use crate::stage::tab::Shows;
use crate::ui::button::{self, Kind};
use crate::ui::paths::shortened;
use crate::ui::theme::{ACCENT, ASKING, DIM, FAILED, FAINT, FOREGROUND};
use crate::vim::hints::Hinted as _;

pub(crate) const KEY_CONTEXT: &str = "Devices";

/// How often the tab reads the devices again, as Android Studio makes or
/// deletes them and emulators start and stop.
const READ_EVERY: Duration = Duration::from_secs(2);

/// The title the tab goes by.
pub const TITLE: &str = "Android devices";

pub struct Devices {
    avds: Vec<Avd>,
    running: Vec<Running>,
    /// Whether the SDK's emulator was found.
    emulator: bool,
    /// The device whose data the human asked to wipe, until they say yes
    /// or no.
    wiping: Option<String>,
    focus_handle: FocusHandle,
    _reading: Task<()>,
}

/// What the human asks the stage to open.
pub struct OpenDevice {
    /// The device, by its name as `emulator -avd` takes it.
    pub name: String,
    pub boot: Boot,
}

impl EventEmitter<OpenDevice> for Devices {}

impl Devices {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let reading = cx.spawn(async move |this, cx| {
            loop {
                let (avds, running, emulator) = cx
                    .background_executor()
                    .spawn(async {
                        (
                            cox_android::avd::all(),
                            cox_android::running::all(),
                            cox_android::sdk::emulator().is_some(),
                        )
                    })
                    .await;
                let read = this.update(cx, |devices, cx| {
                    if devices.avds != avds
                        || devices.running != running
                        || devices.emulator != emulator
                    {
                        devices.avds = avds;
                        devices.running = running;
                        devices.emulator = emulator;
                        cx.notify();
                    }
                });
                if read.is_err() {
                    break;
                }
                cx.background_executor().timer(READ_EVERY).await;
            }
        });
        Self {
            avds: Vec::new(),
            running: Vec::new(),
            emulator: true,
            wiping: None,
            focus_handle: cx.focus_handle(),
            _reading: reading,
        }
    }

    fn running(&self, name: &str) -> Option<&Running> {
        self.running.iter().find(|running| running.device == name)
    }

    fn open(&mut self, name: &str, boot: Boot, cx: &mut Context<Self>) {
        self.wiping = None;
        cx.emit(OpenDevice {
            name: name.to_owned(),
            boot,
        });
        cx.notify();
    }

    fn render_device(&self, index: usize, avd: &Avd, cx: &mut Context<Self>) -> impl IntoElement {
        let button =
            |id: &'static str, label: &'static str| button::button((id, index), label, Kind::Plain);
        let running = self.running(&avd.name).cloned();
        let wiping = self.wiping.as_deref() == Some(avd.name.as_str());
        let name = avd.name.clone();
        let open = {
            let name = name.clone();
            cx.listener(move |devices: &mut Self, _, _, cx| {
                devices.open(&name, Boot::default(), cx);
            })
        };
        let row = parted(div(), index == 0)
            .debug_selector(move || format!("android-device-{index}"))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .child(dot(running.is_some()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_color(rgb(FOREGROUND)).child(avd.title.clone()))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(FAINT))
                            .child(what(avd)),
                    ),
            );
        if wiping {
            let wipe = {
                let name = name.clone();
                cx.listener(move |devices: &mut Self, _, _, cx| {
                    let boot = Boot {
                        cold: true,
                        wipe: true,
                    };
                    devices.open(&name, boot, cx);
                })
            };
            return row
                .child(
                    div()
                        .text_color(rgb(ASKING))
                        .child("Its apps and data go. Wipe?"),
                )
                .child(
                    button::button(("android-wipe-yes", index), "Wipe", Kind::Primary)
                        .on_hint_click(wipe),
                )
                .child(
                    button("android-wipe-no", "Cancel").on_hint_click(cx.listener(
                        |devices, _, _, cx| {
                            devices.wiping = None;
                            cx.notify();
                        },
                    )),
                );
        }
        match running {
            Some(running) => row
                .child(div().text_color(rgb(ACCENT)).child("Running"))
                .child(button("android-show", "Show").on_hint_click(open))
                .child(button("android-stop", "Stop").on_hint_click(cx.listener(
                    move |_, _, _, cx| {
                        cox_android::running::stop(&running);
                        cx.notify();
                    },
                ))),
            None => row
                .child(button("android-open", "Start").on_hint_click(open))
                .child(button("android-cold", "Cold boot").on_hint_click({
                    let name = name.clone();
                    cx.listener(move |devices: &mut Self, _, _, cx| {
                        let boot = Boot {
                            cold: true,
                            wipe: false,
                        };
                        devices.open(&name, boot, cx);
                    })
                }))
                .child(
                    button::button(("android-wipe", index), "Wipe data", Kind::Quiet)
                        .on_hint_click(cx.listener(move |devices, _, _, cx| {
                            devices.wiping = Some(name.clone());
                            cx.notify();
                        })),
                ),
        }
    }
}

/// The dot that says whether a device runs: filled in the accent while it
/// does, and a ring otherwise.
fn dot(runs: bool) -> impl IntoElement {
    let dot = div().flex_shrink_0().size(px(7.)).mr_1().rounded_full();
    if runs {
        dot.bg(rgb(ACCENT))
    } else {
        dot.border_1().border_color(rgb(FAINT))
    }
}

/// What `avd` is, as the line under its name says it: the phone it is
/// made as, the API it runs, its display, and its name, when the human
/// gave it another.
fn what(avd: &Avd) -> String {
    let what: Vec<String> = [
        avd.model.clone(),
        avd.api.as_ref().map(|api| format!("API {api}")),
        avd.display
            .map(|(width, height)| format!("{width}×{height}")),
        (avd.title != avd.name).then(|| avd.name.clone()),
    ]
    .into_iter()
    .flatten()
    .collect();
    what.join(" · ")
}

impl Shows for Devices {
    fn title(&self) -> String {
        TITLE.to_owned()
    }

    fn kept(&self) -> KeptTab {
        KeptTab::AndroidDevices
    }
}

impl Focusable for Devices {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Devices {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .avds
            .clone()
            .iter()
            .enumerate()
            .map(|(index, avd)| self.render_device(index, avd, cx).into_any_element())
            .collect();
        let home = cox_android::avd::home()
            .map_or_else(|| "~/.android/avd".to_owned(), |home| shortened(&home));
        let running = self
            .avds
            .iter()
            .filter(|avd| self.running(&avd.name).is_some())
            .count();
        let said = match (self.avds.len(), running) {
            (0, _) => "none yet".to_owned(),
            (1, 0) => "1 device".to_owned(),
            (1, _) => "1 device, running".to_owned(),
            (count, 0) => format!("{count} devices"),
            (count, running) => format!("{count} devices · {running} running"),
        };
        let empty = self.avds.is_empty().then(|| {
            note(
                "android-no-devices",
                format!(
                    "No virtual devices in {home}. Make one in Android Studio's Device Manager."
                ),
                DIM,
            )
        });
        let no_emulator = (!self.emulator).then(|| {
            note(
                "android-no-emulator",
                "No Android SDK's emulator is found: install it from Android Studio, \
                 or name the SDK with ANDROID_HOME.",
                FAILED,
            )
        });
        let devices = (!rows.is_empty()).then(|| {
            div()
                .px(px(INSET))
                .pt_1()
                .pb_3()
                .child(card().children(rows))
        });
        body()
            .id("android-devices")
            .debug_selector(|| "android-devices".to_owned())
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .overflow_y_scroll()
            .text_color(rgb(FOREGROUND))
            .child(heading("android-heading", TITLE, said, Some(home)))
            .children(no_emulator)
            .children(devices)
            .children(empty)
    }
}
