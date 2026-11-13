//! `cox-fake-emulator` is an Android emulator to run Cox with where none
//! runs, as on a machine with no virtualization, such as one Cox is tried
//! out on with no screen: it takes the arguments Cox starts `emulator`
//! with, writes the file of itself a running emulator writes, and serves
//! the part of the emulator's gRPC service Cox uses, drawing a phone's
//! screen of its own: a clock, a line that shows what is typed, a list
//! that a drag scrolls and a tap chooses a row of, Enter putting the line
//! at its top, and the buttons Back, Home and Overview, which say so.
//!
//! An Android SDK whose `emulator/emulator` is this program, named by
//! `ANDROID_HOME`, is one Cox finds as any other. `cox-fake-emulator
//! --make-avd` writes the device it plays, `Fake_Phone`, in the AVDs'
//! folder that `ANDROID_AVD_HOME` names, for Cox to list. With
//! `--still`, its clock stays at midnight and what Back, Home and
//! Overview say stays up, so that its screen changes only as it is
//! touched, for tests to see the same screen each time.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use cox_android::proto::emulator_controller_server::{
    EmulatorController, EmulatorControllerServer,
};
use cox_android::proto::image_format::ImgFormat;
use cox_android::proto::vm_run_state::RunState;
use cox_android::proto::{
    Image, ImageFormat, KeyboardEvent, Rotation, TouchEvent, VmRunState, WheelEvent,
};
use cox_android::running::{self, Running};
use tokio::sync::{Notify, watch};
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};

/// The device it plays, as `-list-avds` lists it.
const DEVICE: &str = "Fake_Phone";

/// The display, in its own pixels.
const WIDTH_I: i32 = 540;
const HEIGHT_I: i32 = 1170;
const WIDTH: u32 = WIDTH_I.unsigned_abs();
const HEIGHT: u32 = HEIGHT_I.unsigned_abs();

const STATUS_BAR: i32 = 44;
const APP_BAR: i32 = 120;
const LINE: i32 = 200;
const NAV_BAR: i32 = HEIGHT_I - 84;
const ROW: i32 = 96;

/// How far a finger moves before a touch is a drag, not a tap.
const SLOP: i32 = 12;

