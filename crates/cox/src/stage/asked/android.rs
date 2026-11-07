//! What an agent's tools do with the Android emulators: the device acted
//! on shows on a tab of the stage, where the agent's taps, swipes, text
//! and keys go on to its emulator, and the agent sees a picture of its
//! display after, as the human sees it on the tab.

use std::fmt::Write as _;
use std::time::Duration;

use cox_android::device::Boot;
use gpui::{AppContext as _, AsyncApp, Context, Entity, WeakEntity, Window};
use tokio::sync::oneshot;

use super::mac::{FIRST_PICTURE, LOOK_AGAIN, SETTLE, png};
use crate::agent::mcp::{AndroidAct, Seen};
use crate::stage::Stage;
use crate::stage::android::Android;
use crate::stage::tab::TabContent;

/// How many steps a swipe's finger moves in, and how long each takes, as
/// a finger moves.
const SWIPE_STEPS: u32 = 12;
const SWIPE_STEP: Duration = Duration::from_millis(16);

/// How long a look waits for the display of an emulator that boots,
/// before it tells the agent to look again.
const BOOTING: Duration = Duration::from_secs(30);

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
        let Some(name) = act.device().map(str::to_owned) else {
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
        let tab = self.android_tab(&name, cx);
        if let Err(why) = tab.update(cx, |tab, cx| tab.act(&act, cx)) {
            let _ = answer.send(Err(why));
            return;
        }
        let settle = if matches!(act, AndroidAct::Look { .. }) {
            Duration::ZERO
        } else {
            SETTLE
        };
        let tab = tab.downgrade();
        cx.spawn(async move |_, cx| {
            if let AndroidAct::Swipe {
                x, y, to_x, to_y, ..
            } = act
                && let Err(why) = swipe(&tab, (x, y), (to_x, to_y), cx).await
            {
                let _ = answer.send(Err(why));
                return;
            }
            cx.background_executor().timer(settle).await;
            let _ = answer.send(seen(&tab, cx).await);
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

/// Moves the agent's finger, down on the display already, from `from` to
/// `to`, in steps, and lifts it there.
async fn swipe(
    tab: &WeakEntity<Android>,
    from: (u32, u32),
    to: (u32, u32),
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let along = |from: u32, to: u32, step: u32| {
        let (from, to) = (i64::from(from), i64::from(to));
        let at = from + (to - from) * i64::from(step) / i64::from(SWIPE_STEPS);
        u32::try_from(at).unwrap_or_default()
    };
    for step in 1..=SWIPE_STEPS {
        cx.background_executor().timer(SWIPE_STEP).await;
        let (x, y) = (along(from.0, to.0, step), along(from.1, to.1, step));
        let last = step == SWIPE_STEPS;
        tab.update(cx, |tab, cx| {
            tab.touch(x, y, true, cx)?;
            if last {
                tab.touch(x, y, false, cx)?;
            }
            Ok::<(), String>(())
        })
        .map_err(|_| "the device's tab closed".to_owned())??;
    }
    Ok(())
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
