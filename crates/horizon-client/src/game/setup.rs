//! A phone build started without the game files (`missing_resources`): this state downloads
//! them from Mojang here, in the app, and then hands over to the loading screen. Android's
//! `SetupActivity` does the same job in its own window; iOS gets one `UIApplicationMain` per
//! process, so a second window and a second event loop are out, and the download is a state of
//! the game's own loop instead (`App::Setup` -> `App::Loading`).
//!
//! The drawing is `horizon_client::setup_screen` (a screen in the client library, like the
//! others); this module owns the window, the loop and the engine's lifetime.
use super::*;
use horizon_client::setup;
use horizon_client::setup_screen::{Action, Progress as View, Screen};
use horizon_client::ui_theme::Ui;
use winit::event::TouchPhase;

/// The screen while it waits and downloads, and what the loading screen needs once the files
/// are here.
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
    screen: Screen,
    /// The finger or pointer, in physical pixels.
    pointer: (f64, f64),
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
            screen: Screen::new(),
            pointer: (0.0, 0.0),
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

    /// The engine's stage and counts, with this screen's own error (a window that would not
    /// open) ahead of the engine's.
    fn view<'a>(&'a self, numbers: &'a setup::Progress, own: &'a str) -> View<'a> {
        View {
            stage: &numbers.stage,
            files_done: numbers.files_done,
            files_total: numbers.files_total,
            bytes_done: numbers.bytes_done,
            bytes_total: numbers.bytes_total,
            running: numbers.running,
            done: numbers.done,
            error: own,
        }
    }

    fn draw(&mut self) {
        let numbers = setup::progress();
        let own = self.error.as_ref().map(ToString::to_string).unwrap_or_default();
        let error = if own.is_empty() { &numbers.error } else { &own };
        let view = self.view(&numbers, error);
        let mut batch = HudBatch::new(self.size());
        let cursor = (
            self.pointer.0 as f32 / batch.scale,
            self.pointer.1 as f32 / batch.scale,
        );
        {
            let mut ui = Ui::new(&self.gui.atlas, &mut batch, cursor);
            self.screen.draw(&mut ui, &view);
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
        let numbers = setup::progress();
        if numbers.running {
            return;
        }
        let own = self.error.as_ref().map(ToString::to_string).unwrap_or_default();
        let error = if own.is_empty() { &numbers.error } else { &own };
        let view = self.view(&numbers, error);
        let mut batch = HudBatch::new(self.size());
        let cursor = (
            self.pointer.0 as f32 / batch.scale,
            self.pointer.1 as f32 / batch.scale,
        );
        let ui = Ui::new(&self.gui.atlas, &mut batch, cursor);
        let action = self.screen.click(&ui, &view);
        ui.finish();
        if action != Some(Action::Download) {
            return;
        }
        let Some(home) = home() else {
            self.error = Some("no game folder to download into".into());
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };
        log::info!("Horizon: downloading the game files into the game folder");
        self.error = None;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
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
        if self.error.is_some() || self.ready() {
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