/// How long Back, Home and Overview say so.
const TOAST: Duration = Duration::from_millis(900);

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-list-avds") {
        println!("{DEVICE}");
        return std::process::ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--make-avd") {
        return make_avd();
    }
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1))
    };
    let device = value("-avd").map_or(DEVICE, String::as_str).to_owned();
    let still = args.iter().any(|arg| arg == "--still");
    let port = match value("-grpc").map(|port| port.parse::<u16>()) {
        Some(Ok(port)) => port,
        None => 8554,
        Some(Err(error)) => {
            eprintln!("cox-fake-emulator: -grpc: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    match runtime.block_on(serve(device, port, still)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cox-fake-emulator: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Writes the device it plays in the AVDs' folder `ANDROID_AVD_HOME`
/// names, and nowhere else, so as not to add a fake to the human's own.
fn make_avd() -> std::process::ExitCode {
    let Some(home) = std::env::var_os("ANDROID_AVD_HOME").filter(|home| !home.is_empty()) else {
        eprintln!("cox-fake-emulator: --make-avd writes in ANDROID_AVD_HOME, which is not set");
        return std::process::ExitCode::FAILURE;
    };
    let config = format!(
        "avd.ini.displayname=Fake Phone\nhw.device.name=cox_fake\n\
         image.sysdir.1=system-images/android-35/default/fake/\n\
         hw.lcd.width={WIDTH}\nhw.lcd.height={HEIGHT}\n"
    );
    match cox_android::avd::write(std::path::Path::new(&home), DEVICE, &config) {
        Ok(folder) => {
            println!("{}", folder.display());
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cox-fake-emulator: --make-avd: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn serve(device: String, port: u16, still: bool) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let _written = running::write(
        &running::folders()[0],
        &Running {
            pid: i32::try_from(std::process::id())?,
            device: device.clone(),
            port,
            token: None,
        },
    )?;
    println!("INFO    | {device} serves gRPC on 127.0.0.1:{port}");
    let (changed, _) = watch::channel(0_u64);
    let fake = Fake {
        screen: Arc::new(Mutex::new(Screen::new(still))),
        changed: changed.clone(),
        shut: Arc::new(Notify::new()),
    };
    let shut = fake.shut.clone();
    if !still {
        tokio::spawn(tick(changed));
    }
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let serving = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(EmulatorControllerServer::new(fake).max_encoding_message_size(usize::MAX))
            .serve_with_incoming(incoming),
    );
    // The streams of the display never end, so the server is not waited
    // for: it goes with the runtime.
    tokio::select! {
        () = shut.notified() => {}
        _ = terminate.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
        served = serving => served??,
    }
    println!("INFO    | {device} shut down");
    Ok(())
}

/// Draws the screen again each second, for its clock, and as a toast
/// goes.
async fn tick(changed: watch::Sender<u64>) {
    let mut every = tokio::time::interval(Duration::from_millis(250));
    let mut second = now_seconds();
    loop {
        every.tick().await;
        if now_seconds() != second {
            second = now_seconds();
            changed.send_modify(|version| *version += 1);
        }
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

struct Fake {
    screen: Arc<Mutex<Screen>>,
    /// Rises each time the screen changes.
    changed: watch::Sender<u64>,
    shut: Arc<Notify>,
}

impl Fake {
    fn change(&self, change: impl FnOnce(&mut Screen)) {
        if let Ok(mut screen) = self.screen.lock() {
            change(&mut screen);
        }
        self.changed.send_modify(|version| *version += 1);
    }
}

#[tonic::async_trait]
impl EmulatorController for Fake {
    async fn send_key(&self, request: Request<KeyboardEvent>) -> Result<Response<()>, Status> {
        let key = request.into_inner();
        self.change(|screen| screen.key(&key));
        Ok(Response::new(()))
    }

    async fn send_touch(&self, request: Request<TouchEvent>) -> Result<Response<()>, Status> {
        let event = request.into_inner();
        self.change(|screen| {
            for touch in &event.touches {
                screen.touch(touch.x, touch.y, touch.pressure > 0);
            }
        });
        Ok(Response::new(()))
    }

    async fn inject_wheel(
        &self,
        request: Request<Streaming<WheelEvent>>,
    ) -> Result<Response<()>, Status> {
        let mut wheel = request.into_inner();
        while let Some(event) = wheel.next().await {
            let event = event?;
            self.change(|screen| screen.scroll_by(-event.dy * ROW / 240));
        }
        Ok(Response::new(()))
    }

    async fn get_screenshot(
        &self,
        request: Request<ImageFormat>,
    ) -> Result<Response<Image>, Status> {
        let format = request.into_inner();
        Ok(Response::new(self.image(&format, 0)?))
    }

    type streamScreenshotStream = tokio_stream::wrappers::ReceiverStream<Result<Image, Status>>;

    async fn stream_screenshot(
        &self,
        request: Request<ImageFormat>,
    ) -> Result<Response<Self::streamScreenshotStream>, Status> {
        let format = request.into_inner();
        let (images, stream) = tokio::sync::mpsc::channel(2);
        let mut changed = self.changed.subscribe();
        let fake = Self {
            screen: self.screen.clone(),
            changed: self.changed.clone(),
            shut: self.shut.clone(),
        };
        tokio::spawn(async move {
            let mut seq = 0;
            loop {
                let image = fake.image(&format, seq);
                if images.send(image).await.is_err() || changed.changed().await.is_err() {
                    break;
                }
                seq += 1;
            }
        });
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            stream,
        )))
    }

    async fn set_vm_state(&self, request: Request<VmRunState>) -> Result<Response<()>, Status> {
        let state = request.into_inner().state();
        if matches!(state, RunState::Shutdown | RunState::Terminate) {
            self.shut.notify_one();
        }
        Ok(Response::new(()))
    }
}

impl Fake {
    /// The screen as `format` asks for it: RGBA, scaled down to fit its
    /// width and height, keeping its shape, as the emulator scales it.
    fn image(&self, format: &ImageFormat, seq: u32) -> Result<Image, Status> {
        if format.format() != ImgFormat::Rgba8888 {
            return Err(Status::unimplemented(
                "cox-fake-emulator draws RGBA8888 only",
            ));
        }
        let pixels = self
            .screen
            .lock()
            .map_err(|_| Status::internal("the screen is lost"))?
            .draw();
        let (width, height, image) = scaled(&pixels, format.width, format.height);
        Ok(Image {
            format: Some(ImageFormat {
                format: ImgFormat::Rgba8888.into(),
                rotation: Some(Rotation::default()),
                width,
                height,
                display: format.display,
            }),
            image,
            seq,
            timestamp_us: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |since| {
                    u64::try_from(since.as_micros()).unwrap_or(u64::MAX)
                }),
            ..Image::default()
        })
    }
}

/// `pixels`, the display's, scaled down to fit `width` by `height`, none
/// of which being 0 leaving it as it is.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "a display's pixels are far below what f32 keeps exactly"
)]
fn scaled(pixels: &[u8], width: u32, height: u32) -> (u32, u32, Vec<u8>) {
    let fit = |want: u32, have: u32| {
        if want == 0 {
            1.
        } else {
            want as f32 / have as f32
        }
    };
    let scale = fit(width, WIDTH).min(fit(height, HEIGHT)).min(1.);
    let out_width = ((WIDTH as f32 * scale) as u32).max(1);
    let out_height = ((HEIGHT as f32 * scale) as u32).max(1);
    let mut out = Vec::with_capacity((out_width * out_height * 4) as usize);
    for y in 0..out_height {
        let from_y = (y * HEIGHT / out_height) as usize;
        for x in 0..out_width {
            let from_x = (x * WIDTH / out_width) as usize;
            let at = (from_y * WIDTH as usize + from_x) * 4;
            out.extend_from_slice(&pixels[at..at + 4]);
        }
    }
    (out_width, out_height, out)
}

