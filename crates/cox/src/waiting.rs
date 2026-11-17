//! What the human sees while a window waits for coxd to stop for it (see
//! [`coxd::daemon::client`]): a small window of its own, saying which
//! chats coxd is at work on, where they can have coxd stop now, cutting
//! those chats, or quit.
//!
//! The window that waits has not started CEF yet, which is to start
//! after coxd and before GPUI, so it shows this in a process of its own,
//! `cox [--behind] --waiting-for-coxd`, which runs GPUI alone. It tells
//! that process what to show, a [`Told`] a line of JSON on its standard
//! input, and hears what the human chose, a [`Choice`] a line on its
//! standard output. Once its standard input closes, as when coxd took
//! the window, the process ends; once the human closes its window, so
//! does its standard output, which the window takes as their quitting.

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use coxd::daemon::client::{Choice, Waiting};
use coxd::daemon::wire::AtWork;
use coxd::theme::{ASKING, DIM, FAILED, FOREGROUND, GROUND, RAISED, UI_FONT};
use gpui::{
    App, Application, Bounds, Context, SharedString, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use serde::{Deserialize, Serialize};

use crate::ui::button::{Kind, button};

/// What the command line says to be the window that shows the waiting.
pub const OPTION: &str = "--waiting-for-coxd";

/// What the waiting window is told to show.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Told {
    /// That the window waits for coxd, at work on these chats, or on
    /// chats it does not name.
    Waits(Option<Vec<AtWork>>),
    /// That coxd stops, as the human chose.
    Stopping,
    /// That the window does not open, and why.
    Failed(String),
}

/// What the human chose, as the waiting window says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Chose {
    RestartNow,
    Quit,
}

impl From<Chose> for Choice {
    fn from(chose: Chose) -> Self {
        match chose {
            Chose::RestartNow => Self::RestartNow,
            Chose::Quit => Self::Quit,
        }
    }
}

/// The waiting window, as the window that waits shows it: started once
/// there is something to show.
pub struct Shown {
    behind: bool,
    process: Option<Process>,
    /// What it was told last.
    told: Option<Told>,
    /// Whether it could not start, which the human is told otherwise.
    unshown: bool,
}

/// The process of the waiting window.
struct Process {
    child: Child,
    input: Option<ChildStdin>,
    chosen: mpsc::Receiver<Choice>,
}

impl Shown {
    /// No window yet, which opens behind the other apps' windows when
    /// `behind`, as the window that waits would.
    #[must_use]
    pub fn new(behind: bool) -> Self {
        Self {
            behind,
            process: None,
            told: None,
            unshown: false,
        }
    }

    /// Tells the window to show `told`, starting it first.
    fn tell(&mut self, told: &Told) {
        if self.told.as_ref() == Some(told) {
            return;
        }
        self.told = Some(told.clone());
        if self.process.is_none() && !self.unshown {
            match self.start() {
                Ok(process) => self.process = Some(process),
                Err(error) => {
                    tracing::error!("the window that waits for coxd does not open: {error}");
                    self.unshown = true;
                    notify();
                }
            }
        }
        let Some(input) = self
            .process
            .as_mut()
            .and_then(|process| process.input.as_mut())
        else {
            return;
        };
        let Ok(mut line) = serde_json::to_vec(told) else {
            return;
        };
        line.push(b'\n');
        if let Err(error) = input.write_all(&line) {
            tracing::warn!("the window that waits for coxd cannot be told: {error}");
        }
    }

    /// Starts the window's process, and what hears what the human chose.
    fn start(&self) -> std::io::Result<Process> {
        let mut command = Command::new(std::env::current_exe()?);
        if self.behind {
            command.arg("--behind");
        }
        let mut child = command
            .arg(OPTION)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take();
        let output = child.stdout.take();
        let (heard, chosen) = mpsc::channel();
        if let Some(output) = output {
            std::thread::Builder::new()
                .name("waiting window".to_owned())
                .spawn(move || {
                    for line in BufReader::new(output).lines() {
                        let Ok(line) = line else { break };
                        if let Ok(chose) = serde_json::from_str::<Chose>(&line)
                            && heard.send(Choice::from(chose)).is_err()
                        {
                            return;
                        }
                    }
                    // The human closed the window.
                    let _ = heard.send(Choice::Quit);
                })?;
        }
        Ok(Process {
            child,
            input,
            chosen,
        })
    }

