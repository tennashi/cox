//! What an agent's tools do with the Android emulators: the device acted
//! on shows on a tab of the stage, where the agent's taps, swipes, text
//! and keys go on to its emulator, and the agent sees a picture of its
//! display once it settles, as the human sees it on the tab. What the
//! emulator's gRPC service does not tell, as the screen's elements, is
//! asked of the device through `adb`.

use std::fmt::Write as _;
use std::time::Duration;

use cox_android::adb::Adb;
use cox_android::device::Boot;
use cox_android::settle::{self, Settled, Settling};
use cox_android::ui::{self, Find, Node, Scale};
use gpui::{AppContext as _, AsyncApp, Context, Entity, WeakEntity, Window};
use tokio::sync::oneshot;

use super::mac::{FIRST_PICTURE, LOOK_AGAIN, png};
use crate::stage::Stage;
use crate::stage::android::Android;
use crate::stage::tab::TabContent;
use coxd::agent::mcp::{AndroidAct, DeviceAct, Element, Seen, Spot};

/// How many steps a swipe's finger moves in, and how long each takes, as
/// a finger moves.
const SWIPE_STEPS: u32 = 12;
const SWIPE_STEP: Duration = Duration::from_millis(16);

/// How long a look waits for the display of an emulator that boots,
/// before it tells the agent to look again.
const BOOTING: Duration = Duration::from_secs(30);

/// How long typing waits for a field to have the keyboard, and how long
/// the display may take to settle after the field is tapped.
const FOCUS: Duration = Duration::from_secs(2);

/// How often typing asks whether a field has the keyboard.
const ASK_AGAIN: Duration = Duration::from_millis(150);

/// How long typing waits after each character: the keyboard drops some
/// of the characters the emulator types at once.
const TYPE_STEP: Duration = Duration::from_millis(30);

/// What the agent's answer goes to.
type Answer = oneshot::Sender<Result<Seen, String>>;

impl Stage {
    /// Does `act`, which the agent of the chat acting asks, with the
    /// Android emulators, and answers `answer` what it sees then: the
    /// device's tab, the one it has, or a new one of the chat's, which
    /// starts its emulator, shows it, if it is in the human's sight.
    pub(super) fn android_act(
        &mut self,
        act: AndroidAct,
        answer: Answer,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let AndroidAct::On {
            device: name,
            act,
            settle_ms,
        } = act
        else {
            cx.background_executor()
                .spawn(async move {
                    let _ = answer.send(Ok(Seen {
                        text: told_devices(),
                        png: None,
                    }));
                })
                .detach();
            return;
        };
        if !cox_android::avd::all().iter().any(|avd| avd.name == name) {
            let _ = answer.send(Err(format!(
                "No Android device is named {name:?}: list them with android_devices."
            )));
            return;
        }
        let tab = self.android_tab(&name, cx).downgrade();
        let settle = Duration::from_millis(u64::from(settle_ms));
        cx.spawn(async move |_, cx| {
            let seen = match act {
                DeviceAct::Ui => elements(&tab, &name, cx).await,
                act => match acted(&tab, &name, act, cx).await {
                    Ok(()) => settled_seen(&tab, settle, cx).await,
                    Err(why) => Err(why),
                },
            };
            let _ = answer.send(seen);
        })
        .detach();
    }

    /// The tab of the device `name`, its emulator started if it does not
    /// run: the tab it has, shown in its turn if it is in the human's
    /// sight, or a new one of the chat acting.
    fn android_tab(&mut self, name: &str, cx: &mut Context<Self>) -> Entity<Android> {
        let index = self.tabs.iter().position(|tab| match &tab.content {
            TabContent::Android(android) => android.read(cx).name() == name,
            _ => false,
        });
        if let Some(index) = index
            && let Some(TabContent::Android(tab)) =
                self.tabs.iter().nth(index).map(|tab| &tab.content)
        {
            let tab = tab.clone();
            tab.update(cx, |tab, cx| {
                if !tab.is_running() {
                    tab.open(Boot::default(), cx);
                }
            });
            if self.in_sight() && self.tabs.show(index) {
                self.changed(cx);
            }
            return tab;
        }
        let name = name.to_owned();
        let tab = cx.new(|cx| Android::new(name, Some(Boot::default()), cx));
        self.push_android(tab.clone(), true, cx);
        tab
    }
}