/// What the fake phone shows, and how the human's touches and keys
/// change it.
struct Screen {
    rows: Vec<String>,
    /// How far the list is scrolled, in pixels.
    scroll: i32,
    chosen: Option<usize>,
    typed: String,
    /// Where the finger touches, while it does.
    finger: Option<(i32, i32)>,
    /// Where the finger came down, the scroll then, and whether it moved
    /// past [`SLOP`] since.
    down: Option<(i32, i32, i32, bool)>,
    toast: Option<(&'static str, Instant)>,
    /// Whether its clock stays at midnight, and toasts stay up.
    still: bool,
}

impl Screen {
    fn new(still: bool) -> Self {
        Self {
            rows: (1..=40).map(|row| format!("Row {row}")).collect(),
            scroll: 0,
            chosen: None,
            typed: String::new(),
            finger: None,
            down: None,
            toast: None,
            still,
        }
    }

    fn touch(&mut self, x: i32, y: i32, touching: bool) {
        if !touching {
            if let Some((x0, y0, _, false)) = self.down {
                self.tap(x0, y0);
            }
            self.finger = None;
            self.down = None;
            return;
        }
        self.finger = Some((x, y));
        match &mut self.down {
            None => self.down = Some((x, y, self.scroll, false)),
            Some((x0, y0, scroll, moved)) => {
                if (x - *x0).abs() > SLOP || (y - *y0).abs() > SLOP {
                    *moved = true;
                }
                if *moved && *y0 >= LINE && *y0 < NAV_BAR {
                    let to = *scroll - (y - *y0);
                    self.scroll = to;
                    self.scroll_by(0);
                }
            }
        }
    }

    fn tap(&mut self, x: i32, y: i32) {
        if y >= NAV_BAR {
            let third = i32::try_from(WIDTH).unwrap_or(i32::MAX) / 3;
            match x / third {
                0 => self.back(),
                1 => self.home(),
                _ => self.toast = Some(("Overview", Instant::now())),
            }
        } else if y >= LINE {
            let row = usize::try_from((y - LINE + self.scroll) / ROW).unwrap_or(usize::MAX);
            if row < self.rows.len() {
                self.chosen = if self.chosen == Some(row) {
                    None
                } else {
                    Some(row)
                };
            }
        }
    }