    /// Shows that the window does not open, and why, until the human
    /// closes the waiting window, if it is open.
    pub fn failed(mut self, why: &str) {
        if self.process.is_none() {
            return;
        }
        self.tell(&Told::Failed(why.to_owned()));
        if let Some(process) = &self.process {
            while process
                .chosen
                .recv()
                .is_ok_and(|chosen| chosen != Choice::Quit)
            {}
        }
    }
}

impl Waiting for Shown {
    fn waits(&mut self, chats: Option<&[AtWork]>) {
        self.tell(&Told::Waits(chats.map(<[AtWork]>::to_vec)));
    }

    fn chosen(&mut self, within: Duration) -> Option<Choice> {
        let Some(process) = &self.process else {
            std::thread::sleep(within);
            return None;
        };
        match process.chosen.recv_timeout(within) {
            Ok(choice) => Some(choice),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => Some(Choice::Quit),
        }
    }

    fn stopping(&mut self) {
        self.tell(&Told::Stopping);
    }
}

impl Drop for Shown {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            // Its standard input closes, and it ends.
            process.input = None;
            let _ = process.child.wait();
        }
    }
}

/// Tells the human why the window does not open yet, where no waiting
/// window could open: in a notification, on macOS.
fn notify() {
    const WAITING: &str = "coxd restarts for this window once none of its chats is at \
                           work: the window opens then.";
    eprintln!("cox: {WAITING}");
    #[cfg(target_os = "macos")]
    {
        let shown = Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "display notification \"{WAITING}\" with title \"Cox\""
            ))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Err(error) = shown {
            tracing::error!("the human is not told: {error}");
        }
    }
}

