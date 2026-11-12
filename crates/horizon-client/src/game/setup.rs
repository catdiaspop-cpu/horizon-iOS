//! A phone build started without the game files (`missing_resources`): this screen downloads
//! them from Mojang here, in the app, and then hands over to the loading screen. Android's
//! `SetupActivity` does the same job in its own window; iOS gets one `UIApplicationMain` per
//! process, so a second window and a second event loop are out, and the download is a state of
//! the game's own loop instead (`App::Setup` -> `App::Loading`).
use super::*;
// The screen's text and plates, in Horizon's design language: the same pieces the "start it from
// the launcher" screen uses (`horizon_client::no_resources`).
use horizon_client::setup::{self, Progress};
use horizon_client::ui_theme::{self, Face, Ui, color};
use winit::event::TouchPhase;

/// The screen while it waits and downloads, and what the loading screen needs once the
/// files are here.
pub(super) struct Setup {
    options: Option<Options>,
    /// Handed to the loading screen, which reads its settings again.
    pub(super) settings: Settings,
    vanilla_root: PathBuf,
    pack_error: Option<String>,
    started: Instant,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    gui: UiResources,
    /// The finger or pointer, in physical pixels.
    pointer: (f64, f64),
    /// The player pressed Download: the screen draws the engine's progress from here on.
    downloading: bool,
    /// The stage the last frame said (`setup::Progress::stage`), so a finished stage asks for
    /// one redraw rather than one a frame.
    shown: String,
    /// Why the screen stopped (`App` exits the loop with it).
    pub(super) error: Option<Box<dyn std::error::Error>>,
}

impl Setup {
    pub(super) fn new(
        options: Options,
        settings: Settings,
        vanilla_root: PathBuf,
        pack_error: Option<String>,
        started: Instant,
    ) -> Self {
        let gui = UiResources {
            // The fonts bundled in the executable: none of Minecraft's files are here yet.
            atlas: horizon_client::hud::HudAtlas::boot(),
            panorama: Arc::new(horizon_client::gfx::panorama::Faces::colour([0; 4])),
        };
        horizon_client::hud::GUI_SCALE.store(settings.gui_scale, Ordering::Relaxed);
        Self {
            options: Some(options),
            settings,
            vanilla_root,
            pack_error,
            started,
            window: None,
            renderer: None,
            gui,
            pointer: (0.0, 0.0),
            downloading: false,
            shown: String::new(),
            error: None,
        }
    }

    /// The download finished and its files are there: the loading screen takes over from here
    /// (`make_loading`). Files left damaged by an earlier copy don't count: the player presses
    /// Download, and the engine checks and replaces what is wrong.
    pub(super) fn ready(&self) -> bool {
        setup::progress().done && self.resources().is_some_and(|dir| setup::ready(&dir))
    }