    fn back(&mut self) {
        self.chosen = None;
        self.toast = Some(("Back", Instant::now()));
    }

    fn home(&mut self) {
        self.scroll = 0;
        self.chosen = None;
        self.toast = Some(("Home", Instant::now()));
    }

    fn scroll_by(&mut self, by: i32) {
        let rows = i32::try_from(self.rows.len()).unwrap_or(i32::MAX);
        let most = (rows * ROW - (NAV_BAR - LINE)).max(0);
        self.scroll = (self.scroll + by).clamp(0, most);
    }

    fn key(&mut self, key: &KeyboardEvent) {
        if !key.text.is_empty() {
            self.typed.push_str(&key.text);
            return;
        }
        match key.key.as_str() {
            "GoBack" | "Escape" => self.back(),
            "GoHome" => self.home(),
            "AppSwitch" => self.toast = Some(("Overview", Instant::now())),
            "Backspace" => {
                self.typed.pop();
            }
            "Enter" => {
                if !self.typed.is_empty() {
                    self.rows.insert(0, std::mem::take(&mut self.typed));
                    self.scroll = 0;
                    self.chosen = None;
                }
            }
            other if other.chars().count() == 1 => self.typed.push_str(other),
            _ => {}
        }
    }

    /// The screen, as RGBA pixels a row at a time from the top.
    fn draw(&mut self) -> Vec<u8> {
        let mut canvas = Canvas::new();
        canvas.rect(0, 0, WIDTH_I, HEIGHT_I, GROUND);
        // The list, under the bars.
        for (index, row) in self.rows.iter().enumerate() {
            let top = LINE + i32::try_from(index).unwrap_or(i32::MAX) * ROW - self.scroll;
            if top + ROW < LINE || top > NAV_BAR {
                continue;
            }
            let ground = if self.chosen == Some(index) {
                CHOSEN
            } else if index % 2 == 0 {
                GROUND
            } else {
                STRIPE
            };
            canvas.rect(0, top, WIDTH_I, ROW, ground);
            canvas.rect(32, top + ROW - 1, WIDTH_I - 32, 1, RULE);
            canvas.text(32, top + 32, row, 4, INK);
        }
        canvas.rect(0, 0, WIDTH_I, STATUS_BAR, DARK);
        let clock = if self.still {
            "00:00:00".to_owned()
        } else {
            clock()
        };
        canvas.text(20, 14, &clock, 2, WHITE);
        canvas.text(WIDTH_I - 20 - 8 * 2 * 8, 14, "cox fake", 2, FAINT);
        canvas.rect(0, STATUS_BAR, WIDTH_I, APP_BAR - STATUS_BAR, TEAL);
        canvas.text(24, STATUS_BAR + 24, DEVICE, 4, WHITE);
        canvas.rect(0, APP_BAR, WIDTH_I, LINE - APP_BAR, WHITE);
        canvas.rect(20, APP_BAR + 16, WIDTH_I - 40, LINE - APP_BAR - 32, FIELD);
        let line = if self.typed.is_empty() {
            ("Type, then Enter".to_owned(), FAINT)
        } else {
            (format!("{}_", self.typed), INK)
        };
        canvas.text(36, APP_BAR + 30, &line.0, 3, line.1);
        canvas.rect(0, LINE - 1, WIDTH_I, 1, RULE);
        // The buttons of Android's: Back, Home and Overview.
        canvas.rect(0, NAV_BAR, WIDTH_I, HEIGHT_I - NAV_BAR, DARK);
        let third = WIDTH_I / 3;
        let middle = NAV_BAR + (HEIGHT_I - NAV_BAR) / 2;
        canvas.text(third / 2 - 8, middle - 8, "<", 2, WHITE);
        canvas.circle(third + third / 2, middle, 12, WHITE);
        canvas.rect(2 * third + third / 2 - 11, middle - 11, 22, 22, WHITE);
        if let Some((said, since)) = self.toast {
            if self.still || since.elapsed() < TOAST {
                let width = i32::try_from(said.len()).unwrap_or(0) * 8 * 3 + 48;
                let left = (WIDTH_I - width) / 2;
                canvas.rect(left, NAV_BAR - 110, width, 56, DARK);
                canvas.text(left + 24, NAV_BAR - 94, said, 3, WHITE);
            } else {
                self.toast = None;
            }
        }
        if let Some((x, y)) = self.finger {
            canvas.circle(x, y, 34, FINGER);
        }
        canvas.pixels
    }
}

const GROUND: [u8; 4] = [250, 250, 249, 255];
const STRIPE: [u8; 4] = [241, 245, 244, 255];
const CHOSEN: [u8; 4] = [204, 240, 233, 255];
const RULE: [u8; 4] = [220, 224, 223, 255];
const FIELD: [u8; 4] = [238, 241, 240, 255];
const INK: [u8; 4] = [28, 32, 33, 255];
const FAINT: [u8; 4] = [140, 148, 147, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const DARK: [u8; 4] = [27, 27, 31, 255];
const TEAL: [u8; 4] = [15, 118, 110, 255];
const FINGER: [u8; 4] = [60, 60, 60, 90];

/// The time now, as the status bar shows it.
fn clock() -> String {
    let seconds = now_seconds() % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

struct Canvas {
    pixels: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            pixels: vec![0; (WIDTH * HEIGHT * 4) as usize],
        }
    }

    /// Lays `color` over the pixel at `x`, `y`, by its alpha.
    fn dot(&mut self, x: i32, y: i32, color: [u8; 4]) {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return;
        };
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let at = ((y * WIDTH + x) * 4) as usize;
        let alpha = u16::from(color[3]);
        for (channel, over) in color.iter().take(3).enumerate() {
            let under = u16::from(self.pixels[at + channel]);
            let over = u16::from(*over);
            self.pixels[at + channel] =
                u8::try_from((over * alpha + under * (255 - alpha)) / 255).unwrap_or(u8::MAX);
        }
        self.pixels[at + 3] = 255;
    }

