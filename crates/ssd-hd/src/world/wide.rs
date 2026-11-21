//! The wide view: the frame the PPU drew, extended to the left and right by what the scene has
//! beyond the original's 256 pixels, at 16:9 ([`WIDTH_16_9`]: 372 pixels at 239 lines, 8:7 pixels).
//!
//! The wide frame is composed in chunks of 256 pixels (one chunk at width 256, two otherwise: the
//! left and the right end of the frame, overlapping in the middle). Each chunk is a virtual PPU
//! frame, composed by the same code as the Classic frame ([`Renderer`]), placed at its position in
//! the window (its `origin`), with
//!
//! * BG2 of the field, the map, read through the game's own lookup ([`FieldState`]) wherever the
//!   world has a tile, instead of the 35 columns VRAM holds (the [`Scene::Field`]);
//! * the Mode 7 plane of a flight, drawn at the window's positions past the screen's edges (the
//!   [`Scene::Flight`]);
//! * for a picture (battle, title, cutscene, interiors: [`Scene::Bordered`]) the original 256
//!   pixels and black beside them;
//! * BG1 and BG3 of the field, the corner faces and the windows, taken from the PPU's layers and
//!   kept to the 256 pixels they were made for (where those sit is the [`HudPlacement`]);
//! * the objects, from a copy of the OAM with every entry moved into the chunk's coordinates and,
//!   in the field, the objects the original drops added in the entries its OAM leaves free
//!   ([`FieldState::extra_parts`]); the entries are the same in every chunk, so the HD art finds
//!   the groups of sprites over the whole view, the margins' objects as the window's, and an
//!   object across the window's edge as one group ([`WideRenderer::render_hd`]).
//!
//! The original window (the 256 pixels the PPU shows, the [`WideFrame::logical`] rectangle) sits
//! where the view policy puts it: in the middle, and off-centre near a map edge, where the field's
//! view stops at the edge of the map instead of showing what is beyond it. A map narrower than the
//! view (a room, a cave) is centred and the sides are black. At width 256 the frame is the Classic
//! frame, and the part inside the window differs from it at wider views only under the extra
//! objects.

use super::check::check_tiles;
use super::map::{FieldState, GAME_MODE, Rom, is_field_mode};
use super::objects::{ObjectPart, Part, drop_anchors};
use crate::art::poses::{ExtraSprite, View};
use crate::art::{ArtPixel, ArtState, Conditions, Lookup};
use crate::bgart::Probe;
use crate::enhanced::picture;
use crate::font::{Font, TextCapture};
use crate::pack::Pack;
use crate::translate::{Beneath, Overlay, Target, Translator};
use crate::{Binder, Frame, HdFrame, HdOptions, Memories, Pixel, Renderer, Unsupported, WIDTH};
use rayon::prelude::*;
use ssd_catalogue::engine::OnScreen;
use ssd_oracle::{CgramWrite, Color, FrameLog, LineEntry, LineRegs, OamWrite, VramWrite};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The widest view: two chunks of 256 pixels.
pub const MAX_WIDTH: usize = 2 * WIDTH;

/// The view width of the 16:9 frame at 239 lines, 8:7 pixels: 239 x 16 / 9 x 7 / 8 = 372.
pub const WIDTH_16_9: usize = 372;

/// Where the screen layers (BG1 and BG3: the corner faces, the windows and the text) sit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HudPlacement {
    /// With the original window, so the wide frame holds the Classic picture as it was.
    #[default]
    Window,
    /// In the middle of the frame, whatever the world does near a map edge.
    Centred,
}

/// What to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WideOptions {
    /// Width of the frame in pixels, 256 (the Classic frame) to [`MAX_WIDTH`].
    pub width: usize,
    pub hud: HudPlacement,
}

/// The scene class of a frame, as the wide view draws it (the design choices of the HD route).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    /// The field: the map continues on both sides of the window, read through the lookup; a map
    /// narrower than the view (a single-screen room, a cave) is centred with black beside it.
    Field,
    /// A Mode 7 flight: the plane continues across the view.
    Flight,
    /// A picture of the original's 256 pixels (battle, title, cutscene, town and shop interiors):
    /// black at the sides.
    Bordered,
}

/// A rectangle of the frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    #[must_use]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        (self.x..self.x + self.width).contains(&x) && (self.y..self.y + self.height).contains(&y)
    }
}

/// Why a frame is not a wide field frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotField {
    /// The main loop is not in a field state, or there is no map.
    GameMode,
    /// A layer setup the field does not use (another BG mode, 16 x 16 tiles, no BG2).
    Layers,
    /// OAM was written while the frame was displayed.
    OamWrites,
    /// BG2 holds a picture of the game's own, not the map.
    Picture,
}

/// What the wide renderer refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WideError {
    /// Not a width from 256 to [`MAX_WIDTH`].
    BadWidth,
    /// The frame is not one the plain renderer models either.
    Unsupported(Unsupported),
    NotField(NotField),
}

impl std::fmt::Display for WideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadWidth => write!(f, "a width from {WIDTH} to {MAX_WIDTH}"),
            Self::Unsupported(reason) => reason.fmt(f),
            Self::NotField(reason) => write!(f, "not a wide field frame ({reason:?})"),
        }
    }
}

impl std::error::Error for WideError {}

/// Counts of what the frame needed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WideStats {
    /// Parts of objects the original drops, placed in the frame, and the ones that found no
    /// free OAM entry.
    pub extra_parts: u32,
    pub extra_dropped: u32,
    /// Pixels outside the map (beyond its edge), or beside a picture, left black.
    pub outside_skipped: u64,
    /// Pixels drawn from a world position outside the map: always 0.
    pub outside_drawn: u64,
}

/// A wide frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WideFrame {
    pub frame: Frame,
    /// The scene class the frame was drawn as.
    pub scene: Scene,
    /// Where the original 256-pixel picture sits (the world layers and the objects).
    pub logical: Rect,
    /// Where BG1 and BG3 sit.
    pub hud: Rect,
    /// The world x of the frame's left edge: negative when the map is narrower than the view; 0
    /// outside the field.
    pub view_left: i32,
    pub stats: WideStats,
    /// The boxes the extra objects' parts cover.
    pub extra_boxes: Vec<Rect>,
}

impl WideFrame {
    /// Whether pixel `(x, y)` is under a part of an object the original does not draw.
    #[must_use]
    pub fn under_extra(&self, x: i32, y: i32) -> bool {
        self.extra_boxes.iter().any(|rect| rect.contains(x, y))
    }
}

/// The view policy: where the frame's left edge is in the world, and where the original window
/// sits in the frame.
///
/// The window of the world the original shows starts at `window_left`; the wide view is centred
/// on it, but never leaves the map: near an edge the view stops and the window moves toward it.
/// A map narrower than the view is centred.
#[must_use]
pub fn place_window(map_width: i32, window_left: i32, width: usize) -> (i32, i32) {
    let width = width as i32;
    let margin = width - WIDTH as i32;
    let view_left = if map_width >= width {
        (window_left - margin / 2).clamp(0, map_width - width)
    } else {
        (map_width - width).div_euclid(2)
    };
    (view_left, (window_left - view_left).clamp(0, margin))
}

/// `value` as a signed number modulo `modulus`, in `-modulus / 2..modulus / 2`.
fn signed(value: i32, modulus: i32) -> i32 {
    let rest = value.rem_euclid(modulus);
    if rest >= modulus / 2 { rest - modulus } else { rest }
}

/// The world pixel at the top left of the screen for BG2's scroll registers: the BG scrolls to
/// the camera, so its position is the camera's up to the ring the tile map is (64 x 32 tiles).
pub(crate) fn window_origin(regs: &LineRegs, camera: (u16, u16)) -> (i32, i32) {
    let (x, y) = (i32::from(camera.0), i32::from(camera.1));
    (
        x + signed(i32::from(regs.bg_h_scroll[1]) - x, 512),
        y + signed(i32::from(regs.bg_v_scroll[1]) - y, 256),
    )
}

/// The world pixel BG2 shows at `window_x` of scanline `scanline`: the mosaic reads the pixel at
/// the start of its block, on the line its filter reads.
fn map_sample(
    regs: &LineRegs,
    camera: (u16, u16),
    scanline: u16,
    window_x: i32,
    mosaic: bool,
) -> (i32, i32) {
    let (left, top) = window_origin(regs, camera);
    if mosaic {
        let block = i32::from(regs.mosaic_size) + 1;
        let line = i32::from(scanline) - i32::from(regs.mosaic_v_offset);
        (left + window_x.div_euclid(block) * block, top + line)
    } else {
        (left + window_x, top + i32::from(scanline))
    }
}

/// An OAM (low table as bytes, high table).
#[derive(Clone)]
struct Oam {
    low: Vec<u8>,
    high: Vec<u8>,
}

/// One chunk of the frame: a virtual PPU frame of 256 pixels starting at wide x `x0`.
struct Chunk {
    renderer: Renderer,
    x0: i32,
    oam: Oam,
}

/// The wide renderer: keeps the sprite line buffers of its chunks between frames, as the PPU
/// does, so feed it the frames of a run in order.
pub struct WideRenderer {
    chunks: Vec<Renderer>,
    vram_work: Vec<u16>,
    /// The pack's art, for the Enhanced picture of the wide view ([`WideRenderer::render_hd`]).
    art: Option<Box<ArtState>>,
    /// Every frame is drawn in one piece, whatever its lines allow (see
    /// [`WideRenderer::set_serial`]).
    serial: bool,
    /// The HD text font, and the text cells noted while the last view was composed with it on.
    font: Option<Arc<Font>>,
    text_on: bool,
    text: TextCapture,
    /// Frames in a field state are drawn as the field whatever BG2 holds (see
    /// [`WideRenderer::set_trust_field`]).
    trust_field: bool,
    /// The objects the original drops, as the last frame composed placed them.
    extras: Vec<Extra>,
    /// The frame being drawn is a picture (a window with black beside it) that shows the field's
    /// map on BG2 (a menu, a scripted scene, a town's room): its map layer is found by the world
    /// position like the field's.
    mapped: bool,
    /// The translation drawn over the original's message windows, when the run has one.
    translator: Option<Translator>,
    /// The time the renderer's own passes took in the picture being drawn ([`WideRenderer::compose`]).
    compose_time: std::time::Duration,
    /// What the last Enhanced picture drawn cost ([`WideRenderer::draw_cost`]).
    cost: crate::DrawCost,
    /// The lightning of the Super Saiyan transformation (see [`WideRenderer::set_lightning`]), and the frames since the
    /// transformation started while it goes on (the conditions', [`Conditions::with_ssj_lightning`]).
    lightning: Option<crate::lightning::Overlay>,
    ssj_lightning: Option<u32>,
    hide_field_ui: bool,
}

/// A part of an object the original drops, as a wide frame draws it.
#[derive(Debug, Clone, Copy)]
struct Extra {
    part: Part,
    /// The OAM entry it is drawn with, the same in every chunk of the frame (one the original's
    /// OAM leaves free); `None` when it is not in the frame or no entry was free.
    slot: Option<u8>,
    /// The entry of the original's OAM it follows in the order of the original's loop
    /// ([`drop_anchors`]).
    after: Option<i32>,
}

impl Extra {
    /// The sprite the art finds poses in.
    fn sprite(&self, order: usize) -> Option<ExtraSprite> {
        Some(ExtraSprite {
            idx: self.slot?,
            x: self.part.x,
            y: self.part.y,
            tile: self.part.tile,
            attributes: self.part.attributes,
            large: self.part.large,
            after: self.after,
            order: order as u32,
        })
    }
}

/// The parts the original drops, placed in the free entries of its OAM (`low` and `high` are the
/// frame's tables) for a frame of `width` pixels with the window at `offset`: the same entries for
/// every chunk, so the art that finds the objects in the frame sees one OAM. Returns them (all of
/// them: one outside the frame or with no free entry has no slot) and how many found no entry.
fn place_extras(
    all: &[ObjectPart],
    low: &[u8],
    high: &[u8],
    offset: i32,
    width: usize,
) -> (Vec<Extra>, u32) {
    let anchors = drop_anchors(all, low, high);
    let mut free: Vec<u8> =
        (0..128_u8).filter(|&index| low[usize::from(index) * 4 + 1] >= 0xF0).collect();
    free.reverse();
    let mut dropped = 0;
    let mut extras = Vec::new();
    for (object, after) in all.iter().zip(anchors) {
        if object.in_original {
            continue;
        }
        let part = object.part;
        let size = if part.large { 16 } else { 8 };
        let in_frame = offset + part.x + size > 0 && offset + part.x < width as i32;
        let slot = if in_frame {
            let slot = free.pop();
            dropped += u32::from(slot.is_none());
            slot
        } else {
            None
        };
        extras.push(Extra { part, slot, after });
    }
    (extras, dropped)
}

/// The Enhanced picture of a wide frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WideHd {
    /// The picture; its `logical` rectangle is the original window, scaled.
    pub picture: HdFrame,
    /// The scene class and the rectangles of the frame as the 1x wide frame had them.
    pub scene: Scene,
    pub logical: Rect,
    pub hud: Rect,
    pub stats: WideStats,
    /// The picture lacks art the reading thread is still reading (see [`WideRun::set_art_async`]): the picture of the
    /// frame once the art is read is another. Always `false` when the frames read their art.
    pub art_pending: bool,
}

/// The serial of the next picture made: a counter of the process, so two pictures with one serial are one picture.
#[must_use]
pub fn next_serial() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// An Enhanced picture of the wide view, shared with its caller. The serial names the picture: two pictures
/// with one serial are the same pixels, so a caller that shows the picture its frame shows already (the
/// serial it has) keeps its texture, and the picture is never copied (see [`WideRun::note_hd_shared`]).
#[derive(Debug, Clone)]
pub struct SharedHd {
    pub serial: u64,
    pub picture: Arc<WideHd>,
}

impl SharedHd {
    /// `picture`, under a serial of its own.
    #[must_use]
    pub fn new(picture: WideHd) -> Self {
        Self { serial: next_serial(), picture: Arc::new(picture) }
    }
}

