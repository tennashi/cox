//! The keys a human presses in the window go where Cox binds them.

#[expect(dead_code, reason = "these tests use only part of the fake agent")]
mod fake_agent;
#[expect(dead_code, reason = "these tests use only part of the fake daemon")]
mod fake_daemon;
mod keys;
use coxd::testing::scratch;
use coxd::testing::waits;

use cox::ui::translation::Translate;
use gpui::TestAppContext;

#[gpui::test]
fn keys_typed_in_a_new_chat_go_to_its_input(cx: &mut TestAppContext) {
    let home = scratch::folder("home");
    let (workspace, cx) = keys::open_window(cx, home.clone());

    keys::press(cx, &workspace, "secondary-n h i");

    assert_eq!(keys::keyed(cx), ["Workspace", "Conversation", "LineInput"]);
    assert_eq!(keys::shown_input(cx, &workspace), "hi");
    // The window has the keys the app binds beyond the chat's own.
    let translates = cx.update(|window, _| window.bindings_for_action(&Translate).len());
    assert_eq!(translates, 1);
}
