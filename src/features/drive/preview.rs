//! What a drive document is, and the surface it gets. The type is read from the
//! bytes, never from the name alone: a PNG called `report.pdf` is a picture, and
//! a text file with no extension is still text. The preview lives inside the
//! entry modal; there is no second viewer on top of it.

use crate::features::drive::player::Audio;
use crate::features::drive::ui::board::Board;
use crate::shared::i18n::t;
use crate::shared::theme;
use crate::vault::model::File;
use egui::{Align, Color32, Layout, RichText, Ui};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The most text a preview lays out. egui re-lays a label every frame, and a
/// multi-megabyte log is not a preview; the tail is what the download is for.
const TEXT: usize = 128 * 1024;
/// The largest sheet a PDF page is rasterized into. A texture lives in memory,
/// so a generous zoom stops at this many pixels on the long side.
const PIXELS: f32 = 4096.0;
/// A re-render waits for the zoom to settle: dragging the wheel is a gesture,
/// not a request for one page per notch.
const SHARPEN: Duration = Duration::from_millis(300);
/// How far the zoom may drift from the texture already on screen before the
/// page is worth rasterizing again.
const STALE: f32 = 1.3;
const MAX_SCALE: f32 = 8.0;
/// Below this the board's controls and its pagination stop sharing a line and
/// the pages drop to a row of their own.
const WIDE: f32 = 340.0;
/// The page field, wide enough for four digits.
const PAGE: f32 = 34.0;
/// How far a skip button moves the playhead.
const SKIP: f32 = 15.0;
/// The seek bar's hit height: enough for a finger, slim enough to read as a rail.
const SEEK: f32 = 24.0;
/// The height the transport row takes: the play control's own hit target, its
/// 26 px glyph plus the 14 px `control` keeps around it. Taking the room it was
/// offered instead made the player as tall as the modal that holds it.
const TRANSPORT: f32 = 26.0 + 14.0;
/// A drag seeks once the pointer has been still this long. Seeking decodes from a
/// keyframe, so firing one per frame stalls the UI while the pointer moves.
const DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Text,
    Code,
    Pdf,
    Audio,
    Video,
    Archive,
    Other,
}

/// Everything a preview keeps between frames: the open player, the last rendered
/// PDF page, and the image's board. All of it is dropped or reset the moment the
/// selected entry changes.
pub struct Preview {
    audio: Option<Audio>,
    pdf: Option<Pdf>,
    image: Board,
    /// How far the pointer has dragged the seek bar, 0..=1, until it commits.
    scrub: Option<f32>,
    /// When the pending scrub is worth seeking to: the drag has gone quiet.
    due: Option<Instant>,
    /// The session's volume, 0..=1, and whether it is silenced. Held here so it
    /// survives moving to another track.
    level: f32,
    muted: bool,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            audio: None,
            pdf: None,
            image: Board::default(),
            scrub: None,
            due: None,
            level: 1.0,
            muted: false,
        }
    }
}

impl Preview {
    pub fn sync(&mut self, entry: Option<&str>) {
        if self
            .audio
            .as_ref()
            .is_some_and(|a| Some(a.entry.as_str()) != entry)
        {
            self.audio = None;
            self.scrub = None;
            self.due = None;
        }
        if self
            .pdf
            .as_ref()
            .is_some_and(|p| Some(p.entry.as_str()) != entry)
        {
            self.pdf = None;
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut Preview, file: &File, room: f32, icons: &theme::Icons) {
    match kind(ui.ctx(), file) {
        Kind::Image => image(ui, state, file, room),
        Kind::Text | Kind::Code => text(ui, file, room),
        Kind::Pdf => pdf(ui, state, file, room),
        Kind::Audio => audio(ui, state, file, icons),
        Kind::Video => {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| theme::hint(ui, t("no video preview — save it to play")));
        }
        Kind::Archive | Kind::Other => {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| theme::hint(ui, t("no preview for this file")));
        }
    }
}

pub fn kind(ctx: &egui::Context, file: &File) -> Kind {
    // Classifying a file may scan its bytes, so it is worth exactly one look per
    // file version: the drive grid asks for every card on every frame.
    let id = egui::Id::new(("file-kind", file.id.as_str(), file.bytes()));
    if let Some(kind) = ctx.data(|d| d.get_temp::<Kind>(id)) {
        return kind;
    }
    let kind = classify(file);
    ctx.data_mut(|d| d.insert_temp(id, kind));
    kind
}

fn classify(file: &File) -> Kind {
    if is_image(&file.data) {
        return Kind::Image;
    }
    if file.data.starts_with(b"%PDF-") {
        return Kind::Pdf;
    }
    let ext = extension(&file.name);
    if is_audio(&ext) {
        return Kind::Audio;
    }
    if is_video(&ext) {
        return Kind::Video;
    }
    if is_archive(&ext) {
        return Kind::Archive;
    }
    if is_code(&ext) {
        return Kind::Code;
    }
    if as_text(&file.data).is_some() {
        return Kind::Text;
    }
    Kind::Other
}