    fn rect(&mut self, left: i32, top: i32, width: i32, height: i32, color: [u8; 4]) {
        let (from, to) = (left.max(0), (left + width).min(WIDTH_I));
        for y in top.max(0)..(top + height).min(HEIGHT_I) {
            if color[3] == u8::MAX && from < to {
                // An opaque color is copied, a row at a time.
                let row = usize::try_from(y * WIDTH_I).unwrap_or(0);
                let (from, to) = (
                    usize::try_from(from).unwrap_or(0),
                    usize::try_from(to).unwrap_or(0),
                );
                for pixel in self.pixels[(row + from) * 4..(row + to) * 4]
                    .as_chunks_mut::<4>()
                    .0
                {
                    pixel.copy_from_slice(&color);
                }
            } else {
                for x in from..to {
                    self.dot(x, y, color);
                }
            }
        }
    }

    fn circle(&mut self, cx: i32, cy: i32, radius: i32, color: [u8; 4]) {
        for y in -radius..=radius {
            for x in -radius..=radius {
                if x * x + y * y <= radius * radius {
                    self.dot(cx + x, cy + y, color);
                }
            }
        }
    }

    /// `text` in the 8 by 8 letters of `font8x8`, each pixel of them
    /// `scale` pixels wide; a letter it has none of shows as `?`.
    fn text(&mut self, left: i32, top: i32, text: &str, scale: i32, color: [u8; 4]) {
        for (index, letter) in text.chars().enumerate() {
            let glyph = font8x8::legacy::BASIC_LEGACY
                .get(letter as usize)
                .unwrap_or(&font8x8::legacy::BASIC_LEGACY['?' as usize]);
            let x0 = left + i32::try_from(index).unwrap_or(i32::MAX) * 8 * scale;
            for (row, bits) in glyph.iter().enumerate() {
                for column in 0..8 {
                    if bits >> column & 1 == 1 {
                        let row = i32::try_from(row).unwrap_or(0);
                        self.rect(x0 + column * scale, top + row * scale, scale, scale, color);
                    }
                }
            }
        }
    }
}