impl Default for WideRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl WideRenderer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            chunks: (0..2).map(|_| Renderer::new()).collect(),
            vram_work: Vec::new(),
            art: None,
            serial: false,
            font: None,
            text_on: false,
            text: TextCapture::default(),
            trust_field: false,
            extras: Vec::new(),
            mapped: false,
            translator: None,
            compose_time: std::time::Duration::ZERO,
            cost: crate::DrawCost::default(),
            lightning: None,
            ssj_lightning: None,
            hide_field_ui: false,
        }
    }

    /// What the last Enhanced picture drawn ([`WideRenderer::render_hd`]) cost, by its stages.
    #[must_use]
    pub fn draw_cost(&self) -> crate::DrawCost {
        self.cost
    }

    /// Makes [`WideRenderer::render_hd`] draw the game's UI text from `font` at a scale of 2 or
    /// more (`None`: from the look alone); see [`crate::font`].
    pub fn set_font(&mut self, font: Option<Arc<Font>>) {
        self.font = font;
    }

    /// Makes [`WideRenderer::render_hd`] draw the translation of `overlay` over the original's message windows, at
    /// a scale of 2 or more with the HD text on (`None`: the original's text); see [`crate::translate`].
    pub fn set_overlay(&mut self, overlay: Option<Arc<Overlay>>) {
        self.translator = overlay.map(Translator::new);
    }

    /// The page the message engine named at the start of the next frame drawn ([`ssd_catalogue::engine`]), for
    /// the translation.
    pub fn set_text_screen(&mut self, screen: Option<OnScreen>) {
        if let Some(translator) = self.translator.as_mut() {
            translator.screen = screen;
        }
    }

    /// Draws every frame in one piece on one thread (`true`) instead of in segments of lines that
    /// the thread pool draws together: the pictures are the same, which the tests and
    /// `SSD_HD_SERIAL` (set in the environment, the same as `true`) rely on to check.
    pub fn set_serial(&mut self, serial: bool) {
        self.serial = serial;
    }

    /// Draws every frame of a field state as the field (the map read through the lookup), without
    /// checking that BG2 holds the map. A tool that moves the camera by writing the machine's
    /// variables (`xtask hd-tour`) leaves BG2's tile columns as they were, which the check takes
    /// for a picture of the game's own; the lookup does not read them. Not for frames of a
    /// scene that draws its own picture on BG2.
    pub fn set_trust_field(&mut self, trust: bool) {
        self.trust_field = trust;
    }

    /// Makes [`WideRenderer::render_hd`] draw the art of `pack` (`None`: none).
    pub fn set_art(&mut self, pack: Option<Arc<Pack>>) {
        self.art = pack.map(|pack| Box::new(ArtState::new(pack)));
    }

    /// Makes [`WideRenderer::render_hd`] measure the colours the poses draw against the
    /// original's ([`crate::ArtStats::drawn`]).
    pub fn set_art_measure(&mut self, on: bool) {
        if let Some(art) = self.art.as_mut() {
            art.measure = on;
        }
    }

    /// Keeps the layers' art with its colour transfer between frames (`on`, the default), or applies each transfer to
    /// each colour as it is drawn (off: the reference the `bench-hd --check` compares with). The pictures are the same.
    pub fn set_transfer_cache(&mut self, on: bool) {
        if let Some(art) = self.art.as_mut() {
            art.set_transfer_cache(on);
        }
    }

    /// The conditions of the frame the next picture with art is drawn from (see [`crate::Renderer::set_conditions`]).
    pub fn set_conditions(&mut self, conditions: Conditions) {
        self.ssj_lightning = conditions.ssj_lightning();
        if let Some(art) = self.art.as_mut() {
            art.conditions = conditions;
        }
    }

    /// Makes [`WideRenderer::render_hd`] draw the pictures of `lightning` over the PPU's picture while Goku's first
    /// Super Saiyan transformation goes on (see [`crate::lightning`]); `None` (the start) draws none.
    pub fn set_lightning(&mut self, lightning: Option<Arc<crate::lightning::Lightning>>) {
        self.lightning = lightning.map(crate::lightning::Overlay::new);
    }

    /// Makes [`WideRenderer::render_hd`] draw the art of the backgrounds alone (the layers, not the
    /// poses, portraits or shadows): the picture an audit compares with the original's colours.
    pub fn set_art_backgrounds_only(&mut self, on: bool) {
        if let Some(art) = self.art.as_mut() {
            art.backgrounds_only = on;
        }
    }

    /// Reads the pack's art on a thread of its own (see [`crate::Renderer::set_art_async`]). Call it
    /// after [`WideRenderer::set_art`].
    pub fn set_art_async(&mut self, on: bool) {
        if let Some(art) = self.art.as_mut() {
            art.set_async(on);
        }
    }

    /// Whether the last picture lacks art it asked the reading thread for: the picture drawn once it
    /// is read is another.
    pub(crate) fn art_pending(&self) -> bool {
        self.art.as_ref().is_some_and(|art| art.art_pending())
    }

    /// What the art did in the last picture drawn with it.
    #[must_use]
    pub fn art_stats(&self) -> crate::ArtStats {
        self.art.as_ref().map(|art| art.stats.clone()).unwrap_or_default()
    }

    /// The portraits of the last picture: where each one's cells come from (see
    /// [`crate::art::portrait_check::ArtState::portrait_uses`]).
    #[must_use]
    pub fn portrait_uses(&self) -> Vec<crate::PortraitUse> {
        self.art.as_ref().map(|art| art.portrait_uses()).unwrap_or_default()
    }

    /// The box of the last picture (see [`crate::art::ArtState::picture_map`]).
    #[must_use]
    pub fn picture_map(&self, x: i32, y: i32, w: usize, h: usize) -> Vec<(u8, Option<u32>, u8)> {
        self.art.as_ref().map(|art| art.picture_map(x, y, w, h)).unwrap_or_default()
    }

    /// The background slots of the last picture (see [`crate::art::ArtState::bg_slot_report`]).
    #[must_use]
    pub fn bg_slot_report(&self) -> Vec<(u32, u8, u8, u16, u16, u8, bool)> {
        self.art.as_ref().map(|art| art.bg_slot_report()).unwrap_or_default()
    }

    /// Turns the tally on or off ([`WideRenderer::set_art`] first): [`WideRenderer::render_hd`]
    /// then counts the pixels of every picture by what they are made of ([`crate::FrameTally`])
    /// and does not draw the art, only the filter's picture (what a tour measures is which pixels
    /// the art would draw, so it need not decode the art files).
    pub fn set_tally(&mut self, on: bool) {
        if let Some(art) = self.art.as_mut() {
            art.set_tally(on);
        }
    }

    /// The tally of the last picture drawn with the tally on (empty without it).
    #[must_use]
    pub fn tally(&self) -> crate::FrameTally {
        self.art.as_ref().map(|art| art.tally.clone()).unwrap_or_default()
    }

    /// The Enhanced picture of any frame of `log` in the wide view (see [`WideRenderer::render_view`]):
    /// the wide 1x frame made again at the scale of `hd` with its look, and the pack's art over it
    /// where there is some. A Mode 7 plane is not sampled again at the picture's resolution here:
    /// the wide frame is filtered like the rest.
    ///
    /// # Errors
    ///
    /// As for [`WideRenderer::render_view`].
    ///
    /// # Panics
    ///
    /// `hd.scale` is not in `1..=MAX_SCALE`.
    pub fn render_hd(
        &mut self,
        rom: Rom<'_>,
        game_mode: u8,
        field: Option<&FieldState>,
        log: &FrameLog,
        options: WideOptions,
        hd: HdOptions,
    ) -> Result<WideHd, WideError> {
        let quiet_field;
        let log = if self.hide_field_ui {
            quiet_field = without_field_ui(log);
            &quiet_field
        } else {
            log
        };
        let started = std::time::Instant::now();
        self.compose_time = std::time::Duration::ZERO;
        assert!((1..=crate::MAX_SCALE).contains(&hd.scale), "the scale is 1 to 8");
        check_width(options.width)?;
        check_frame(log)?;
        let scene = classify_with(rom, game_mode, field, log, self.trust_field);
        self.mapped =
            scene == Scene::Bordered && field.is_some_and(|state| shows_map(rom, state, log));
        let scale = hd.scale as usize;
        let mut art = self.art.take().filter(|art| art.has_art());
        if let Some(art) = art.as_mut() {
            art.begin(options.width, usize::from(log.height), scale);
            art.begin_bg(log);
            // A picture of the pack's own art on BG2 (the island of a story picture) is not the
            // map, whatever the tile check says of a picture of mostly empty words beyond the map
            let picture = scene != Scene::Field
                && art.bg_slots().iter().any(|slot| slot.bg == 1 && !slot.field);
            if picture {
                self.mapped = false;
            }
            art.field_frame = Some(matches!(game_mode & 0x7F, 0x00 | 0x01 | 0x03 | 0x05 | 0x06));
            // Whether the frame shows the field's map in any main loop state: a field the original
            // draws that the art does not take for the field is a frame the art misses
            let in_any_state = field.map(|state| FieldState { game_mode: 0, ..state.clone() });
            // (a frame the game shows dark, in forced blanking or at brightness 0 on every line,
            // has no map to draw: the fade around a battle keeps the map in VRAM)
            if !dark_frame(log)
                && !picture
                && (scene == Scene::Field
                    || self.mapped
                    || classify_with(rom, 0, in_any_state.as_ref(), log, self.trust_field)
                        == Scene::Field)
            {
                art.stats.field_map.available = art.field_entry(field).map(|at| at as u32);
            }
            if scene == Scene::Field || self.mapped {
                art.begin_field(field);
            }
            art.active = true;
        }
        let before: Vec<_> = self.chunks.iter().map(|chunk| chunk.obj_pixels).collect();
        let tallying = art.as_ref().is_some_and(|art| art.tallying());
        self.text_on = self.font.is_some() && (hd.scale >= 2 || tallying);
        // The layers the translation takes the place of (the intro's, `Translator::quiet_layers`): their letters are
        // read from the frame as the game draws it (for the translation: `lettered`), then the frame is drawn again
        // without those layers, and the art (which draws its pictures only where the frame shows no text over them),
        // the picture and the HD text are of that frame: a cell of another layer the letters hid part of (the
        // stripes behind the intro, as blank cells with the pack) would take its ground with the letters' shapes cut
        // out of it
        let quiet = self
            .translator
            .as_ref()
            .map(Translator::quiet_layers)
            .filter(|layers| self.text_on && !tallying && layers.contains(&true))
            .map(|layers| {
                let mut quiet = log.clone();
                for entry in &mut quiet.entries {
                    for (shown, off) in entry.regs.main_bg.iter_mut().zip(layers) {
                        *shown &= !off;
                    }
                }
                quiet
            });
        let mut lettered = None;
        let first = match &quiet {
            Some(quiet) => {
                let _ = self.compose(rom, field, log, options, scene, None, 0, None);
                lettered = Some(self.text.cells().to_vec());
                for (chunk, saved) in self.chunks.iter_mut().zip(&before) {
                    chunk.obj_pixels = *saved;
                }
                self.compose(rom, field, quiet, options, scene, art.as_deref_mut(), 0, None)
            }
            None => self.compose(rom, field, log, options, scene, art.as_deref_mut(), 0, None),
        };
        self.text_on = false;
        let wide = match first {
            Ok(wide) => wide,
            Err(error) => {
                self.art = art;
                return Err(error);
            }
        };
        // The sprites the poses are made of: in the field, those of the whole view (the margins
        // too, with the objects the original drops), as the original's window has them
        let view = if scene == Scene::Field && options.width > WIDTH {
            View {
                left: -wide.logical.x,
                right: options.width as i32 - wide.logical.x,
                window_x: wide.logical.x,
                extras: self
                    .extras
                    .iter()
                    .enumerate()
                    .filter_map(|(order, extra)| extra.sprite(order))
                    .collect(),
            }
        } else {
            View::window_at(wide.logical.x)
        };
        let found = art.as_mut().map_or_else(Vec::new, |art| {
            art.active = false;
            art.find_poses_in(log, &view)
        });
        if tallying && let Some(art) = art.as_mut() {
            art.tally_frame(log, self.text.cells());
        }
        let mut under = None;
        if !found.is_empty() && !tallying {
            let after: Vec<_> = self.chunks.iter().map(|chunk| chunk.obj_pixels).collect();
            for (chunk, saved) in self.chunks.iter_mut().zip(&before) {
                chunk.obj_pixels = *saved;
            }
            let hidden = found.iter().fold(0, |all, (pose, _)| all | pose.sprites);
            let rows = crate::art::pose_rows(&found, wide.frame.height);
            // (the art records what the layers are under the hidden sprites as this picture is drawn)
            let saved = art.as_mut().map(|art| {
                art.active = true;
                art.begin_under()
            });
            let drawn = quiet.as_ref().unwrap_or(log);
            under = self
                .compose(rom, field, drawn, options, scene, art.as_deref_mut(), hidden, rows)
                .ok();
            if let (Some(art), Some(saved)) = (art.as_mut(), saved) {
                art.active = false;
                art.end_under(saved, under.is_some(), hidden, rows);
            }
            if let (Some(under), Some((first_row, past))) = (under.as_mut(), rows) {
                // The lines the poses do not reach are the first frame's
                let width = wide.frame.width;
                for row in (0..first_row).chain(past..wide.frame.height) {
                    let span = row * width..(row + 1) * width;
                    under.frame.pixels[span.clone()].copy_from_slice(&wide.frame.pixels[span]);
                }
            }
            for (chunk, saved) in self.chunks.iter_mut().zip(&after) {
                chunk.obj_pixels = *saved;
            }
        }
        // A plane drawn from art with `replace`: the frame again without BG1, the picture under the plane (only then)
        let under_plane = if !tallying && art.as_ref().is_some_and(|art| art.plane_replaces()) {
            let saved: Vec<_> = self.chunks.iter().map(|chunk| chunk.obj_pixels).collect();
            let drawn = crate::art::without_bg1(quiet.as_ref().unwrap_or(log));
            let frame = self.compose(rom, field, &drawn, options, scene, None, 0, None).ok();
            for (chunk, saved) in self.chunks.iter_mut().zip(&saved) {
                chunk.obj_pixels = *saved;
            }
            frame
        } else {
            None
        };
        // The picture under the title's text, for each BG layer that holds clear glyphs: the frame
        // drawn again with only that layer off, where the ink of the title hid the picture
        let beneath = if self.font.is_some() && hd.scale >= 2 && !tallying {
            crate::font::beneath_logs(quiet.as_ref().unwrap_or(log), self.text.cells()).map(
                |blank| {
                    blank.and_then(|blank| {
                        self.compose(rom, field, &blank, options, scene, None, 0, None).ok()
                    })
                },
            )
        } else {
            [None, None, None, None]
        };
        let beneath_pixels: [Option<&[Color]>; 4] =
            std::array::from_fn(|bg| beneath[bg].as_ref().map(|frame| &frame.frame.pixels[..]));
        let base = under.as_ref().map_or(&wide.frame, |under| &under.frame);
        // The filter must not see the letters the font redraws: a pixel beside one would keep
        // sub-pixels of its colour as specks around the HD glyph
        let mut cleaned = (self.font.is_some() && hd.scale >= 2 && !tallying).then(|| Frame {
            pixels: crate::font::without_letters(
                &base.pixels,
                base.width,
                self.text.cells(),
                beneath_pixels,
            ),
            sources: Vec::new(),
            ..*base
        });
        // (the intro's lines the translation draws: their English goes, the marks the font does not know too)
        if let (Some(cleaned), Some(translator)) = (cleaned.as_mut(), self.translator.as_ref()) {
            let width = cleaned.width;
            translator.clean_intro(
                &mut cleaned.pixels,
                width,
                self.text.cells(),
                beneath_pixels,
                wide.logical.x,
            );
        }
        if let (Some(art), Some(under_plane)) = (art.as_mut(), under_plane.as_ref()) {
            let under = picture(&under_plane.frame, &crate::enhanced::Capture::default(), None, hd);
            art.set_plane_under(under);
        }
        let mut picture = picture(
            cleaned.as_ref().unwrap_or(base),
            &crate::enhanced::Capture::default(),
            None,
            hd,
        );
        let logical = wide.logical;
        let scaled = |value: i32| value as u32 * hd.scale;
        picture.logical = crate::Rect {
            x: scaled(logical.x),
            y: scaled(logical.y),
            width: scaled(logical.width),
            height: scaled(logical.height),
        };
        if let Some(art) = art.as_mut().filter(|_| !tallying) {
            let clip = if wide.scene == Scene::Bordered {
                (logical.x, 0, logical.x + logical.width, wide.frame.height as i32)
            } else {
                (0, 0, wide.frame.width as i32, wide.frame.height as i32)
            };
            art.compose(&mut picture, &wide.frame, &found, logical.x, clip);
        }
        // (the transformation's lightning: over the frame and the art, in the PPU's picture, under the HD text)
        if let (Some(overlay), Some(frames), false) =
            (self.lightning.as_mut(), self.ssj_lightning, tallying)
        {
            overlay.draw(&mut picture, frames);
        }
        // (the pixels of the text cells a pose variant was drawn on in front of: the text leaves them to the art)
        let uncovered =
            art.as_ref().filter(|_| !tallying).and_then(|art| art.uncover(self.text.cells()));
        let uncovered_lettered = match (art.as_ref().filter(|_| !tallying), lettered.as_deref()) {
            (Some(art), Some(lettered)) => art.uncover(lettered),
            _ => None,
        };
        self.art = art;
        let font = self.font.clone();
        if let (Some(font), true) = (font.as_ref(), hd.scale >= 2 && !tallying) {
            let text_cells = uncovered.as_deref().unwrap_or(self.text.cells());
            // (the letters the translation draws in place of, the intro's: not drawn, the picture under them shows)
            let drawn =
                self.translator.as_ref().and_then(|translator| translator.font_cells(text_cells));
            // (the picture before the HD text draws the letters: what the ink of a letter the overlay takes out of
            // the picture past its window covered around its cell, `translate::paint_spilled`; kept only for a frame
            // that may have such cells, a page named or shown or cells spilled before)
            let unlettered = self
                .translator
                .as_ref()
                .filter(|translator| translator.may_spill())
                .map(|_| picture.rgba.clone());
            font.draw_with(
                &mut picture.rgba,
                picture.width as usize,
                scale,
                drawn.as_deref().unwrap_or(text_cells),
                &wide.frame.pixels,
                wide.frame.width,
                beneath_pixels,
            );
            // (the card tooltip, over the HD text, unless the translation turns it off)
            crate::tooltip::draw(
                font,
                &mut picture.rgba,
                picture.width as usize,
                scale,
                drawn.as_deref().unwrap_or(text_cells),
                self.translator.as_ref().map(|translator| &*translator.overlay),
            );
            if let Some(mut translator) = self.translator.take() {
                let width = picture.width as usize;
                let mut target = Target { rgba: &mut picture.rgba, width, scale };
                let character = |glyph| font.character(glyph);
                let cells = uncovered_lettered
                    .as_deref()
                    .or(lettered.as_deref())
                    .unwrap_or(text_cells)
                    .to_vec();
                // (the picture under the cells an English line spilled out of its window: the frame drawn again
                // with the art, without the layers of the letters there, once a frame)
                let drawn = quiet.as_ref().unwrap_or(log);
                let clip = if wide.scene == Scene::Bordered {
                    (logical.x, 0, logical.x + logical.width, wide.frame.height as i32)
                } else {
                    (0, 0, wide.frame.width as i32, wide.frame.height as i32)
                };
                let height = wide.frame.height;
                let mut made: Option<BeneathMade> = None;
                let mut beneath = |spilled: &[(i32, i32)]| {
                    let mut layers = [false; 4];
                    for cell in cells.iter().filter(|cell| spilled.contains(&(cell.x, cell.y))) {
                        layers[usize::from(cell.layer & 3)] = true;
                    }
                    if !layers.contains(&true) {
                        return None;
                    }
                    // (only the lines of the spilled cells are drawn again, a line over and under them too)
                    let top = spilled.iter().map(|&(_, y)| y).min().unwrap_or(0) - 1;
                    let bottom = spilled.iter().map(|&(_, y)| y).max().unwrap_or(0) + 9;
                    let rows = (
                        top.clamp(0, height as i32) as usize,
                        bottom.clamp(0, height as i32) as usize,
                    );
                    if let Some((had, had_rows, under)) = &made
                        && layers.iter().zip(had).all(|(wanted, had)| !wanted || *had)
                        && had_rows.0 <= rows.0
                        && rows.1 <= had_rows.1
                    {
                        return under.clone();
                    }
                    let under = self
                        .spilled_beneath(
                            rom,
                            field,
                            drawn,
                            options,
                            scene,
                            hd,
                            layers,
                            rows,
                            &found,
                            (logical.x, clip),
                        )
                        .map(Arc::new);
                    made = Some((layers, rows, under.clone()));
                    under
                };
                translator.draw(
                    &mut target,
                    &cells,
                    &wide.frame,
                    log,
                    logical.x,
                    &character,
                    &mut beneath,
                    unlettered.as_deref(),
                );
                self.translator = Some(translator);
            }
        }
        let mut cost = self.art.as_ref().map_or_else(crate::DrawCost::default, |art| art.cost());
        cost.draw = self.compose_time;
        cost.total = started.elapsed();
        self.cost = cost;
        Ok(WideHd {
            picture,
            scene: wide.scene,
            logical: wide.logical,
            hud: wide.hud,
            stats: wide.stats,
            art_pending: self.art_pending(),
        })
    }

    /// Renders the wide frame of a field frame of `log`.
    ///
    /// # Errors
    ///
    /// The width is out of range, or the frame is not one the plain renderer models or not a
    /// field frame ([`WideError`]).
    pub fn render(
        &mut self,
        rom: Rom<'_>,
        state: &FieldState,
        log: &FrameLog,
        options: WideOptions,
    ) -> Result<WideFrame, WideError> {
        check_width(options.width)?;
        check_field(log, state)?;
        if !check_tiles(rom, state, log).is_map() {
            return Err(WideError::NotField(NotField::Picture));
        }
        self.compose(rom, Some(state), log, options, Scene::Field, None, 0, None)
    }

    /// Renders any frame of `log` in the wide view: as the field where the field's map is on
    /// screen, as a flight or as a picture ([`classify`]). `game_mode` and `field` are read from
    /// the work RAM the frame was rendered from ([`FieldState::from_wram`] for `field`).
    ///
    /// # Errors
    ///
    /// The width is out of range, or the frame is not one the plain renderer models ([`WideError`]).
    pub fn render_view(
        &mut self,
        rom: Rom<'_>,
        game_mode: u8,
        field: Option<&FieldState>,
        log: &FrameLog,
        options: WideOptions,
    ) -> Result<WideFrame, WideError> {
        check_width(options.width)?;
        let scene = classify_with(rom, game_mode, field, log, self.trust_field);
        self.compose(rom, field, log, options, scene, None, 0, None)
    }

    /// The picture beneath the cells an English line spilled out of its window ([`Beneath`]): the frame of `log`
    /// drawn again without the BG `layers` of the letters there, with the art and the poses `found` in the frame (a
    /// portrait beside the window, as a picture or as the pose of its sprites, where the letters hid it), at the
    /// scale of `hd`. Only its lines `rows` (first, past the last) are drawn, the lines of the cells: the others are
    /// black and show no picture. The art's record of the frame and the renderer's sprites are put back as they
    /// were. `None` without art.
    #[allow(clippy::too_many_arguments)]
    fn spilled_beneath(
        &mut self,
        rom: Rom<'_>,
        field: Option<&FieldState>,
        log: &FrameLog,
        options: WideOptions,
        scene: Scene,
        hd: HdOptions,
        layers: [bool; 4],
        rows: (usize, usize),
        found: &[(crate::art::poses::Pose, u32)],
        (x_offset, clip): (i32, (i32, i32, i32, i32)),
    ) -> Option<Beneath> {
        let mut art = self.art.take()?;
        let mut quiet = log.clone();
        for entry in &mut quiet.entries {
            for (shown, off) in entry.regs.main_bg.iter_mut().zip(layers) {
                *shown &= !off;
            }
        }
        let before: Vec<_> = self.chunks.iter().map(|chunk| chunk.obj_pixels).collect();
        art.active = true;
        let saved = art.begin_under();
        let drawn = self.compose(rom, field, &quiet, options, scene, Some(&mut art), 0, Some(rows));
        art.active = false;
        let under = drawn.ok().map(|wide| {
            let mut beneath = picture(&wide.frame, &crate::enhanced::Capture::default(), None, hd);
            art.compose(&mut beneath, &wide.frame, found, x_offset, clip);
            Beneath {
                rgba: std::mem::take(&mut beneath.rgba),
                width: beneath.width as usize,
                pictures: art.picture_pixels(),
                frame_width: wide.frame.width,
            }
        });
        art.end_under(saved, false, 0, None);
        for (chunk, saved) in self.chunks.iter_mut().zip(&before) {
            chunk.obj_pixels = *saved;
        }
        self.art = Some(art);
        under
    }

    /// [`WideRenderer::compose_frame`], timed into the picture's cost.
    #[allow(clippy::too_many_arguments)]
    fn compose(
        &mut self,
        rom: Rom<'_>,
        state: Option<&FieldState>,
        log: &FrameLog,
        options: WideOptions,
        scene: Scene,
        art: Option<&mut ArtState>,
        hidden: u128,
        only_rows: Option<(usize, usize)>,
    ) -> Result<WideFrame, WideError> {
        let started = std::time::Instant::now();
        let frame = self.compose_frame(rom, state, log, options, scene, art, hidden, only_rows);
        self.compose_time += started.elapsed();
        frame
    }

    #[allow(clippy::too_many_arguments)]
    fn compose_frame(
        &mut self,
        rom: Rom<'_>,
        state: Option<&FieldState>,
        log: &FrameLog,
        options: WideOptions,
        scene: Scene,
        mut art: Option<&mut ArtState>,
        hidden: u128,
        only_rows: Option<(usize, usize)>,
    ) -> Result<WideFrame, WideError> {
        let width = options.width;
        check_frame(log)?;
        let height = usize::from(log.height);
        let mut frame = Frame {
            width,
            height,
            pixels: vec![Color::BLACK; width * height],
            sources: Vec::new(),
        };

        // The view: from the first line that shows anything, in the field; centred otherwise
        let field = state.filter(|_| scene == Scene::Field);
        let (view_left, offset) = match field {
            Some(state) => {
                let reference = log.entries.iter().find(|entry| !entry.regs.forced_blanking);
                let (window_left, _) = reference.map_or((i32::from(state.camera.0), 0), |entry| {
                    window_origin(&entry.regs, state.camera)
                });
                place_window(state.map.size().0, window_left, width)
            }
            None => (0, ((width - WIDTH) / 2) as i32),
        };
        let hud_offset = match options.hud {
            HudPlacement::Window => offset,
            HudPlacement::Centred => (width as i32 - WIDTH as i32) / 2,
        };
        let rect = |x: i32| Rect { x, y: 0, width: WIDTH as i32, height: height as i32 };
        let mut wide = WideFrame {
            frame: Frame { width, height, pixels: Vec::new(), sources: Vec::new() },
            scene,
            logical: rect(offset),
            hud: rect(hud_offset),
            view_left,
            stats: WideStats::default(),
            extra_boxes: Vec::new(),
        };
        // The objects the original drops, in the frame's coordinates
        let (extras, dropped) = match field {
            Some(state) if width > WIDTH => {
                place_extras(&state.all_parts(rom), &log.oam_low, &log.oam_high, offset, width)
            }
            _ => (Vec::new(), 0),
        };
        wide.stats.extra_parts = extras.len() as u32;
        wide.stats.extra_dropped = dropped;
        for extra in &extras {
            let size = if extra.part.large { 16 } else { 8 };
            wide.extra_boxes.push(Rect {
                x: offset + extra.part.x,
                y: i32::from(extra.part.y),
                width: size,
                height: size,
            });
        }

        while self.chunks.len() < 2 {
            self.chunks.push(Renderer::new());
        }
        let chunk_starts = if width == WIDTH { vec![0] } else { vec![0, (width - WIDTH) as i32] };
        let mut chunks = Vec::new();
        for (renderer, x0) in self.chunks.drain(..).zip(chunk_starts) {
            let oam = chunk_oam(&log.oam_low, &log.oam_high, offset - x0, &extras);
            chunks.push(Chunk { renderer, x0, oam });
        }
        for chunk in &mut chunks {
            chunk.renderer.cgram.copy_from_slice(&log.cgram_start);
            chunk.renderer.hidden_objects = hidden;
        }
        // The art's binding follows the VRAM of each line, as the frame's writes change it
        let mut binder = art.as_mut().and_then(|art| art.binder.take());
        if let Some(binder) = binder.as_mut() {
            binder.update(&log.vram);
        }

        let world =
            field.or_else(|| state.filter(|_| self.mapped)).map(|state| World { rom, state });
        let font = if self.text_on { self.font.clone() } else { None };
        let shared = Shared {
            log,
            width,
            height,
            offset,
            hud_offset,
            scene,
            world,
            only_rows,
            font: font.as_deref(),
        };
        let steady = log.vram_writes.is_empty() && log.oam_writes.is_empty();
        let cuts = if steady && !self.serial && !serial_env() {
            segment_starts(log, only_rows, height)
        } else {
            vec![0]
        };
        let done: Vec<Done> = if cuts.len() == 1 {
            // One run over the whole frame: it may write VRAM and OAM, and the binding follows
            let job = Job {
                range: 0..log.entries.len(),
                chunks,
                rows: &mut frame.pixels[..],
                row0: 0,
                art: art.as_mut().map(|art| art.capture_parts()),
                text: font.as_ref().map(|_| TextCapture::default()),
            };
            let access = binder.as_mut().map(BinderRef::Own);
            vec![shared.run(job, &mut self.vram_work, access, &extras)]
        } else {
            // Lines are independent once the sprite line buffer is rebuilt: the segments start at
            // lines that rebuild it, so each can be drawn on its own
            let mut jobs = Vec::new();
            let mut rest_frame: &mut [Color] = &mut frame.pixels[..];
            let mut rest_art: Option<(&Lookup, &mut [ArtPixel])> =
                art.as_mut().map(|art| art.capture_parts());
            let mut taken_rows = 0_usize;
            let mut first_chunks = Some(chunks);
            for (n, &start) in cuts.iter().enumerate() {
                let end = cuts.get(n + 1).copied().unwrap_or(log.entries.len());
                let range = start..end;
                let (rows_lo, rows_hi) = line_span(log, &range, height)
                    .map_or((taken_rows, taken_rows), |(lo, hi)| (lo - 1, hi));
                let skip = (rows_lo - taken_rows) * width;
                let take = (rows_hi - rows_lo) * width;
                let (_, tail) = std::mem::take(&mut rest_frame).split_at_mut(skip);
                let (mine, tail) = tail.split_at_mut(take);
                rest_frame = tail;
                let art_rows = rest_art.take().map(|(lookup, pixels)| {
                    let (_, tail) = pixels.split_at_mut(skip);
                    let (mine, tail) = tail.split_at_mut(take);
                    rest_art = Some((lookup, tail));
                    (lookup, mine)
                });
                taken_rows = rows_hi;
                // The first segment goes on from the last frame's line buffers; the others start
                // from a rebuild
                let segment_chunks = match first_chunks.take() {
                    Some(chunks) => chunks,
                    None => fresh_chunks(&shared, hidden, &extras),
                };
                jobs.push(Job {
                    range,
                    chunks: segment_chunks,
                    rows: mine,
                    row0: rows_lo,
                    art: art_rows,
                    text: font.as_ref().map(|_| TextCapture::default()),
                });
            }
            let access = binder.as_ref();
            jobs.into_par_iter()
                .map(|job| shared.run(job, &mut Vec::new(), access.map(BinderRef::Shared), &extras))
                .collect()
        };
        let mut portrait = 0;
        let last = done.len().saturating_sub(1);
        let mut kept = Vec::new();
        if font.is_some() {
            self.text.clear();
        }
        for (n, result) in done.into_iter().enumerate() {
            if let Some(text) = &result.text {
                self.text.merge(text);
            }
            wide.stats.outside_skipped += result.stats.outside_skipped;
            wide.stats.outside_drawn += result.stats.outside_drawn;
            portrait += result.portrait;
            if n == last {
                kept = result.chunks;
            }
        }
        // Back to the renderer, for the next frame's first line
        if let Some(art) = art {
            art.binder = binder;
            art.stats.portrait_pixels_without_art += portrait;
        }
        self.chunks = kept
            .into_iter()
            .map(|mut chunk| {
                chunk.renderer.hidden_objects = 0;
                chunk.renderer
            })
            .collect();
        wide.frame = frame;
        self.extras = extras;
        Ok(wide)
    }
}

