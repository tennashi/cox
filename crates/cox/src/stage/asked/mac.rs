//! What an agent's tools do with the windows of the Mac's other apps: the
//! window acted on shows on a tab of the stage, where the agent's clicks,
//! keys and text go on to its app, and the agent sees a picture of it
//! after, as the human sees it on the tab.

use std::fmt::Write as _;
use std::io::Cursor;
use std::time::Duration;

use base64::Engine as _;
use gpui::{AsyncWindowContext, Context, Entity, WeakEntity, Window};
use tokio::sync::oneshot;

use crate::agent::mcp::{MacAct, Seen};
use crate::stage::Stage;
use crate::stage::mac_window::MacWindow;
use crate::stage::tab::{Shows, TabContent};

/// How long the app has to answer what the agent did, before the agent
/// sees its window again.
const SETTLE: Duration = Duration::from_millis(400);

/// How long a tab just opened has to get the window's first picture.
const FIRST_PICTURE: Duration = Duration::from_secs(3);

/// How often a tab with no picture yet is looked at again.
const LOOK_AGAIN: Duration = Duration::from_millis(100);

/// What the agent's answer goes to.
type Answer = oneshot::Sender<Result<Seen, String>>;

impl Stage {
    /// Does `act`, which the agent of the chat acting asks, with the
    /// windows of the Mac's other apps, and answers `answer` what it sees
    /// then: the window's tab, the one showing it, or a new one of the
    /// chat's, shows it, if it is in the human's sight.
    pub(super) fn mac_act(
        &mut self,
        act: MacAct,
        answer: Answer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chat = self.acting;
        let Some(id) = act.window() else {
            cx.background_executor()
                .spawn(async move {
                    let listed = cox_mac::windows().map_err(|error| format!("{error:#}"));
                    let _ = answer.send(listed.map(|windows| Seen {
                        text: told_windows(&windows),
                        png: None,
                    }));
                })
                .detach();
            return;
        };
        if let Some(tab) = self.mac_window_shown(id, cx) {
            self.act_in(&tab, &act, answer, cx);
            return;
        }
        cx.spawn_in(window, async move |stage, cx| {
            let found = cx
                .background_executor()
                .spawn(async move {
                    let windows = cox_mac::windows().map_err(|error| format!("{error:#}"))?;
                    windows
                        .into_iter()
                        .find(|window| window.id == id)
                        .ok_or_else(|| format!("No Mac window {id} is open: list them again."))
                })
                .await;
            let app_window = match found {
                Ok(app_window) => app_window,
                Err(why) => {
                    let _ = answer.send(Err(why));
                    return;
                }
            };
            open_and_act(&stage, chat, app_window, &act, answer, cx);
        })
        .detach();
    }

    /// The tab showing the window `id`, shown in its turn if it is in the
    /// human's sight.
    fn mac_window_shown(&mut self, id: u32, cx: &mut Context<Self>) -> Option<Entity<MacWindow>> {
        let index = self.tabs.iter().position(|tab| match &tab.content {
            TabContent::MacWindow(tab) => {
                let tab = tab.read(cx);
                tab.is_showing() && tab.window().is_some_and(|window| window.id == id)
            }
            _ => false,
        })?;
        let TabContent::MacWindow(tab) = &self.tabs.iter().nth(index)?.content else {
            return None;
        };
        let tab = tab.clone();
        if self.in_sight() && self.tabs.show(index) {
            self.changed(cx);
        }
        Some(tab)
    }

    /// Does `act` in the window `tab` shows, and answers `answer` the
    /// window as the agent sees it once its app has answered.
    fn act_in(
        &mut self,
        tab: &Entity<MacWindow>,
        act: &MacAct,
        answer: Answer,
        cx: &mut Context<Self>,
    ) {
        let chat = self.acting;
        if let Err(why) = tab.update(cx, |tab, cx| tab.act(act, chat, cx)) {
            let _ = answer.send(Err(why));
            return;
        }
        let settle = if matches!(act, MacAct::Look { .. }) {
            Duration::ZERO
        } else {
            SETTLE
        };
        let tab = tab.downgrade();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(settle).await;
            let seen = seen(&tab, cx).await;
            let _ = answer.send(seen);
        })
        .detach();
    }
}

