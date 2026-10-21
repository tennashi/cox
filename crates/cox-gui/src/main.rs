//! `cox-gui <agent command> [args...]` starts the agent and opens a window to
//! talk to it.

use gpui::{
    App, Application, Bounds, Focusable, WindowBounds, WindowOptions, prelude::*, px, size,
};

fn main() {
    env_logger::init();
    let command: Vec<String> = std::env::args().skip(1).collect();
    Application::new().run(move |cx: &mut App| {
        let agent = cox_gui::agent::spawn(move || cox_acp::launch(command));
        let bounds = Bounds::centered(None, size(px(800.), px(600.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| cox_gui::view::CoxView::new(agent, cx)),
            )
            .expect("the window opens");
        window
            .update(cx, |view, window, cx| window.focus(&view.focus_handle(cx)))
            .expect("the window is open");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