/// The glyph a document wears on its card and at the head of its editor. PDF and
/// plain text share the document glyph; the image, code, audio, video and archive
/// kinds each get their own.
pub fn icon(kind: Kind) -> theme::Icon {
    match kind {
        Kind::Image => theme::Icon::FileImage,
        Kind::Text | Kind::Pdf => theme::Icon::FileText,
        Kind::Code => theme::Icon::FileCode,
        Kind::Audio => theme::Icon::FileAudio,
        Kind::Video => theme::Icon::FileVideo,
        Kind::Archive => theme::Icon::FileArchive,
        Kind::Other => theme::Icon::File,
    }
}

/// Bytes of an image in a format the `image` dependency is built with. Read from
/// the magic bytes, so an extensionless screenshot still previews.
fn is_image(data: &[u8]) -> bool {
    matches!(
        image::guess_format(data),
        Ok(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
    )
}

fn is_audio(ext: &str) -> bool {
    // The codecs `rodio` is built with (`Cargo.toml`): AAC/MP4 is not one.
    matches!(ext, "mp3" | "wav" | "flac" | "ogg" | "oga")
}

fn is_video(ext: &str) -> bool {
    matches!(
        ext,
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "wmv" | "flv" | "mpg" | "mpeg"
    )
}

fn is_archive(ext: &str) -> bool {
    matches!(
        ext,
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "lz4" | "cab"
    )
}

/// Source and markup, so the card says `</>` instead of a plain page. Plain prose
/// (txt, md, log) keeps the document glyph.
fn is_code(ext: &str) -> bool {
    matches!(
        ext,
        "rs" | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "ts"
            | "tsx"
            | "py"
            | "rb"
            | "go"
            | "java"
            | "kt"
            | "kts"
            | "swift"
            | "c"
            | "h"
            | "cpp"
            | "cc"
            | "cxx"
            | "hpp"
            | "cs"
            | "php"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "ps1"
            | "bat"
            | "sql"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "sass"
            | "less"
            | "xml"
            | "svg"
            | "yaml"
            | "yml"
            | "toml"
            | "ini"
            | "cfg"
            | "conf"
            | "json"
            | "jsonc"
            | "lua"
            | "pl"
            | "r"
            | "dart"
            | "scala"
            | "clj"
            | "ex"
            | "exs"
            | "erl"
            | "hs"
            | "ml"
            | "fs"
            | "vue"
            | "svelte"
            | "astro"
            | "zig"
            | "nim"
            | "asm"
            | "tex"
            | "proto"
            | "graphql"
            | "gql"
            | "dockerfile"
            | "makefile"
            | "cmake"
    )
}

fn extension(name: &str) -> String {
    name.rsplit('.').next().unwrap_or_default().to_lowercase()
}

/// The first [`TEXT`] bytes as UTF-8, and whether the document goes on. A cut
/// through a multibyte character is not a binary file: the valid prefix is kept.
fn as_text(data: &[u8]) -> Option<(&str, bool)> {
    let head = &data[..data.len().min(TEXT)];
    let text = match std::str::from_utf8(head) {
        Ok(text) => text,
        Err(e) if e.error_len().is_none() => std::str::from_utf8(&head[..e.valid_up_to()]).ok()?,
        Err(_) => return None,
    };
    if text.contains('\0') {
        return None;
    }
    Some((text, data.len() > text.len()))
}

/// A picture on its own board: wheel and pinch zoom at the gesture, a drag pans,
/// and a double click goes back to fit or on to 1:1.
fn image(ui: &mut Ui, state: &mut Preview, file: &File, room: f32) {
    let Some(texture) = texture(ui.ctx(), file) else {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| theme::hint(ui, t("no preview for this file")));
        return;
    };
    let natural = texture.size_vec2();
    if natural.x <= 0.0 || natural.y <= 0.0 {
        return;
    }

    let before = ui.cursor().top();
    ui.horizontal(|ui| zoom_row(ui, &mut state.image));
    let used = ui.cursor().top() - before;

    let key = egui::Id::new(("image", file.id.as_str()));
    state
        .image
        .show(ui, key, &texture, natural, (room - used).max(1.0));
}