/// The picture beneath the spilled cells drawn in a frame (`WideRenderer::spilled_beneath`), with the BG layers it
/// was drawn without and its lines (first, past the last): a later ask of the frame that these hold takes it again.
type BeneathMade = ([bool; 4], (usize, usize), Option<Arc<Beneath>>);

/// Whether the environment asks for every frame in one piece (`SSD_HD_SERIAL`).
fn serial_env() -> bool {
    // (read once: the environment does not change while a run draws)
    static SERIAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SERIAL.get_or_init(|| std::env::var_os("SSD_HD_SERIAL").is_some())
}

/// What every segment of a frame shares.
struct Shared<'a> {
    log: &'a FrameLog,
    width: usize,
    height: usize,
    offset: i32,
    hud_offset: i32,
    scene: Scene,
    world: Option<World<'a>>,
    only_rows: Option<(usize, usize)>,
    /// The HD text font, when the cells of text are noted.
    font: Option<&'a Font>,
}

/// The binding the capture reads: one that follows VRAM writes, or the frame's (it has none).
enum BinderRef<'a> {
    Own(&'a mut Binder<'static>),
    Shared(&'a Binder<'static>),
}

impl BinderRef<'_> {
    fn get(&self) -> &Binder<'static> {
        match self {
            Self::Own(binder) => binder,
            Self::Shared(binder) => binder,
        }
    }

    fn update(&mut self, vram: &[u16]) {
        if let Self::Own(binder) = self {
            binder.update(vram);
        }
    }
}

/// A run of entries of the log, drawn on its own: its chunks, and the rows of the frame and of
/// the art's record that its lines fill (`row0` is the first of them).
struct Job<'a> {
    range: std::ops::Range<usize>,
    chunks: Vec<Chunk>,
    rows: &'a mut [Color],
    row0: usize,
    art: Option<(&'a Lookup, &'a mut [ArtPixel])>,
    /// Notes the text cells of the lines, when the view draws HD text.
    text: Option<TextCapture>,
}

/// What a segment leaves: its chunks (the last segment's go back to the renderer), its counts,
/// and the characters of portraits it found without a picture.
struct Done {
    chunks: Vec<Chunk>,
    stats: WideStats,
    portrait: u64,
    text: Option<TextCapture>,
}

impl Shared<'_> {
    /// Draws the entries of `job`: the chunks' lines, from the state the entries before it left.
    fn run(
        &self,
        job: Job<'_>,
        work: &mut Vec<u16>,
        mut binder: Option<BinderRef<'_>>,
        extras: &[Extra],
    ) -> Done {
        let Shared { log, width, height, offset, hud_offset, scene, world, only_rows, font } =
            *self;
        let Job { range, mut chunks, rows, row0, art, mut text } = job;
        let mut stats = WideStats::default();
        let start = range.start;
        // The state the segment starts from: the CGRAM writes of the entries before it
        let before = log.cgram_writes.partition_point(|write| usize::from(write.entry) < start);
        for chunk in &mut chunks {
            for write in &log.cgram_writes[..before] {
                chunk.renderer.cgram[usize::from(write.index)] = write.value;
            }
        }
        let mut cgram_writes = log.cgram_writes[before..].iter().peekable();
        if !log.vram_writes.is_empty() {
            work.clear();
            work.extend_from_slice(&log.vram);
        }
        let mut vram_writes = log.vram_writes.iter().peekable();
        // The OAM as the writes during the display leave it, as the PPU's renderer sees it
        let mut oam_writes = log.oam_writes.iter().peekable();
        let mut oam_work =
            (!log.oam_writes.is_empty()).then(|| (log.oam_low.clone(), log.oam_high.clone()));
        let mut record =
            art.map(|(lookup, pixels)| crate::art::Rows::new(lookup, pixels, width, row0 + 1));
        let mut previous_line = 0;
        let mut stale = false;
        for index in range {
            let entry = &log.entries[index];
            while let Some(&CgramWrite { index: slot, value, .. }) =
                cgram_writes.next_if(|write| usize::from(write.entry) <= index)
            {
                for chunk in &mut chunks {
                    chunk.renderer.cgram[usize::from(slot)] = value;
                }
            }
            let mut vram_changed = false;
            while let Some(&VramWrite { address, value, .. }) =
                vram_writes.next_if(|write| usize::from(write.entry) <= index)
            {
                work[usize::from(address)] = value;
                vram_changed = true;
            }
            // The binding is only made for a line that shows something: a game that loads VRAM
            // with the display off writes it hundreds of times
            stale |= vram_changed;
            let mut oam_changed = false;
            while let Some(&OamWrite { high, index: slot, value, .. }) =
                oam_writes.next_if(|write| usize::from(write.entry) <= index)
            {
                let slot = usize::from(slot);
                let oam = oam_work.as_mut().expect("OAM writes have a copy");
                if high {
                    oam.1[slot] = value as u8;
                } else {
                    oam.0[2 * slot..2 * slot + 2].copy_from_slice(&value.to_le_bytes());
                }
                oam_changed = true;
            }
            if let (true, Some(oam)) = (oam_changed, oam_work.as_ref()) {
                // A write during the display: the chunks' objects from this line on (the extras
                // are only placed in frames without writes, which the field does not show)
                for chunk in &mut chunks {
                    let shift = offset - chunk.x0;
                    chunk.oam = chunk_oam(&oam.0, &oam.1, shift, extras);
                }
            }
            let vram = if log.vram_writes.is_empty() { &log.vram } else { &work[..] };
            let line = usize::from(entry.scanline);
            if line == 0 || line > height || usize::from(entry.from_pixel) >= WIDTH {
                continue;
            }
            if only_rows.is_some_and(|(first, past)| !(first..past).contains(&(line - 1))) {
                continue;
            }
            if stale && !entry.regs.forced_blanking {
                if let Some(binder) = binder.as_mut() {
                    binder.update(work);
                }
                stale = false;
            }
            let row = &mut rows[(line - 1 - row0) * width..(line - row0) * width];
            let from = if entry.from_pixel == 0 { i32::MIN } else { i32::from(entry.from_pixel) };
            let first_line = line != std::mem::replace(&mut previous_line, line);
            let layout = Layout { width, offset, hud_offset, from, count: first_line, scene };
            let (low, high) = match oam_work.as_ref() {
                Some((low, high)) => (&low[..], &high[..]),
                None => (&log.oam_low[..], &log.oam_high[..]),
            };
            if let Some(text) = text.as_mut() {
                text.new_entry();
            }
            for chunk in &mut chunks {
                let memories = Memories { vram, oam: (low, high) };
                let capture = match (record.as_mut(), binder.as_ref()) {
                    (Some(rows), Some(binder)) => Some((rows, binder.get())),
                    _ => None,
                };
                let notes = text.as_mut().zip(font);
                chunk.render_entry(world, memories, entry, layout, row, &mut stats, capture, notes);
            }
        }
        Done { chunks, stats, portrait: record.map_or(0, |rows| rows.portrait), text }
    }
}

/// The chunks of a segment that starts from a rebuilt line buffer: new renderers with the frame's
/// first CGRAM and the objects of the frame.
fn fresh_chunks(shared: &Shared<'_>, hidden: u128, extras: &[Extra]) -> Vec<Chunk> {
    let width = shared.width;
    let starts = if width == WIDTH { vec![0] } else { vec![0, (width - WIDTH) as i32] };
    starts
        .into_iter()
        .map(|x0| {
            let log = shared.log;
            let mut renderer = Renderer::new();
            renderer.cgram.copy_from_slice(&log.cgram_start);
            renderer.hidden_objects = hidden;
            let shift = shared.offset - x0;
            let oam = chunk_oam(&log.oam_low, &log.oam_high, shift, extras);
            Chunk { renderer, x0, oam }
        })
        .collect()
}

/// The entries a segment starts at, the first being 0: lines that rebuild the sprite line buffer,
/// spread so the lines drawn are shared out evenly. One segment when the entries are not in line
/// order or too few to be worth sharing.
fn segment_starts(log: &FrameLog, only_rows: Option<(usize, usize)>, height: usize) -> Vec<usize> {
    let entries = &log.entries;
    let ordered = entries.windows(2).all(|pair| pair[0].scanline <= pair[1].scanline);
    let drawn = |entry: &LineEntry| {
        let line = usize::from(entry.scanline);
        line != 0
            && line <= height
            && usize::from(entry.from_pixel) < WIDTH
            && only_rows.is_none_or(|(first, past)| (first..past).contains(&(line - 1)))
    };
    let total = entries.iter().filter(|entry| drawn(entry)).count();
    let parts = rayon::current_num_threads().clamp(1, 6).min(total / 24);
    if !ordered || parts < 2 {
        return vec![0];
    }
    let mut cuts = vec![0];
    let mut seen = 0;
    for (index, entry) in entries.iter().enumerate() {
        if cuts.len() < parts
            && seen >= cuts.len() * total / parts
            && index > 0
            && entry.from_pixel == 0
            && entry.sprites.rebuilt
            && entry.scanline != entries[index - 1].scanline
        {
            cuts.push(index);
        }
        seen += usize::from(drawn(entry));
    }
    cuts
}

/// The first and the last line (1-based) drawn by the entries of `range`; `None` when it draws
/// none.
fn line_span(
    log: &FrameLog,
    range: &std::ops::Range<usize>,
    height: usize,
) -> Option<(usize, usize)> {
    let mut span: Option<(usize, usize)> = None;
    for entry in &log.entries[range.clone()] {
        let line = usize::from(entry.scanline);
        if line == 0 || line > height || usize::from(entry.from_pixel) >= WIDTH {
            continue;
        }
        span = Some(span.map_or((line, line), |(lo, hi)| (lo.min(line), hi.max(line))));
    }
    span
}

/// What the worker runs once on its own thread before the first picture (the app registers the thread with the
/// system's performance hints there).
pub type WorkerStart = Arc<dyn Fn() + Send + Sync>;

/// A frame handed to the worker, with the renderer that draws it.
struct DrawJob {
    renderer: WideRenderer,
    rom: Arc<[u8]>,
    game_mode: u8,
    field: Option<FieldState>,
    log: FrameLog,
    options: WideOptions,
    hd: HdOptions,
}

/// What the worker hands back: the renderer, and the picture (`None`: the frame is refused).
type DrawDone = (WideRenderer, Option<WideHd>);

/// The thread `hd-draw` and the two channels to it. It lives as long as the [`WideRun`] (its jobs channel closing ends
/// it), so a picture in the background costs no thread of its own.
struct Worker {
    jobs: std::sync::mpsc::Sender<DrawJob>,
    done: std::sync::mpsc::Receiver<DrawDone>,
}

impl Worker {
    fn start(first: Option<WorkerStart>) -> Self {
        let (jobs, job_queue) = std::sync::mpsc::channel::<DrawJob>();
        let (done_tx, done) = std::sync::mpsc::channel::<DrawDone>();
        std::thread::Builder::new()
            .name("hd-draw".to_owned())
            .spawn(move || {
                if let Some(first) = first {
                    first();
                }
                while let Ok(mut job) = job_queue.recv() {
                    let rom = Rom(&job.rom);
                    let picture = job
                        .renderer
                        .render_hd(
                            rom,
                            job.game_mode,
                            job.field.as_ref(),
                            &job.log,
                            job.options,
                            job.hd,
                        )
                        .ok();
                    if done_tx.send((job.renderer, picture)).is_err() {
                        break;
                    }
                }
            })
            .expect("a thread for the picture");
        Self { jobs, done }
    }
}

/// How far the picture in the background has got.
enum Settled {
    /// None is being drawn.
    Idle,
    /// It is still being drawn.
    Busy,
    /// It is done: the picture, or `None` for a frame the renderer refused.
    Done(Option<WideHd>),
}

/// The wide view of a run, one frame at a time: each frame is drawn as its scene class, from the
/// work RAM it started from (the end of the frame before it), so the field's variables are the
/// ones its NMI used. Feed it every frame of the run in order: the sprite line buffers carry over.
pub struct WideRun {
    hide_field_ui: bool,
    rom: Arc<[u8]>,
    /// The renderer; `None` while a picture is being drawn in the background (`drawing`).
    renderer: Option<WideRenderer>,
    /// The thread that draws the pictures in the background ([`WideRun::redraw_hd_pipelined`]), started on the first
    /// one (or by [`WideRun::start_worker`]) and kept for the run.
    worker: Option<Worker>,
    /// A picture is being drawn in the background: the renderer is with the worker and comes back with it.
    drawing: bool,
    /// What the worker does first, on its own thread (see [`WideRun::set_worker_start`]).
    worker_start: Option<WorkerStart>,
    /// The main loop's state and the field variables at the end of the last frame noted.
    ended: Option<Started>,
    /// What the last frame skipped ([`WideRun::skip`]) started from, until it is drawn after all
    /// ([`WideRun::redraw`]) or another frame is noted.
    skipped: Option<Started>,
    /// With a translation ([`WideRun::set_overlay`]): the page the message engine named at the end of the last
    /// frame noted, and the one the frame being drawn started from.
    text: Option<TextRun>,
    /// The last picture [`WideRun::note_hd_shared`] drew, with the inputs it was drawn from.
    shared: Option<Drawn>,
    /// The frames since the transformation started for the frame noted next ([`WideRun::set_ssj_lightning`]).
    ssj_lightning: Option<u32>,
    /// What the Enhanced pictures drawn since [`WideRun::take_cost`] cost, added up, and whether the last was drawn
    /// in the background.
    cost: Option<(crate::DrawCost, bool)>,
}

/// What a frame started from, as the work RAM says it: the main loop's state, the field variables and the pose variants'
/// conditions.
type Started = (u8, Option<FieldState>, Conditions);

/// What a picture of [`WideRun::note_hd_shared`] was drawn from: the frame's start, the text page handed to the
/// renderer, the HD options and the render log. Equal inputs draw the same picture.
struct Inputs {
    started: Started,
    text: Option<OnScreen>,
    hd: HdOptions,
    log: FrameLog,
}

impl Inputs {
    fn is(
        &self,
        started: &Started,
        text: Option<&OnScreen>,
        hd: HdOptions,
        log: &FrameLog,
    ) -> bool {
        self.hd == hd
            && self.started == *started
            && self.text.as_ref() == text
            && same_log(&self.log, log)
    }
}

/// The last picture [`WideRun::note_hd_shared`] drew, and what it was drawn from.
struct Drawn {
    inputs: Inputs,
    shared: SharedHd,
}

fn without_field_ui(log: &FrameLog) -> FrameLog {
    let mut quiet = log.clone();
    for entry in &mut quiet.entries {
        let regs = &mut entry.regs;
        // The field's faces and windows only. BG2, OBJ, scroll and Mode 7 are untouched.
        if regs.bg_mode == 1 && regs.bg_map_base[0] == 0x6800 && regs.bg_map_base[2] == 0x2C00 {
            for bg in [0, 2] {
                regs.main_bg[bg] = false;
                regs.sub_bg[bg] = false;
            }
        }
    }
    quiet
}

/// Whether two render logs hold the same frame (`FrameLog` has no `PartialEq`). The pattern names every field, so a
/// field the log gains is an error here until it is compared as well.
fn same_log(a: &FrameLog, b: &FrameLog) -> bool {
    let FrameLog {
        complete,
        height,
        h_hi_res,
        v_hi_res,
        entries,
        cgram_start,
        cgram_writes,
        vram,
        vram_writes,
        oam_low,
        oam_high,
        oam_writes,
    } = a;
    *complete == b.complete
        && *height == b.height
        && *h_hi_res == b.h_hi_res
        && *v_hi_res == b.v_hi_res
        && *entries == b.entries
        && *cgram_start == b.cgram_start
        && *cgram_writes == b.cgram_writes
        && *vram == b.vram
        && *vram_writes == b.vram_writes
        && *oam_low == b.oam_low
        && *oam_high == b.oam_high
        && *oam_writes == b.oam_writes
}

/// The pages the message engine named, for the translation: at the end of the last frame noted, and at the start
/// of the frame noted (or skipped) last; and the last page named at the end of a frame that was skipped, for the
/// frame drawn next.
#[derive(Debug, Default)]
struct TextRun {
    ended: Option<OnScreen>,
    started: Option<OnScreen>,
    /// A page written at once (the Item help) is named for two frames only: when those frames run in a batch the
    /// window never draws (a fast-forward, or the frames the clock owes at speed 1), the frame drawn next starts
    /// from no page, and the overlay would keep the page before it (or none) over the new page's English.
    missed: Option<OnScreen>,
}