/// Opens the tab of `app_window` on `stage`, for the chat `chat`, and
/// does `act` in it.
fn open_and_act(
    stage: &WeakEntity<Stage>,
    chat: Option<u64>,
    app_window: cox_mac::Window,
    act: &MacAct,
    answer: Answer,
    cx: &mut AsyncWindowContext,
) {
    let mut answer = Some(answer);
    let opened = stage.update(cx, |stage, cx| {
        stage.acting_for(chat, |stage| {
            let id = app_window.id;
            stage.mac_window_tab(app_window, cx);
            if let Some(tab) = stage.mac_window_shown(id, cx)
                && let Some(answer) = answer.take()
            {
                stage.act_in(&tab, act, answer, cx);
            }
        });
    });
    if let Some(answer) = answer {
        let why = if opened.is_err() {
            "the stage went away"
        } else {
            "the window's tab did not open"
        };
        let _ = answer.send(Err(why.to_owned()));
    }
}

/// The window `tab` shows, as the agent sees it now: what it is, and its
/// picture, once one came.
async fn seen(tab: &WeakEntity<MacWindow>, cx: &mut gpui::AsyncApp) -> Result<Seen, String> {
    let mut waited = Duration::ZERO;
    let (picture, title) = loop {
        let (picture, title) = tab
            .read_with(cx, |tab, _| (tab.picture(), tab.title()))
            .map_err(|_| "the window's tab closed".to_owned())?;
        if let Some(picture) = picture {
            break (picture, title);
        }
        if waited >= FIRST_PICTURE {
            return Err(format!(
                "{title} sent no picture yet: look again in a moment."
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
            "{title}, as the human sees it on the stage: {}×{} pixels, which x and y are in.",
            size.0, size.1
        ),
        png: Some(png),
    })
}

/// The picture `width` by `height`, as BGRA, as PNG in base64, `size`
/// big.
fn png(width: u32, height: u32, mut bgra: Vec<u8>, size: (u32, u32)) -> Result<String, String> {
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let picture = image::RgbaImage::from_raw(width, height, bgra)
        .ok_or("the window's picture is not whole")?;
    let picture = if (width, height) == size {
        picture
    } else {
        image::imageops::resize(
            &picture,
            size.0,
            size.1,
            image::imageops::FilterType::Triangle,
        )
    };
    let mut written = Cursor::new(Vec::new());
    picture
        .write_to(&mut written, image::ImageFormat::Png)
        .map_err(|error| format!("cannot write the window's picture: {error}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(written.into_inner()))
}

/// The windows, as the agent is told of them.
fn told_windows(windows: &[cox_mac::Window]) -> String {
    if windows.is_empty() {
        return "No window of another app is on screen.".to_owned();
    }
    let mut told = String::from("The windows of the Mac's other apps on screen, by number:");
    for window in windows {
        let title = if window.title.is_empty() {
            String::new()
        } else {
            format!(" · {}", window.title)
        };
        let _ = write!(
            told,
            "\n{}: {}{title} ({:.0}×{:.0} points)",
            window.id, window.app, window.frame.width, window.frame.height
        );
    }
    told
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_windows_by_number() {
        let window = cox_mac::Window {
            id: 42,
            pid: 7,
            app: "Notes".to_owned(),
            title: "Groceries".to_owned(),
            frame: cox_mac::Rect {
                x: 0.,
                y: 0.,
                width: 800.,
                height: 600.,
            },
        };
        assert_eq!(
            told_windows(&[window]),
            "The windows of the Mac's other apps on screen, by number:\n42: Notes · Groceries \
             (800×600 points)"
        );
        assert_eq!(told_windows(&[]), "No window of another app is on screen.");
    }

    #[test]
    fn writes_the_picture_as_the_agent_sees_it() {
        // Blue, green, red and alpha, as the window's picture comes.
        let bgra = [255, 0, 0, 255].repeat(4);
        let png = png(2, 2, bgra, (1, 1)).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png)
            .unwrap();
        let read = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(read.dimensions(), (1, 1));
        assert_eq!(read.get_pixel(0, 0).0, [0, 0, 255, 255]);
    }
}
