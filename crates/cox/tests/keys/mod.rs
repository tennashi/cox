//! The keys a human presses in the window, as a test sees where they go:
//! the window with every key Cox binds, what has the keyboard, and what
//! the keys typed there.

use std::path::PathBuf;

use cox::workspace::Workspace;
use gpui::{Entity, TestAppContext, VisualTestContext};

use super::fake_daemon::{self, FakeStages, Fakes, Kept, settled};

/// Opens the window with no chat and every key Cox binds, its chats
/// working in `home` and kept in memory.
pub fn open_window(
    cx: &mut TestAppContext,
    home: PathBuf,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    open_window_on(cx, home, FakeStages::default())
}

/// Opens the window like [`open_window`], its chats' stages made by
/// `stages`.
pub fn open_window_on(
    cx: &mut TestAppContext,
    home: PathBuf,
    stages: FakeStages,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (workspace, cx) =
        fake_daemon::open_window_with_every_key(cx, home, Kept::Memory, Fakes::default(), stages);
    settled(cx, &workspace);
    (workspace, cx)
}

/// Presses `keys`, as `simulate_keystrokes` reads them ("g g", "3 j",
/// "escape"), one at a time, as a human does: each once the daemon has
/// done what the one before asked, and the window has drawn it.
pub fn press(cx: &mut VisualTestContext, workspace: &Entity<Workspace>, keys: &str) {
    for key in keys.split(' ') {
        cx.simulate_keystrokes(key);
        settled(cx, workspace);
    }
}

/// The key contexts on the way to what has the keyboard, the window's
/// first: `["Workspace", "Conversation", "LineInput"]` in a chat's input.
pub fn keyed(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|window, _| {
        window
            .context_stack()
            .iter()
            .filter_map(|context| context.primary().map(|entry| entry.key.to_string()))
            .collect()
    })
}

/// What the input of the chat shown holds.
pub fn shown_input(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> String {
    workspace.read_with(cx, |workspace, cx| {
        let chat = workspace.shown().expect("a chat is shown");
        let view = chat.view().expect("the chat shown has a view");
        view.read(cx).input().read(cx).text().to_owned()
    })
}
