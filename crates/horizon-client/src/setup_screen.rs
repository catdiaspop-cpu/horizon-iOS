//! The screen of a phone build started without the game files: it downloads them from Mojang
//! here, in the app, and hands over to the loading screen. Drawn in Horizon's design language
//! with the fonts bundled in the executable: none of Minecraft's files are there yet.
//!
//! The engine is `horizon_client::setup` (Android's `SetupActivity` drives the same one through
//! JNI); this module only turns its numbers into a screen, which is why it lives in the client
//! library next to the other screens and takes a plain [`Progress`] rather than the engine's
//! type.
use crate::ui_theme::{self, Face, Ui, color};

/// What the download engine reports, as the screen reads it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Progress<'a> {
    /// `version`, `client`, `index`, `extract`, `assets`, `derived`, `done`.
    pub stage: &'a str,
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// A download runs right now.
    pub running: bool,
    /// It finished, and the files are ready to play.
    pub done: bool,
    /// What stopped it, empty when nothing did.
    pub error: &'a str,
}

/// What a click on the screen does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start (or retry) the download.
    Download,
}

pub struct Screen;

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen {
    pub fn new() -> Self {
        Self
    }

    pub fn draw(&self, ui: &mut Ui, progress: &Progress<'_>) {
        let layout = layout(ui, progress);
        ui.rounded(0.0, 0.0, ui.w, ui.h, 0.0, color::BG);
        ui.text_line(
            Face::Medium,
            TITLE_SIZE,
            TITLE_LINE,
            layout.x,
            layout.title_y,
            color::TEXT,
            TITLE,
        );
        let mut y = layout.title_y + TITLE_LINE + 16.0;
        for paragraph in &layout.lines {
            for line in paragraph {
                ui.text_line(Face::Regular, BODY_SIZE, LINE, layout.x, y, color::TEXT, line);
                y += LINE;
            }
            y += PARAGRAPH_GAP;
        }
        // The bar: done over the total, by bytes when the download counts them, else by files.
        let (done, total) = if progress.bytes_total > 0 {
            (progress.bytes_done, progress.bytes_total)
        } else {
            (progress.files_done as u64, progress.files_total as u64)
        };
        let fraction = match total {
            0 => 0.0,
            total => (done.min(total) as f32 / total as f32).clamp(0.0, 1.0),
        };
        ui.rounded(layout.x, layout.bar_y, layout.width, BAR, BAR / 2.0, color::RAISED);
        if fraction > 0.0 {
            let filled = (layout.width * fraction).max(BAR);
            ui.rounded(layout.x, layout.bar_y, filled, BAR, BAR / 2.0, color::ACCENT);
        }
        ui.text_line(
            Face::Regular,
            ui_theme::SMALL,
            SMALL_LINE,
            layout.x,
            layout.stage_y,
            color::MUTED,
            &stage_line(progress, fraction),
        );
        // One button: it starts the download, or the next try after a failure.
        let button = &layout.button;
        let busy = progress.running;
        let label = if busy { "Downloading..." } else { "Download" };
        let (plate, ink) = if busy {
            (color::RAISED, color::MUTED)
        } else if button.contains(ui.cursor) {
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
        if !progress.error.is_empty() {
            for (i, line) in wrap(ui, Face::Regular, ui_theme::SMALL, progress.error, layout.width)
                .iter()
                .enumerate()
            {
                ui.text_line(
                    Face::Regular,
                    ui_theme::SMALL,
                    SMALL_LINE,
                    layout.x,
                    layout.error_y + i as f32 * SMALL_LINE,
                    color::TEXT,
                    line,
                );
            }
        }
    }

    /// The button under the cursor (`ui.cursor`, in dp), if the download may start again.
    pub fn click(&self, ui: &Ui, progress: &Progress<'_>) -> Option<Action> {
        if progress.running {
            return None;
        }
        layout(ui, progress)
            .button
            .contains(ui.cursor)
            .then_some(Action::Download)
    }
}

const TITLE: &str = "Download Minecraft's files";
const BODY: [&str; 2] = [
    "Horizon runs on your own copy of Minecraft: Java Edition and takes its files from Mojang. \
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
    x: f32,
    width: f32,
    title_y: f32,
    lines: Vec<Vec<String>>,
    bar_y: f32,
    stage_y: f32,
    button: Rect,
    error_y: f32,
}

/// One layout for drawing and for clicks.
fn layout(ui: &Ui, progress: &Progress<'_>) -> Layout {
    let width = COLUMN.min(ui.w - 64.0).max(240.0);
    let x = ((ui.w - width) / 2.0).round();
    let lines: Vec<Vec<String>> = BODY
        .iter()
        .map(|p| wrap(ui, Face::Regular, BODY_SIZE, p, width))
        .collect();
    let body_h = lines.iter().map(|l| l.len() as f32 * LINE).sum::<f32>()
        + (lines.len().saturating_sub(1)) as f32 * PARAGRAPH_GAP;
    let error_h = if progress.error.is_empty() {
        0.0
    } else {
        wrap(ui, Face::Regular, ui_theme::SMALL, progress.error, width).len() as f32 * SMALL_LINE
            + 16.0
    };
    let height = TITLE_LINE + 16.0 + body_h + 28.0 + BAR + 18.0 + SMALL_LINE + 28.0 + CONTROL
        + error_h;
    let top = ((ui.h - height) / 2.0).max(24.0).round();
    let bar_y = top + TITLE_LINE + 16.0 + body_h + 28.0;
    let stage_y = bar_y + BAR + 18.0;
    let button_y = stage_y + SMALL_LINE + 28.0;
    Layout {
        x,
        width,
        title_y: top,
        lines,
        bar_y,
        stage_y,
        button: Rect {
            x: ((ui.w - BUTTON_WIDTH) / 2.0).round(),
            y: button_y,
            w: BUTTON_WIDTH,
            h: CONTROL,
        },
        error_y: button_y + CONTROL + 16.0,
    }
}

/// What the line under the bar says: the stage in the player's words, and how far it is.
fn stage_line(progress: &Progress<'_>, fraction: f32) -> String {
    if !progress.error.is_empty() {
        return "Stopped: fix what the line below says, then press Download again".into();
    }
    if progress.done {
        return "Done: starting the game".into();
    }
    if !progress.running {
        return "Press Download to fetch the game files from Mojang".into();
    }
    let stage = match progress.stage {
        "version" => "asking Mojang for the version",
        "client" => "downloading the game",
        "index" => "reading the asset index",
        "extract" => "taking the game's models apart",
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
