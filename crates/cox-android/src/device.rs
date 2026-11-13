//! One emulator, as a tab of the stage shows it: Cox starts it, with no
//! window of its own, or reaches it if it runs already, then streams its
//! display, at the size the tab draws it, and sends it the human's
//! touches, keys and wheel.
//!
//! The emulator is talked to on a thread of its own, so that the tab only
//! hands it input and picks up the newest frame: [`Device`] is the tab's
//! end of it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint};

use crate::proto::emulator_controller_client::EmulatorControllerClient;
use crate::proto::image_format::ImgFormat;
use crate::proto::keyboard_event::KeyEventType;
use crate::proto::{ImageFormat, KeyboardEvent, Touch, TouchEvent, WheelEvent};
use crate::running::{self, Running};

/// How long an emulator Cox starts has to answer, as it boots.
const START_TIMEOUT: Duration = Duration::from_mins(3);

/// How often Cox asks an emulator that has not answered yet.
const RETRY: Duration = Duration::from_millis(500);

/// The longer side of the display's pictures, in pixels, until the tab
/// asks for pictures of its size.
const UNDRAWN_SIDE: u32 = 1280;

/// The most of what the emulator printed that Cox says when it ends.
const KEPT_LINES: usize = 12;

/// The tab's end of one emulator.
pub struct Device {
    name: String,
    to: UnboundedSender<Input>,
    newest: Arc<Newest>,
    /// The process of the emulator Cox started, once it runs.
    started: Arc<AtomicI32>,
}

/// What a device tells the tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// How the device stands now.
    Status(Status),
    /// A new frame waits for [`Device::take_frame`].
    Frame,
}

/// How a device stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Cox started the emulator, which has not answered yet.
    Starting,
    /// The emulator ran already, and Cox reaches it.
    Reaching,
    /// The display shows.
    Showing,
    /// The emulator ended, or could not be reached, and why.
    Ended(String),
}

/// The display as the emulator last drew it, at the size the tab asked
/// for, or less.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// The pixels, a row at a time from the top, four bytes each: blue,
    /// green, red and alpha, as GPUI keeps images.
    pub bgra: Vec<u8>,
    /// How many frames the emulator made since the stream began, counting
    /// those it dropped.
    pub seq: u32,
}

/// How an emulator Cox starts boots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Boot {
    /// From nothing, not from the state it saved as it stopped last.
    pub cold: bool,
    /// With the device's data taken away, as when it was new.
    pub wipe: bool,
}

/// What the tab sends the emulator.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// The size the tab draws the display at, in pixels: frames come at
    /// that size, or less, keeping the display's shape.
    Size { width: u32, height: u32 },
    /// A finger at `x` and `y`, from 0 to 1 across the display, touching
    /// while `down` is, and lifting as it is not.
    Touch { x: f32, y: f32, down: bool },
    /// A key, as the W3C names it, as `Enter`, or one of Android's, as
    /// `GoBack`, `GoHome` or `AppSwitch`.
    Key(String),
    /// Text typed.
    Text(String),
    /// The wheel turned, 120 a click.
    Wheel { dx: i32, dy: i32 },
}