impl TextRun {
    /// The frame that just ran is not drawn (yet): the page it started from is kept for the frame drawn next. Not
    /// the page it ended on: the app draws the last frame of a batch after skipping it ([`WideRun::redraw_hd`]), and
    /// that frame must not get the page its own end names a frame early.
    fn skipped(&mut self) {
        if self.started.is_some() {
            self.missed.clone_from(&self.started);
        }
    }

    /// The page to hand the frame about to be drawn: the one it started from, else the last one a skipped frame
    /// ended on. Either way nothing is kept for the frames after it.
    fn hand(&mut self) -> Option<OnScreen> {
        let missed = self.missed.take();
        self.started.clone().or(missed)
    }
}

impl WideRun {
    /// A run of the cartridge image `rom`.
    #[must_use]
    pub fn new(rom: &[u8]) -> Self {
        Self {
            hide_field_ui: false,
            rom: Arc::from(rom),
            renderer: Some(WideRenderer::new()),
            worker: None,
            drawing: false,
            worker_start: None,
            ended: None,
            skipped: None,
            text: None,
            shared: None,
            ssj_lightning: None,
            cost: None,
        }
    }

    /// What the Enhanced pictures drawn since the last call cost, added up, and whether the last one was drawn in the
    /// background (a picture drawn there is counted when it comes back); `None` when none was drawn.
    pub fn take_cost(&mut self) -> Option<(crate::DrawCost, bool)> {
        self.cost.take()
    }

    /// Counts the picture the renderer just drew, here or in the background (`worker`).
    fn note_cost(&mut self, worker: bool) {
        let Some(drawn) = self.renderer.as_ref().map(WideRenderer::draw_cost) else { return };
        let (cost, on_worker) = self.cost.get_or_insert_default();
        cost.add(&drawn);
        *on_worker = worker;
    }

    /// Draws the translation of `overlay` over the original's message windows in the Enhanced pictures (`None`:
    /// the original's text); see [`crate::translate`].
    pub fn set_overlay(&mut self, overlay: Option<Arc<Overlay>>) {
        self.text = overlay.is_some().then(TextRun::default);
        self.shared = None;
        self.renderer().set_overlay(overlay);
    }

    /// Tells the renderer the page the frame about to be drawn started from.
    fn hand_text(&mut self) {
        // The page the frame started from, else the last page a skipped frame ended on (see [`TextRun::missed`])
        let started = self.text.as_mut().and_then(TextRun::hand);
        if self.text.is_some() {
            self.renderer().set_text_screen(started);
        }
    }

    /// Looks at the picture drawn in the background: with `wait` the call waits for it. The renderer is back when the
    /// picture is `Done`.
    ///
    /// # Panics
    ///
    /// The drawing in the background panicked.
    fn settle(&mut self, wait: bool) -> Settled {
        if !self.drawing {
            return Settled::Idle;
        }
        let worker = self.worker.as_ref().expect("a picture in the background has its worker");
        let done = if wait {
            worker.done.recv().ok()
        } else {
            match worker.done.try_recv() {
                Ok(done) => Some(done),
                Err(std::sync::mpsc::TryRecvError::Empty) => return Settled::Busy,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
            }
        };
        let (renderer, picture) = done.expect("the drawing in the background panicked");
        self.drawing = false;
        self.renderer = Some(renderer);
        if picture.is_some() {
            self.note_cost(true);
        }
        Settled::Done(picture)
    }

    fn renderer(&mut self) -> &mut WideRenderer {
        // (a picture still in the background is thrown away)
        self.settle(true);
        self.renderer.as_mut().expect("the renderer is back once the drawing is done")
    }

    /// Sets what the worker thread does first, before its first picture; the app uses it to register the thread
    /// with the system's performance hints. Call it before [`WideRun::start_worker`].
    pub fn set_worker_start(&mut self, start: WorkerStart) {
        self.worker_start = Some(start);
    }

    /// Starts the worker thread now instead of at the first picture drawn in the background.
    pub fn start_worker(&mut self) {
        if self.worker.is_none() {
            self.worker = Some(Worker::start(self.worker_start.clone()));
        }
    }

    /// Whether a picture is being drawn in the background, or done and not yet taken.
    #[must_use]
    pub fn drawing(&self) -> bool {
        self.drawing
    }

    /// Waits for the picture being drawn in the background and returns it (`None`: none is, or the frame was refused).
    pub fn finish_pipelined(&mut self) -> Option<WideHd> {
        match self.settle(true) {
            Settled::Done(picture) => picture,
            Settled::Idle | Settled::Busy => None,
        }
    }

    /// A frame ran and ended with the work RAM `wram`; `log` is its render log, if the log is on
    /// and the frame was logged from its start. Returns the frame's 16:9 view ([`WIDTH_16_9`]
    /// wide), or `None` when there is no log or the frame is one the renderer refuses.
    pub fn note(&mut self, wram: &[u8], log: Option<&FrameLog>) -> Option<WideFrame> {
        let (game_mode, field, _) = self.advance(wram);
        self.skipped = None;
        let log = log?;
        let options = WideOptions { width: WIDTH_16_9, hud: HudPlacement::Window };
        let rom = Arc::clone(&self.rom);
        self.renderer().render_view(Rom(&rom), game_mode, field.as_ref(), log, options).ok()
    }

    /// [`WideRun::note`], as the Enhanced picture at the scale and look of `hd`, with the pack's
    /// art when [`WideRun::set_art`] gave one.
    pub fn note_hd(
        &mut self,
        wram: &[u8],
        log: Option<&FrameLog>,
        hd: HdOptions,
    ) -> Option<WideHd> {
        let (game_mode, field, conditions) = self.advance(wram);
        self.skipped = None;
        let log = log?;
        let options = WideOptions { width: WIDTH_16_9, hud: HudPlacement::Window };
        let rom = Arc::clone(&self.rom);
        self.hand_text();
        let renderer = self.renderer();
        renderer.set_conditions(conditions);
        let picture =
            renderer.render_hd(Rom(&rom), game_mode, field.as_ref(), log, options, hd).ok();
        if picture.is_some() {
            self.note_cost(false);
        }
        picture
    }

    /// [`WideRun::note_hd`] that shares its picture (see [`SharedHd`]). When the frame's inputs are those the picture
    /// drawn last came from (its start state, text page, HD options and render log), that picture comes back with
    /// its serial and nothing is drawn; otherwise the frame is drawn. The pictures are [`WideRun::note_hd`]'s.
    pub fn note_hd_shared(
        &mut self,
        wram: &[u8],
        log: Option<&FrameLog>,
        hd: HdOptions,
    ) -> Option<SharedHd> {
        let started = self.advance(wram);
        self.skipped = None;
        self.shared_picture(started, log?, hd)
    }

    /// [`WideRun::redraw_hd`] that shares its picture (see [`WideRun::note_hd_shared`]).
    pub fn redraw_hd_shared(&mut self, log: Option<&FrameLog>, hd: HdOptions) -> Option<SharedHd> {
        let started = self.skipped.take()?;
        self.shared_picture(started, log?, hd)
    }

    /// The picture of a frame that started as `started` with render log `log`: the picture drawn last when its
    /// inputs are the same, else a new one. A frame the renderer refuses is not kept.
    fn shared_picture(
        &mut self,
        started: Started,
        log: &FrameLog,
        hd: HdOptions,
    ) -> Option<SharedHd> {
        let text = self.text.as_ref().and_then(|text| text.started.clone());
        if let Some(drawn) = &self.shared
            && drawn.inputs.is(&started, text.as_ref(), hd, log)
        {
            return Some(drawn.shared.clone());
        }
        let (game_mode, field, conditions) = started.clone();
        let options = WideOptions { width: WIDTH_16_9, hud: HudPlacement::Window };
        let rom = Arc::clone(&self.rom);
        self.hand_text();
        let renderer = self.renderer();
        renderer.set_conditions(conditions);
        let picture =
            renderer.render_hd(Rom(&rom), game_mode, field.as_ref(), log, options, hd).ok()?;
        // (a picture that lacks art still being read is not the frame's: the next one is drawn again)
        let pending = renderer.art_pending();
        self.note_cost(false);
        let shared = SharedHd { serial: next_serial(), picture: Arc::new(picture) };
        let inputs = Inputs { started, text, hd, log: log.clone() };
        self.shared = (!pending).then(|| Drawn { inputs, shared: shared.clone() });
        Some(shared)
    }

    /// A frame ran and ended with the work RAM `wram`, and its picture is not wanted (a frame the
    /// player never sees, as the others of a fast-forward's batch): only the run's state moves on,
    /// so the next frame drawn starts from the right work RAM. Nothing is drawn. The frame can still
    /// be drawn with [`WideRun::redraw`] until another one is noted.
    pub fn skip(&mut self, wram: &[u8]) {
        self.skipped = Some(self.advance(wram));
        if let Some(text) = self.text.as_mut() {
            text.skipped();
        }
    }

    /// The 16:9 view of the frame last skipped ([`WideRun::skip`]), from its render log `log`:
    /// what [`WideRun::note`] would have returned for it. `None` when no frame was skipped since
    /// the last one noted, or there is no log or the frame is one the renderer refuses.
    pub fn redraw(&mut self, log: Option<&FrameLog>) -> Option<WideFrame> {
        let (game_mode, field, _) = self.skipped.take()?;
        let options = WideOptions { width: WIDTH_16_9, hud: HudPlacement::Window };
        let rom = Arc::clone(&self.rom);
        self.renderer().render_view(Rom(&rom), game_mode, field.as_ref(), log?, options).ok()
    }

    /// [`WideRun::redraw`] as the Enhanced picture ([`WideRun::note_hd`]).
    pub fn redraw_hd(&mut self, log: Option<&FrameLog>, hd: HdOptions) -> Option<WideHd> {
        let (game_mode, field, conditions) = self.skipped.take()?;
        let options = WideOptions { width: WIDTH_16_9, hud: HudPlacement::Window };
        let rom = Arc::clone(&self.rom);
        self.hand_text();
        let renderer = self.renderer();
        renderer.set_conditions(conditions);
        let picture =
            renderer.render_hd(Rom(&rom), game_mode, field.as_ref(), log?, options, hd).ok();
        if picture.is_some() {
            self.note_cost(false);
        }
        picture
    }

    /// [`WideRun::redraw_hd`] drawn in the background, for a run whose pictures are shown a window
    /// frame late (a fast-forward): the frame is handed to the worker and the call returns the
    /// picture of the frame handed over by the call before if it is done; if it is not (a batch of
    /// frames is shorter than the drawing, or the machine is loaded), the call returns `None` and
    /// draws nothing, so the run never waits for a picture. `None` is the first call after the run's other
    /// calls (nothing was handed over before it), a frame the renderer refuses, or no frame to
    /// draw (as for [`WideRun::redraw_hd`]; then the picture of the call before is still
    /// returned). Any other call of the run waits for the picture in the background first and
    /// throws it away.
    ///
    /// # Panics
    ///
    /// The thread cannot be started, or the drawing in the background panicked.
    pub fn redraw_hd_pipelined(&mut self, log: Option<&FrameLog>, hd: HdOptions) -> Option<WideHd> {
        self.redraw_hd_ahead(log, hd, false)
    }

    /// [`WideRun::redraw_hd_pipelined`] for a run that shows every frame, one window frame late (the game at its
    /// own speed): when the picture of the call before is not done the call waits for it, so no picture is dropped.
    /// The wait is what the drawing takes beyond the frame emulated meanwhile.
    ///
    /// # Panics
    ///
    /// As for [`WideRun::redraw_hd_pipelined`].
    pub fn redraw_hd_next(&mut self, log: Option<&FrameLog>, hd: HdOptions) -> Option<WideHd> {
        self.redraw_hd_ahead(log, hd, true)
    }

    fn redraw_hd_ahead(
        &mut self,
        log: Option<&FrameLog>,
        hd: HdOptions,
        wait: bool,
    ) -> Option<WideHd> {
        // A picture still being drawn is not waited for (unless asked): this frame is not drawn (the
        // window keeps the last picture it has, and the next frame of the run is drawn instead). The
        // skipped frame stays for the next call, which finds the worker done.
        let previous = match self.settle(wait) {
            Settled::Busy => return None,
            Settled::Done(picture) => picture,
            Settled::Idle => None,
        };
        let started = self.skipped.take().zip(log).map(|((game_mode, field, conditions), log)| {
            (game_mode, field, conditions, log.clone())
        });
        let Some((game_mode, field, conditions, log)) = started else {
            return previous;
        };
        let options = WideOptions { width: WIDTH_16_9, hud: HudPlacement::Window };
        let rom = Arc::clone(&self.rom);
        self.hand_text();
        let mut renderer =
            self.renderer.take().expect("the renderer is back once the drawing is done");
        renderer.set_conditions(conditions);
        self.start_worker();
        let job = DrawJob { renderer, rom, game_mode, field, log, options, hd };
        let worker = self.worker.as_ref().expect("the worker is started");
        worker.jobs.send(job).expect("the worker of the pictures is running");
        self.drawing = true;
        previous
    }

    /// The art the Enhanced pictures draw (`None`: none).
    pub fn set_art(&mut self, pack: Option<Arc<Pack>>) {
        self.shared = None;
        self.renderer().set_art(pack);
    }

    /// The lightning the Enhanced pictures draw over the PPU's picture while Goku's first Super Saiyan transformation
    /// goes on (`None`: none; see [`crate::lightning`]).
    pub fn set_lightning(&mut self, lightning: Option<Arc<crate::lightning::Lightning>>) {
        self.shared = None;
        self.renderer().set_lightning(lightning);
    }

    /// The frames since Goku's first Super Saiyan transformation started, while it goes on (`None`: it does not), for
    /// the frame noted next ([`WideRun::note_hd_shared`], [`WideRun::skip`]): the app's watch says it, the work RAM
    /// does not. A state loaded or a reset is `None` until the app says otherwise.
    pub fn set_ssj_lightning(&mut self, frames: Option<u32>) {
        self.ssj_lightning = frames;
    }

    /// Makes the Enhanced pictures of a scale of 2 or more draw the game's UI text from `font`.
    pub fn set_font(&mut self, font: Option<Arc<Font>>) {
        self.shared = None;
        self.renderer().set_font(font);
    }

    /// Suppresses the native field corner portraits and command window during Fly/Land transitions
    /// in the 4:3 view, and invalidates the shared-picture cache when the flag changes.
    pub fn set_hide_field_ui(&mut self, hide_field_ui: bool) {
        if self.hide_field_ui != hide_field_ui {
            self.hide_field_ui = hide_field_ui;
            self.shared = None;
            self.renderer().hide_field_ui = hide_field_ui;
        }
    }

    /// Measures the colours the art draws against the original's ([`crate::ArtStats::drawn`]).
    pub fn set_art_measure(&mut self, on: bool) {
        self.shared = None;
        self.renderer().set_art_measure(on);
    }

    /// Keeps the layers' art with its colour transfer between frames, or (off) applies each transfer as it draws
    /// ([`WideRenderer::set_transfer_cache`]).
    pub fn set_transfer_cache(&mut self, on: bool) {
        self.shared = None;
        self.renderer().set_transfer_cache(on);
    }

    /// Draws the art of the backgrounds alone ([`WideRenderer::set_art_backgrounds_only`]).
    pub fn set_art_backgrounds_only(&mut self, on: bool) {
        self.shared = None;
        self.renderer().set_art_backgrounds_only(on);
    }

    /// Reads the pack's art on a thread of its own ([`WideRenderer::set_art_async`]). Call it after
    /// [`WideRun::set_art`].
    pub fn set_art_async(&mut self, on: bool) {
        self.shared = None;
        self.renderer().set_art_async(on);
    }

    /// What the art did in the last Enhanced picture.
    #[must_use]
    pub fn art_stats(&self) -> crate::ArtStats {
        // (None while a picture is drawn in the background)
        self.renderer.as_ref().map(WideRenderer::art_stats).unwrap_or_default()
    }

    /// Counts the pixels of the pictures instead of drawing the art ([`WideRenderer::set_tally`]).
    pub fn set_tally(&mut self, on: bool) {
        self.shared = None;
        self.renderer().set_tally(on);
    }

    /// Draws the frames of a field state as the field whatever BG2 holds
    /// ([`WideRenderer::set_trust_field`]).
    pub fn set_trust_field(&mut self, trust: bool) {
        self.shared = None;
        self.renderer().set_trust_field(trust);
    }

    /// The tally of the last picture ([`WideRenderer::tally`]).
    #[must_use]
    pub fn tally(&mut self) -> crate::FrameTally {
        self.renderer().tally()
    }

    /// The portraits of the last picture ([`WideRenderer::portrait_uses`]).
    #[must_use]
    pub fn portrait_uses(&self) -> Vec<crate::PortraitUse> {
        self.renderer.as_ref().map(WideRenderer::portrait_uses).unwrap_or_default()
    }

    /// The box of the last picture ([`WideRenderer::picture_map`]).
    #[must_use]
    pub fn picture_map(&self, x: i32, y: i32, w: usize, h: usize) -> Vec<(u8, Option<u32>, u8)> {
        self.renderer.as_ref().map(|r| r.picture_map(x, y, w, h)).unwrap_or_default()
    }

    /// The background slots of the last picture ([`WideRenderer::bg_slot_report`]).
    #[must_use]
    pub fn bg_slot_report(&self) -> Vec<(u32, u8, u8, u16, u16, u8, bool)> {
        self.renderer.as_ref().map(WideRenderer::bg_slot_report).unwrap_or_default()
    }

    /// Notes the work RAM at the end of a frame: the state the frame just run started from.
    fn advance(&mut self, wram: &[u8]) -> Started {
        if let Some(text) = self.text.as_mut() {
            let hint = text.ended.as_ref().map(|screen| screen.page);
            let now = ssd_catalogue::engine::on_screen(&self.rom, wram, hint);
            text.started = std::mem::replace(&mut text.ended, now);
        }
        let rom = Rom(&self.rom);
        let current = (
            wram[GAME_MODE],
            FieldState::from_wram(wram).map(|state| state.in_room(rom)),
            Conditions::of_work_ram(wram).with_ssj_lightning(self.ssj_lightning),
        );
        self.ended.replace(current.clone()).unwrap_or(current)
    }
}

/// The scene class of a frame: the field where the game's field is on screen (the main loop in a
/// field state, the field's layers, and BG2 holding the map); a Mode 7 flight in the flight's main
/// loop states (`$06` and `$07`, not `$87`: the title shows the same plane) when a line shows the
/// plane; and a picture otherwise.
#[must_use]
pub fn classify(rom: Rom<'_>, game_mode: u8, field: Option<&FieldState>, log: &FrameLog) -> Scene {
    classify_with(rom, game_mode, field, log, false)
}

/// [`classify`], optionally taking BG2 for the map without checking it (`trust_map`).
fn classify_with(
    rom: Rom<'_>,
    game_mode: u8,
    field: Option<&FieldState>,
    log: &FrameLog,
    trust_map: bool,
) -> Scene {
    let field_mode = is_field_mode(game_mode);
    if let Some(state) = field
        && field_mode
        && check_field(log, state).is_ok()
        && (trust_map || check_tiles(rom, state, log).is_map())
    {
        return Scene::Field;
    }
    let flight_state = game_mode & 0x80 == 0 && matches!(game_mode, 0x06 | 0x07);
    if flight_state && log.entries.iter().any(|entry| is_plane(&entry.regs)) {
        Scene::Flight
    } else {
        Scene::Bordered
    }
}

/// Whether a line shows the Mode 7 plane alone (BG1 and no other BG), as a flight's lines do.
fn is_plane(regs: &LineRegs) -> bool {
    !regs.forced_blanking && regs.bg_mode == 7 && regs.main_bg == [true, false, false, false]
}

fn check_width(width: usize) -> Result<(), WideError> {
    if (WIDTH..=MAX_WIDTH).contains(&width) { Ok(()) } else { Err(WideError::BadWidth) }
}

/// Whether the game shows nothing in the frame: every line is in forced blanking or at brightness
/// 0 (the fade around a battle, with the field's map still in VRAM).
fn dark_frame(log: &FrameLog) -> bool {
    log.entries.iter().all(|entry| entry.regs.forced_blanking || entry.regs.brightness == 0)
}

/// What every frame needs to be drawn wide: a complete log, and a frame the plain renderer models.
fn check_frame(log: &FrameLog) -> Result<(), WideError> {
    if !log.complete {
        return Err(WideError::Unsupported(Unsupported::Incomplete));
    }
    if log.h_hi_res || log.v_hi_res {
        return Err(WideError::Unsupported(Unsupported::HiRes));
    }
    for entry in &log.entries {
        let regs = &entry.regs;
        if regs.interlaced || regs.pseudo_h_hi_res || regs.pseudo_obj_hi_res {
            return Err(WideError::Unsupported(Unsupported::HiRes));
        }
        if !matches!(regs.bg_mode, 0 | 1 | 7) {
            return Err(WideError::Unsupported(Unsupported::BgMode(regs.bg_mode)));
        }
    }
    Ok(())
}