/// Text is shown, not edited: a document is read-only, and the download is the
/// way out of the box.
fn text(ui: &mut Ui, file: &File, room: f32) {
    let Some((body, truncated)) = as_text(&file.data) else {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| theme::hint(ui, t("no preview for this file")));
        return;
    };
    let hint = if truncated { 20.0 } else { 0.0 };
    let height = (room - hint).max(32.0);
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        Layout::top_down(Align::Min),
        |ui| {
            let rect = ui.max_rect();
            let corner = ui.visuals().widgets.inactive.corner_radius;
            ui.painter()
                .rect_filled(rect, corner, ui.visuals().text_edit_bg_color());
            theme::band(
                ui,
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .auto_shrink([false, false]),
                |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(body).monospace())
                            .selectable(true)
                            .wrap(),
                    );
                },
            );
        },
    );
    if truncated {
        theme::hint(ui, t("preview truncated"));
    }
}

fn pdf(ui: &mut Ui, state: &mut Preview, file: &File, room: f32) {
    if state.pdf.as_ref().is_none_or(|p| p.entry != file.id) {
        state.pdf = Some(Pdf::new(file.id.clone()));
    }
    let Some(pdf) = state.pdf.as_mut() else {
        return;
    };
    pdf.poll(ui.ctx());
    if pdf.texture.is_none() && pdf.failed.is_none() && pdf.job.is_none() {
        pdf.start(
            file,
            room,
            ui.available_width(),
            ui.ctx().pixels_per_point(),
        );
    }

    let before = ui.cursor().top();
    let mut nav = Nav::None;
    if pdf.pages > 0 {
        // Controls to the left, pagination to the right, one line while there is
        // room for both; below the breakpoint the pages get their own row so the
        // box is never widened by its own toolbar.
        if pdf.pages > 1 && ui.available_width() >= WIDE {
            let (page, pages) = (pdf.page, pdf.pages);
            let board = &mut pdf.board;
            let input = &mut pdf.page_input;
            egui::Sides::new().show(
                ui,
                |ui| zoom_row(ui, board),
                |ui| nav = page_row(ui, page, pages, input),
            );
        } else {
            ui.horizontal(|ui| zoom_row(ui, &mut pdf.board));
            if pdf.pages > 1 {
                let (page, pages) = (pdf.page, pdf.pages);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    nav = page_row(ui, page, pages, &mut pdf.page_input);
                });
            }
        }
    }
    let used = ui.cursor().top() - before;
    match nav {
        Nav::Step(delta) => pdf.go(delta),
        Nav::Commit => pdf.commit(),
        Nav::None => {}
    }

    let room = (room - used).max(1.0);
    if let Some(texture) = &pdf.texture {
        let natural = pdf.natural;
        let key = egui::Id::new(("pdf", pdf.entry.as_str(), pdf.page));
        let scale = pdf.board.show(ui, key, texture, natural, room);
        pdf.sharpen(file, scale, ui.ctx());
    } else if let Some(e) = &pdf.failed {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| theme::hint(ui, e));
    } else {
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(15.0));
            theme::hint(ui, t("rendering page…"));
        });
    }
}

/// The zoom strip above a board: out, the live size, in, and back to fit.
fn zoom_row(ui: &mut Ui, board: &mut Board) {
    ui.spacing_mut().item_spacing.x = 6.0;
    if ui
        .add(egui::Button::new("−").small())
        .on_hover_text(t("zoom out"))
        .clicked()
    {
        board.zoom_out();
    }
    ui.allocate_ui_with_layout(
        egui::vec2(48.0, 20.0),
        Layout::centered_and_justified(egui::Direction::LeftToRight),
        |ui| {
            ui.label(
                RichText::new(board.label())
                    .small()
                    .monospace()
                    .color(theme::palette().muted),
            );
        },
    );
    if ui
        .add(egui::Button::new("+").small())
        .on_hover_text(t("zoom in"))
        .clicked()
    {
        board.zoom_in();
    }
    if ui.add(egui::Button::new(t("fit")).small()).clicked() {
        board.fit();
    }
}

/// Page steppers around a field that holds the number, drawn right to left so
/// the whole thing reads `− [n] / total +` at the end of its row.
fn page_row(ui: &mut Ui, page: usize, pages: usize, input: &mut String) -> Nav {
    let mut nav = Nav::None;
    ui.spacing_mut().item_spacing.x = 6.0;
    if ui
        .add_enabled(page + 1 < pages, egui::Button::new("+").small())
        .on_hover_text(t("next"))
        .clicked()
    {
        nav = Nav::Step(1);
    }
    ui.label(
        RichText::new(format!("/ {pages}"))
            .small()
            .color(theme::palette().muted),
    );
    let field = ui.add(
        egui::TextEdit::singleline(input)
            .desired_width(PAGE)
            .horizontal_align(Align::Center)
            .char_limit(4)
            .margin(egui::Margin::symmetric(6, 4)),
    );
    if field.gained_focus() {
        theme::select_all(ui, &field, input.chars().count());
    }
    if field.lost_focus() && matches!(nav, Nav::None) {
        nav = Nav::Commit;
    }
    if ui
        .add_enabled(page > 0, egui::Button::new("−").small())
        .on_hover_text(t("previous"))
        .clicked()
    {
        nav = Nav::Step(-1);
    }
    nav
}

