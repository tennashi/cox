//! One emulator, as a tab of the stage shows it: Cox starts it, with no
//! window of its own, or reaches it if it runs already, then streams its
//! display, at the size the tab draws it, and sends it the human's
//! touches, keys and wheel.
//!
//! The emulator is talked to on a thread of its own, so that the tab only
//! hands it input and picks up the newest frame: [`Device`] is the tab's
//! end of it.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
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
    frame: Arc<Mutex<Option<Frame>>>,
    /// Whether a frame waits in `frame` that the tab was told of.
    told: Arc<AtomicBool>,
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
            frame: Arc::default(),
            told: Arc::default(),
            started: Arc::default(),
        };
        let worker = Worker {
            device: device.to_owned(),
            program,
            boot,
            events,
            frame: this.frame.clone(),
            told: this.told.clone(),
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
        let frame = self.frame.lock().ok()?.take();
        self.told.store(false, Ordering::Release);
        frame
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
    frame: Arc<Mutex<Option<Frame>>>,
    told: Arc<AtomicBool>,
    started: Arc<AtomicI32>,
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
        let mut size = None;
        // Until the tab asks for pictures of its size, as while it is not
        // drawn, the display comes at a size of its own, for an agent to
        // see it by.
        let (width, height) = fitted(display, UNDRAWN_SIDE);
        let mut frames = Some(stream(&mut client, width, height).await?);
        loop {
            tokio::select! {
                input = from_tab.recv() => {
                    let Some(input) = input else {
                        // The tab closed: the device stops as it drops.
                        return Ok(());
                    };
                    if let Input::Size { width, height } = input {
                        if size != Some((width, height)) && width > 0 && height > 0 {
                            size = Some((width, height));
                            frames = Some(stream(&mut client, width, height).await?);
                        }
                    } else {
                        send(&mut client, input, display).await;
                    }
                }
                image = next(frames.as_mut()) => {
                    match image {
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
        let Some(frame) = frame_of(image) else {
            return;
        };
        if let Ok(mut newest) = self.frame.lock() {
            *newest = Some(frame);
        }
        if !self.told.swap(true, Ordering::AcqRel) {
            self.events.send(Event::Frame).ok();
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

/// A port no one listens on now, for the emulator to.
fn free_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
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

async fn next(
    frames: Option<&mut tonic::Streaming<crate::proto::Image>>,
) -> Option<Result<crate::proto::Image, tonic::Status>> {
    match frames {
        Some(frames) => frames.message().await.transpose(),
        None => std::future::pending().await,
    }
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
}