/// Runs the waiting window, as `cox --waiting-for-coxd` does, behind the
/// other apps' windows when `behind`, until its standard input closes or
/// the human closes it.
///
/// # Panics
///
/// When its threads or its window do not start.
pub fn run(behind: bool) -> ExitCode {
    let (heard, mut told) = tokio::sync::mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("told".to_owned())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<Told>(&line) {
                    Ok(told) => {
                        if heard.send(Some(told)).is_err() {
                            return;
                        }
                    }
                    Err(error) => tracing::error!("the waiting window was told {line}: {error}"),
                }
            }
            let _ = heard.send(None);
        })
        .expect("the thread that hears the window that waits starts");
    Application::new().run(move |cx: &mut App| {
        #[cfg(target_os = "macos")]
        if behind {
            crate::app::no_dock_icon();
        }
        let bounds = Bounds::centered(None, size(px(460.), px(300.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: !behind,
                    ..Default::default()
                },
                |_, cx| cx.new(|_| View::new(Rc::new(choose))),
            )
            .expect("the waiting window opens");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.spawn(async move |cx| {
            while let Some(Some(next)) = told.recv().await {
                let shown = window.update(cx, |view, window, cx| {
                    window.set_window_title(view_title(&next));
                    view.told = Some(next);
                    cx.notify();
                });
                if shown.is_err() {
                    return;
                }
            }
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
        if !behind {
            cx.activate(true);
        }
    });
    ExitCode::SUCCESS
}

/// The title of the waiting window as it shows `told`.
fn view_title(told: &Told) -> &'static str {
    match told {
        Told::Waits(_) => "Cox waits for coxd",
        Told::Stopping => "Cox stops coxd",
        Told::Failed(_) => "Cox did not open",
    }
}

/// What the human chose, said to the window that waits on standard
/// output.
fn choose(chose: Chose, cx: &mut App) {
    if let Ok(line) = serde_json::to_string(&chose) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
    if chose == Chose::Quit {
        cx.quit();
    }
}

/// What says what the human chose in the waiting window.
type Choose = Rc<dyn Fn(Chose, &mut App)>;

/// The waiting window's view.
struct View {
    told: Option<Told>,
    /// What says what the human chose.
    choose: Choose,
}

impl View {
    fn new(choose: Choose) -> Self {
        Self { told: None, choose }
    }
}

/// What the button that has coxd stop now says, for `chats` at work, or
/// chats coxd does not name.
fn restart_label(chats: Option<&[AtWork]>) -> SharedString {
    match chats.map(<[AtWork]>::len) {
        Some(1) => "Restart now (cuts 1 chat)".into(),
        Some(count) if count > 1 => format!("Restart now (cuts {count} chats)").into(),
        _ => "Restart now (cuts the chats at work)".into(),
    }
}

/// A chat at work as a row says it.
fn chat_line(chat: &AtWork) -> String {
    match &chat.project {
        Some(project) => format!("{project} · {}", chat.chat),
        None => format!("Cox · {}", chat.chat),
    }
}

impl Render for View {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let heading = |text: &str| {
            div()
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(FOREGROUND))
                .child(text.to_owned())
        };
        let said = |text: String| div().text_size(px(13.)).text_color(rgb(DIM)).child(text);
        let quit = button("waiting-quit", "Quit", Kind::Quiet)
            .on_click(cx.listener(|view, _, _, cx| (view.choose)(Chose::Quit, cx)));
        let body = div().flex().flex_col().gap_3().flex_1();
        let (body, buttons) = match &self.told {
            None => (body.child(heading("Cox waits for coxd")), div()),
            Some(Told::Waits(chats)) => {
                let list =
                    match chats.as_deref() {
                        Some(chats) if !chats.is_empty() => div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(chats.iter().map(|chat| {
                                div()
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.))
                                    .bg(rgb(RAISED))
                                    .text_size(px(13.))
                                    .text_color(rgb(FOREGROUND))
                                    .child(chat_line(chat))
                            })),
                        Some(_) => div().child(said(
                            "A chat's agent goes on by itself, as with a command it left running."
                                .to_owned(),
                        )),
                        None => div().child(said(
                            "coxd is of an older build, which does not say which chats.".to_owned(),
                        )),
                    };
                (
                    body.child(heading("Cox waits for coxd to restart"))
                        .child(said(
                            "coxd, which runs the chats, is to restart for this build of the \
                             window, once none of its chats is at work. The window opens then."
                                .to_owned(),
                        ))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(ASKING))
                                .child("At work"),
                        )
                        .child(list),
                    div().flex().gap_2().justify_end().child(quit).child(
                        button(
                            "waiting-restart-now",
                            restart_label(chats.as_deref()),
                            Kind::Plain,
                        )
                        .text_color(rgb(ASKING))
                        .on_click(
                            cx.listener(|view, _, _, cx| (view.choose)(Chose::RestartNow, cx)),
                        ),
                    ),
                )
            }
            Some(Told::Stopping) => (
                body.child(heading("Cox stops coxd")).child(said(
                    "The chats at work are cut. The window opens once coxd has stopped and \
                     started again."
                        .to_owned(),
                )),
                div(),
            ),
            Some(Told::Failed(why)) => (
                body.child(heading("Cox did not open")).child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(FAILED))
                        .child(why.clone()),
                ),
                div().flex().justify_end().child(quit),
            ),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .pt(px(36.))
            .bg(rgb(GROUND))
            .font_family(UI_FONT)
            .child(body)
            .child(buttons)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_restart_button_says_how_many_chats_it_cuts() {
        let chat = AtWork {
            project: Some("cox".to_owned()),
            chat: "Fix the build".to_owned(),
        };
        assert_eq!(
            restart_label(Some(std::slice::from_ref(&chat))),
            "Restart now (cuts 1 chat)"
        );
        assert_eq!(
            restart_label(Some(&[chat.clone(), chat])),
            "Restart now (cuts 2 chats)"
        );
        assert_eq!(restart_label(None), "Restart now (cuts the chats at work)");
    }

    #[test]
    fn what_the_waiting_window_is_told_and_chooses_goes_as_a_line() {
        for told in [
            Told::Waits(None),
            Told::Waits(Some(vec![AtWork {
                project: None,
                chat: "Plan".to_owned(),
            }])),
            Told::Stopping,
            Told::Failed("coxd did not stop".to_owned()),
        ] {
            let line = serde_json::to_string(&told).unwrap();
            assert!(!line.contains('\n'));
            assert_eq!(serde_json::from_str::<Told>(&line).unwrap(), told);
        }
        assert_eq!(
            serde_json::to_string(&Chose::RestartNow).unwrap(),
            r#""RestartNow""#
        );
    }

    #[gpui::test]
    fn the_human_has_coxd_restart_now_from_the_waiting_window(cx: &mut gpui::TestAppContext) {
        let chose = Rc::new(std::cell::RefCell::new(Vec::new()));
        let heard = Rc::clone(&chose);
        let (view, cx) = cx.add_window_view(|_, _| {
            View::new(Rc::new(move |chosen, _: &mut App| {
                heard.borrow_mut().push(chosen);
            }))
        });
        view.update(cx, |view, cx| {
            view.told = Some(Told::Waits(Some(vec![AtWork {
                project: None,
                chat: "Plan".to_owned(),
            }])));
            cx.notify();
        });
        cx.run_until_parked();

        for selector in ["waiting-restart-now", "waiting-quit"] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is drawn"));
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        }

        assert_eq!(*chose.borrow(), [Chose::RestartNow, Chose::Quit]);
    }
}