/// What the pagination asked for this frame.
enum Nav {
    None,
    Step(i32),
    Commit,
}

/// One rendered PDF page at a time. Rendering is CPU work on a document the user
/// just opened, so it runs on its own thread and lands as a texture: the modal
/// only ever draws the page it already has.
struct Pdf {
    entry: String,
    page: usize,
    pages: usize,
    /// What the page field holds while the user is editing it.
    page_input: String,
    texture: Option<egui::TextureHandle>,
    natural: egui::Vec2,
    /// Texture pixels per page point in the texture on screen.
    rendered: f32,
    /// The size the board is asking for, and when it last changed.
    wanted: f32,
    dirty: Option<Instant>,
    failed: Option<String>,
    board: Board,
    job: Option<Job>,
}

struct Job {
    page: usize,
    rx: mpsc::Receiver<Result<Rendered, String>>,
}

struct Rendered {
    pages: usize,
    natural: egui::Vec2,
    scale: f32,
    image: egui::ColorImage,
}

#[derive(Clone, Copy)]
enum Target {
    /// The first look at a page: fit it into this pixel box.
    Fit((u32, u32)),
    /// A sharper or cheaper copy, at this many texture pixels per point.
    Scale(f32),
}

impl Pdf {
    fn new(entry: String) -> Self {
        Self {
            entry,
            page: 0,
            pages: 0,
            page_input: "1".to_owned(),
            texture: None,
            natural: egui::Vec2::ZERO,
            rendered: 0.0,
            wanted: 0.0,
            dirty: None,
            failed: None,
            board: Board::default(),
            job: None,
        }
    }

    fn start(&mut self, file: &File, room: f32, width: f32, density: f32) {
        let max = (
            (width * density).clamp(64.0, PIXELS) as u32,
            (room * density).clamp(64.0, PIXELS) as u32,
        );
        self.job = Some(spawn(file, self.page, Target::Fit(max)));
    }

    fn start_at(&mut self, file: &File, scale: f32) {
        self.job = Some(spawn(file, self.page, Target::Scale(scale)));
    }