/// What a frame needs to be a wide field frame. A frame in forced blanking all the way (a map
/// being loaded, with its VRAM and OAM written) is black whatever else it does.
fn check_field(log: &FrameLog, state: &FieldState) -> Result<(), WideError> {
    check_frame(log)?;
    if !state.is_field() {
        return Err(WideError::NotField(NotField::GameMode));
    }
    let mut shown = false;
    for entry in &log.entries {
        let regs = &entry.regs;
        if regs.forced_blanking {
            continue;
        }
        shown = true;
        if regs.bg_mode != 1 || regs.bg_large_tiles[1] || !(regs.main_bg[1] || regs.sub_bg[1]) {
            return Err(WideError::NotField(NotField::Layers));
        }
    }
    if shown && !log.oam_writes.is_empty() {
        return Err(WideError::NotField(NotField::OamWrites));
    }
    Ok(())
}

/// Whether a picture frame (not the field's own scene) shows the field's map: BG2 of mode 1 holds
/// the tiles the map lookup gives at the world position the scroll registers and the camera say.
/// A menu laid over the map, a scripted scene on it, or a town's room, are such frames.
fn shows_map(rom: Rom<'_>, state: &FieldState, log: &FrameLog) -> bool {
    if check_frame(log).is_err() {
        return false;
    }
    let mut shown = false;
    for entry in &log.entries {
        let regs = &entry.regs;
        if regs.forced_blanking {
            continue;
        }
        shown = true;
        if regs.bg_mode != 1 || regs.bg_large_tiles[1] || !(regs.main_bg[1] || regs.sub_bg[1]) {
            return false;
        }
    }
    shown && check_tiles(rom, state, log).is_map()
}

/// What the world layers are read from.
#[derive(Clone, Copy)]
struct World<'a> {
    rom: Rom<'a>,
    state: &'a FieldState,
}

/// How a chunk's line is laid out in the wide row.
#[derive(Clone, Copy)]
struct Layout {
    width: usize,
    /// Where the original window starts in the frame, and BG1 and BG3.
    offset: i32,
    hud_offset: i32,
    /// The first pixel of the window (its x) this entry applies to; `i32::MIN` for all.
    from: i32,
    /// Whether to count this line's pixels in the stats (the first entry of the line).
    count: bool,
    scene: Scene,
}

impl Chunk {
    #[allow(clippy::too_many_arguments)]
    fn render_entry(
        &mut self,
        world: Option<World<'_>>,
        window: Memories<'_>,
        entry: &LineEntry,
        layout: Layout,
        row: &mut [Color],
        stats: &mut WideStats,
        mut art: Option<(&mut crate::art::Rows<'_>, &Binder<'_>)>,
        mut text: Option<(&mut TextCapture, &Font)>,
    ) {
        let regs = &entry.regs;
        let vram = window.vram;
        let Layout { width, offset, hud_offset, from, count, scene } = layout;
        // A flight's line that is not the plane (a caption's text) is a picture's line: its sides
        // are black
        let scene = if scene == Scene::Flight && !is_plane(regs) { Scene::Bordered } else { scene };
        // The renderer's window positions: the chunk's first pixel is at `x0 - offset` from the
        // window's left edge
        self.renderer.origin = self.x0 - offset;
        if entry.from_pixel == 0 && entry.sprites.rebuilt {
            if scene == Scene::Field {
                // The field's objects: the chunk's copy of the OAM, with the extra parts
                let memories = Memories { vram, oam: (&self.oam.low, &self.oam.high) };
                self.renderer.build_sprite_line(&memories, &entry.sprites, 0);
            } else {
                // The window's objects, taken and counted where they lie on the screen
                self.renderer.build_sprite_line(&window, &entry.sprites, self.x0 - offset);
            }
        }
        // The wide x range this chunk is responsible for: the left chunk the first 256 pixels,
        // the right one the rest
        let (lo, hi) = if self.x0 == 0 { (0, width.min(WIDTH)) } else { (WIDTH, width) };
        let committed = |x: usize| (x as i32 - offset) >= from;

        if regs.forced_blanking {
            for (x, pixel) in row.iter_mut().enumerate().take(hi).skip(lo) {
                if committed(x) {
                    *pixel = Color::BLACK;
                }
            }
            return;
        }

        // The layers: each BG scrolled so the chunk shows the window where it sits. A BG with
        // the mosaic on is drawn flat from the line the filter reads, and its blocks are laid
        // on the window's pixels here, as the PPU lays them on the screen's.
        let mosaic = if regs.mosaic_size > 0 { regs.mosaic_bg } else { [false; 4] };
        let mut shifted = *regs;
        let world_shift = (self.x0 - offset) as u16;
        let hud_shift = (self.x0 - hud_offset) as u16;
        for (bg, scroll) in shifted.bg_h_scroll.iter_mut().enumerate() {
            *scroll = scroll.wrapping_add(if bg == 1 { world_shift } else { hud_shift });
        }
        shifted.mosaic_bg = [false; 4];
        let only = |on: [bool; 4]| {
            let mut only = shifted;
            for (bg, &on) in on.iter().enumerate() {
                only.main_bg[bg] &= on;
                only.sub_bg[bg] &= on;
            }
            only
        };
        self.renderer.render_bg_layers(vram, &only(mosaic.map(|on| !on)), entry.scanline, 0);
        if mosaic.contains(&true) {
            let line = entry.scanline.wrapping_sub(u16::from(regs.mosaic_v_offset));
            self.renderer.render_bg_layers(vram, &only(mosaic), line, 0);
            let block = i32::from(regs.mosaic_size) + 1;
            for (bg, _) in mosaic.iter().enumerate().filter(|&(_, &on)| on) {
                let flat = self.renderer.bg_pixels[bg];
                // The world's layers (BG2, and a flight's plane) are laid on the window, the screen's
                // on the HUD's place
                let world_layer = bg == 1 || scene == Scene::Flight;
                let shift = self.x0 - if world_layer { offset } else { hud_offset };
                for v in 0..WIDTH {
                    let source = (shift + v as i32).div_euclid(block) * block - shift;
                    self.renderer.bg_pixels[bg][v] = usize::try_from(source)
                        .ok()
                        .and_then(|source| flat.get(source).copied())
                        .unwrap_or(Pixel::TRANSPARENT);
                }
            }
        }
        // The screen layers (BG1 and BG3) show in the window only; a flight's BG1 is the plane,
        // which continues
        let screen_layers: &[usize] = if scene == Scene::Flight { &[2, 3] } else { &[0, 2, 3] };
        for &bg in screen_layers {
            for v in 0..WIDTH {
                let x = self.x0 + v as i32 - hud_offset;
                if !(0..WIDTH as i32).contains(&x) {
                    self.renderer.bg_pixels[bg][v] = Pixel::TRANSPARENT;
                }
            }
        }
        let outside = self.fill_map(world, vram, regs, entry.scanline, offset, mosaic[1]);

        let mut line = [Color::BLACK; WIDTH];
        self.renderer.render_scanline(regs, &mut line, 0);
        let mut black = line;
        if outside.iter().any(|&flag| flag) {
            // What shows outside the map is black, with the objects over it
            let backdrop = std::mem::replace(&mut self.renderer.cgram[0], 0);
            self.renderer.render_scanline(regs, &mut black, 0);
            self.renderer.cgram[0] = backdrop;
        }
        for (x, out) in row.iter_mut().enumerate().take(hi).skip(lo) {
            if !committed(x) {
                continue;
            }
            let v = x - self.x0 as usize;
            let window_x = x as i32 - offset;
            if scene == Scene::Bordered && !(0..WIDTH as i32).contains(&window_x) {
                // A picture has nothing beyond its window: black, whatever the objects drew there
                *out = Color::BLACK;
                stats.outside_skipped += u64::from(count);
                continue;
            }
            if outside[v] {
                *out = black[v];
                stats.outside_skipped += u64::from(count);
                continue;
            }
            // Beside a flight's window the plane goes on where it has texels; where it has none
            // (a picture on the plane, such as the speed lines of a cutscene, ends at its edge)
            // the backdrop is not drawn: the sides of a picture are black
            if scene == Scene::Flight
                && !(0..WIDTH as i32).contains(&window_x)
                && self.renderer.main.rendered[v].layer == crate::screen::Layer::Backdrop
            {
                *out = Color::BLACK;
                stats.outside_skipped += u64::from(count);
                continue;
            }
            *out = line[v];
            // A pixel outside the window is the map's: check it is a tile of the map
            if let (Some(World { state, .. }), true) = (world, count)
                && !(0..WIDTH as i32).contains(&window_x)
            {
                let (world_x, world_y) =
                    map_sample(regs, state.camera, entry.scanline, window_x, mosaic[1]);
                if !state.map.contains(world_x, world_y) {
                    stats.outside_drawn += 1;
                }
            }
        }
        // What the art passes need of the pixels this chunk drew: nothing beside a picture's window
        if let Some((art, binder)) = art.as_mut() {
            let line = usize::from(entry.scanline);
            for x in lo..hi {
                let window_x = x as i32 - offset;
                if !committed(x)
                    || (scene == Scene::Bordered && !(0..WIDTH as i32).contains(&window_x))
                {
                    continue;
                }
                let v = x - self.x0 as usize;
                let renderer = &self.renderer;
                let plan_as = |layer, palette| renderer.plan_as(regs, v, layer, palette);
                let layer_colour = |bg| renderer.bg_colour(regs, bg, v);
                let layer_index = |bg| renderer.bg_index(regs, bg, v);
                // The world pixel the map layer shows here (what the field map's art is found by)
                let world_at = world
                    .filter(|_| !outside[v])
                    .map(|w| map_sample(regs, w.state.camera, entry.scanline, window_x, mosaic[1]));
                let probe = Probe {
                    rendered: &renderer.main.rendered[v],
                    plan_as: &plan_as,
                    layer_colour: &layer_colour,
                    layer_index: &layer_index,
                    backdrop: renderer.cgram[0],
                    window_x,
                    line,
                    world: world_at,
                };
                art.put(line, x, &probe, regs, &renderer.cgram, binder);
            }
        }
        // The text cells of the pixels this chunk drew, over the same pixels
        if let Some((text, font)) = text.as_mut() {
            let line = usize::from(entry.scanline);
            let slots: &[crate::bgart::Slot] =
                art.as_ref().map_or(&[][..], |(rows, _)| rows.bg_slots());
            for x in lo..hi {
                let window_x = x as i32 - offset;
                if !committed(x)
                    || (scene == Scene::Bordered && !(0..WIDTH as i32).contains(&window_x))
                {
                    continue;
                }
                let v = x - self.x0 as usize;
                // A pixel of a background that the art draws is not text
                if let Some(bg) = crate::bg::of_layer(self.renderer.main.rendered[v].layer)
                    && crate::bgart::covers(slots, regs, bg, window_x, line)
                {
                    continue;
                }
                text.note(
                    font,
                    line - 1,
                    x as i32,
                    &self.renderer.main.rendered[v],
                    || self.renderer.plan(regs, v),
                    regs,
                    &self.renderer.cgram,
                    vram,
                );
            }
        }
    }

    /// Puts the map's pixels of the line in BG2's buffer, outside the original window (inside
    /// it the buffer is the PPU's); returns the pixels that are beyond the map (left transparent
    /// here, black in the end). Without a field there is no map and nothing is beyond it.
    fn fill_map(
        &mut self,
        world: Option<World<'_>>,
        vram: &[u16],
        regs: &LineRegs,
        scanline: u16,
        offset: i32,
        mosaic: bool,
    ) -> [bool; WIDTH] {
        let mut outside = [false; WIDTH];
        let Some(World { rom, state }) = world else { return outside };
        if !(regs.main_bg[1] || regs.sub_bg[1]) {
            return outside;
        }
        let char_base = regs.bg_char_base[1];
        let mut cell = (i32::MIN, 0_u16);
        for (v, beyond) in outside.iter_mut().enumerate() {
            let window_x = self.x0 + v as i32 - offset;
            if (0..WIDTH as i32).contains(&window_x) {
                // The window keeps the PPU's BG2, which is the Classic picture (the game's own
                // tile map: it holds stale rows below the 224 lines it was made for)
                continue;
            }
            let (world_x, world_y) = map_sample(regs, state.camera, scanline, window_x, mosaic);
            if !state.map.contains(world_x, world_y) {
                self.renderer.bg_pixels[1][v] = Pixel::TRANSPARENT;
                *beyond = true;
                continue;
            }
            if cell.0 != world_x >> 3 {
                cell = (world_x >> 3, state.map.tile_word(rom, world_x, world_y));
            }
            self.renderer.bg_pixels[1][v] =
                map_pixel(vram, char_base, cell.1, (world_x & 7) as usize, (world_y & 7) as usize);
        }
        outside
    }
}

/// A pixel of a 4-bit BG tile (mode 1's BG2): tile map word `entry`, pixel `(x, y)` of the tile.
fn map_pixel(vram: &[u16], char_base: u16, entry: u16, x: usize, y: usize) -> Pixel {
    let tile = entry & 0x3FF;
    let x = if entry & 0x4000 != 0 { 7 - x } else { x };
    let row = if entry & 0x8000 != 0 { 7 - y } else { y };
    let address = usize::from(char_base.wrapping_add(tile.wrapping_mul(16)) & 0x7FFF);
    let bit = 7 - x;
    let mut color = 0_u8;
    for plane in (0..4).step_by(2) {
        let word = vram[(address + row + 4 * plane) & 0x7FFF];
        color |= u8::from((word >> bit) & 1 != 0) << plane;
        color |= u8::from((word >> (bit + 8)) & 1 != 0) << (plane + 1);
    }
    Pixel {
        palette: ((entry >> 10) & 7) as u8,
        color,
        priority: u8::from(entry & 0x2000 != 0),
        char: address as u16,
        pos: crate::pos(x as u8, row as u8, entry & 0x4000 != 0, entry & 0x8000 != 0),
        oam: crate::NO_OBJECT,
    }
}

/// The OAM of a chunk: every entry of the frame's moved by `shift` pixels (one beyond what an
/// entry's 9 bits hold is hidden), and the extra parts in their entries ([`place_extras`]), those
/// that touch the chunk.
fn chunk_oam(low: &[u8], high: &[u8], shift: i32, extras: &[Extra]) -> Oam {
    let mut oam = Oam { low: low.to_vec(), high: high.to_vec() };
    for index in 0..128_usize {
        let bits = oam.high[index >> 2] >> (2 * (index & 3));
        let x = i32::from(oam.low[index * 4]) | (i32::from(bits & 1) << 8);
        let x = if x >= 256 { x - 512 } else { x } + shift;
        if oam.low[index * 4 + 1] >= 0xF0 {
            continue;
        }
        if (-256..256).contains(&x) {
            set_x(&mut oam, index, x);
        } else {
            oam.low[index * 4 + 1] = 0xF0;
        }
    }
    for extra in extras {
        let Some(slot) = extra.slot else { continue };
        let index = usize::from(slot);
        let part = &extra.part;
        let x = part.x + shift;
        let size = if part.large { 16 } else { 8 };
        if x + size <= 0 || x >= WIDTH as i32 {
            continue;
        }
        oam.low[index * 4 + 1] = part.y;
        oam.low[index * 4 + 2] = part.tile;
        oam.low[index * 4 + 3] = part.attributes;
        set_x(&mut oam, index, x);
        let shift = 2 * (index & 3);
        let bits = &mut oam.high[index >> 2];
        *bits = (*bits & !(2 << shift)) | (u8::from(part.large) << (shift + 1));
    }
    oam
}