/// Does `act` on the device of `tab`, `name`, as the agent asks: a look
/// does nothing.
async fn acted(
    tab: &WeakEntity<Android>,
    name: &str,
    act: DeviceAct,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    if matches!(act, DeviceAct::Look | DeviceAct::Ui) {
        return Ok(());
    }
    on_tab(tab, cx, |tab, _| tab.ready())?;
    match act {
        DeviceAct::Look | DeviceAct::Ui => {}
        DeviceAct::Tap {
            at,
            times,
            interval_ms,
            hold_ms,
        } => {
            let at = match at {
                Spot::At { x, y } => (x, y),
                Spot::On(element) => found(tab, name, &element, cx).await?,
            };
            let (interval, hold) = (ms(interval_ms), ms(hold_ms));
            for time in 0..times.max(1) {
                if time > 0 {
                    cx.background_executor().timer(interval).await;
                }
                tap(tab, at, hold, cx).await?;
            }
        }
        DeviceAct::Swipe { x, y, to_x, to_y } => swipe(tab, (x, y), (to_x, to_y), cx).await?,
        DeviceAct::Type {
            text,
            into,
            clear_first,
        } => {
            let adb = adb_of(name, cx).await;
            if let Some(into) = into {
                let at = found(tab, name, &into, cx).await?;
                tap(tab, at, Duration::ZERO, cx).await?;
                settled(tab, FOCUS, cx).await?;
            }
            match &adb {
                Ok(adb) => wait_for_field(adb, cx).await?,
                Err(why) if clear_first => return Err(why.clone()),
                // With no adb, as on a fake emulator, the text is typed
                // where it goes.
                Err(_) => {}
            }
            if clear_first && let Ok(adb) = adb {
                cx.background_executor()
                    .spawn(async move { adb.clear_field() })
                    .await?;
            }
            for typed in text.chars() {
                on_tab(tab, cx, |tab, cx| {
                    tab.type_text(typed.to_string(), cx);
                    Ok(())
                })?;
                cx.background_executor().timer(TYPE_STEP).await;
            }
        }
        DeviceAct::Key { key, times } => on_tab(tab, cx, |tab, cx| {
            for _ in 0..times.max(1) {
                tab.press(key.clone(), cx);
            }
            Ok(())
        })?,
    }
    Ok(())
}

fn ms(ms: u32) -> Duration {
    Duration::from_millis(u64::from(ms))
}

/// `act` on the tab, as it is now.
fn on_tab<T>(
    tab: &WeakEntity<Android>,
    cx: &mut AsyncApp,
    act: impl FnOnce(&mut Android, &mut Context<Android>) -> Result<T, String>,
) -> Result<T, String> {
    tab.update(cx, act)
        .map_err(|_| "the device's tab closed".to_owned())?
}

/// Taps `at`, holding the finger down `hold`.
async fn tap(
    tab: &WeakEntity<Android>,
    (x, y): (u32, u32),
    hold: Duration,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    on_tab(tab, cx, |tab, cx| tab.touch(x, y, true, cx))?;
    if !hold.is_zero() {
        cx.background_executor().timer(hold).await;
    }
    on_tab(tab, cx, |tab, cx| tab.touch(x, y, false, cx))
}