    /// Where the engine writes: `HORIZON_RESOURCES` (`entry::prepare_platform` sets the game
    /// folder's `resources` on a build without them in the bundle).
    fn resources(&self) -> Option<PathBuf> {
        std::env::var_os("HORIZON_RESOURCES")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| Some(self.vanilla_root.clone()))
    }

    /// Takes the launch's inputs for `make_loading`, once `ready`.
    pub(super) fn take_inputs(
        &mut self,
    ) -> Option<(Options, Settings, PathBuf, Option<String>, Instant)> {
        let options = self.options.take()?;
        Some((
            options,
            self.settings.clone(),
            self.vanilla_root.clone(),
            self.pack_error.clone(),
            self.started,
        ))
    }

    fn size(&self) -> [u32; 2] {
        self.renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height])
    }

    fn draw(&mut self) {
        let progress = setup::progress();
        // A stage change is worth a frame; the progress inside one is not.
        if progress.stage != self.shown {
            self.shown = progress.stage.clone();
        }
        let mut batch = HudBatch::new(self.size());
        let cursor = (
            self.pointer.0 as f32 / batch.scale,
            self.pointer.1 as f32 / batch.scale,
        );
        {
            let mut ui = Ui::new(&self.gui.atlas, &mut batch, cursor);
            let error = self.error.as_ref().map(ToString::to_string);
            let error = error
                .or_else(|| (!progress.error.is_empty()).then(|| progress.error.clone()));
            draw_screen(&mut ui, &progress, self.downloading, error.as_deref());
            ui.finish();
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match renderer.render(&loading_frame(&batch)) {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => self.error = Some(e),
        }
    }

    /// Starts the download when the pointer is on the button.
    fn click(&mut self, event_loop: &ActiveEventLoop) {
        let progress = setup::progress();
        if progress.running || progress.done {
            return;
        }
        let mut batch = HudBatch::new(self.size());
        let cursor = (
            self.pointer.0 as f32 / batch.scale,
            self.pointer.1 as f32 / batch.scale,
        );
        let error = self.error.as_ref().map(ToString::to_string);
        let error = error.or_else(|| (!progress.error.is_empty()).then(|| progress.error.clone()));
        let mut ui = Ui::new(&self.gui.atlas, &mut batch, cursor);
        let layout = layout(&ui, &progress, error.as_deref());
        let pressed = layout.button.contains(ui.cursor);
        ui.finish();
        if !pressed {
            return;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        let Some(home) = home() else {
            self.error = Some("no game folder to download into".into());
            return;
        };
        log::info!("Horizon: downloading the game files into the game folder");
        self.downloading = true;
        self.error = None;
        setup::start(home, setup::default_url().to_string());
    }

    pub(super) fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            // The app came back from the background: the surface has to be made again.
            if let Some(renderer) = &mut self.renderer
                && let Err(e) = renderer.regain_surface()
            {
                log::warn!("Horizon: surface not recreated: {e}");
            }
            return;
        }
        let attributes = horizon_client::window::with_horizon_icon(
            Window::default_attributes().with_title("Horizon"),
        )
        .with_inner_size(winit::dpi::LogicalSize::new(1040.0, 700.0));
        let made = event_loop
            .create_window(attributes)
            .map_err(Into::into)
            .and_then(|window| {
                let window = Arc::new(window);
                let renderer = loading_renderer(
                    window.clone(),
                    false,
                    Some(&self.gui.atlas),
                    Some(&self.gui.panorama),
                    None,
                    self.settings.horizon.api,
                )?;
                Ok((window, renderer))
            });
        match made {
            Ok((window, renderer)) => {
                window.request_redraw();
                self.window = Some(window);
                self.renderer = Some(renderer);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    pub(super) fn suspended(&mut self, _event_loop: &ActiveEventLoop) {}

    pub(super) fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone().filter(|w| w.id() == id) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
                window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            } => self.click(event_loop),
            // A phone's screen is all it has: the finger presses the button as a mouse would.
            WindowEvent::Touch(touch) => {
                self.pointer = (touch.location.x, touch.location.y);
                match touch.phase {
                    TouchPhase::Ended => self.click(event_loop),
                    TouchPhase::Started | TouchPhase::Moved => window.request_redraw(),
                    TouchPhase::Cancelled => {}
                }
            }
            _ => {}
        }
    }

    /// Wakes up while the download runs, so the bar moves without input.
    pub(super) fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() {
            return;
        }
        if self.ready() {
            return;
        }
        let wait = if setup::progress().running {
            Duration::from_millis(200)
        } else {
            Duration::from_millis(1000)
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + wait));
    }
}