/// Sets entry `index`'s x (9 bits, signed) and leaves its size bit.
fn set_x(oam: &mut Oam, index: usize, x: i32) {
    let x = (x & 0x1FF) as u16;
    oam.low[index * 4] = x as u8;
    let shift = 2 * (index & 3);
    let bits = &mut oam.high[index >> 2];
    *bits = (*bits & !(1 << shift)) | (((x >> 8) as u8) << shift);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A log of 224 lines with no registers set and empty memories.
    pub(super) fn empty_log() -> FrameLog {
        FrameLog {
            complete: true,
            height: 224,
            h_hi_res: false,
            v_hi_res: false,
            entries: Vec::new(),
            cgram_start: vec![0; 256],
            cgram_writes: Vec::new(),
            vram: vec![0; 32768],
            vram_writes: Vec::new(),
            oam_low: vec![0; 512],
            oam_high: vec![0; 32],
            oam_writes: Vec::new(),
        }
    }

    #[test]
    fn a_frame_the_game_fades_to_black_is_dark() {
        let mut log = empty_log();
        assert!(dark_frame(&log));
        let lit = LineRegs { brightness: 15, ..LineRegs::default() };
        let blanked = LineRegs { brightness: 15, forced_blanking: true, ..LineRegs::default() };
        let faded = LineRegs { brightness: 0, ..LineRegs::default() };
        let entry = |regs| LineEntry { scanline: 1, regs, ..LineEntry::default() };
        log.entries = vec![entry(blanked), entry(faded)];
        assert!(dark_frame(&log));
        log.entries.push(entry(lit));
        assert!(!dark_frame(&log));
    }
    #[test]
    fn the_window_sits_in_the_middle_of_a_wide_view() {
        // Mid-map: 58 pixels either side at 372, 116 at 488, none at 256
        assert_eq!(place_window(3072, 1000, 372), (942, 58));
        assert_eq!(place_window(3072, 1000, 488), (884, 116));
        assert_eq!(place_window(3072, 1000, 256), (1000, 0));
    }

    #[test]
    fn near_a_map_edge_the_view_stops_and_the_window_moves() {
        // 20 pixels from the left edge: the view starts at 0 and the window is 20 in
        assert_eq!(place_window(3072, 20, 372), (0, 20));
        assert_eq!(place_window(3072, 0, 372), (0, 0));
        // Window flush with the right end: 372 wide ends at 3072, the window's right edge too
        assert_eq!(place_window(3072, 3072 - 256, 372), (3072 - 372, 116));
    }

    #[test]
    fn a_map_narrower_than_the_view_is_centred() {
        assert_eq!(place_window(256, 0, 372), (-58, 58));
        assert_eq!(place_window(256, 0, 488), (-116, 116));
        assert_eq!(place_window(256, 0, 256), (0, 0));
        // An odd margin rounds the left side down
        assert_eq!(place_window(256, 0, 373), (-59, 58 + 1));
    }

    #[test]
    fn the_scroll_registers_give_the_window_origin_near_the_camera() {
        let mut regs = LineRegs::default();
        regs.bg_h_scroll[1] = 0x0123;
        regs.bg_v_scroll[1] = 0x0040;
        // The camera at the scroll, one frame behind it, and across the 512-pixel ring
        assert_eq!(window_origin(&regs, (0x0123, 0x0040)), (0x123, 0x40));
        assert_eq!(window_origin(&regs, (0x011F, 0x003D)), (0x123, 0x40));
        assert_eq!(window_origin(&regs, (0x0323, 0x0140)), (0x323, 0x140));
        regs.bg_h_scroll[1] = 0x3FF;
        assert_eq!(window_origin(&regs, (0, 0)), (-1, 0x40));
    }

    #[test]
    fn an_entry_moved_into_a_chunk_keeps_its_size_and_y() {
        let mut log = empty_log();
        // Entry 3: x = 10, y = 20, large; entry 4: x = -5 (9 bits 507); the rest hidden
        for index in 0..128 {
            log.oam_low[index * 4 + 1] = 0xF0;
        }
        log.oam_low[12..16].copy_from_slice(&[10, 20, 7, 0x21]);
        log.oam_high[0] |= 0b10 << 6;
        log.oam_low[16..20].copy_from_slice(&[251, 30, 8, 0x21]);
        log.oam_high[1] |= 0b01;
        let oam = chunk_oam(&log.oam_low, &log.oam_high, 58, &[]);
        assert_eq!(&oam.low[12..16], &[68, 20, 7, 0x21]);
        assert_eq!(oam.high[0] >> 6, 0b10, "large, x bit 8 clear");
        assert_eq!(&oam.low[16..20], &[53, 30, 8, 0x21]);
        assert_eq!(oam.high[1] & 0b11, 0b00, "-5 + 58 = 53 is positive");
        // Moved left of the chunk: -5 - 300 does not fit 9 bits and the entry is hidden
        let oam = chunk_oam(&log.oam_low, &log.oam_high, -300, &[]);
        assert_eq!(oam.low[17], 0xF0, "-5 - 300 does not fit nine bits");
        assert_eq!(oam.low[13], 0xF0, "nor does 10 - 300");
    }

    #[test]
    fn extra_parts_take_free_entries_and_overflow_is_counted() {
        let mut log = empty_log();
        for index in 0..128 {
            log.oam_low[index * 4 + 1] = 0xF0;
        }
        // 127 entries are used; one is free
        for index in 0..127 {
            log.oam_low[index * 4 + 1] = 40;
        }
        let part = |x| ObjectPart {
            part: Part { x, y: 90, tile: 5, attributes: 0x21, large: true },
            in_original: false,
        };
        // The window at 58 of a frame of 372: a part is in the frame from -74 to 313
        let all = [part(256), part(250), part(260), part(-20), part(-10), part(400), part(-90)];
        let (extras, dropped) = place_extras(&all, &log.oam_low, &log.oam_high, 58, 372);
        // 400 and -90 are off the frame: no entry. The first part takes the free entry (the same
        // in every chunk); the next four in the frame find none
        let slots: Vec<Option<u8>> = extras.iter().map(|extra| extra.slot).collect();
        assert_eq!(slots, [Some(127), None, None, None, None, None, None]);
        assert_eq!(dropped, 4);
        // Beyond the right chunk's pixels (214 + 58 -> 256): not in the left chunk, in the right
        let left = chunk_oam(&log.oam_low, &log.oam_high, 58, &extras);
        let right = chunk_oam(&log.oam_low, &log.oam_high, 58 - 116, &extras);
        assert_eq!(left.low[127 * 4 + 1], 0xF0, "256 + 58 is beyond the left chunk");
        assert_eq!(&right.low[127 * 4..127 * 4 + 4], &[198, 90, 5, 0x21]);
        assert_eq!(right.high[31] >> 6, 0b10);
    }

    #[test]
    fn a_hidden_entry_of_the_original_is_not_taken_by_one_chunk_alone() {
        // The entries the chunks hide (out of range) are not free for the extras: the same
        // entries serve every chunk, so the art finds one OAM
        let mut log = empty_log();
        for index in 0..128 {
            log.oam_low[index * 4 + 1] = 0xF0;
        }
        log.oam_low[0..4].copy_from_slice(&[200, 40, 1, 0x21]);
        let part = ObjectPart {
            part: Part { x: -30, y: 90, tile: 5, attributes: 0x21, large: false },
            in_original: false,
        };
        let (extras, dropped) = place_extras(&[part], &log.oam_low, &log.oam_high, 58, 372);
        assert_eq!((extras[0].slot, dropped), (Some(1), 0));
    }

    #[test]
    fn the_parts_of_an_object_the_original_drops_follow_the_entry_of_their_object() {
        let at = |x: i32, in_original| ObjectPart {
            part: Part { x, y: 60, tile: 3, attributes: 0x09, large: false },
            in_original,
        };
        // Two objects: the first is culled (both parts dropped), the second has a part in the
        // OAM (entries 5 and 6 hold its parts) and one past x = 255
        let all = [at(-20, false), at(-12, false), at(250, true), at(252, true), at(260, false)];
        let mut log = empty_log();
        for (n, object) in all.iter().filter(|object| object.in_original).enumerate() {
            let (bytes, bits) = object.part.oam_entry();
            log.oam_low[(5 + n) * 4..(5 + n) * 4 + 4].copy_from_slice(&bytes);
            log.oam_high[(5 + n) >> 2] |= (bits & 3) << (2 * ((5 + n) & 3));
        }
        let anchors = drop_anchors(&all, &log.oam_low, &log.oam_high);
        // The culled object's parts come before entry 5; the last follows entry 6
        assert_eq!(anchors, [Some(4), Some(4), None, None, Some(6)]);
        // The block is not in the OAM (a frame of a map load): the order is not known
        let none = drop_anchors(&all, &empty_log().oam_low, &empty_log().oam_high);
        assert_eq!(none, [None; 5]);
    }

    // A field in miniature: a map of 2 x 1 blocks (the synthetic ROM of `map`), BG2 as the
    // game keeps it (the whole map fits the 64 x 32 ring), objects in the OAM.

    use super::super::map::tests::{put, synthetic_rom};
    use crate::Renderer;
    use ssd_oracle::SpriteLine;

    const CAMERA: (u16, u16) = (200, 8);
    const BG2_MAP: usize = 0x6000;
    const BG2_CHARS: u16 = 0x2000;

    fn noise(seed: u32) -> u32 {
        let mut x = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
        x ^= x >> 15;
        x = x.wrapping_mul(0x2C1B_3C6D);
        x ^ (x >> 12)
    }

    /// The ROM: the synthetic map's tables, and the metasprite lists: type 1 is one 8 x 8 part
    /// of tile 3 at the origin.
    fn rom() -> Vec<u8> {
        let mut rom = synthetic_rom();
        put(&mut rom, 2, 0x802E + 2, &0x9000_u16.to_le_bytes());
        put(&mut rom, 2, 0x9000, &[0, 0, 3, 0x02, 0, 0x80]);
        rom
    }

    /// The field variables: the map `blocks` wide, the camera, and map objects at the given
    /// positions relative to the camera.
    fn state(blocks: u8, camera: (u16, u16), objects: &[(i32, i32)]) -> FieldState {
        let objects: Vec<(u8, i32, i32)> = objects.iter().map(|&(x, y)| (1, x, y)).collect();
        state_of(blocks, camera, &objects)
    }

    /// [`state`] with the type of each object.
    fn state_of(blocks: u8, camera: (u16, u16), objects: &[(u8, i32, i32)]) -> FieldState {
        let mut wram = vec![0; 0x20000];
        wram[0x24] = 0x06;
        wram[0x702] = 1;
        wram[0x703] = blocks;
        wram[0x704..0x707].copy_from_slice(&[0x00, 0x80, 0x04]);
        wram[0x707..0x70A].copy_from_slice(&[0x00, 0x80, 0x03]);
        wram[0x33..0x35].copy_from_slice(&camera.0.to_le_bytes());
        wram[0x35..0x37].copy_from_slice(&camera.1.to_le_bytes());
        for (index, &(kind, x, y)) in objects.iter().enumerate() {
            let at = 0x1000 + index * 6;
            wram[at] = 0x80 | kind;
            wram[at + 2..at + 4].copy_from_slice(&((x + i32::from(camera.0)) as u16).to_le_bytes());
            wram[at + 4..at + 6].copy_from_slice(&((y + i32::from(camera.1)) as u16).to_le_bytes());
        }
        FieldState::from_wram(&wram).unwrap()
    }

    /// A render log of the field for `state`: noise for the characters and the palette, BG2's
    /// tile map from the lookup, and the given sprites (x, y, tile, large).
    fn field_log(rom: &[u8], state: &FieldState, sprites: &[(i32, u8, u8, bool)]) -> FrameLog {
        let mut log = empty_log();
        for (index, word) in log.vram.iter_mut().enumerate() {
            *word = noise(index as u32) as u16;
        }
        for (index, color) in log.cgram_start.iter_mut().enumerate() {
            *color = (noise(1000 + index as u32) & 0x7FFF) as u16;
        }
        let (width, height) = state.map.size();
        for column in 0..64_i32 {
            for row in 0..32_i32 {
                let word = state.map.tile_word(Rom(rom), column * 8 % width, row * 8 % height);
                let at = BG2_MAP
                    + (column as usize >> 5) * 0x400
                    + row as usize * 32
                    + (column as usize & 31);
                log.vram[at] = word;
            }
        }
        for index in 0..128 {
            log.oam_low[index * 4 + 1] = 0xF0;
        }
        for (index, &(x, y, tile, large)) in sprites.iter().enumerate() {
            log.oam_low[index * 4..index * 4 + 4].copy_from_slice(&[
                (x & 0xFF) as u8,
                y,
                tile,
                0x21,
            ]);
            let bits = (((x >> 8) & 1) as u8) | (u8::from(large) << 1);
            log.oam_high[index >> 2] |= bits << (2 * (index & 3));
        }
        log.height = 224;
        let regs = LineRegs {
            brightness: 15,
            bg_mode: 1,
            main_bg: [false, true, false, false],
            main_obj: true,
            bg_map_base: [0, BG2_MAP as u16, 0, 0],
            bg_char_base: [0, BG2_CHARS, 0, 0],
            bg_screen_size: [0, 1, 0, 0],
            bg_h_scroll: [0, state.camera.0, 0, 0],
            bg_v_scroll: [0, state.camera.1, 0, 0],
            color_math: 3,
            ..LineRegs::default()
        };
        log.entries = (1..=224)
            .map(|scanline| LineEntry {
                scanline,
                from_pixel: 0,
                regs,
                sprites: SpriteLine {
                    eval_line: scanline - 1,
                    rebuilt: true,
                    ..SpriteLine::default()
                },
            })
            .collect();
        log
    }

    /// Makes the object tiles (the first 0x2000 words of VRAM) opaque in their top 8 rows.
    fn opaque_objects(log: &mut FrameLog) {
        for (index, word) in log.vram.iter_mut().enumerate().take(0x2000) {
            *word = if index % 16 < 8 { 0x00FF } else { 0 };
        }
    }

    fn wide(rom: &[u8], state: &FieldState, log: &FrameLog, width: usize) -> WideFrame {
        let options = WideOptions { width, hud: HudPlacement::Window };
        WideRenderer::new().render(Rom(rom), state, log, options).unwrap()
    }

    fn row_slice(frame: &Frame, y: usize, x: usize, length: usize) -> &[Color] {
        &frame.pixels[y * frame.width + x..y * frame.width + x + length]
    }

    #[test]
    fn at_width_256_the_wide_frame_is_the_classic_frame() {
        let rom = rom();
        let state = state(2, CAMERA, &[]);
        let log =
            field_log(&rom, &state, &[(10, 40, 5, false), (250, 90, 6, true), (507, 120, 7, true)]);
        let classic = Renderer::new().render(&log).unwrap();
        let wide = wide(&rom, &state, &log, 256);
        assert_eq!(wide.frame, classic);
        assert_eq!(wide.logical, Rect { x: 0, y: 0, width: 256, height: 224 });
        assert_eq!(wide.stats, WideStats::default());
    }

    /// A pack with the art of the synthetic map (2 x 1 blocks): every art pixel one colour, the
    /// source picture `source` (RGBA, 512 x 256).
    fn map_art_pack(dir: &std::path::Path, source: &[u8], art: [u8; 4]) -> Arc<Pack> {
        use crate::art_index::ArtIndex;
        use crate::mapart;
        let maps = dir.join("art").join("maps");
        std::fs::create_dir_all(&maps).unwrap();
        crate::png::write_rgba(&maps.join("source.png"), 512, 256, source).unwrap();
        let side = (mapart::TILE * mapart::SCALE) as u32;
        let tile: Vec<u8> = art.iter().copied().cycle().take((side * side * 4) as usize).collect();
        for x in 0..2 {
            crate::png::write_rgba(&maps.join(format!("t{x}_0.png")), side, side, &tile).unwrap();
        }
        let index = crate::pack::Index::new(Vec::new());
        Arc::new(Pack {
            dir: dir.to_path_buf(),
            manifest: crate::pack::Manifest {
                format: crate::pack::FORMAT,
                scale: 4,
                converter: 1,
                rom_crc32: "00000000".to_owned(),
                assets: 0,
            },
            index,
            art: ArtIndex::new(Vec::new(), Vec::new()),
            maps: mapart::Index {
                layer: vec![mapart::Entry {
                    id: "map-00".to_owned(),
                    maps: vec![0],
                    blocks: [2, 1],
                    grid: "04:8000".to_owned(),
                    metatiles: "03:8000".to_owned(),
                    source: "source.png".to_owned(),
                    palette: String::new(),
                    palette_variant: false,
                    tiles: "t{x}_{y}.png".to_owned(),
                }],
            },
            bg: crate::bgart::Index::default(),
            mode7: crate::mode7art::Index::default(),
            variants: Vec::new(),
            pose_art: Vec::new(),
        })
    }

    /// A pack whose only art is `objects`, each a solid blue picture of its box (art pixels at
    /// the scale of 4).
    fn sprite_art_pack(dir: &std::path::Path, objects: Vec<crate::art_index::Object>) -> Arc<Pack> {
        let art = dir.join("art");
        std::fs::create_dir_all(&art).unwrap();
        for object in &objects {
            let (w, h) = (u32::from(object.width) * 4, u32::from(object.height) * 4);
            let blue: Vec<u8> =
                [0, 0, 255, 255].into_iter().cycle().take((w * h * 4) as usize).collect();
            crate::png::write_rgba(&dir.join(&object.art), w, h, &blue).unwrap();
        }
        Arc::new(Pack {
            dir: dir.to_path_buf(),
            manifest: crate::pack::Manifest {
                format: crate::pack::FORMAT,
                scale: 4,
                converter: 1,
                rom_crc32: "00000000".to_owned(),
                assets: 0,
            },
            index: crate::pack::Index::new(Vec::new()),
            art: crate::art_index::ArtIndex::new(Vec::new(), objects),
            maps: crate::mapart::Index::default(),
            bg: crate::bgart::Index::default(),
            mode7: crate::mode7art::Index::default(),
            variants: Vec::new(),
            pose_art: Vec::new(),
        })
    }

    /// The field group of a pose: its key is the same wherever the object is.
    fn group_art(key: u64) -> crate::art_index::Object {
        crate::art_index::Object {
            key,
            id: "field-test-000".to_owned(),
            luma: false,
            palette: crate::art_index::Palette::default(),
            mask: Vec::new(),
            width: 16,
            height: 8,
            art: "art/o.png".to_owned(),
        }
    }

    #[test]
    fn an_object_in_the_margins_is_drawn_from_the_art_as_one_in_the_window_is() {
        let mut rom = rom();
        // Type 2: two 8 x 8 parts side by side, palette 4 (a field group by its palette)
        put(&mut rom, 2, 0x802E + 4, &0x9100_u16.to_le_bytes());
        put(&mut rom, 2, 0x9100, &[0, 0, 3, 0x08, 0, 0, 3, 0x08, 8, 0x80]);
        // Left margin, window, right margin, and one across the window's right edge (its second
        // part is past x = 255, which the original drops)
        let state = state_of(2, CAMERA, &[(2, -40, 60), (2, 100, 60), (2, 270, 60), (2, 252, 60)]);
        let mut log = field_log(&rom, &state, &[]);
        opaque_objects(&mut log);
        // BG2 is transparent: its characters are empty
        log.vram[usize::from(BG2_CHARS)..BG2_MAP].fill(0);
        // The OAM as the original writes the parts it keeps
        let kept = state.original_parts(Rom(&rom));
        assert_eq!(kept.len(), 3, "the window's object and the first part of the one at the edge");
        for (n, part) in kept.iter().enumerate() {
            let (bytes, bits) = part.oam_entry();
            log.oam_low[n * 4..n * 4 + 4].copy_from_slice(&bytes);
            log.oam_high[n >> 2] |= (bits & 3) << (2 * (n & 3));
        }
        log.cgram_start[0x80 + 4 * 16 + 1] = 0x7FFF;
        let options = WideOptions { width: 372, hud: HudPlacement::Window };
        let hd = HdOptions { scale: 2, look: crate::Look::Crisp };
        let rom = rom;
        let draw = |pack: Arc<Pack>| {
            let mut renderer = WideRenderer::new();
            renderer.set_art(Some(pack));
            let drawn = renderer
                .render_hd(Rom(&rom), 0x06, Some(&state), &log, options, hd)
                .expect("a field frame");
            (drawn, renderer.art_stats())
        };
        // Without art for them: the keys of the groups
        let dir = tempfile::tempdir().unwrap();
        let (plain, stats) = draw(sprite_art_pack(dir.path(), vec![group_art(1)]));
        assert_eq!(plain.scene, Scene::Field);
        let groups: Vec<_> = stats.poses.iter().filter(|pose| pose.field).collect();
        assert_eq!(groups.len(), 4, "one group for each object: {groups:?}");
        assert!(groups.iter().all(|pose| pose.size == (16, 8) && pose.key == groups[0].key));
        assert!(groups.iter().all(|pose| !pose.matched));
        assert_eq!(stats.sprites_margin.art, 0);
        // With the art of that group: every one of them is drawn from it
        let key = groups[0].key;
        let dir = tempfile::tempdir().unwrap();
        let (drawn, stats) = draw(sprite_art_pack(dir.path(), vec![group_art(key)]));
        assert_eq!(stats.poses.iter().filter(|pose| pose.field && pose.matched).count(), 4);
        assert!(stats.sprites_margin.total > 0 && stats.sprites_window.total > 0);
        assert_eq!(stats.sprites_margin.art, stats.sprites_margin.total);
        assert_eq!(stats.sprites_window.art, stats.sprites_window.total);
        let offset = drawn.logical.x;
        // The art is blue; the game's colour math tints it, the original's white sprite is not
        let bluish = |picture: &crate::HdFrame, x: i32, y: i32| {
            let at = (y as usize * picture.width as usize + x as usize) * 4;
            picture.rgba[at + 2] > picture.rgba[at].saturating_add(80)
        };
        let blue = |x: i32, y: i32| bluish(&drawn.picture, x, y);
        // The middle of each object's first part, and of the edge object's two parts: the
        // original's last pixel at x = 255 and the first one the original drops
        for x in [-40, 100, 270, 252, 260] {
            assert!(blue((offset + x + 4) * 2, (60 + 4) * 2), "x = {x}, offset {offset}");
        }
        assert!(blue((offset + 255) * 2, 64 * 2) && blue((offset + 256) * 2, 64 * 2));
        // The same frame with art for another group: the objects keep the game's own look
        let (look, _) = draw(sprite_art_pack(dir.path(), vec![group_art(key ^ 1)]));
        assert!(!bluish(&look.picture, (offset - 40 + 4) * 2, (60 + 4) * 2));
        assert!(!bluish(&look.picture, (offset + 100 + 4) * 2, (60 + 4) * 2));
    }

    #[test]
    fn the_map_art_is_drawn_where_the_map_shows_by_the_world_position() {
        let rom = rom();
        let state = state(2, CAMERA, &[]);
        let log = field_log(&rom, &state, &[]);
        let hd = HdOptions { scale: 2, look: crate::Look::Crisp };
        let options = WideOptions { width: 372, hud: HudPlacement::Window };
        let plain = {
            let mut renderer = WideRenderer::new();
            renderer.render_hd(Rom(&rom), 0x06, Some(&state), &log, options, hd).unwrap()
        };
        // A source with nothing opaque: the art is drawn as it is, whatever colours the game shows
        let source = vec![0_u8; 512 * 256 * 4];
        let dir = tempfile::tempdir().unwrap();
        let pack = map_art_pack(dir.path(), &source, [10, 200, 30, 255]);
        let mut renderer = WideRenderer::new();
        renderer.set_art(Some(pack));
        let art = renderer.render_hd(Rom(&rom), 0x06, Some(&state), &log, options, hd).unwrap();
        assert_eq!(
            (art.picture.width, art.picture.height),
            (plain.picture.width, plain.picture.height)
        );
        let (width, height) = (art.picture.width as usize, art.picture.height as usize);
        let mut green = 0_usize;
        let mut same = 0_usize;
        for index in 0..width * height {
            let (a, p) = (
                &art.picture.rgba[index * 4..index * 4 + 3],
                &plain.picture.rgba[index * 4..index * 4 + 3],
            );
            if a == [10, 200, 30] {
                green += 1;
            } else if a == p {
                same += 1;
            }
        }
        // The map's pixels (the whole 256 x 224 window and the columns of the map beside it) are
        // the art; what is left is black beyond the map's left edge
        assert!(green > width * height / 2, "{green} of {} pixels are art", width * height);
        assert_eq!(green + same, width * height, "every other pixel is the plain picture's");
    }

    #[test]
    fn a_picture_that_shows_the_map_has_its_art_in_the_window_and_black_beside_it() {
        let rom = rom();
        let state = state(2, CAMERA, &[]);
        let log = field_log(&rom, &state, &[]);
        let hd = HdOptions { scale: 2, look: crate::Look::Crisp };
        let options = WideOptions { width: 372, hud: HudPlacement::Window };
        let source = vec![0_u8; 512 * 256 * 4];
        let dir = tempfile::tempdir().unwrap();
        let pack = map_art_pack(dir.path(), &source, [10, 200, 30, 255]);
        let mut renderer = WideRenderer::new();
        renderer.set_art(Some(pack));
        // A menu laid over the map: the main loop is not in a field state
        let art = renderer.render_hd(Rom(&rom), 0x03, Some(&state), &log, options, hd).unwrap();
        assert_eq!(art.scene, Scene::Bordered);
        let (width, height) = (art.picture.width as usize, art.picture.height as usize);
        let logical = art.picture.logical;
        let at =
            |x: usize, y: usize| &art.picture.rgba[(y * width + x) * 4..(y * width + x) * 4 + 3];
        let (x, y) = (logical.x as usize + 20, 20);
        assert_eq!(at(x, y), [10, 200, 30], "the map's art shows in the window");
        assert_eq!(at(2, y), [0, 0, 0], "a picture has nothing beside its window");
        assert!(height > y);
    }

    #[test]
    fn a_wider_view_keeps_the_window_and_continues_the_map() {
        let rom = rom();
        // The window is 200..456 of a 512-pixel map: at 372 the view stops at the map's end
        let state = state(2, CAMERA, &[]);
        let mut log = field_log(&rom, &state, &[(10, 40, 5, false), (250, 90, 6, true)]);
        let classic = Renderer::new().render(&log).unwrap();
        let wide_frame = wide(&rom, &state, &log, 372);
        assert_eq!(wide_frame.view_left, 140, "512 - 372");
        assert_eq!(wide_frame.logical.x, 60);
        for y in 0..224 {
            assert_eq!(
                row_slice(&wide_frame.frame, y, 60, 256),
                row_slice(&classic, y, 0, 256),
                "row {y}"
            );
        }
        assert_eq!(wide_frame.stats.outside_drawn, 0);
        // The sides, without objects, are what the PPU draws when the BG scrolls there: views
        // of 256 pixels from the view's left end and from its right end
        for entry in &mut log.entries {
            entry.regs.main_obj = false;
        }
        let wide_frame = wide(&rom, &state, &log, 372);
        for (x, scroll) in [(0, 140), (116, 256)] {
            let mut side = log.clone();
            for entry in &mut side.entries {
                entry.regs.bg_h_scroll[1] = scroll;
            }
            let expected = Renderer::new().render(&side).unwrap();
            for y in 0..224 {
                assert_eq!(
                    row_slice(&wide_frame.frame, y, x, 256),
                    row_slice(&expected, y, 0, 256),
                    "row {y}, view from {x}"
                );
            }
        }
    }

    #[test]
    fn a_sprite_cut_by_the_left_edge_continues_into_the_margin() {
        let rom = rom();
        let state = state(2, CAMERA, &[]);
        let mut log = field_log(&rom, &state, &[(-5, 40, 5, false)]);
        opaque_objects(&mut log);
        // BG2 is transparent: its characters are empty
        log.vram[usize::from(BG2_CHARS)..BG2_MAP].fill(0);
        log.cgram_start[0] = 0;
        log.cgram_start[0x81] = 0x7FFF;
        let wide_frame = wide(&rom, &state, &log, 372);
        let offset = wide_frame.logical.x as usize;
        // Tile 5 is opaque in its top rows, and 8 wide: x from -5 to 2
        let red = |x: usize| wide_frame.frame.pixels[40 * 372 + x].r;
        assert_eq!(red(offset - 6), 0);
        assert_eq!(red(offset - 5), 255);
        assert_eq!(red(offset + 2), 255);
        assert_eq!(red(offset + 3), 0);
        let classic = Renderer::new().render(&log).unwrap();
        assert_eq!(classic.pixels[40 * 256 + 2].r, 255);
        assert_eq!(classic.pixels[40 * 256 + 3].r, 0);
    }

    #[test]
    fn extra_objects_show_in_the_margins_and_under_them_only_the_window_changes() {
        let rom = rom();
        // Origins at x = 266 and -12 (beyond the right and left edge) and at -4, whose part
        // reaches 4 pixels into the window
        let state = state(2, CAMERA, &[(266, 60), (-12, 80), (-4, 100)]);
        let mut log = field_log(&rom, &state, &[]);
        opaque_objects(&mut log);
        // BG2 is transparent: its characters are empty
        log.vram[usize::from(BG2_CHARS)..BG2_MAP].fill(0);
        log.cgram_start[0] = 0;
        // Tile 3 of the second name table: palette 0 of the objects, colour 1
        log.cgram_start[0x80 + 1] = 0x7FFF;
        let classic = Renderer::new().render(&log).unwrap();
        let wide_frame = wide(&rom, &state, &log, 372);
        assert_eq!(wide_frame.stats.extra_parts, 3);
        assert_eq!(wide_frame.extra_boxes.len(), 3);
        let offset = wide_frame.logical.x as usize;
        let mut differ = 0;
        for y in 0..224 {
            for x in 0..256 {
                if wide_frame.frame.pixels[y * 372 + offset + x] != classic.pixels[y * 256 + x] {
                    assert!(wide_frame.under_extra((offset + x) as i32, y as i32), "({x}, {y})");
                    differ += 1;
                }
            }
        }
        assert_eq!(differ, 4 * 8, "the part at -4 shows 4 pixels of 8 rows");
        let lit = |x: usize, y: usize| wide_frame.frame.pixels[y * 372 + x] != Color::BLACK;
        assert!(lit(offset + 266, 60) && lit(offset + 273, 67) && !lit(offset + 274, 67));
        assert!(lit(offset - 12, 80) && lit(offset - 5, 87) && !lit(offset - 13, 80));
        assert!(lit(offset, 100) && lit(offset + 3, 107) && !lit(offset + 4, 107));
        assert_eq!(wide_frame.stats.outside_drawn, 0);
    }

    #[test]
    fn a_single_block_map_is_centred_and_its_sides_are_black() {
        let rom = rom();
        let state = state(1, (0, 8), &[]);
        let log = field_log(&rom, &state, &[(10, 40, 5, false)]);
        let classic = Renderer::new().render(&log).unwrap();
        for width in [372, 488] {
            let wide_frame = wide(&rom, &state, &log, width);
            let margin = (width - 256) / 2;
            assert_eq!(wide_frame.logical.x as usize, margin);
            assert_eq!(wide_frame.view_left, -(margin as i32));
            for y in 0..224 {
                assert_eq!(
                    row_slice(&wide_frame.frame, y, margin, 256),
                    row_slice(&classic, y, 0, 256)
                );
                assert!(
                    row_slice(&wide_frame.frame, y, 0, margin).iter().all(|&c| c == Color::BLACK)
                );
                assert!(
                    row_slice(&wide_frame.frame, y, margin + 256, margin)
                        .iter()
                        .all(|&c| c == Color::BLACK)
                );
            }
            assert_eq!(wide_frame.stats.outside_drawn, 0);
            assert_eq!(wide_frame.stats.outside_skipped, 224 * 2 * margin as u64);
        }
    }

    #[test]
    fn the_hud_can_stay_in_the_middle_of_the_frame() {
        let rom = rom();
        // Near the left edge the window sits at x = 20 of the view; the HUD can stay at 58
        let state = state(2, (20, 8), &[]);
        let mut log = field_log(&rom, &state, &[]);
        for entry in &mut log.entries {
            entry.regs.main_bg[2] = true;
            entry.regs.bg_map_base[2] = 0x7000;
            entry.regs.bg_char_base[2] = 0x3000;
        }
        let render = |hud| {
            WideRenderer::new()
                .render(Rom(&rom), &state, &log, WideOptions { width: 372, hud })
                .unwrap()
        };
        let (window, centred) = (render(HudPlacement::Window), render(HudPlacement::Centred));
        assert_eq!((window.logical.x, centred.logical.x), (20, 20));
        assert_eq!((window.hud.x, centred.hud.x), (20, 58));
        assert_ne!(window.frame, centred.frame);
    }

    #[test]
    fn a_frame_that_is_not_the_field_is_refused() {
        let rom = rom();
        let mut state = state(2, CAMERA, &[]);
        let log = field_log(&rom, &state, &[]);
        let mut renderer = WideRenderer::new();
        let options = WideOptions { width: 372, hud: HudPlacement::Window };
        assert_eq!(
            renderer.render(Rom(&rom), &state, &log, WideOptions { width: 600, ..options }),
            Err(WideError::BadWidth)
        );
        let mut mode_7 = log.clone();
        mode_7.entries[3].regs.bg_mode = 7;
        assert_eq!(
            renderer.render(Rom(&rom), &state, &mode_7, options),
            Err(WideError::NotField(NotField::Layers))
        );
        // No BG2, or a picture of the game's own where the map should be
        let mut no_map = log.clone();
        for entry in &mut no_map.entries {
            entry.regs.main_bg = [false, false, true, false];
        }
        assert_eq!(
            renderer.render(Rom(&rom), &state, &no_map, options),
            Err(WideError::NotField(NotField::Layers))
        );
        let mut picture = log.clone();
        for (index, word) in picture.vram[BG2_MAP..BG2_MAP + 0x800].iter_mut().enumerate() {
            *word = noise(7000 + index as u32) as u16;
        }
        assert_eq!(
            renderer.render(Rom(&rom), &state, &picture, options),
            Err(WideError::NotField(NotField::Picture))
        );
        state.game_mode = 0x07;
        assert_eq!(
            renderer.render(Rom(&rom), &state, &log, options),
            Err(WideError::NotField(NotField::GameMode))
        );
    }

    #[test]
    fn a_frame_in_forced_blanking_is_black_even_while_the_game_loads_memory() {
        let rom = rom();
        let state = state(2, CAMERA, &[]);
        let mut log = field_log(&rom, &state, &[]);
        for entry in &mut log.entries {
            entry.regs.forced_blanking = true;
        }
        log.oam_writes.push(ssd_oracle::OamWrite { entry: 3, high: false, index: 0, value: 1 });
        let wide_frame = wide(&rom, &state, &log, 372);
        assert!(wide_frame.frame.pixels.iter().all(|&c| c == Color::BLACK));
    }
    /// A Mode 7 plane of 32 tile columns, each column a stripe of its own colour; the identity
    /// matrix, BG1 alone, and `lines` as the frame's lines (their registers).
    fn plane(lines: u16) -> FrameLog {
        let mut log = empty_log();
        // The map is the low bytes of the words, the characters their high bytes
        for tile_row in 0..128_usize {
            for tile_column in 0..128_usize {
                log.vram[tile_row * 128 + tile_column] = (tile_column % 32) as u16;
            }
        }
        for tile in 0..32_usize {
            for row in 0..8 {
                for col in 0..8 {
                    log.vram[64 * tile + 8 * row + col] |= (((tile % 15) + 1) as u16) << 8;
                }
            }
        }
        for colour in 1..16 {
            log.cgram_start[colour] = 0x0421 * colour as u16;
        }
        let regs = LineRegs {
            brightness: 15,
            bg_mode: 7,
            main_bg: [true, false, false, false],
            m7_a: 0x0100,
            m7_d: 0x0100,
            ..LineRegs::default()
        };
        log.entries = (1..=lines)
            .map(|scanline| LineEntry {
                scanline,
                from_pixel: 0,
                regs,
                sprites: SpriteLine::default(),
            })
            .collect();
        log.height = 224;
        log
    }

    /// The intro's screen (`ssd_catalogue::engine::intro_on_screen`): BG1's line 0 on the frame's row 40 (scroll
    /// 512 - 41), BG2's lines below the frame.
    fn intro_screen() -> OnScreen {
        OnScreen {
            table: ssd_catalogue::engine::INTRO_TABLE,
            string: 0x0100,
            page: 0,
            key: ssd_catalogue::engine::INTRO_KEY.to_owned(),
            state: 2,
            stopped: false,
            line: 0,
            cell: 0,
            base: 512 - 41,
            portrait: false,
            lines: vec![32; ssd_catalogue::engine::INTRO_LINES],
            english: Vec::new(),
            progress: 1.0,
            names: [None, None, None],
            numbers: Default::default(),
        }
    }

    #[test]
    fn in_the_intro_the_frame_is_drawn_without_the_layers_the_translation_takes_the_place_of() {
        // BG2 shows the field's noise, BG1 the intro's text: the title set's `L` (a bar of colour 3, white) at
        // column 0 of lines 0 and 3, on the frame's rows 40 and 88. With a translation of every line, the Enhanced
        // picture is the frame's without BG1 and BG2 but for the Chinese from column 6: the HD text and the art see
        // no letter of the intro (with the pack, the stripes' art and their blank cells left the letters' shapes
        // under the translation), and the letter gives the lines their ink. A layer one of whose lines the
        // translation lacks stays; outside the intro the frame is the game's.
        let mut rom = rom();
        rom.resize(0x10_0000, 0);
        let bar = [0x1818_u16; 8];
        let mut decoded = vec![0_u8; 256 * 16];
        for (row, word) in bar.iter().enumerate() {
            decoded[0xEF * 16 + row] = *word as u8;
            decoded[0xEF * 16 + 8 + row] = (*word >> 8) as u8;
        }
        let mut stream = (decoded.len() as u16).to_le_bytes().to_vec();
        for group in decoded.chunks(8) {
            stream.push(0xFF);
            stream.extend_from_slice(group);
        }
        rom[0x5F39..0x5F39 + stream.len()].copy_from_slice(&stream);
        let state = state(2, CAMERA, &[]);
        let mut log = field_log(&rom, &state, &[]);
        log.vram[0x4000..0x4008].fill(0);
        log.vram[0x4008..0x4010].copy_from_slice(&bar);
        log.vram[0x7000..0x7400].fill(0);
        log.vram[0x7000] = 1 | 0x2000;
        log.vram[0x7000 + 6 * 32] = 1 | 0x2000;
        log.cgram_start[3] = 0x7FFF;
        for entry in &mut log.entries {
            entry.regs.bg_mode = 0;
            entry.regs.main_bg[0] = true;
            entry.regs.bg_map_base[0] = 0x7000;
            entry.regs.bg_char_base[0] = 0x4000;
            entry.regs.bg_v_scroll[0] = 512 - 41;
        }
        let without = |log: &FrameLog, layers: [bool; 4]| {
            let mut quiet = log.clone();
            for entry in &mut quiet.entries {
                for (shown, off) in entry.regs.main_bg.iter_mut().zip(layers) {
                    *shown &= !off;
                }
            }
            quiet
        };
        let font = Arc::new(Font::standard(&rom).unwrap());
        let face = Arc::new(
            crate::translate::ChineseFace::new(crate::font::SHARE_TECH_MONO.to_vec()).unwrap(),
        );
        let overlay = |lacking: Option<usize>| {
            let rows = (0..ssd_catalogue::engine::INTRO_LINES)
                .filter(|&line| Some(line) != lacking)
                .fold(String::new(), |rows, line| {
                    rows + &ssd_catalogue::engine::intro_key(line) + "\tA\n"
                });
            let table = ssd_catalogue::translation::Translation::parse(&format!("key\tzh\n{rows}"))
                .unwrap();
            Arc::new(Overlay::new(Arc::new(table), Arc::clone(&face)))
        };
        let options = WideOptions { width: 372, hud: HudPlacement::Window };
        let hd = HdOptions { scale: 2, look: crate::Look::Crisp };
        let draw = |log: &FrameLog, overlay: Option<Arc<Overlay>>, screen: Option<OnScreen>| {
            let mut renderer = WideRenderer::new();
            renderer.set_font(Some(Arc::clone(&font)));
            renderer.set_overlay(overlay);
            renderer.set_text_screen(screen);
            renderer.render_hd(Rom(&rom), 0x87, None, log, options, hd).unwrap().picture
        };
        // The column of the picture (2x) the Chinese takes, from column 6 (each line on the screen is drawn: the
        // letter on line 0 gives the ink, the scroll the others' rows)
        let x0 = 2 * (58 + 48);
        let chinese = |x: usize, _y: usize| (x0..x0 + 2 * 24).contains(&x);
        let differ = |a: &crate::HdFrame, b: &crate::HdFrame, inside: bool| {
            let w = a.width as usize;
            (0..a.rgba.len() / 4)
                .filter(|&at| chinese(at % w, at / w) == inside)
                .any(|at| a.rgba[at * 4..at * 4 + 3] != b.rgba[at * 4..at * 4 + 3])
        };
        let game = draw(&log, None, None);
        let quiet = draw(&without(&log, [true, true, false, false]), None, None);
        // The game's frame: the letter, drawn by the HD font at column 0 (outside the Chinese)
        assert!(differ(&game, &quiet, false), "the letter shows");
        let translated = draw(&log, Some(overlay(None)), Some(intro_screen()));
        assert!(
            !differ(&translated, &quiet, false),
            "nothing but the Chinese differs from the frame without the text"
        );
        assert!(differ(&translated, &quiet, true), "line 0's Chinese, in the letter's ink");
        // Line 3 lacking: BG1 is drawn, line 3's letter shows; line 0's is taken out of the frame (its cells take
        // the picture without BG1) and its Chinese drawn
        let lacking = draw(&log, Some(overlay(Some(3))), Some(intro_screen()));
        let rows = |picture: &crate::HdFrame, lines: std::ops::Range<usize>| {
            let w = picture.width as usize;
            lines
                .flat_map(move |y| (0..x0).map(move |x| (y * w + x) * 4))
                .map(|at| picture.rgba[at..at + 3].to_vec())
                .collect::<Vec<_>>()
        };
        assert_ne!(rows(&lacking, 176..192), rows(&quiet, 176..192), "line 3's English stays");
        assert_eq!(rows(&lacking, 80..96), rows(&quiet, 80..96), "line 0's goes");
        assert_eq!(draw(&log, Some(overlay(None)), None), game, "not the intro");
        // The HD text of the frame is read from the frame without BG1 and BG2: no cell of theirs (a cell of another
        // layer under a letter would be one with the letter's shape cut out of it, painted with its ground)
        let mut renderer = WideRenderer::new();
        renderer.set_font(Some(Arc::clone(&font)));
        renderer.set_overlay(Some(overlay(None)));
        renderer.set_text_screen(Some(intro_screen()));
        renderer.render_hd(Rom(&rom), 0x87, None, &log, options, hd).unwrap();
        assert!(
            !renderer.text.cells().iter().any(|cell| cell.layer < 2),
            "the text of the frame without BG1, BG2"
        );
        renderer.set_text_screen(None);
        renderer.render_hd(Rom(&rom), 0x87, None, &log, options, hd).unwrap();
        assert!(
            renderer.text.cells().iter().any(|cell| cell.layer == 0 && cell.bold),
            "outside the intro, the `L`"
        );
    }

    #[test]
    fn a_picture_keeps_its_window_and_is_black_beside_it() {
        let rom = rom();
        let state = state(2, CAMERA, &[]);
        let log = field_log(&rom, &state, &[(10, 40, 5, false), (250, 90, 6, true)]);
        let classic = Renderer::new().render(&log).unwrap();
        // The battle's mode (game mode $02) is not the field, whatever its map says
        let options = WideOptions { width: 372, hud: HudPlacement::Window };
        let view =
            WideRenderer::new().render_view(Rom(&rom), 0x02, Some(&state), &log, options).unwrap();
        assert_eq!(view.scene, Scene::Bordered);
        assert_eq!(view.logical, Rect { x: 58, y: 0, width: 256, height: 224 });
        for y in 0..224 {
            assert_eq!(
                row_slice(&view.frame, y, 58, 256),
                row_slice(&classic, y, 0, 256),
                "row {y}"
            );
            assert!(row_slice(&view.frame, y, 0, 58).iter().all(|&c| c == Color::BLACK));
            assert!(row_slice(&view.frame, y, 314, 58).iter().all(|&c| c == Color::BLACK));
        }
        assert_eq!(view.stats.outside_skipped, 224 * 116);
    }

    #[test]
    fn the_field_is_the_scene_only_where_its_map_shows() {
        let rom = rom();
        let state = state(2, CAMERA, &[]);
        let log = field_log(&rom, &state, &[]);
        assert_eq!(classify(Rom(&rom), 0x00, Some(&state), &log), Scene::Field);
        assert_eq!(classify(Rom(&rom), 0x06, Some(&state), &log), Scene::Field);
        // The map the party walks back to after a building ($01) is $05, until the next battle
        assert_eq!(classify(Rom(&rom), 0x05, Some(&state), &log), Scene::Field);
        assert_eq!(classify(Rom(&rom), 0x85, Some(&state), &log), Scene::Field);
        assert_eq!(classify(Rom(&rom), 0x01, Some(&state), &log), Scene::Bordered, "the building");
        // A picture of the game's own on BG2 is not the map, even in the field's mode
        let mut picture = log.clone();
        for (index, word) in picture.vram[BG2_MAP..BG2_MAP + 0x800].iter_mut().enumerate() {
            *word = noise(7000 + index as u32) as u16;
        }
        assert_eq!(classify(Rom(&rom), 0x00, Some(&state), &picture), Scene::Bordered);
    }

    #[test]
    fn a_flight_is_the_plane_in_the_flight_states_and_a_title_is_not() {
        let rom = rom();
        let log = plane(224);
        // The flight's states are $06 and $07; the title shows the same plane in $87
        assert_eq!(classify(Rom(&rom), 0x07, None, &log), Scene::Flight);
        assert_eq!(classify(Rom(&rom), 0x06, None, &log), Scene::Flight);
        assert_eq!(classify(Rom(&rom), 0x87, None, &log), Scene::Bordered);
        assert_eq!(classify(Rom(&rom), 0x0C, None, &log), Scene::Bordered, "the last fight");
    }

    #[test]
    fn a_flight_continues_the_plane_on_both_sides_of_the_window() {
        let rom = rom();
        let log = plane(224);
        let classic = Renderer::new().render(&log).unwrap();
        let options = WideOptions { width: 372, hud: HudPlacement::Window };
        let view = WideRenderer::new().render_view(Rom(&rom), 0x07, None, &log, options).unwrap();
        assert_eq!(view.scene, Scene::Flight);
        assert_eq!(view.logical.x, 58);
        for y in 0..224 {
            assert_eq!(
                row_slice(&view.frame, y, 58, 256),
                row_slice(&classic, y, 0, 256),
                "row {y}"
            );
        }
        // Column q of the frame shows the plane at window x = q - 58: the Classic frame with the
        // Mode 7 scroll moved by q - 58 shows that position at its first column (the identity
        // matrix moves the plane by whole pixels)
        for q in (0..58).chain(314..372) {
            let mut moved = log.clone();
            for entry in &mut moved.entries {
                entry.regs.m7_h_scroll = (q as i32 - 58) as u16;
            }
            let expected = Renderer::new().render(&moved).unwrap();
            for y in 0..224 {
                assert_eq!(
                    view.frame.pixels[y * 372 + q],
                    expected.pixels[y * 256],
                    "column {q}, row {y}"
                );
            }
        }
        assert_eq!(view.stats.outside_drawn, 0);
    }

    #[test]
    fn a_picture_on_the_plane_has_black_sides_where_the_plane_has_no_texel() {
        let rom = rom();
        let mut log = plane(224);
        // The plane's columns past the first 32 hold the empty tile: nothing is drawn there, the
        // backdrop shows (a picture on the plane, as a cutscene's speed lines)
        for tile_row in 0..128_usize {
            for tile_column in 32..128_usize {
                log.vram[tile_row * 128 + tile_column] = 100;
            }
        }
        log.cgram_start[0] = 0x7C00;
        let classic = Renderer::new().render(&log).unwrap();
        let options = WideOptions { width: 372, hud: HudPlacement::Window };
        let view = WideRenderer::new().render_view(Rom(&rom), 0x07, None, &log, options).unwrap();
        assert_eq!(view.scene, Scene::Flight);
        for y in 0..224 {
            assert_eq!(
                row_slice(&view.frame, y, 58, 256),
                row_slice(&classic, y, 0, 256),
                "row {y}"
            );
            // The sides: the plane's texels where it has some, black where it shows the backdrop
            for q in (0..58).chain(314..372) {
                assert_eq!(view.frame.pixels[y * 372 + q], Color::BLACK, "column {q}, row {y}");
            }
        }
    }

    #[test]
    fn a_caption_line_of_a_flight_is_a_picture_line_with_black_sides() {
        let rom = rom();
        let mut log = plane(224);
        // Line 150 is a text line: BG3 alone in mode 1, over a backdrop that is not black
        for entry in log.entries.iter_mut().filter(|entry| entry.scanline >= 150) {
            entry.regs.bg_mode = 1;
            entry.regs.main_bg = [false, false, true, false];
        }
        log.cgram_start[0] = 0x001F;
        let options = WideOptions { width: 372, hud: HudPlacement::Window };
        let view = WideRenderer::new().render_view(Rom(&rom), 0x07, None, &log, options).unwrap();
        assert_eq!(view.scene, Scene::Flight);
        // Scanline 150 is row 149 of the frame: the plane above it (row 148) continues, the text
        // (row 149) has black sides
        assert!(row_slice(&view.frame, 148, 0, 58).iter().all(|&c| c != Color::BLACK));
        assert!(row_slice(&view.frame, 149, 0, 58).iter().all(|&c| c == Color::BLACK));
        assert!(row_slice(&view.frame, 149, 314, 58).iter().all(|&c| c == Color::BLACK));
    }

    #[test]
    fn the_run_gives_no_view_without_a_log_and_one_with_it() {
        let rom = rom();
        let log = plane(224);
        let mut run = WideRun::new(&rom);
        let wram = vec![0_u8; 0x20000];
        assert!(run.note(&wram, None).is_none());
        let view = run.note(&wram, Some(&log)).unwrap();
        assert_eq!((view.frame.width, view.scene), (WIDTH_16_9, Scene::Bordered));
    }
    #[test]
    fn the_run_draws_the_lightning_it_is_told_and_gives_no_kept_picture_for_another_count() {
        let rom = rom();
        let wram = vec![0_u8; 0x20000];
        let hd = HdOptions { scale: 4, look: crate::Look::Crisp };
        let log = plane(224);
        let dir = tempfile::tempdir().unwrap();
        // Two pictures (made here, a block of one colour each on black), shown in turn
        for (n, colour) in [(1, [0_u8, 0, 200]), (2, [0, 200, 0])] {
            let mut rgba = Vec::new();
            for y in 0..896 {
                for x in 0..1024 {
                    rgba.extend_from_slice(&if x < 512 && y < 448 {
                        [colour[0], colour[1], colour[2], 255]
                    } else {
                        [0, 0, 0, 0]
                    });
                }
            }
            crate::png::write_rgba(&dir.path().join(format!("frame-{n}.png")), 1024, 896, &rgba)
                .unwrap();
        }
        let mut run = WideRun::new(&rom);
        run.set_lightning(crate::lightning::Lightning::load(dir.path()).map(Arc::new));
        // The frame noted starts from the end of the one before it: the lightning comes a frame after the app says it
        let picture = |run: &mut WideRun, frames: Option<u32>| {
            run.set_ssj_lightning(frames);
            run.note_hd_shared(&wram, Some(&log), hd).unwrap().picture.picture.rgba.clone()
        };
        let plain = picture(&mut run, None);
        assert!(picture(&mut run, Some(0)) == plain, "said for the frame drawn next");
        let first = picture(&mut run, Some(4));
        assert!(first != plain, "the first picture is drawn over the frame");
        let second = picture(&mut run, Some(8));
        assert!(second != first, "the same log and work RAM, another picture of the lightning");
        assert!(picture(&mut run, None) != plain, "the frame that started in the transformation");
        assert!(
            picture(&mut run, None) == plain,
            "the transformation over (or a state loaded): none at once"
        );
    }

    #[test]
    fn hide_field_ui_hides_only_field_faces_and_windows_and_invalidates_cached_pictures() {
        let mut log = plane(224);
        for entry in &mut log.entries {
            entry.regs.bg_mode = 1;
            entry.regs.main_bg = [true, true, true, false];
            entry.regs.sub_bg = [true, false, true, false];
            entry.regs.bg_map_base[0] = 0x6800;
            entry.regs.bg_map_base[2] = 0x2C00;
        }
        let quiet = without_field_ui(&log);
        for (original, hidden) in log.entries.iter().zip(&quiet.entries) {
            assert_eq!(hidden.regs.main_bg, [false, true, false, false]);
            assert_eq!(hidden.regs.sub_bg, [false; 4]);
            let mut restored = hidden.clone();
            restored.regs.main_bg = original.regs.main_bg;
            restored.regs.sub_bg = original.regs.sub_bg;
            assert_eq!(
                &restored, original,
                "objects, scroll and animation registers are unchanged"
            );
        }
        let mode7 = plane(224);
        assert!(same_log(&mode7, &without_field_ui(&mode7)), "never remove a Mode 7 plane");
        let mut run = WideRun::new(&rom());
        let wram = vec![0; 0x20000];
        let hd = HdOptions { scale: 2, look: crate::Look::Pixel };
        let original = run.note_hd_shared(&wram, Some(&mode7), hd).unwrap();
        let kept = run.note_hd_shared(&wram, Some(&mode7), hd).unwrap();
        assert_eq!(original.serial, kept.serial);
        run.set_hide_field_ui(true);
        let changed = run.note_hd_shared(&wram, Some(&mode7), hd).unwrap();
        assert_ne!(original.serial, changed.serial, "never reuse a different presentation");
        assert_eq!(
            original.picture.picture.rgba, changed.picture.picture.rgba,
            "an unrelated scene remains pixel-identical"
        );
        run.set_hide_field_ui(false);
        let restored = run.note_hd_shared(&wram, Some(&mode7), hd).unwrap();
        assert_ne!(changed.serial, restored.serial);
        assert_eq!(original.picture.picture.rgba, restored.picture.picture.rgba);
    }

    #[test]
    fn a_picture_that_lacks_art_being_read_is_not_given_again_for_the_same_frame() {
        let rom = rom();
        let wram = vec![0_u8; 0x20000];
        let hd = HdOptions { scale: 4, look: crate::Look::Crisp };
        let dir = tempfile::tempdir().unwrap();
        let log = crate::art::tests::plane_frame();
        let pack = crate::art::tests::plane_pack(dir.path(), &log.vram);
        let picture = |run: &mut WideRun| {
            let shared = run.note_hd_shared(&wram, Some(&log), hd).unwrap();
            let pending = shared.picture.art_pending;
            (shared.serial, shared.picture.picture.rgba.clone(), pending)
        };
        let original = picture(&mut WideRun::new(&rom)).1;
        let mut synchronous = WideRun::new(&rom);
        synchronous.set_art(Some(Arc::clone(&pack)));
        let with_art = picture(&mut synchronous).1;
        assert!(with_art != original, "the plane is drawn from its art");
        let mut run = WideRun::new(&rom);
        run.set_art(Some(pack));
        run.set_art_async(true);
        let (serial, first, pending) = picture(&mut run);
        assert!(first == original, "the first picture is the original's");
        assert!(pending, "and it says it lacks art");
        // The same frame again, until the art is read: never the first picture given back
        let started = std::time::Instant::now();
        let mut last = (serial, first, pending);
        while last.1 != with_art && started.elapsed() < std::time::Duration::from_secs(30) {
            std::thread::sleep(std::time::Duration::from_millis(5));
            let next = picture(&mut run);
            assert_ne!(next.0, last.0, "a picture that lacked art is drawn again");
            last = next;
        }
        assert!(last.1 == with_art, "the picture once the art is read is the one with it");
        assert!(!last.2, "and it lacks none");
        // Drawn with all its art, the picture is given back for the same frame
        assert_eq!(picture(&mut run).0, last.0);
    }

    #[test]
    fn the_worker_draws_in_the_background_and_gives_each_picture_back_once_in_order() {
        let rom = rom();
        let wram = vec![0_u8; 0x20000];
        let hd = HdOptions { scale: 2, look: crate::Look::Crisp };
        let (plain, lit) = (plane(224), {
            let mut log = plane(224);
            log.cgram_start[3] = 0x7FFF;
            log
        });
        // The pictures drawn in the foreground are the reference
        let mut reference = WideRun::new(&rom);
        let expected: Vec<_> = [&plain, &lit]
            .into_iter()
            .map(|log| {
                reference.skip(&wram);
                reference.redraw_hd(Some(log), hd).unwrap().picture.rgba.clone()
            })
            .collect();
        assert_ne!(expected[0], expected[1], "the two frames differ");

        let mut run = WideRun::new(&rom);
        let started = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&started);
        run.set_worker_start(Arc::new(move || {
            seen.lock().unwrap().push(std::thread::current().name().map(str::to_owned));
        }));
        assert!(!run.drawing());
        run.skip(&wram);
        assert!(run.redraw_hd_next(Some(&plain), hd).is_none(), "nothing was drawn before it");
        assert!(run.drawing());
        run.skip(&wram);
        let first = run.redraw_hd_next(Some(&lit), hd).expect("the first picture, a call later");
        assert_eq!(first.picture.rgba, expected[0]);
        // Nothing to draw: the picture of the call before still comes back, once
        let second = run.redraw_hd_next(None, hd).expect("the second picture");
        assert_eq!(second.picture.rgba, expected[1]);
        assert!(!run.drawing());
        assert!(run.redraw_hd_next(None, hd).is_none() && run.finish_pipelined().is_none());
        // A frame that is waited for at the end of the run
        run.skip(&wram);
        assert!(run.redraw_hd_pipelined(Some(&plain), hd).is_none());
        assert_eq!(run.finish_pipelined().unwrap().picture.rgba, expected[0]);
        // One thread for all of it, started by the first picture and named
        assert_eq!(*started.lock().unwrap(), [Some("hd-draw".to_owned())]);
        // The foreground draws again once the picture in the background is thrown away
        run.skip(&wram);
        assert!(run.redraw_hd_pipelined(Some(&lit), hd).is_none());
        run.skip(&wram);
        assert_eq!(run.redraw_hd(Some(&plain), hd).unwrap().picture.rgba, expected[0]);
        assert!(!run.drawing());
    }

    #[test]
    fn an_object_moved_during_the_display_shows_as_the_ppu_draws_it() {
        let rom = rom();
        let state = state(2, CAMERA, &[]);
        let mut log = field_log(&rom, &state, &[(10, 40, 5, false)]);
        // Object 0 moves from x 10 to x 60 while the frame is drawn, before its own lines (40 to 47)
        log.oam_writes.push(ssd_oracle::OamWrite {
            entry: 20,
            high: false,
            index: 0,
            value: 0x3C | (0x28 << 8),
        });
        let classic = Renderer::new().render(&log).unwrap();
        let mut without = log.clone();
        without.oam_writes.clear();
        assert_ne!(classic.pixels, Renderer::new().render(&without).unwrap().pixels);
        // A picture (not the field): the wide frame at 256 is the Classic frame, writes included
        let options = WideOptions { width: 256, hud: HudPlacement::Window };
        let view = WideRenderer::new().render_view(Rom(&rom), 0x02, None, &log, options).unwrap();
        assert_eq!(view.frame.pixels, classic.pixels);
    }
    #[test]
    fn a_picture_takes_its_objects_as_the_ppu_does_wherever_they_lie() {
        let rom = rom();
        let state = state(2, CAMERA, &[]);
        let mut log = field_log(&rom, &state, &[]);
        opaque_objects(&mut log);
        // BG2 is transparent: its characters are empty
        log.vram[usize::from(BG2_CHARS)..BG2_MAP].fill(0);
        log.cgram_start[0] = 0;
        log.cgram_start[0x81] = 0x7FFF;
        // Line 11: 32 objects at x 0 to 31 take the PPU's 32 places; the 33rd, at x 200, is not
        // taken, though it lies in the second chunk, where the chunk counts fewer objects
        for index in 0..32_usize {
            log.oam_low[index * 4..index * 4 + 4].copy_from_slice(&[index as u8, 10, 3, 0x21]);
        }
        log.oam_low[32 * 4..32 * 4 + 4].copy_from_slice(&[200, 10, 3, 0x21]);
        let classic = Renderer::new().render(&log).unwrap();
        let options = WideOptions { width: 372, hud: HudPlacement::Window };
        let view = WideRenderer::new().render_view(Rom(&rom), 0x02, None, &log, options).unwrap();
        assert_eq!(view.scene, Scene::Bordered);
        for y in 0..224 {
            assert_eq!(
                row_slice(&view.frame, y, 58, 256),
                row_slice(&classic, y, 0, 256),
                "row {y}"
            );
        }
    }

    /// Frames drawn in segments of lines by the thread pool are the frames drawn in one piece:
    /// with objects, CGRAM writes during the display, and the line buffers carried between frames.
    #[test]
    fn segments_of_lines_draw_the_frames_one_piece_would() {
        let rom = rom();
        let state = state(2, CAMERA, &[]);
        let mut parallel = WideRenderer::new();
        let mut serial = WideRenderer::new();
        serial.set_serial(true);
        for (frame, width) in [(0_i32, 372), (1, 372), (2, 256), (3, 372)] {
            let sprites: Vec<(i32, u8, u8, bool)> = (0..20)
                .map(|n| {
                    (n * 17 + frame * 5, (n * 11 + frame * 3) as u8 % 200, 3 + n as u8, n % 3 == 0)
                })
                .collect();
            let mut log = field_log(&rom, &state, &sprites);
            opaque_objects(&mut log);
            for line in 0..30_u16 {
                log.cgram_writes.push(ssd_oracle::CgramWrite {
                    entry: line * 7 + 3,
                    index: (line % 16) as u8,
                    value: noise(u32::from(line) + 77) as u16 & 0x7FFF,
                });
            }
            let options = WideOptions { width, hud: HudPlacement::Window };
            let a = parallel.render_view(Rom(&rom), 0x02, None, &log, options).unwrap();
            let b = serial.render_view(Rom(&rom), 0x02, None, &log, options).unwrap();
            assert_eq!(a, b, "frame {frame}");
        }
    }

    #[test]
    fn segments_start_at_lines_that_rebuild_the_objects_and_share_the_lines_out() {
        let mut log = empty_log();
        let entry = |scanline: u16, from_pixel: u16, rebuilt: bool| LineEntry {
            scanline,
            from_pixel,
            regs: LineRegs::default(),
            sprites: SpriteLine { rebuilt, ..SpriteLine::default() },
        };
        log.entries = (1..=224_u16).map(|line| entry(line, 0, true)).collect();
        let cuts = segment_starts(&log, None, 224);
        let threads = rayon::current_num_threads().clamp(1, 6);
        assert_eq!(cuts.len(), threads, "{cuts:?}");
        assert_eq!(cuts[0], 0);
        assert!(cuts.windows(2).all(|pair| pair[0] < pair[1]));
        // Lines that do not rebuild the buffer, or a second entry of a line, are no place to cut
        log.entries = (1..=224_u16).map(|line| entry(line, 0, false)).collect();
        assert_eq!(segment_starts(&log, None, 224), [0]);
        log.entries = (1..=224_u16)
            .flat_map(|line| [entry(line, 0, line % 2 == 0), entry(line, 40, true)])
            .collect();
        let cuts = segment_starts(&log, None, 224);
        assert!(
            cuts.iter().all(|&at| at == 0
                || (log.entries[at].from_pixel == 0 && log.entries[at].sprites.rebuilt))
        );
        // Entries out of line order are drawn in one piece
        log.entries = (1..=224_u16).map(|line| entry(line, 0, true)).collect();
        log.entries.swap(100, 101);
        assert_eq!(segment_starts(&log, None, 224), [0]);
        // Only the lines of the rows asked for count
        log.entries = (1..=224_u16).map(|line| entry(line, 0, true)).collect();
        assert_eq!(segment_starts(&log, Some((10, 20)), 224), [0]);
    }

    #[test]
    fn two_logs_are_the_same_frame_only_when_every_part_of_them_is() {
        let log = FrameLog {
            complete: true,
            height: 224,
            h_hi_res: false,
            v_hi_res: false,
            entries: Vec::new(),
            cgram_start: vec![0; 256],
            cgram_writes: Vec::new(),
            vram: vec![0; 0x8000],
            vram_writes: Vec::new(),
            oam_low: vec![0; 512],
            oam_high: vec![0; 32],
            oam_writes: Vec::new(),
        };
        assert!(same_log(&log, &log.clone()));
        let mut other = log.clone();
        other.vram[0x1234] = 1;
        assert!(!same_log(&log, &other), "VRAM");
        let mut other = log.clone();
        other.oam_high[3] = 0x80;
        assert!(!same_log(&log, &other), "OAM");
        let mut other = log.clone();
        other.cgram_start[7] = 0x1F;
        assert!(!same_log(&log, &other), "CGRAM");
        let mut other = log.clone();
        other.complete = false;
        assert!(!same_log(&log, &other), "complete");
    }

    fn help_page(key: &str) -> OnScreen {
        OnScreen {
            table: 0,
            string: 25,
            page: 0,
            key: key.to_owned(),
            state: 0,
            stopped: false,
            line: 0,
            cell: 0,
            base: 0,
            portrait: false,
            lines: vec![12, 13, 13],
            english: Vec::new(),
            progress: 1.0,
            names: [None, None, None],
            numbers: Default::default(),
        }
    }

    /// A frame ran and ended naming `now` ([`WideRun::advance`]'s text part).
    fn ran(text: &mut TextRun, now: Option<&str>) {
        text.started = std::mem::replace(&mut text.ended, now.map(help_page));
    }

    fn key(page: Option<OnScreen>) -> Option<String> {
        page.map(|page| page.key)
    }

    #[test]
    fn a_page_named_only_in_skipped_frames_is_handed_to_the_frame_drawn_next() {
        // The Item help is named at the end of two frames; they and the one after run in a batch the window does not
        // draw (the harness's way: skip, skip, skip, then note the next frame)
        let mut text = TextRun::default();
        for now in [Some("help:25"), Some("help:25"), None] {
            ran(&mut text, now);
            text.skipped();
        }
        // The frame drawn next starts from no page: it gets the help all the same
        ran(&mut text, None);
        assert_eq!(key(text.hand()), Some("help:25".to_owned()));
        // and only that frame: the next one starts from what the engine names then
        ran(&mut text, None);
        assert_eq!(text.hand(), None);
    }

    #[test]
    fn the_app_drawing_each_frame_after_skipping_it_gets_a_new_page_when_it_starts_from_it() {
        // The app at speed 1, not behind: every frame is skipped (its state followed), then drawn (redraw_hd)
        let mut text = TextRun::default();
        ran(&mut text, Some("help:25"));
        text.skipped();
        assert_eq!(
            text.hand(),
            None,
            "the frame that names the page at its end does not show it yet"
        );
        ran(&mut text, Some("help:25"));
        text.skipped();
        assert_eq!(
            key(text.hand()),
            Some("help:25".to_owned()),
            "the frame after it starts from it"
        );
    }

    #[test]
    fn a_page_named_once_at_the_end_of_a_drawn_frame_reaches_the_batch_after_it() {
        // A frame starts from A and ends on B, named for that one frame (written at once); the next two frames run in
        // a batch the window does not draw, and the app draws the last of them
        let mut text = TextRun::default();
        ran(&mut text, Some("help:2"));
        ran(&mut text, Some("help:25"));
        text.skipped();
        assert_eq!(key(text.hand()), Some("help:2".to_owned()), "the frame drawn starts from A");
        ran(&mut text, None);
        text.skipped();
        ran(&mut text, None);
        text.skipped();
        assert_eq!(
            key(text.hand()),
            Some("help:25".to_owned()),
            "B, which only a skipped frame started from"
        );
    }

    #[test]
    fn the_page_a_frame_started_from_comes_before_a_missed_one() {
        let mut text = TextRun::default();
        ran(&mut text, Some("help:25"));
        ran(&mut text, Some("help:2"));
        text.skipped();
        ran(&mut text, None);
        text.started = Some(help_page("help:2"));
        assert_eq!(key(text.hand()), Some("help:2".to_owned()));
        assert_eq!(text.missed, None, "nothing is kept past the frame drawn");
    }

    #[test]
    fn frames_that_are_drawn_keep_nothing_missed() {
        let mut text = TextRun::default();
        ran(&mut text, Some("help:25"));
        // A drawn frame never calls skipped(): with no page at its start it gets none
        assert_eq!(text.hand(), None);
    }
}
