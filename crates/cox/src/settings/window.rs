//! The window's side of the settings: it follows `settings.json` and
//! `agents.json` as they change, and holds the agents for the settings
//! screen to list.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AsyncApp, Global, WindowHandle};

use super::{Saved, home_for_chats, view};
use crate::agents::Agents;
use crate::stage::follow::Stamp;
use crate::storage::Kept;
use crate::workspace::Workspace;

/// How often Cox looks whether `settings.json` or `agents.json` changed.
const LOOK_EVERY: Duration = Duration::from_millis(500);

/// The agents the window knows, for the settings screen to list.
pub struct AppAgents(pub Agents);

impl Global for AppAgents {}

/// Follows `settings.json` and `agents.json` as they change, as when the
/// human edits them, for as long as `window` is: the agents are those
/// `agents.json` lists now, a new chat talking to the one `settings.json`
/// names, Cox's own chats open from then on work in the home
/// it names, unless `--home` gave another, and Vim's keys are on or off
/// as it says.
pub fn follow(window: WindowHandle<Workspace>, agents: Agents, cx: &mut App) {
    let files: Vec<PathBuf> = [Kept::Settings, Kept::Agents, Kept::Servers]
        .into_iter()
        .filter_map(crate::storage::path)
        .collect();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let stamps = |files: Vec<PathBuf>| async move {
            files
                .iter()
                .map(|file| Stamp::of(file).ok())
                .collect::<Vec<_>>()
        };
        let mut seen = cx.background_executor().spawn(stamps(files.clone())).await;
        loop {
            cx.background_executor().timer(LOOK_EVERY).await;
            let now = cx.background_executor().spawn(stamps(files.clone())).await;
            if now == seen {
                continue;
            }
            seen = now;
            let agents = agents.clone();
            let (home, vim) = cx
                .background_executor()
                .spawn(async move {
                    let saved = Saved::read();
                    agents.reload(saved.agent.as_deref());
                    (home_for_chats(saved.home), saved.vim)
                })
                .await;
            let followed = window.update(cx, |workspace, window, cx| {
                crate::vim::switch(vim, window, cx);
                workspace.follow_home(home, cx);
                if let Some(view) = workspace.settings().cloned() {
                    view.update(cx, view::SettingsView::reread);
                }
            });
            if followed.is_err() {
                break;
            }
        }
    })
    .detach();
}