impl Device {
    /// The emulator of `device`, by its name as `emulator -avd` takes it:
    /// reached if it runs, or else started with `program`, booting as
    /// `boot` says.
    #[must_use]
    pub fn open(
        program: Option<PathBuf>,
        device: &str,
        boot: Boot,
    ) -> (Self, UnboundedReceiver<Event>) {
        let (to, from_tab) = unbounded_channel();
        let (events, from_device) = unbounded_channel();
        let this = Self {
            name: device.to_owned(),
            to,
            newest: Arc::default(),
            started: Arc::default(),
        };
        let worker = Worker {
            device: device.to_owned(),
            program,
            boot,
            events,
            newest: this.newest.clone(),
            started: this.started.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name(format!("emulator {device}"))
            .spawn(move || worker.run(from_tab));
        if let Err(error) = spawned {
            tracing::error!("no thread for the emulator: {error}");
        }
        (this, from_device)
    }

    /// The device, by its name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sends `input` to the emulator.
    pub fn send(&self, input: Input) {
        self.to.send(input).ok();
    }

    /// The newest frame, if one came since the last taken.
    #[must_use]
    pub fn take_frame(&self) -> Option<Frame> {
        self.newest.take()
    }

    /// Stops the emulator, if Cox started it: one that ran already goes
    /// on. It shuts down as it does when its window closes, saving its
    /// state for the next start, whether Cox waits for it or not.
    pub fn stop(&self) {
        let pid = self.started.swap(0, Ordering::AcqRel);
        if pid > 0 {
            // SAFETY: the emulator is a process Cox started, by its id.
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The thread's end of one emulator.
struct Worker {
    device: String,
    program: Option<PathBuf>,
    boot: Boot,
    events: UnboundedSender<Event>,
    newest: Arc<Newest>,
    started: Arc<AtomicI32>,
}

/// The newest frame, which the thread puts and the tab takes.
#[derive(Default)]
struct Newest {
    frame: Mutex<Option<Frame>>,
    /// Whether the tab was told of the frame waiting, and has not taken
    /// it since.
    told: AtomicBool,
}

impl Newest {
    /// Keeps `frame` as the newest, and tells the tab by `events`, unless
    /// it was told already of one it has not taken.
    fn put(&self, frame: Frame, events: &UnboundedSender<Event>) {
        if let Ok(mut newest) = self.frame.lock() {
            *newest = Some(frame);
        }
        if !self.told.swap(true, Ordering::AcqRel) {
            events.send(Event::Frame).ok();
        }
    }

    /// The newest frame, if one came since the last taken. The tab is
    /// readied to be told before the frame is taken, so that one put as it
    /// is taken is told of, not left waiting untold.
    fn take(&self) -> Option<Frame> {
        self.told.store(false, Ordering::Release);
        self.frame.lock().ok()?.take()
    }
}

impl Worker {
    fn run(self, from_tab: UnboundedReceiver<Input>) {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                self.status(Status::Ended(format!(
                    "no runtime for the emulator: {error}"
                )));
                return;
            }
        };
        let ended = runtime.block_on(self.work(from_tab));
        if let Err(error) = ended {
            self.status(Status::Ended(format!("{error:#}")));
        }
    }

    fn status(&self, status: Status) {
        self.events.send(Event::Status(status)).ok();
    }

    async fn work(&self, mut from_tab: UnboundedReceiver<Input>) -> anyhow::Result<()> {
        let (running, mut child) = if let Some(running) = running::of(&self.device) {
            self.status(Status::Reaching);
            (running, None)
        } else {
            self.status(Status::Starting);
            let (running, child) = self.start()?;
            (running, Some(child))
        };
        let printed = Arc::new(Mutex::new(Vec::new()));
        if let Some(child) = &mut child {
            keep_printed(child, &printed);
        }
        let (mut client, display) = tokio::select! {
            reached = reach(&running) => reached?,
            ended = wait(child.as_mut()) => {
                anyhow::bail!("the emulator ended before it answered{}", said(&printed, &ended));
            }
        };
        self.status(Status::Showing);
        // What the tab sent as the emulator started is taken first, so
        // that the first frames come at the size it asked for last.
        let mut size = None;
        loop {
            match from_tab.try_recv() {
                Ok(Input::Size { width, height }) => {
                    size = resized(size, width, height).or(size);
                }
                Ok(input) => send(&mut client, input, display).await,
                Err(TryRecvError::Empty) => break,
                // The tab closed: the device stops as it drops.
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        // Until the tab asks for pictures of its size, as while it is not
        // drawn, the display comes at a size of its own, for an agent to
        // see it by.
        let (width, height) = size.unwrap_or_else(|| fitted(display, UNDRAWN_SIDE));
        let mut frames = stream(&mut client, width, height).await?;
        loop {
            tokio::select! {
                input = from_tab.recv() => {
                    let Some(input) = input else {
                        // The tab closed: the device stops as it drops.
                        return Ok(());
                    };
                    if let Input::Size { width, height } = input {
                        if let Some(new) = resized(size, width, height) {
                            size = Some(new);
                            frames = stream(&mut client, new.0, new.1).await?;
                        }
                    } else {
                        send(&mut client, input, display).await;
                    }
                }
                image = frames.message() => {
                    match image.transpose() {
                        Some(image) => self.show(image?),
                        None => anyhow::bail!("the emulator stopped sending its display"),
                    }
                }
                ended = wait(child.as_mut()) => {
                    anyhow::bail!("the emulator ended{}", said(&printed, &ended));
                }
            }
        }
    }

    /// Starts the emulator of the device, with no window, its gRPC service
    /// on a port free now, which asks for no token.
    fn start(&self) -> anyhow::Result<(Running, tokio::process::Child)> {
        let program = self
            .program
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no Android SDK's emulator is found"))?;
        let port = free_port()?;
        let mut command = command(program);
        command
            .args(["-avd", &self.device, "-grpc", &port.to_string()])
            .args(["-qt-hide-window", "-no-boot-anim"]);
        if self.boot.cold {
            command.arg("-no-snapshot-load");
        }
        if self.boot.wipe {
            command.arg("-wipe-data");
        }
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(false)
            .spawn()
            .map_err(|error| anyhow::anyhow!("cannot start `{}`: {error}", program.display()))?;
        let pid = child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .unwrap_or_default();
        self.started.store(pid, Ordering::Release);
        let running = Running {
            pid,
            device: self.device.clone(),
            port,
            token: None,
        };
        Ok((running, child))
    }

    /// Keeps `image` as the newest frame, and tells the tab, unless it was
    /// told already of one it has not taken.
    fn show(&self, image: crate::proto::Image) {
        if let Some(frame) = frame_of(image) {
            self.newest.put(frame, &self.events);
        }
    }
}

/// The emulator's program, run as a command of its own.
fn command(program: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    // Its own process group, so that Control-C in Cox's terminal does not
    // reach it.
    command.process_group(0);
    command
}

/// The ports Cox hands the emulators it starts: below those the system
/// hands out for connections, on macOS and on Linux, so that none is
/// taken by one between Cox finding it free and the emulator listening on
/// it.
const PORTS: std::ops::Range<u16> = 20_000..30_000;

/// The ports this process handed out, so that emulators it starts at once
/// are not handed the same.
static HANDED: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

/// A port no one listens on now, among [`PORTS`], for the emulator to:
/// they are tried from one of the process's own, so that two processes
/// starting emulators at once seldom try the same.
///
/// A port is tried by connecting to it, not by listening on it: on macOS
/// a socket is made to close as a program starts only after it is made,
/// so an emulator another thread starts in between could keep it, and
/// with it the port, which the emulator it is handed to could then not
/// listen on.
fn free_port() -> anyhow::Result<u16> {
    let mut handed = HANDED.lock().unwrap_or_else(PoisonError::into_inner);
    let count = u32::from(PORTS.end - PORTS.start);
    // Processes started one after another have ids next to each other,
    // which this spreads across the ports.
    let first = std::process::id().wrapping_mul(2_654_435_761) % count;
    let port = (0..count)
        .filter_map(|offset| u16::try_from((first + offset) % count).ok())
        .map(|offset| PORTS.start + offset)
        .find(|port| !handed.contains(port) && !listened_on(*port))
        .ok_or_else(|| anyhow::anyhow!("no port is free for the emulator"))?;
    handed.insert(port);
    Ok(port)
}

/// Whether anything listens on `port` of this machine, or may: only a
/// connection refused says nothing does.
fn listened_on(port: u16) -> bool {
    !matches!(
        std::net::TcpStream::connect(("127.0.0.1", port)),
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused
    )
}

type Client =
    EmulatorControllerClient<tonic::service::interceptor::InterceptedService<Channel, Authorize>>;

/// Adds the token the emulator asks for to each call, if it asks for one.
#[derive(Clone)]
struct Authorize(Option<MetadataValue<tonic::metadata::Ascii>>);

impl tonic::service::Interceptor for Authorize {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, tonic::Status> {
        if let Some(token) = &self.0 {
            request
                .metadata_mut()
                .insert("authorization", token.clone());
        }
        Ok(request)
    }
}

/// The emulator's service once it answers with its display, and the
/// display's size in its own pixels, which touches are in.
async fn reach(running: &Running) -> anyhow::Result<(Client, (u32, u32))> {
    let token = match &running.token {
        Some(token) => Some(format!("Bearer {token}").parse()?),
        None => None,
    };
    let endpoint = Endpoint::from_shared(format!("http://127.0.0.1:{}", running.port))?;
    let waited = tokio::time::Instant::now();
    let mut last = String::new();
    while waited.elapsed() < START_TIMEOUT {
        match endpoint.connect().await {
            Ok(channel) => {
                let mut client =
                    EmulatorControllerClient::with_interceptor(channel, Authorize(token.clone()))
                        // A frame of a whole display is 10 MB and more.
                        .max_decoding_message_size(usize::MAX);
                match display(&mut client).await {
                    Ok(Some(size)) => return Ok((client, size)),
                    Ok(None) => "the display is not on yet".clone_into(&mut last),
                    Err(error) => error.message().clone_into(&mut last),
                }
            }
            Err(error) => last = error.to_string(),
        }
        tokio::time::sleep(RETRY).await;
    }
    anyhow::bail!(
        "the emulator did not answer on port {}: {last}",
        running.port
    )
}

/// The display's size, as a screenshot at its own size has it, or none
/// while it is not on.
async fn display(client: &mut Client) -> Result<Option<(u32, u32)>, tonic::Status> {
    let format = ImageFormat {
        format: ImgFormat::Rgba8888.into(),
        ..ImageFormat::default()
    };
    let image = client.get_screenshot(format).await?.into_inner();
    let (width, height) = size_of(&image);
    Ok((width > 0 && height > 0).then_some((width, height)))
}

/// `size`, made smaller if need be, so that its longer side is `most_side`
/// pixels at most.
fn fitted(size: (u32, u32), most_side: u32) -> (u32, u32) {
    let longer = size.0.max(size.1).max(1);
    if longer <= most_side {
        return size;
    }
    let scaled = |length: u32| {
        let length = u64::from(length) * u64::from(most_side) / u64::from(longer);
        u32::try_from(length).unwrap_or(most_side).max(1)
    };
    (scaled(size.0), scaled(size.1))
}

fn size_of(image: &crate::proto::Image) -> (u32, u32) {
    #[expect(deprecated, reason = "older emulators say the size only there")]
    let old = (image.width, image.height);
    match &image.format {
        Some(format) if format.width > 0 => (format.width, format.height),
        _ => old,
    }
}

/// The display's frames, each time it changes, at `width` by `height`
/// pixels or less.
async fn stream(
    client: &mut Client,
    width: u32,
    height: u32,
) -> anyhow::Result<tonic::Streaming<crate::proto::Image>> {
    let format = ImageFormat {
        format: ImgFormat::Rgba8888.into(),
        width,
        height,
        ..ImageFormat::default()
    };
    Ok(client.stream_screenshot(format).await?.into_inner())
}

/// The size the tab asks for, `width` by `height`, if it is not `size`,
/// which the frames come at now, and has pixels.
fn resized(size: Option<(u32, u32)>, width: u32, height: u32) -> Option<(u32, u32)> {
    (size != Some((width, height)) && width > 0 && height > 0).then_some((width, height))
}

/// Sends `input` to the emulator, whose display is `display` pixels: what
/// it does not take is said in the log, and the human tries again.
async fn send(client: &mut Client, input: Input, display: (u32, u32)) {
    let sent = match input {
        Input::Size { .. } => return,
        Input::Touch { x, y, down } => {
            let touch = Touch {
                x: across(x, display.0),
                y: across(y, display.1),
                pressure: i32::from(down),
                ..Touch::default()
            };
            client
                .send_touch(TouchEvent {
                    touches: vec![touch],
                    display: 0,
                })
                .await
                .map(drop)
        }
        Input::Key(key) => client
            .send_key(KeyboardEvent {
                event_type: KeyEventType::Keypress.into(),
                key,
                ..KeyboardEvent::default()
            })
            .await
            .map(drop),
        Input::Text(text) => client
            .send_key(KeyboardEvent {
                text,
                ..KeyboardEvent::default()
            })
            .await
            .map(drop),
        Input::Wheel { dx, dy } => {
            let wheel = WheelEvent { dx, dy, display: 0 };
            client
                .inject_wheel(tokio_stream::iter([wheel]))
                .await
                .map(drop)
        }
    };
    if let Err(error) = sent {
        tracing::warn!("the emulator did not take the input: {}", error.message());
    }
}

/// `at`, from 0 to 1, as a pixel of `pixels`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "a display's pixels are far below what f32 keeps exactly"
)]
fn across(at: f32, pixels: u32) -> i32 {
    let last = pixels.saturating_sub(1) as f32;
    (at.clamp(0., 1.) * last).round() as i32
}

/// `image` as a frame for the tab, unless it is empty, as while the
/// display is off.
fn frame_of(image: crate::proto::Image) -> Option<Frame> {
    let (width, height) = size_of(&image);
    let mut bgra = image.image;
    let pixels = usize::try_from(u64::from(width) * u64::from(height)).ok()?;
    if pixels == 0 || bgra.len() != pixels * 4 {
        return None;
    }
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(Frame {
        width,
        height,
        bgra,
        seq: image.seq,
    })
}

/// Keeps the last lines the emulator prints, to say why it ended.
fn keep_printed(child: &mut tokio::process::Child, printed: &Arc<Mutex<Vec<String>>>) {
    fn keep(from: impl AsyncRead + Unpin + Send + 'static, printed: Arc<Mutex<Vec<String>>>) {
        tokio::spawn(async move {
            let mut lines = BufReader::new(from).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "cox_android::emulator", "{line}");
                if let Ok(mut printed) = printed.lock() {
                    if printed.len() == KEPT_LINES {
                        printed.remove(0);
                    }
                    printed.push(line);
                }
            }
        });
    }
    if let Some(stdout) = child.stdout.take() {
        keep(stdout, printed.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        keep(stderr, printed.clone());
    }
}

/// Waits for `child` to end, if there is one, and forever otherwise.
async fn wait(child: Option<&mut tokio::process::Child>) -> String {
    match child {
        Some(child) => match child.wait().await {
            Ok(status) => status.to_string(),
            Err(error) => error.to_string(),
        },
        None => std::future::pending().await,
    }
}

/// How the emulator ended, and the last of what it printed.
fn said(printed: &Mutex<Vec<String>>, ended: &str) -> String {
    let lines = printed
        .lock()
        .map(|lines| lines.join("\n"))
        .unwrap_or_default();
    if lines.is_empty() {
        format!(" ({ended})")
    } else {
        format!(" ({ended}):\n{lines}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_handed_out_are_not_handed_again_and_none_is_the_systems() {
        let ports: BTreeSet<u16> = (0..20).map(|_| free_port().unwrap()).collect();
        assert_eq!(ports.len(), 20, "each port once");
        assert!(ports.iter().all(|port| PORTS.contains(port)));
    }

    #[test]
    fn a_port_is_listened_on_while_a_listener_has_it() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(listened_on(port));
        drop(listener);
        assert!(!listened_on(port));
    }

    #[test]
    fn an_undrawn_display_comes_no_bigger_than_its_side() {
        assert_eq!(fitted((1080, 2424), 1280), (570, 1280));
        assert_eq!(fitted((540, 1170), 1280), (540, 1170));
        assert_eq!(fitted((2560, 1600), 1280), (1280, 800));
    }

    #[test]
    fn a_touch_goes_to_the_pixel_across_the_display() {
        assert_eq!(across(0., 1080), 0);
        assert_eq!(across(1., 1080), 1079);
        assert_eq!(across(0.5, 1081), 540);
        assert_eq!(across(-0.2, 1080), 0);
    }

    #[test]
    fn a_frame_comes_as_bgra() {
        let image = crate::proto::Image {
            format: Some(ImageFormat {
                width: 2,
                height: 1,
                ..ImageFormat::default()
            }),
            image: vec![1, 2, 3, 4, 5, 6, 7, 8],
            seq: 3,
            ..crate::proto::Image::default()
        };
        let frame = frame_of(image).expect("a frame");
        assert_eq!((frame.width, frame.height, frame.seq), (2, 1, 3));
        assert_eq!(frame.bgra, [3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn an_empty_image_is_no_frame() {
        assert!(frame_of(crate::proto::Image::default()).is_none());
    }

    fn frame(seq: u32) -> Frame {
        Frame {
            width: 1,
            height: 1,
            bgra: vec![0; 4],
            seq,
        }
    }

    #[test]
    fn the_tab_is_told_once_of_frames_it_has_not_taken_and_takes_the_newest() {
        let (events, mut told) = unbounded_channel();
        let newest = Newest::default();
        newest.put(frame(1), &events);
        newest.put(frame(2), &events);
        assert_eq!(told.try_recv(), Ok(Event::Frame));
        assert!(told.try_recv().is_err(), "told once");
        assert_eq!(newest.take().map(|frame| frame.seq), Some(2));
        assert!(newest.take().is_none(), "a frame is taken once");
    }

    #[test]
    fn a_frame_put_after_one_is_taken_is_told_again() {
        let (events, mut told) = unbounded_channel();
        let newest = Newest::default();
        newest.put(frame(1), &events);
        assert_eq!(told.try_recv(), Ok(Event::Frame));
        assert!(newest.take().is_some());
        newest.put(frame(2), &events);
        assert_eq!(told.try_recv(), Ok(Event::Frame));
        assert_eq!(newest.take().map(|frame| frame.seq), Some(2));
    }
}
