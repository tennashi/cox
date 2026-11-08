//! Off macOS: no windows of other apps to show.

use tokio::sync::mpsc::UnboundedSender;

use crate::{Event, Input, Shared, Window};

pub(crate) fn windows() -> anyhow::Result<Vec<Window>> {
    anyhow::bail!("the windows of other apps are shown on macOS only")
}

pub(crate) fn may_record() -> bool {
    false
}

pub(crate) fn ask_to_record() {}

pub(crate) fn may_control() -> bool {
    false
}

pub(crate) fn ask_to_control() {}

pub(crate) fn bring_to_front(_pid: i32) {}

pub(crate) fn idle() -> Option<std::time::Duration> {
    None
}

pub(crate) fn mirror(
    _id: u32,
    _shared: &std::sync::Arc<Shared>,
    _inputs: &std::sync::mpsc::Receiver<Input>,
    events: &UnboundedSender<Event>,
) {
    let _ = events.send(Event::Ended(
        "The windows of other apps are shown on macOS only.".to_owned(),
    ));
}