    /// Once the gesture settles, a page zoomed past its texture is rasterized
    /// again at the size the board is showing. The old texture stays up while it
    /// happens, so the page never blinks.
    fn sharpen(&mut self, file: &File, scale: f32, ctx: &egui::Context) {
        let wanted = scale * ctx.pixels_per_point();
        if wanted <= 0.0 || self.natural.x <= 0.0 {
            return;
        }
        let ratio = wanted / self.rendered.max(f32::EPSILON);
        if (1.0 / STALE..=STALE).contains(&ratio) {
            self.dirty = None;
            return;
        }
        if (wanted - self.wanted).abs() > 0.01 {
            self.wanted = wanted;
            self.dirty = Some(Instant::now());
        }
        let Some(at) = self.dirty else {
            return;
        };
        let left = SHARPEN.saturating_sub(at.elapsed());
        if !left.is_zero() {
            ctx.request_repaint_after(left);
            return;
        }
        if self.job.is_none() {
            self.start_at(file, self.wanted);
            self.dirty = None;
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(job) = self.job.take() else {
            return;
        };
        match job.rx.try_recv() {
            Ok(Ok(rendered)) => {
                self.pages = rendered.pages;
                self.natural = rendered.natural;
                self.rendered = rendered.scale;
                self.texture = Some(ctx.load_texture(
                    format!("pdf-{}-{}", self.entry, job.page),
                    rendered.image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            Ok(Err(e)) => self.failed = Some(e),
            Err(mpsc::TryRecvError::Empty) => {
                self.job = Some(job);
                ctx.request_repaint_after(Duration::from_millis(80));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.failed = Some(t("could not preview this file").to_owned());
            }
        }
    }

    fn go(&mut self, delta: i32) {
        let next = if delta < 0 {
            self.page.saturating_sub(1)
        } else {
            (self.page + 1).min(self.pages.saturating_sub(1))
        };
        self.go_to(next);
    }

    /// What the user typed in the page field: a number in `1..=pages`, clamped,
    /// and the field put back in step with whatever page it landed on.
    fn commit(&mut self) {
        let wanted = self
            .page_input
            .trim()
            .parse::<usize>()
            .unwrap_or(self.page + 1);
        let page = wanted.clamp(1, self.pages.max(1)) - 1;
        self.go_to(page);
        self.page_input = (self.page + 1).to_string();
    }

    fn go_to(&mut self, page: usize) {
        if page == self.page || page >= self.pages {
            return;
        }
        self.page = page;
        self.page_input = (page + 1).to_string();
        self.texture = None;
        self.natural = egui::Vec2::ZERO;
        self.rendered = 0.0;
        self.wanted = 0.0;
        self.dirty = None;
        self.failed = None;
        self.board.fit();
        self.job = None;
    }
}

fn spawn(file: &File, page: usize, target: Target) -> Job {
    let bytes = file.data.to_vec();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(render(&bytes, page, target));
    });
    Job { page, rx }
}

/// Rasterize one page. `WHITE` keeps the pixmap opaque, so its premultiplied
/// buffer is also a plain RGBA one and egui can take it as is.
fn render(bytes: &[u8], page: usize, target: Target) -> Result<Rendered, String> {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec())
        .map_err(|_| t("could not preview this file").to_owned())?;
    let pages = pdf.pages().len();
    if pages == 0 {
        return Err(t("could not preview this file").to_owned());
    }
    let page = &pdf.pages()[page.min(pages - 1)];
    let (width, height) = page.render_dimensions();
    let natural = egui::vec2(width, height);
    let mut scale = match target {
        Target::Fit((max_w, max_h)) => (max_w as f32 / width).min(max_h as f32 / height),
        Target::Scale(scale) => scale,
    };
    scale = scale.min(PIXELS / width.max(height)).clamp(0.05, MAX_SCALE);
    let pixmap = hayro::render(
        page,
        &hayro::RenderCache::new(),
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &hayro::RenderSettings::default(),
        &hayro::PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    let size = [pixmap.width() as usize, pixmap.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, pixmap.data_as_u8_slice());
    Ok(Rendered {
        pages,
        natural,
        scale,
        image,
    })
}

/// The decoded texture for a document, cached by the entry it belongs to:
/// decoding on every repaint would be a decode per frame.
fn texture(ctx: &egui::Context, file: &File) -> Option<egui::TextureHandle> {
    let id = egui::Id::new(("file", file.id.as_str(), file.name.as_str(), file.bytes()));
    theme::texture(ctx, id, &file.data)
}

fn audio(ui: &mut Ui, state: &mut Preview, file: &File, icons: &theme::Icons) {
    if state.audio.as_ref().is_none_or(|a| a.entry != file.id) {
        state.scrub = None;
        state.due = None;
        state.audio = Some(Audio::open(
            file.id.clone(),
            &file.data,
            state.level,
            state.muted,
        ));
    }
    let Preview {
        audio,
        scrub,
        due,
        level,
        muted,
        ..
    } = state;
    let Some(audio) = audio.as_mut() else {
        return;
    };
    if let Some(e) = &audio.failed {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| theme::hint(ui, e));
        return;
    }

    let total = audio.total;
    let played = audio.position();
    let playing = audio.playing();
    let ended = audio.ended();
    let (mut toggle, mut skip, mut seek) = (false, 0.0, None);

    egui::Frame::new()
        .fill(theme::palette().paper)
        .stroke(theme::stroke(theme::palette().hair, 1.0))
        .corner_radius(theme::radius(10))
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if let Some(total) = total {
                let seconds = total.as_secs_f32().max(f32::EPSILON);
                let base = scrub.unwrap_or((played.as_secs_f32() / seconds).clamp(0.0, 1.0));
                let (resp, value) = track(
                    ui,
                    base,
                    ui.available_width(),
                    SEEK,
                    (SKIP / seconds).clamp(0.001, 1.0),
                    theme::ACCENT,
                    "seek",
                );
                let at = Duration::from_secs_f32(value * seconds);
                let drag = resp.is_pointer_button_down_on();
                if resp.changed() {
                    if drag {
                        *scrub = Some(value);
                        *due = Some(Instant::now() + DEBOUNCE);
                    } else {
                        seek = Some(at);
                    }
                }
                if resp.drag_stopped() || resp.clicked() {
                    seek = Some(at);
                    *scrub = None;
                    *due = None;
                } else if drag && due.is_some_and(|due| Instant::now() >= due) {
                    seek = Some(at);
                    *due = None;
                }
            } else {
                ui.add(
                    egui::ProgressBar::new(0.0)
                        .animate(true)
                        .desired_width(ui.available_width())
                        .fill(theme::ACCENT),
                );
            }
            if let Some(when) = *due {
                ui.ctx()
                    .request_repaint_after(when.saturating_duration_since(Instant::now()));
            }

            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(clock(played))
                        .small()
                        .monospace()
                        .color(theme::palette().text),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(total.map_or_else(|| "--:--".to_owned(), clock))
                            .small()
                            .monospace()
                            .color(theme::palette().muted),
                    );
                });
            });

            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                let track_width = (ui.available_width() - 200.0).clamp(48.0, 120.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(192.0 + track_width, TRANSPORT),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        if control(
                            ui,
                            icons,
                            theme::Icon::Rewind,
                            18.0,
                            theme::palette().muted,
                            "back 15 seconds",
                        )
                        .clicked()
                        {
                            skip = -SKIP;
                        }
                        let (icon, tip) = if playing {
                            (theme::Icon::Pause, "pause")
                        } else if ended {
                            (theme::Icon::Play, "play again")
                        } else {
                            (theme::Icon::Play, "play")
                        };
                        if control(ui, icons, icon, 26.0, theme::ACCENT, tip).clicked() {
                            toggle = true;
                        }
                        if control(
                            ui,
                            icons,
                            theme::Icon::Forward,
                            18.0,
                            theme::palette().muted,
                            "forward 15 seconds",
                        )
                        .clicked()
                        {
                            skip = SKIP;
                        }
                        ui.add_space(18.0);
                        let silent = *muted || *level <= 0.0;
                        let (icon, tip) = if silent {
                            (theme::Icon::Mute, "unmute")
                        } else {
                            (theme::Icon::Volume, "mute")
                        };
                        if control(ui, icons, icon, 16.0, theme::palette().muted, tip).clicked() {
                            if silent {
                                if *level <= 0.0 {
                                    *level = 1.0;
                                }
                                *muted = false;
                            } else {
                                *muted = true;
                            }
                            audio.set_volume(*level, *muted);
                        }
                        let (resp, value) = track(
                            ui,
                            *level,
                            track_width,
                            20.0,
                            0.05,
                            theme::palette().muted,
                            "volume",
                        );
                        if resp.changed() {
                            *level = value;
                            *muted = value <= 0.0;
                            audio.set_volume(*level, *muted);
                        }
                    },
                );
            });
        });

    if toggle {
        if ended {
            // The queue is drained: queueing it again is what starts it over, and
            // it keeps the same device instead of opening a second one.
            audio.seek(&file.data, Duration::ZERO);
        } else {
            audio.toggle();
        }
    }
    if let Some(to) = seek {
        audio.seek(&file.data, to);
    }
    if skip != 0.0 {
        audio.skip(&file.data, skip);
    }
    if playing {
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
}