/// The game folder: `HORIZON_HOME` (the app's `Documents`), else the working directory, which
/// `entry::prepare_platform` sets to it.
fn home() -> Option<PathBuf> {
    std::env::var_os("HORIZON_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
}

const TITLE: &str = "Download Minecraft's files";
const BODY: [&str; 2] = [
    "Horizon runs on your own copy of Minecraft: Java Edition, and takes its files from Mojang. \
     This download is about 520 MB, once.",
    "Keep the app open until it finishes: the next start goes straight to the game.",
];
const COLUMN: f32 = 620.0;
const TITLE_SIZE: f32 = horizon_theme::TITLE.px;
const TITLE_LINE: f32 = 40.0;
const BODY_SIZE: f32 = horizon_theme::BODY.px;
const LINE: f32 = 24.0;
const PARAGRAPH_GAP: f32 = 10.0;
const BUTTON_TEXT: f32 = horizon_theme::UI.px;
const SMALL_LINE: f32 = 18.0;
const CONTROL: f32 = horizon_theme::CONTROL_LARGE;
const BAR: f32 = 8.0;
const BUTTON_WIDTH: f32 = 340.0;

struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn contains(&self, (cx, cy): (f32, f32)) -> bool {
        cx >= self.x && cx < self.x + self.w && cy >= self.y && cy < self.y + self.h
    }
}

struct Layout {
    title: Rect,
    lines: Vec<Vec<String>>,
    bar: Rect,
    stage_y: f32,
    button: Rect,
    error_y: f32,
}

/// One layout for drawing and for clicks: where the title, the paragraphs, the bar, the stage
/// line, the button and the error go.
fn layout(ui: &Ui, progress: &Progress, error: Option<&str>) -> Layout {
    let width = COLUMN.min(ui.w - 64.0).max(240.0);
    let x = ((ui.w - width) / 2.0).round();
    let lines: Vec<Vec<String>> = BODY
        .iter()
        .map(|p| wrap(ui, Face::Regular, BODY_SIZE, p, width))
        .collect();
    let body_h = lines.iter().map(|l| l.len() as f32 * LINE).sum::<f32>()
        + (lines.len().saturating_sub(1)) as f32 * PARAGRAPH_GAP;
    let error_text = error.unwrap_or_default();
    let error_lines = if error_text.is_empty() {
        0.0
    } else {
        wrap(ui, Face::Regular, ui_theme::SMALL, error_text, width).len() as f32 * SMALL_LINE
    };
    let height = TITLE_LINE + 16.0 + body_h + 28.0 + BAR + 18.0 + SMALL_LINE + 28.0 + CONTROL
        + if error_lines > 0.0 { 16.0 + error_lines } else { 0.0 };
    let top = ((ui.h - height) / 2.0).max(24.0).round();
    let bar_y = top + TITLE_LINE + 16.0 + body_h + 28.0;
    let stage_y = bar_y + BAR + 18.0;
    let button_x = ((ui.w - BUTTON_WIDTH) / 2.0).round();
    let button_y = stage_y + SMALL_LINE + 28.0;
    Layout {
        title: Rect { x, y: top, w: width, h: TITLE_LINE },
        lines,
        bar: Rect { x, y: bar_y, w: width, h: BAR },
        stage_y,
        button: Rect { x: button_x, y: button_y, w: BUTTON_WIDTH, h: CONTROL },
        error_y: button_y + CONTROL + 16.0,
    }
}

