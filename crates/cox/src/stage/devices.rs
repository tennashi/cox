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

use crate::stage::tab::Shows;
use crate::ui::theme::{
    ACCENT, ASKING, BORDER, BUTTON, BUTTON_HOVER, DIM, FAILED, FAINT, FOREGROUND,
};

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
        let button = |id: &'static str, label: &'static str| button(id, index, label);
        let running = self.running(&avd.name).cloned();
        let wiping = self.wiping.as_deref() == Some(avd.name.as_str());
        let name = avd.name.clone();
        let open = {
            let name = name.clone();
            cx.listener(move |devices: &mut Self, _, _, cx| {
                devices.open(&name, Boot::default(), cx);
            })
        };
        let row = div()
            .debug_selector(move || format!("android-device-{index}"))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(BORDER))
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
                .child(button("android-wipe-yes", "Wipe").on_click(wipe))
                .child(button("android-wipe-no", "Cancel").on_click(cx.listener(
                    |devices, _, _, cx| {
                        devices.wiping = None;
                        cx.notify();
                    },
                )));
        }
        match running {
            Some(running) => row
                .child(div().text_color(rgb(ACCENT)).child("Running"))
                .child(button("android-show", "Show").on_click(open))
                .child(
                    button("android-stop", "Stop").on_click(cx.listener(move |_, _, _, cx| {
                        cox_android::running::stop(&running);
                        cx.notify();
                    })),
                ),
            None => row
                .child(button("android-open", "Start").on_click(open))
                .child(button("android-cold", "Cold boot").on_click({
                    let name = name.clone();
                    cx.listener(move |devices: &mut Self, _, _, cx| {
                        let boot = Boot {
                            cold: true,
                            wipe: false,
                        };
                        devices.open(&name, boot, cx);
                    })
                }))
                .child(button("android-wipe", "Wipe data").on_click(cx.listener(
                    move |devices, _, _, cx| {
                        devices.wiping = Some(name.clone());
                        cx.notify();
                    },
                ))),
        }
    }
}

/// A button of a device's row, the `index`th.
fn button(id: &'static str, index: usize, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id((id, index))
        .debug_selector(move || format!("{id}-{index}"))
        .px_2()
        .py(px(2.))
        .rounded(px(6.))
        .cursor_pointer()
        .bg(rgb(BUTTON))
        .hover(|style| style.bg(rgb(BUTTON_HOVER)))
        .child(label)
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
        let empty = self.avds.is_empty().then(|| {
            let home = cox_android::avd::home().map_or_else(
                || "~/.android/avd".to_owned(),
                |home| home.display().to_string(),
            );
            div()
                .debug_selector(|| "android-no-devices".to_owned())
                .p_3()
                .text_color(rgb(DIM))
                .child(format!(
                    "No virtual devices in {home}. Make one in Android Studio's Device Manager."
                ))
        });
        let no_emulator = (!self.emulator).then(|| {
            div()
                .debug_selector(|| "android-no-emulator".to_owned())
                .p_3()
                .text_color(rgb(FAILED))
                .child(
                    "No Android SDK's emulator is found: install it from Android Studio, \
                     or name the SDK with ANDROID_HOME.",
                )
        });
        div()
            .id("android-devices")
            .debug_selector(|| "android-devices".to_owned())
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .children(no_emulator)
            .children(rows)
            .children(empty)
    }
}