/// Lands the agent's finger at `from`, moves it to `to`, in steps, and
/// lifts it there.
async fn swipe(
    tab: &WeakEntity<Android>,
    from: (u32, u32),
    to: (u32, u32),
    cx: &mut AsyncApp,
) -> Result<(), String> {
    on_tab(tab, cx, |tab, cx| tab.touch(from.0, from.1, true, cx))?;
    let along = |from: u32, to: u32, step: u32| {
        let (from, to) = (i64::from(from), i64::from(to));
        let at = from + (to - from) * i64::from(step) / i64::from(SWIPE_STEPS);
        u32::try_from(at).unwrap_or_default()
    };
    for step in 1..=SWIPE_STEPS {
        cx.background_executor().timer(SWIPE_STEP).await;
        let (x, y) = (along(from.0, to.0, step), along(from.1, to.1, step));
        let last = step == SWIPE_STEPS;
        on_tab(tab, cx, |tab, cx| {
            tab.touch(x, y, true, cx)?;
            if last {
                tab.touch(x, y, false, cx)?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

/// `adb` for the device `name`, as its emulator runs now.
async fn adb_of(name: &str, cx: &mut AsyncApp) -> Result<Adb, String> {
    let name = name.to_owned();
    cx.background_executor()
        .spawn(async move {
            let running = cox_android::running::of(&name)
                .ok_or_else(|| format!("{name} does not run yet: look at it first."))?;
            Adb::of(&running)
        })
        .await
}

/// The screen's elements, and how its pixels go to those of the picture
/// the agent sees.
async fn nodes(
    tab: &WeakEntity<Android>,
    name: &str,
    cx: &mut AsyncApp,
) -> Result<(Vec<Node>, Scale), String> {
    let scale = on_tab(tab, cx, |tab, _| {
        tab.ready()?;
        let display = tab.display();
        let picture = tab.picture_size();
        display
            .zip(picture)
            .map(|(display, picture)| Scale { display, picture })
            .ok_or_else(|| format!("{} shows no display yet: look again.", tab.name()))
    })?;
    let adb = adb_of(name, cx).await?;
    let nodes = cx
        .background_executor()
        .spawn(async move { adb.ui() })
        .await?;
    Ok((nodes, scale))
}

/// Where the agent's picture has the middle of `element`.
async fn found(
    tab: &WeakEntity<Android>,
    name: &str,
    element: &Element,
    cx: &mut AsyncApp,
) -> Result<(u32, u32), String> {
    let (nodes, scale) = nodes(tab, name, cx).await?;
    let find = match element {
        Element::Text(text) => Find::Text(text.clone()),
        Element::Id(id) => Find::Id(id.clone()),
    };
    let node = find.in_nodes(&nodes, scale)?;
    Ok(scale.to_picture(node.bounds.center()))
}

/// The screen's elements, as the agent is told of them.
async fn elements(
    tab: &WeakEntity<Android>,
    name: &str,
    cx: &mut AsyncApp,
) -> Result<Seen, String> {
    let (nodes, scale) = nodes(tab, name, cx).await?;
    let (width, height) = scale.picture;
    Ok(Seen {
        text: format!(
            "The elements on {name}'s screen, in the {width}×{height} pixels of the picture \
             android_look answers: `at` is the middle, then [left,top right,bottom].\n{}",
            ui::told(&nodes, scale)
        ),
        png: None,
    })
}

/// Waits for a text field of the device to have the keyboard.
async fn wait_for_field(adb: &Adb, cx: &mut AsyncApp) -> Result<(), String> {
    let mut waited = Duration::ZERO;
    loop {
        let asked = adb.clone();
        let editing = cx
            .background_executor()
            .spawn(async move { asked.is_editing() })
            .await?;
        if editing {
            return Ok(());
        }
        if waited >= FOCUS {
            return Err(
                "No text field has the keyboard: tap one first, or name it with into_text or \
                 into_resource_id."
                    .to_owned(),
            );
        }
        cx.background_executor().timer(ASK_AGAIN).await;
        waited += ASK_AGAIN;
    }
}

/// Waits up to `most` for the display `tab` shows to stop changing.
async fn settled(
    tab: &WeakEntity<Android>,
    most: Duration,
    cx: &mut AsyncApp,
) -> Result<Settled, String> {
    let mut settling = Settling::new(most);
    let mut at = Duration::ZERO;
    loop {
        cx.background_executor().timer(settle::LOOK_AGAIN).await;
        at += settle::LOOK_AGAIN;
        let picture = tab
            .read_with(cx, |tab, _| tab.picture())
            .map_err(|_| "the device's tab closed".to_owned())?;
        let sum = match picture {
            Some(((_, _, bgra), _)) => {
                cx.background_executor()
                    .spawn(async move { settle::sum(&bgra) })
                    .await
            }
            None => 0,
        };
        if let Some(settled) = settling.see(at, sum) {
            return Ok(settled);
        }
    }
}

/// The display `tab` shows, as the agent sees it once it settles, waiting
/// up to `most` for it.
async fn settled_seen(
    tab: &WeakEntity<Android>,
    most: Duration,
    cx: &mut AsyncApp,
) -> Result<Seen, String> {
    let settled = if most.is_zero() {
        None
    } else {
        Some(settled(tab, most, cx).await?)
    };
    let mut seen = seen(tab, cx).await?;
    match settled {
        Some(Settled::Still(at)) => {
            let _ = write!(seen.text, " It settled {} ms after.", at.as_millis());
        }
        Some(Settled::Moving(at)) => {
            let _ = write!(
                seen.text,
                " It was changing still {} ms after: look again if it should have stopped.",
                at.as_millis()
            );
        }
        None => {}
    }
    Ok(seen)
}

/// The display `tab` shows, as the agent sees it now: what it is, and its
/// picture, once one came.
async fn seen(tab: &WeakEntity<Android>, cx: &mut AsyncApp) -> Result<Seen, String> {
    let mut waited = Duration::ZERO;
    let (picture, name) = loop {
        let (picture, name, booting) = tab
            .read_with(cx, |tab, _| {
                (tab.picture(), tab.name().to_owned(), tab.is_booting())
            })
            .map_err(|_| "the device's tab closed".to_owned())?;
        if let Some(picture) = picture {
            break (picture, name);
        }
        if waited >= if booting { BOOTING } else { FIRST_PICTURE } {
            return Err(format!(
                "{name} shows no display yet, as while its emulator boots: look again in a \
                 while."
            ));
        }
        cx.background_executor().timer(LOOK_AGAIN).await;
        waited += LOOK_AGAIN;
    };
    let ((width, height, bgra), size) = picture;
    let png = cx
        .background_executor()
        .spawn(async move { png(width, height, bgra, size) })
        .await?;
    Ok(Seen {
        text: format!(
            "{name}'s display, as the human sees it on the stage: {}×{} pixels, which x and y \
             are in.",
            size.0, size.1
        ),
        png: Some(png),
    })
}

/// The devices, as the agent is told of them.
fn told_devices() -> String {
    let avds = cox_android::avd::all();
    if avds.is_empty() {
        return "No Android virtual device is made: Android Studio's Device Manager makes \
                one."
            .to_owned();
    }
    let running: Vec<String> = cox_android::running::all()
        .into_iter()
        .map(|running| running.device)
        .collect();
    let mut told = String::from("The Android virtual devices, by name:");
    for avd in avds {
        let mut what = vec![avd.title.clone()];
        what.extend(avd.model.clone());
        what.extend(avd.api.as_ref().map(|api| format!("API {api}")));
        what.extend(
            avd.display
                .map(|(width, height)| format!("{width}×{height}")),
        );
        let runs = if running.contains(&avd.name) {
            "runs"
        } else {
            "does not run"
        };
        let _ = write!(told, "\n{}: {}; {runs}", avd.name, what.join(", "));
    }
    told
}