fn draw_screen(ui: &mut Ui, progress: &Progress, downloading: bool, error: Option<&str>) {
    let layout = layout(ui, progress, error);
    ui.rounded(0.0, 0.0, ui.w, ui.h, 0.0, color::BG);
    ui.text_line(
        Face::Medium,
        TITLE_SIZE,
        TITLE_LINE,
        layout.title.x,
        layout.title.y,
        color::TEXT,
        TITLE,
    );
    let mut y = layout.title.y + TITLE_LINE + 16.0;
    for paragraph in &layout.lines {
        for line in paragraph {
            ui.text_line(
                Face::Regular,
                BODY_SIZE,
                LINE,
                layout.title.x,
                y,
                color::TEXT,
                line,
            );
            y += LINE;
        }
        y += PARAGRAPH_GAP;
    }
    // The bar: done over the total, by bytes when the download counts them, else by files.
    let total = if progress.bytes_total > 0 {
        progress.bytes_total
    } else {
        progress.files_total as u64
    };
    let done = if progress.bytes_total > 0 {
        progress.bytes_done
    } else {
        progress.files_done as u64
    };
    let fraction = match total {
        0 => 0.0,
        total => (done.min(total) as f32 / total as f32).clamp(0.0, 1.0),
    };
    let bar = &layout.bar;
    ui.rounded(bar.x, bar.y, bar.w, bar.h, bar.h / 2.0, color::RAISED);
    if fraction > 0.0 {
        let filled = (bar.w * fraction).max(bar.h);
        ui.rounded(bar.x, bar.y, filled, bar.h, bar.h / 2.0, color::ACCENT);
    }
    let stage = stage_line(progress, fraction, downloading);
    ui.text_line(
        Face::Regular,
        ui_theme::SMALL,
        SMALL_LINE,
        bar.x,
        layout.stage_y,
        color::MUTED,
        &stage,
    );
    // One button: it starts the download, or the next try after a failure.
    let button = &layout.button;
    let hovered = button.contains(ui.cursor);
    let label = if progress.running || downloading {
        "Downloading..."
    } else if progress.done {
        "Done"
    } else {
        "Download"
    };
    let (plate, ink) = if progress.running || downloading {
        (color::RAISED, color::MUTED)
    } else if hovered {
        (color::with_alpha(color::ACCENT, 0.85), color::ON_ACCENT)
    } else {
        (color::ACCENT, color::ON_ACCENT)
    };
    ui.rounded(button.x, button.y, button.w, button.h, button.h / 2.0, plate);
    let label_w = ui.measure(Face::Medium, BUTTON_TEXT, label);
    ui.text_line(
        Face::Medium,
        BUTTON_TEXT,
        button.h,
        button.x + ((button.w - label_w) / 2.0).round(),
        button.y,
        ink,
        label,
    );
    if let Some(error) = error
        && !error.is_empty()
    {
        for (i, line) in wrap(ui, Face::Regular, ui_theme::SMALL, error, layout.bar.w)
            .iter()
            .enumerate()
        {
            ui.text_line(
                Face::Regular,
                ui_theme::SMALL,
                SMALL_LINE,
                layout.bar.x,
                layout.error_y + i as f32 * SMALL_LINE,
                color::TEXT,
                line,
            );
        }
    }
}

/// What the line under the bar says: the stage in the player's words, and how far it is.
fn stage_line(progress: &Progress, fraction: f32, downloading: bool) -> String {
    if !progress.error.is_empty() {
        return "Stopped: fix what the line below says, then press Download again".into();
    }
    if progress.done {
        return "Done: starting the game".into();
    }
    if !progress.running && !downloading {
        return "Press Download to fetch the game files from Mojang".into();
    }
    let stage = match progress.stage.as_str() {
        "version" => "asking Mojang for the version",
        "client" => "downloading the game",
        "index" => "reading the asset index",
        "assets" => "downloading sounds and languages",
        "derived" => "unpacking the derived pack",
        "done" => "done",
        other if other.is_empty() => "starting",
        other => other,
    };
    let percent = (fraction * 100.0).round() as u32;
    if progress.bytes_total > 0 && progress.bytes_done > 0 {
        format!(
            "{stage}: {percent}%, {} of {} MB",
            progress.bytes_done / (1 << 20),
            progress.bytes_total / (1 << 20)
        )
    } else if progress.files_total > 0 {
        format!(
            "{stage}: {percent}%, {} of {} files",
            progress.files_done, progress.files_total
        )
    } else {
        format!("{stage}: {percent}%")
    }
}

/// `text` broken at spaces into lines no wider than `width` dp (a longer word, such as a path,
/// gets a line of its own).
fn wrap(ui: &Ui, face: Face, size: f32, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if ui.measure(face, size, &candidate) <= width || line.is_empty() {
            line = candidate;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_owned();
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