/// A hand-painted track: the travelled part filled, a rail for the rest, and the
/// round knob the pointer drags. A click anywhere jumps there, the arrows nudge by
/// `step` while it holds the keyboard, and its ring says so. Returns the value
/// after the pointer had its say, 0..=1.
fn track(
    ui: &mut Ui,
    value: f32,
    width: f32,
    height: f32,
    step: f32,
    fill: Color32,
    tip: &str,
) -> (egui::Response, f32) {
    let (rect, mut resp) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
    let radius = (height / 2.0).min(6.0);
    let (left, right) = (rect.left() + radius, rect.right() - radius);
    let span = (right - left).max(1.0);
    let center_y = rect.center().y;
    let asked = value.clamp(0.0, 1.0);
    let mut value = asked;
    if let Some(pos) = resp.interact_pointer_pos() {
        value = ((pos.x - left) / span).clamp(0.0, 1.0);
    }
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                resp.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            )
        });
        let keys = ui.input(|i| {
            i.num_presses(egui::Key::ArrowRight) as i32 - i.num_presses(egui::Key::ArrowLeft) as i32
        });
        value = (value + keys as f32 * step).clamp(0.0, 1.0);
    }
    if value != asked {
        resp.mark_changed();
    }
    let center = egui::pos2(left + value * span, center_y);
    let rail = egui::Rect::from_min_max(
        egui::pos2(left, center_y - 2.0),
        egui::pos2(right, center_y + 2.0),
    );
    ui.painter()
        .rect_filled(rail, theme::radius(2), theme::palette().rail);
    if value > 0.0 {
        ui.painter().rect_filled(
            egui::Rect::from_min_max(rail.min, egui::pos2(center.x, rail.max.y)),
            theme::radius(2),
            fill,
        );
    }
    ui.painter()
        .circle_filled(center, radius, theme::palette().text);
    if resp.has_focus() {
        resp.scroll_to_me(Some(egui::Align::Center));
        ui.painter()
            .circle_stroke(center, radius + 3.0, theme::stroke(theme::ACCENT, 2.0));
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else {
        resp = resp.on_hover_cursor(egui::CursorIcon::Grab);
    }
    let name = crate::shared::i18n::text(tip).into_owned();
    resp = resp.on_hover_text(name.clone());
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(value), &name));
    (resp, value)
}

/// An icon-only control: the glyph and nothing behind it. The pointer lifts it
/// from `tint` to the full text colour, and the keyboard gets the ring.
fn control(
    ui: &mut Ui,
    icons: &theme::Icons,
    icon: theme::Icon,
    size: f32,
    tint: Color32,
    tip: &str,
) -> egui::Response {
    let (rect, mut resp) =
        ui.allocate_exact_size(egui::vec2(size + 14.0, size + 14.0), egui::Sense::click());
    let color = if resp.hovered() || resp.has_focus() {
        theme::palette().text
    } else {
        tint
    };
    if let Some(img) = icons.image(icon, size, color) {
        img.paint_at(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(size, size)),
        );
    }
    if resp.has_focus() {
        resp.scroll_to_me(Some(egui::Align::Center));
        ui.painter().circle_stroke(
            rect.center(),
            size / 2.0 + 3.0,
            theme::stroke(theme::ACCENT, 2.0),
        );
    }
    let name = crate::shared::i18n::text(tip).into_owned();
    resp = resp
        .on_hover_text(name.clone())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &name)
    });
    resp
}

fn clock(time: Duration) -> String {
    let secs = time.as_secs();
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, data: &[u8]) -> File {
        File::new(name.to_owned(), data.to_vec(), 0)
    }

    #[test]
    fn the_type_comes_from_the_bytes_not_the_name() {
        let png = include_bytes!("../../../assets/icon.png");
        assert_eq!(classify(&file("report.pdf", png)), Kind::Image);
        assert_eq!(classify(&file("shot", png)), Kind::Image);
        assert_eq!(classify(&file("letter.pdf", b"%PDF-1.7\n...")), Kind::Pdf);
        assert_eq!(classify(&file("notes.txt", b"hello\nworld")), Kind::Text);
        assert_eq!(classify(&file("no-extension", b"plain text")), Kind::Text);
        assert_eq!(classify(&file("binary", &[0, 1, 2, 3])), Kind::Other);
        assert_eq!(classify(&file("song.mp3", b"ID3")), Kind::Audio);
        assert_eq!(classify(&file("clip.MP4", b"...")), Kind::Video);
        assert_eq!(classify(&file("backup.zip", b"PK...")), Kind::Archive);
        assert_eq!(classify(&file("main.rs", b"fn main() {}")), Kind::Code);
    }

    /// Every type the row can name wears a glyph of its own; plain text and a PDF
    /// share the document one.
    #[test]
    fn a_kind_wears_its_own_glyph() {
        assert_eq!(icon(Kind::Image), theme::Icon::FileImage);
        assert_eq!(icon(Kind::Pdf), theme::Icon::FileText);
        assert_eq!(icon(Kind::Text), theme::Icon::FileText);
        assert_eq!(icon(Kind::Code), theme::Icon::FileCode);
        assert_eq!(icon(Kind::Audio), theme::Icon::FileAudio);
        assert_eq!(icon(Kind::Video), theme::Icon::FileVideo);
        assert_eq!(icon(Kind::Archive), theme::Icon::FileArchive);
        assert_eq!(icon(Kind::Other), theme::Icon::File);
    }

    #[test]
    fn the_clock_grows_an_hour_field() {
        assert_eq!(clock(Duration::from_secs(5)), "00:05");
        assert_eq!(clock(Duration::from_secs(65)), "01:05");
        assert_eq!(clock(Duration::from_secs(3661)), "1:01:01");
    }

    /// A press anywhere on the track takes that position, which is what makes the
    /// bar seek backwards and not only forwards.
    #[test]
    fn a_press_on_the_track_picks_the_position_under_the_pointer() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let seen = std::cell::Cell::new(1.0_f32);
        let draw = |events: Vec<egui::Event>| {
            crate::shared::ui::shot::frame(&ctx, egui::vec2(200.0, 40.0), events, |ui| {
                let (_, value) = track(ui, 0.8, 200.0, 20.0, 0.05, theme::ACCENT, "seek");
                seen.set(value);
            });
        };
        draw(Vec::new());
        assert!((seen.get() - 0.8).abs() < 0.01, "rest shows the value");
        let at = egui::pos2(50.0, 20.0);
        draw(vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        assert!(
            seen.get() < 0.4,
            "a press seeks to the pointer, not forward only: {}",
            seen.get()
        );
    }

    /// The player is its icons and its seek bar: the controls are no longer the
    /// words `play`/`pause`, and the times sit apart from the rail.
    #[test]
    fn the_audio_player_is_icons_and_a_seek_bar() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let icons = theme::Icons::load(&ctx);
        let song = file("song.mp3", b"ID3");
        let mut state = Preview {
            audio: Some(Audio::detached(
                song.id.clone(),
                Some(Duration::from_secs(215)),
            )),
            ..Default::default()
        };
        let mut draw = || {
            crate::shared::ui::shot::frame(&ctx, egui::vec2(600.0, 300.0), Vec::new(), |ui| {
                show(ui, &mut state, &song, 260.0, &icons)
            })
        };
        draw();
        let said = crate::shared::ui::shot::words(&draw());
        assert!(said.iter().any(|line| line == "00:00"), "{said:?}");
        assert!(said.iter().any(|line| line == "03:35"), "{said:?}");
        assert!(
            said.iter().all(|line| line != "play" && line != "pause"),
            "the controls are icons, not words: {said:?}"
        );
    }

    /// The player is its own height, not the box it sits in: the transport row
    /// used to take every point the modal offered, so an audio entry opened a
    /// modal as tall as the window.
    #[test]
    fn the_audio_player_is_only_as_tall_as_its_transport() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let icons = theme::Icons::load(&ctx);
        let song = file("song.mp3", b"ID3");
        let mut state = Preview {
            audio: Some(Audio::detached(
                song.id.clone(),
                Some(Duration::from_secs(215)),
            )),
            ..Default::default()
        };
        let shapes =
            crate::shared::ui::shot::frame(&ctx, egui::vec2(600.0, 600.0), Vec::new(), |ui| {
                show(ui, &mut state, &song, 520.0, &icons)
            });
        let player = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == theme::palette().paper => Some(r.rect),
                _ => None,
            })
            .max_by(|a, b| a.height().total_cmp(&b.height()))
            .expect("the player paints its own card");
        assert!(
            player.height() < 200.0,
            "the player grew to the box it was handed: {player:?}"
        );
    }

    /// The breakpoint has to hold both toolbars and the gap between them, or the
    /// row would widen the modal instead of moving the pages down.
    #[test]
    fn the_wide_breakpoint_fits_both_toolbars() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut board = Board::default();
        let mut input = "7".to_owned();
        let (mut zoom, mut pages) = (0.0_f32, 0.0_f32);
        let shapes =
            crate::shared::ui::shot::frame(&ctx, egui::vec2(500.0, 200.0), Vec::new(), |ui| {
                ui.horizontal(|ui| {
                    zoom_row(ui, &mut board);
                    zoom = ui.min_rect().width();
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    page_row(ui, 6, 12, &mut input);
                    pages = ui.min_rect().width();
                });
            });
        assert!(
            zoom + pages + 12.0 <= WIDE,
            "the breakpoint is too tight: {zoom} + {pages} > {WIDE}"
        );

        let at = |want: &str| {
            shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(text) if text.galley.text() == want => Some(text.pos.x),
                    _ => None,
                })
                .next_back()
                .unwrap_or_else(|| panic!("no {want:?} was painted"))
        };
        let (minus, slash, plus) = (at("−"), at("/ 12"), at("+"));
        assert!(
            minus < slash && slash < plus,
            "the pagination is out of order: {minus} {slash} {plus}"
        );
    }

    /// The modal repaints ~60x/s, so the texture must be reused across frames.
    #[test]
    fn an_image_decodes_once_and_is_cached() {
        let ctx = egui::Context::default();
        let png = include_bytes!("../../../assets/icon.png");
        let shot = file("shot.png", png);
        let first = texture(&ctx, &shot).unwrap();
        let again = texture(&ctx, &shot).unwrap();
        assert_eq!(first.id(), again.id(), "the same entry reuses its texture");
        let other = file("other.png", png);
        assert_ne!(
            first.id(),
            texture(&ctx, &other).unwrap().id(),
            "one texture per entry"
        );
    }

    #[test]
    fn text_is_cut_but_not_dropped() {
        let long = vec![b'a'; TEXT + 10];
        let (body, truncated) = as_text(&long).expect("ascii is text");
        assert_eq!(body.len(), TEXT);
        assert!(truncated);
        assert!(!as_text(b"short").unwrap().1);
    }

    #[test]
    fn a_cut_through_a_multibyte_character_is_not_binary() {
        let mut data = vec![b'a'; TEXT - 1];
        data.extend_from_slice("ñ".as_bytes());
        let (body, truncated) = as_text(&data).expect("the valid prefix is text");
        assert_eq!(body.len(), TEXT - 1);
        assert!(truncated);
    }

    /// The rasterizer is the one piece the type tests cannot reach: a real page
    /// comes back as a texture-sized image that fits the box it was given.
    #[test]
    fn a_pdf_page_renders_to_pixels() {
        let pdf = include_bytes!("../../../tests/preview.pdf");
        let out = render(pdf, 0, Target::Fit((200, 100))).expect("the fixture renders");
        assert_eq!(out.pages, 1);
        assert_eq!(out.natural, egui::vec2(200.0, 100.0));
        assert!((out.scale - 1.0).abs() < 0.01, "{}", out.scale);
        assert_eq!(out.image.size, [200, 100]);
    }

    #[test]
    fn a_broken_pdf_reports_instead_of_panicking() {
        assert!(render(b"%PDF-1.7\nbroken", 0, Target::Fit((100, 100))).is_err());
    }
}
