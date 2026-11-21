//! Drawing of the window with egui: the game frame, the header and controls bars and the debug
//! panel when it is open, the "F1 debug" pill and the toast when it is closed. Thin on purpose:
//! all state and decisions are in `panel.rs`, `debugger.rs` and `replays.rs`; this reads them and
//! pushes [`PanelAction`]s.

use crate::canvas::GameImage;
use crate::debugger::StepKind;
use crate::hd::{self, Art};
use crate::hook_setup::HooksArg;
use crate::hotkeys::Speed;
use crate::layout;
use crate::panel::{
    Controls, DebugAction, HooksSwitch, PANEL_WIDTH, PanelAction, PanelState, SLOTS, Tab, grouped,
    rust_share_text,
};
use crate::tab_debugger;
use crate::tab_memory;
use crate::tab_replays;
use crate::tab_routines::{self, FrameHistory};
use crate::theme::{
    self, ACCENT, ACCENT_FILL, CONTROL, CONTROL_HOVER, DISABLED, DISABLED_LINE, GROUND, LINE,
    LINE_STRONG, MUTED, MUTED_LIGHT, PANEL, TEXT, TOAST_FILL, TOAST_LINE, TOAST_TEXT, VERIFY,
};
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, Layout, Margin, Order,
    Panel, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use ssd_menus::DrawList;
use ssd_oracle::{Machine, Registers, Symbols};

/// What the header shows about the running machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct Status {
    /// The frame the machine has reached.
    pub frame: u64,
    /// Emulated frames per second.
    pub fps: f32,
    /// Share of the instructions of the last 60 frames that ran as Rust, in percent; `None` when
    /// there is nothing to show yet.
    pub rust_percent: Option<f64>,
    /// Routines run in Rust without a measured mean: the share is not known until a Verify run.
    pub means_missing: bool,
    /// Freezes the machine holds.
    pub freezes: usize,
    /// The sound and the battle speed of the window frame; `None` where no window frame runs paced (a snapshot).
    pub live: Option<Live>,
}

/// What the pill shows of the window frame beside the rate: the sound queued and its gaps, as the touch overlay shows
/// them, and whether the battle speed is on with the frames the window frame asked for and ran.
#[derive(Debug, Clone, Copy, Default)]
pub struct Live {
    /// The sound queued at the device, in milliseconds (`Audio::health`).
    pub queued_ms: f32,
    /// The times the queue ran dry with sound waiting (`Audio::health`).
    pub gaps: u32,
    /// The battle speed, if it is on (`Pacer::tempo`: frames of the game, refreshes).
    pub tempo: Option<(u32, u32)>,
    /// The frames of the game the window frame asked for.
    pub requested: u32,
    /// The frames of the game the window frame ran.
    pub run: u32,
    /// The slow window frames of the run (over 100 ms) and the slowest, in milliseconds.
    pub slow: u32,
    pub slowest_ms: f32,
}

/// The pill's text of the window frame: `sound 42 ms · 0 gaps · battle 1.5x · 2/2 frames · slow 0 · max 0 ms` (or
/// `battle 2x`), with `battle 1x` off.
fn live_text(live: &Live) -> String {
    let sound = format!("sound {:.0} ms \u{b7} {} gaps", live.queued_ms, live.gaps);
    let battle = match live.tempo {
        Some((frames, refreshes)) => format!(
            "battle {}x \u{b7} {}/{} frames",
            f64::from(frames) / f64::from(refreshes.max(1)),
            live.requested,
            live.run
        ),
        None => "battle 1x".to_owned(),
    };
    let slow = format!("slow {} \u{b7} max {:.0} ms", live.slow, live.slowest_ms);
    format!("{sound} \u{b7} {battle} \u{b7} {slow}")
}

/// Everything the drawing reads (the game views come apart, see [`draw`]).
pub struct View<'a> {
    pub status: Status,
    pub controls: &'a Controls,
    /// The CPU's registers of the machine: the debugger shows them.
    pub registers: &'a Registers,
    /// Whether a recording runs: breakpoints wait for it to end.
    pub recording: bool,
    /// The names of the two game views (A and B) while a verification runs.
    pub verify_labels: Option<[String; 2]>,
    /// The machine, for what the tabs read: its hook table and counters, its memory.
    pub machine: &'a Machine,
    /// The symbols of the run, if the file could be read.
    pub symbols: Option<&'a Symbols>,
    /// The frames of the Routines bar strip.
    pub history: &'a FrameHistory,
    /// The touch controls (M9), drawn over the game when they are on.
    pub touch: Option<&'a crate::touch::Touch>,
}

/// Width the controls bar needs in one row; narrower, it takes two.
const ONE_ROW_WIDTH: f32 = 770.0;

/// The HD layer of a frame: a draw list and the textures it paints with. A menu on screen takes the game's
/// picture (`menu`); an overlay of the field (the cheats' notice) is painted over the picture.
pub struct Layer<'a> {
    pub list: &'a DrawList,
    pub art: &'a mut Art,
    pub menu: bool,
}

/// The HD layer of a frame, if there is one.
pub type Hd<'a> = Option<Layer<'a>>;

/// Draws the window. `games` are the game views, one frame each: the machine's, or the two of a
/// verification (A, then B). Actions the user asked for are pushed to `actions`. While an HD menu
/// is on screen (`hd`), it takes the game's area, and the "F1 debug" pill is left out; it is also left out without
/// the debug tools ([`Controls::debug`]).
pub fn draw(
    ui: &mut Ui,
    view: &View<'_>,
    games: &[GameImage],
    panel: &mut PanelState,
    actions: &mut Vec<PanelAction>,
    hd: Hd<'_>,
) {
    let on_screen = hd.as_ref().is_some_and(|layer| layer.menu);
    // The touch controls (M9) shrink the picture's area to what the controls leave, and paint over everything.
    let touch = view.touch.filter(|t| t.enabled && !panel.open);
    let area = touch.and_then(crate::touch::Touch::picture_area);
    let stretch = touch.is_some_and(crate::touch::Touch::stretches_picture);
    if panel.open {
        header(ui, view, actions);
        side_panel(ui, view, panel, actions);
        let now = ui.ctx().input(|i| i.time);
        controls_bar(ui, view.controls, panel.notice(now), actions);
        game_area(ui, games, view.verify_labels.as_ref(), GROUND, hd, None, false);
    } else {
        game_area(ui, games, view.verify_labels.as_ref(), Color32::BLACK, hd, area, stretch);
        if view.controls.debug && !on_screen && touch.is_none() {
            pill(ui.ctx(), view.status, actions);
        }
        toast(ui.ctx(), panel, actions);
        let now = ui.ctx().input(|i| i.time);
        if let Some(text) = panel.notice(now).map(str::to_owned) {
            notice(ui.ctx(), &text);
        }
    }
    if let Some(t) = touch {
        let layer = egui::LayerId::new(Order::Foreground, Id::new("touch-controls"));
        crate::touch::draw::paint(&ui.ctx().layer_painter(layer), t);
    }
}

/// The game alone, as the window shows it with the panel closed and nothing over it: the picture of a
/// screenshot. An HD menu on screen takes the area as in [`draw`].
pub fn draw_game(ui: &mut Ui, games: &[GameImage], hd: Hd<'_>, stretch: bool) {
    game_area(ui, games, None, Color32::BLACK, hd, None, stretch);
}

// --- the game ---------------------------------------------------------------------------------

/// The game views in what the panels leave: one frame, as large as a whole scale allows and
/// centred, or stretched to fill the area; or two side by side, each captioned with its label. An HD menu on screen
/// replaces the frame: it is painted over the frame's own picture (the rect the live frame is drawn in), so the
/// menu's backdrop does not move the picture.
fn game_area(
    ui: &mut Ui,
    games: &[GameImage],
    labels: Option<&[String; 2]>,
    ground: Color32,
    hd: Hd<'_>,
    inset: Option<Rect>,
    stretch: bool,
) {
    let full = ui.available_rect_before_wrap();
    ui.painter().rect_filled(full, 0.0, ground);
    let rect = inset.unwrap_or(full);
    let overlay = match hd {
        Some(layer @ Layer { menu: true, .. }) => {
            let picture = games.first().map_or(rect, |game| picture_of(ui, rect, game, stretch));
            paint_hd(ui, picture, layer, stretch);
            ui.allocate_rect(rect, Sense::hover());
            return;
        }
        other => other,
    };
    match games {
        [game] => fitted(ui, rect, game, stretch),
        [a, b] => {
            let gap = 16.0;
            let half = (rect.width() - gap) / 2.0;
            let caption = 24.0;
            let left = Rect::from_min_size(rect.min, vec2(half, rect.height() - caption));
            let right_min = pos2(rect.min.x + half + gap, rect.min.y);
            let right = Rect::from_min_size(right_min, vec2(half, rect.height() - caption));
            fitted(ui, left, a, stretch);
            fitted(ui, right, b, stretch);
            if let Some([label_a, label_b]) = labels {
                let y = rect.max.y - caption / 2.0;
                let font = FontId::proportional(11.0);
                let at_a = pos2(left.center().x, y);
                let at_b = pos2(right.center().x, y);
                ui.painter().text(at_a, Align2::CENTER_CENTER, label_a, font.clone(), MUTED);
                ui.painter().text(at_b, Align2::CENTER_CENTER, label_b, font, MUTED);
            }
        }
        _ => {}
    }
    // An overlay of the field is painted over the picture the frame drew.
    if let Some(layer) = overlay
        && let Some(game) = games.first()
    {
        let picture = picture_of(ui, rect, game, stretch);
        paint_hd(ui, picture, layer, stretch);
    }
    ui.allocate_rect(rect, Sense::hover());
}

/// The egui memory's key of the rect the HD layer was last painted over.
const HD_PICTURE: &str = "hd-picture";

fn paint_hd(ui: &Ui, picture: Rect, Layer { list, art, menu }: Layer<'_>, stretch: bool) {
    let canvas = if menu {
        hd::paint_menu(ui, picture, list, art, stretch)
    } else {
        hd::paint(ui, picture, list, art);
        picture
    };
    let (width, height) = list.canvas_size();
    let mut fit = hd::Fit::for_list(canvas, list);
    // The quit question's hit boxes use the default canvas, centred within this screen.
    fit.origin +=
        vec2((width - ssd_menus::layout::WIDTH) / 2.0, (height - ssd_menus::layout::HEIGHT) / 2.0)
            * fit.scale;
    ui.ctx().data_mut(|data| data.insert_temp(Id::new(HD_PICTURE), (picture, canvas, fit)));
}

/// Esc's question under the mouse, once it is drawn: the cursor follows the pointer onto an answer, and a click on
/// one gives it. The question lies in the HD layer's logical space, fitted to the picture it was painted over.
pub fn quit_confirm_pointer(
    ui: &Ui,
    question: &mut crate::quit_confirm::QuitConfirm,
) -> Option<crate::quit_confirm::Outcome> {
    let (picture, canvas, fit): (Rect, Rect, hd::Fit) =
        ui.ctx().data(|data| data.get_temp(Id::new(HD_PICTURE)))?;
    let projection = egui::emath::RectTransform::from_to(picture, canvas);
    if fit.scale <= 0.0 {
        return None;
    }
    let logical = |at: egui::Pos2| {
        let at = projection * at;
        ((at.x - fit.origin.x) / fit.scale, (at.y - fit.origin.y) / fit.scale)
    };
    let (moved, hover, click) = ui.ctx().input(|input| {
        let pointer = &input.pointer;
        let click = if pointer.primary_clicked() { pointer.interact_pos() } else { None };
        (pointer.delta() != egui::Vec2::ZERO, pointer.hover_pos(), click)
    });
    if moved && let Some(at) = hover {
        question.hover(logical(at));
    }
    click.and_then(|at| question.click(logical(at)))
}

/// One game view fitted in `area`: letterboxed, or stretched to fill it.
fn fitted(ui: &Ui, area: Rect, game: &GameImage, stretch: bool) {
    let image = picture_of(ui, area, game, stretch);
    ui.painter().image(
        game.id,
        image,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// The rect a game view is drawn in, within `area`: the one [`fitted`] paints, and the one an HD
/// menu's backdrop and layout are fitted to.
fn picture_of(ui: &Ui, area: Rect, game: &GameImage, stretch: bool) -> Rect {
    let picture = if stretch {
        area
    } else {
        // With the touch controls (a phone) an HD picture fills its area at any scale: its whole multiples do not
        // fit there. The machine's own frame, and the desktop, keep whole scales.
        let whole = !(crate::touch::wanted() && game.height > layout::MACHINE_LINES);
        layout::picture_rect(
            ui.ctx().pixels_per_point(),
            area,
            game.width,
            game.height,
            game.pixel_aspect,
            whole,
        )
    };
    log::debug!("game picture {picture:?} in area {area:?}");
    picture
}

// --- panel closed -----------------------------------------------------------------------------

/// The small "F1 debug 60 fps" label top-right, with the window frame's sound and battle speed ([`live_text`]);
/// clicking it opens the panel.
fn pill(ctx: &egui::Context, status: Status, actions: &mut Vec<PanelAction>) {
    let fps = status.fps;
    egui::Area::new(Id::new("pill"))
        .anchor(Align2::RIGHT_TOP, vec2(-12.0, 12.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            let frame = Frame::new()
                .fill(Color32::from_rgba_unmultiplied(19, 22, 27, 217))
                .stroke(Stroke::new(1.0, LINE_STRONG))
                .corner_radius(CornerRadius::same(255))
                .inner_margin(Margin::symmetric(10, 5));
            let inner = frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    kbd(ui, "F1");
                    ui.label(RichText::new("debug").size(11.0).color(MUTED_LIGHT));
                    ui.label(
                        RichText::new(format!("{fps:.0} fps")).font(theme::mono(11.0)).color(TEXT),
                    );
                    if let Some(live) = status.live {
                        ui.label(
                            RichText::new(live_text(&live)).font(theme::mono(11.0)).color(TEXT),
                        );
                    }
                });
            });
            let response = ui.interact(inner.response.rect, Id::new("pill-click"), Sense::click());
            if response.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if response.clicked() {
                actions.push(PanelAction::TogglePanel);
            }
        });
}

/// The line after a screenshot, bottom-centre, while the panel is closed.
fn notice(ctx: &egui::Context, text: &str) {
    egui::Area::new(Id::new("shot-notice"))
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -16.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            Frame::new()
                .fill(TOAST_FILL)
                .stroke(Stroke::new(1.0, TOAST_LINE))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.label(RichText::new(text).color(TOAST_TEXT));
                });
        });
    // Keep the countdown running even when nothing else changes.
    ctx.request_repaint();
}

/// The message bottom-right while the panel is closed.
fn toast(ctx: &egui::Context, panel: &mut PanelState, actions: &mut Vec<PanelAction>) {
    let now = ctx.input(|i| i.time);
    let Some(toast) = panel.toast(now).cloned() else { return };
    egui::Area::new(Id::new("toast"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -16.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            Frame::new()
                .fill(TOAST_FILL)
                .stroke(Stroke::new(1.0, TOAST_LINE))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 4.0, ACCENT);
                        ui.label(RichText::new(&toast.text).color(TOAST_TEXT));
                        let open = ui.add(
                            egui::Label::new(RichText::new("Open").color(ACCENT))
                                .sense(Sense::click()),
                        );
                        if open.hovered() {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                        }
                        if open.clicked() {
                            actions.push(PanelAction::OpenTab(toast.tab));
                        }
                    });
                });
        });
    // Keep the countdown running even when nothing else changes.
    ctx.request_repaint();
}

// --- panel open: header -----------------------------------------------------------------------

/// A key cap: "F1".
fn kbd(ui: &mut Ui, key: &str) {
    let font = theme::mono(11.0);
    let galley = ui.painter().layout_no_wrap(key.to_owned(), font.clone(), TEXT);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 12.0, 18.0), Sense::hover());
    ui.painter().rect(rect, 4.0, CONTROL, Stroke::new(1.0, LINE_STRONG), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, key, font, TEXT);
}

/// "label value" in the header: a muted word and a bright number.
fn stat(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new(label).font(theme::mono(12.0)).color(MUTED_LIGHT));
        ui.label(RichText::new(value).font(theme::mono(12.0)).color(TEXT));
    });
}

/// A vertical hairline between groups.
fn divider(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, LINE_STRONG);
}

fn header(ui: &mut Ui, view: &View<'_>, actions: &mut Vec<PanelAction>) {
    Panel::top("header")
        .exact_size(40.0)
        .resizable(false)
        .frame(Frame::new().fill(PANEL).inner_margin(Margin::symmetric(16, 0)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.label(RichText::new("SSD Recompiled").font(theme::semibold(13.0)).color(ACCENT));
                divider(ui, 18.0);
                ui.spacing_mut().item_spacing.x = 14.0;
                stat(ui, "frame", &grouped(view.status.frame));
                stat(ui, "", &format!("{:.1} fps", view.status.fps));
                stat(ui, "Rust, last 60 frames", &rust_share_text(view.status.rust_percent));
                if view.status.means_missing {
                    let hint = RichText::new(tab_routines::MEANS_HINT).size(11.0).color(MUTED);
                    ui.label(hint);
                }
                if view.status.freezes > 0 {
                    let plural = if view.status.freezes == 1 { "" } else { "s" };
                    let text = format!("{} freeze{plural} on", view.status.freezes);
                    ui.label(RichText::new(text).font(theme::mono(12.0)).color(VERIFY));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    kbd(ui, "F1");
                    ui.label(RichText::new("Panel").size(12.0).color(MUTED_LIGHT));
                    ui.add_space(8.0);
                    hooks_select(ui, view.controls, actions);
                    ui.label(RichText::new("Hooks").size(12.0).color(MUTED_LIGHT));
                });
            });
        });
}

/// The hook mode select of the header: every routine gets the mode picked; "Mixed" while they
/// differ (picking any mode, the one shown or not, sets them all).
fn hooks_select(ui: &mut Ui, controls: &Controls, actions: &mut Vec<PanelAction>) {
    let allowed = controls.hooks_switch == HooksSwitch::Allowed;
    let shown = if controls.hooks_mixed { "Mixed" } else { controls.hooks.label() };
    let mut chosen = None;
    let response = ui
        .add_enabled_ui(allowed, |ui| {
            egui::ComboBox::from_id_salt("hooks-mode")
                .width(88.0)
                .selected_text(RichText::new(shown).size(12.0).color(TEXT))
                .show_ui(ui, |ui| {
                    for mode in HooksArg::ALL {
                        let mut highlighted = controls.hooks;
                        if ui.selectable_value(&mut highlighted, mode, mode.label()).clicked() {
                            chosen = Some(mode);
                        }
                    }
                });
        })
        .response;
    if let HooksSwitch::Locked(why) = controls.hooks_switch {
        response.on_hover_text(why);
    }
    if let Some(mode) = chosen {
        actions.push(PanelAction::SetHooks(mode));
    }
}

// --- panel open: side panel -------------------------------------------------------------------

fn side_panel(
    ui: &mut Ui,
    view: &View<'_>,
    panel: &mut PanelState,
    actions: &mut Vec<PanelAction>,
) {
    Panel::right("debug-panel")
        .exact_size(PANEL_WIDTH)
        .resizable(false)
        .frame(Frame::new().fill(PANEL))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            tab_strip(ui, panel);
            match panel.tab {
                Tab::Routines => {
                    let routines = tab_routines::view(view.machine, view.symbols, view.history);
                    tab_routines::show(ui, &routines, view.controls, actions);
                }
                Tab::Memory => {
                    let memory = tab_memory::view(view.machine, view.symbols, &panel.memory);
                    let freezes = view.controls.freezes_allowed;
                    tab_memory::show(
                        ui,
                        &memory,
                        &mut panel.memory,
                        view.symbols,
                        freezes,
                        actions,
                    );
                }
                Tab::Debugger => {
                    tab_debugger::draw(ui, view, &mut panel.debug, actions);
                }
                Tab::Replays => {
                    tab_replays::draw(ui, &panel.replays, actions);
                }
            }
        });
}

fn tab_strip(ui: &mut Ui, panel: &mut PanelState) {
    Frame::new().inner_margin(Margin { left: 8, right: 8, top: 8, bottom: 0 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for tab in Tab::ALL {
                let selected = panel.tab == tab;
                let galley = ui.painter().layout_no_wrap(
                    tab.label().to_owned(),
                    FontId::proportional(12.0),
                    TEXT,
                );
                let size = vec2(galley.size().x + 24.0, galley.size().y + 16.0);
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                let color = if selected {
                    ACCENT
                } else if response.hovered() {
                    TEXT
                } else {
                    MUTED_LIGHT
                };
                ui.painter().text(
                    rect.center() - vec2(0.0, 1.0),
                    Align2::CENTER_CENTER,
                    tab.label(),
                    FontId::proportional(12.0),
                    color,
                );
                if selected {
                    let underline =
                        Rect::from_min_max(pos2(rect.min.x, rect.max.y - 2.0), rect.max);
                    ui.painter().rect_filled(underline, 0.0, ACCENT);
                }
                if response.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
                if response.clicked() {
                    panel.tab = tab;
                }
            }
        });
    });
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, LINE);
}

// --- panel open: controls bar -----------------------------------------------------------------

/// A button of the controls bar. `outline` replaces the border colour (the selected slot).
pub(crate) fn button(
    ui: &mut Ui,
    label: &str,
    font: FontId,
    width: Option<f32>,
    enabled: bool,
    outline: Option<Color32>,
) -> bool {
    let color = if enabled { TEXT } else { DISABLED };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), color);
    let size = vec2(width.unwrap_or(galley.size().x + 24.0), 30.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let fill = if enabled && response.hovered() { CONTROL_HOVER } else { CONTROL };
    let border = outline.unwrap_or(if enabled { LINE_STRONG } else { DISABLED_LINE });
    ui.painter().rect(rect, 6.0, fill, Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, color);
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    enabled && response.clicked()
}

/// A 24-point button of the lists (Play, Verify, Remove); returns whether it was clicked.
pub(crate) fn small_button(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    let font = FontId::proportional(11.0);
    let color = if enabled { TEXT } else { DISABLED };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), color);
    let size = vec2(galley.size().x + 16.0, 24.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let fill = if enabled && response.hovered() { CONTROL_HOVER } else { CONTROL };
    let border = if enabled { LINE_STRONG } else { DISABLED_LINE };
    ui.painter().rect(rect, 5.0, fill, Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, color);
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    enabled && response.clicked()
}

/// A row of cells in one rounded border, one of them selected; returns the cell clicked.
fn segmented(
    ui: &mut Ui,
    id: &str,
    labels: &[String],
    selected: usize,
    cell: Vec2,
) -> Option<usize> {
    let size = vec2(cell.x * labels.len() as f32, cell.y);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let font = theme::mono(12.0);
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let cell_rect = Rect::from_min_size(rect.min + vec2(cell.x * i as f32, 0.0), cell);
        let response = ui.interact(cell_rect, Id::new((id, i)), Sense::click());
        let (fill, color) = if i == selected {
            (ACCENT_FILL, ACCENT)
        } else if response.hovered() {
            (CONTROL_HOVER, TEXT)
        } else {
            (CONTROL, MUTED_LIGHT)
        };
        ui.painter().rect_filled(cell_rect.shrink(1.0), 0.0, fill);
        ui.painter().text(cell_rect.center(), Align2::CENTER_CENTER, label, font.clone(), color);
        if i > 0 {
            ui.painter().vline(cell_rect.min.x, cell_rect.y_range(), Stroke::new(1.0, LINE_STRONG));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if response.clicked() {
            clicked = Some(i);
        }
    }
    // Round the outer corners: paint the border last, over the cells' square fills.
    ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, LINE_STRONG), StrokeKind::Inside);
    clicked
}

/// Runs `add_contents` as one row centred in the available width. The row's width is measured
/// the first time and remembered, so from the second frame on it is placed exactly.
fn centered_row(ui: &mut Ui, id: &str, add_contents: impl FnOnce(&mut Ui)) {
    let key = Id::new(("row-width", id));
    let width: f32 = ui.data(|d| d.get_temp(key)).unwrap_or(0.0);
    let available = ui.available_width();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let lead = ((available - width) / 2.0).max(0.0);
        ui.add_space(lead);
        let start = ui.cursor().min.x;
        add_contents(ui);
        let used = ui.min_rect().right() - start;
        ui.data_mut(|d| d.insert_temp(key, used));
    });
}

fn controls_bar(
    ui: &mut Ui,
    controls: &Controls,
    notice: Option<&str>,
    actions: &mut Vec<PanelAction>,
) {
    Panel::bottom("controls")
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(GROUND).inner_margin(Margin::symmetric(12, 12)))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let one_row = ui.available_width() >= ONE_ROW_WIDTH;
            if one_row {
                centered_row(ui, "all", |ui| {
                    transport(ui, controls, actions);
                    divider(ui, 20.0);
                    speed(ui, controls, actions);
                    divider(ui, 20.0);
                    states(ui, controls, actions);
                });
            } else {
                centered_row(ui, "transport", |ui| {
                    transport(ui, controls, actions);
                    divider(ui, 20.0);
                    speed(ui, controls, actions);
                });
                centered_row(ui, "states", |ui| states(ui, controls, actions));
            }
            centered_row(ui, "screenshots", |ui| screenshots(ui, notice, actions));
        });
}

fn transport(ui: &mut Ui, controls: &Controls, actions: &mut Vec<PanelAction>) {
    let font = FontId::proportional(12.0);
    let label = if controls.paused { "Continue" } else { "Pause" };
    if button(ui, label, font.clone(), None, true, None) {
        actions.push(PanelAction::TogglePause);
    }
    // A frame is refused while a verification runs: it has the machines.
    let frame = controls.paused && !controls.verifying;
    if button(ui, "Next frame", font.clone(), None, frame, None) {
        actions.push(PanelAction::NextFrame);
    }
    let steps = controls.paused && controls.can_step;
    for kind in StepKind::ALL {
        if button(ui, kind.label(), font.clone(), Some(44.0), steps, None) {
            actions.push(PanelAction::Debug(DebugAction::Step(kind)));
        }
    }
}

fn speed(ui: &mut Ui, controls: &Controls, actions: &mut Vec<PanelAction>) {
    let labels: Vec<String> =
        Speed::ALL.iter().map(|s| format!("\u{d7}{}", s.multiplier())).collect();
    let selected = Speed::ALL.iter().position(|s| *s == controls.speed).unwrap_or(0);
    if let Some(i) = segmented(ui, "speed", &labels, selected, vec2(38.0, 30.0)) {
        actions.push(PanelAction::SetSpeed(Speed::ALL[i]));
    }
}

/// The screenshot buttons, and the line that tells where the last one went.
fn screenshots(ui: &mut Ui, notice: Option<&str>, actions: &mut Vec<PanelAction>) {
    let font = FontId::proportional(12.0);
    ui.label(RichText::new("Screenshot").size(12.0).color(MUTED));
    ui.spacing_mut().item_spacing.x = 8.0;
    if button(ui, "Save screenshot", font.clone(), None, true, None) {
        actions.push(PanelAction::Screenshot);
    }
    if button(ui, "Save 1x frame", font, None, true, None) {
        actions.push(PanelAction::ScreenshotFrame);
    }
    if let Some(text) = notice {
        ui.label(RichText::new(text).size(11.0).color(TOAST_TEXT));
    }
}

fn states(ui: &mut Ui, controls: &Controls, actions: &mut Vec<PanelAction>) {
    let font = FontId::proportional(12.0);
    ui.label(RichText::new("State").size(12.0).color(MUTED));
    ui.spacing_mut().item_spacing.x = 4.0;
    for slot in SLOTS {
        let selected = slot == controls.slot;
        let outline = selected.then_some(ACCENT);
        if button(ui, &slot.to_string(), theme::mono(12.0), Some(30.0), true, outline) {
            actions.push(PanelAction::SelectSlot(slot));
        }
    }
    ui.spacing_mut().item_spacing.x = 8.0;
    if button(ui, "Save", font.clone(), None, true, None) {
        actions.push(PanelAction::SaveState);
    }
    if button(ui, "Load", font, None, controls.load_allowed, None) {
        actions.push(PanelAction::LoadState);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::Tab;
    use crate::test_support::sram_counter_machine;

    fn run_frame(
        ctx: &egui::Context,
        size: Vec2,
        panel: &mut PanelState,
        controls: &Controls,
        events: Vec<egui::Event>,
    ) -> Vec<PanelAction> {
        let (machine, _store) = sram_counter_machine();
        run_with(ctx, size, panel, controls, events, &machine, None)
    }

    /// One frame of the window over `machine`, with `symbols` if any.
    fn run_with(
        ctx: &egui::Context,
        size: Vec2,
        panel: &mut PanelState,
        controls: &Controls,
        events: Vec<egui::Event>,
        machine: &Machine,
        symbols: Option<&Symbols>,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)),
            events,
            time: Some(1.0),
            ..egui::RawInput::default()
        };
        let history = FrameHistory::default();
        let registers = machine.cpu_registers();
        let view = View {
            status: Status {
                frame: 4812,
                fps: 60.0,
                rust_percent: Some(0.7),
                means_missing: false,
                freezes: 1,
                live: None,
            },
            controls,
            registers: &registers,
            recording: false,
            verify_labels: None,
            machine,
            symbols,
            history: &history,
            touch: None,
        };
        let games = [GameImage {
            id: egui::TextureId::default(),
            width: 256,
            height: 224,
            pixel_aspect: 1.0,
        }];
        let output = ctx.run_ui(raw, |ui| draw(ui, &view, &games, panel, &mut actions, None));
        assert!(output.shapes.len() > 1, "something was drawn");
        output.drop_without_applying_deltas();
        actions
    }

    #[test]
    fn the_routines_and_memory_tabs_draw_the_routines_and_symbols_of_a_hooked_machine() {
        let ctx = ctx();
        let symbols =
            Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml")).unwrap();
        let (mut machine, _store) = crate::test_support::sram_counter_machine();
        let switch = crate::hook_setup::HookSwitch::load(
            Some(std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../re/symbols.toml"
            ))),
            None,
        )
        .unwrap();
        switch.apply(&mut machine, HooksArg::Verify).unwrap();
        let controls = Controls::new(HooksArg::Verify, HooksSwitch::Allowed, true, true);
        let size = vec2(1440.0, 900.0);
        assert!(tab_routines::rows_of(&machine, Some(&symbols)).len() > 1, "the routines listed");
        for tab in [Tab::Routines, Tab::Memory] {
            let mut panel = PanelState::default();
            panel.open_tab(tab);
            panel.memory.select(0x7E_1400, "side_record_a");
            let actions =
                run_with(&ctx, size, &mut panel, &controls, vec![], &machine, Some(&symbols));
            assert!(actions.is_empty(), "{tab:?}: drawing alone asks for nothing");
        }
    }

    fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(pos), button(true), button(false)]
    }

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx
    }

    fn image_mesh(
        shapes: &[egui::epaint::ClippedShape],
        id: egui::TextureId,
    ) -> &egui::epaint::Mesh {
        shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == id => Some(mesh.as_ref()),
                _ => None,
            })
            .expect("the game image was drawn")
    }

    #[test]
    fn stretch_fills_phone_panels_without_cropping_with_or_without_a_gamepad() {
        use crate::touch::settings::{Picture, Settings};

        let ctx = ctx();
        let controls = Controls::new(HooksArg::Original, HooksSwitch::Allowed, true, true);
        let (machine, _store) = sram_counter_machine();
        let history = FrameHistory::default();
        let registers = machine.cpu_registers();
        let pictures = [Picture::Standard, Picture::Large, Picture::Full, Picture::Stretch];
        for size in
            [vec2(1600.0, 900.0), vec2(2100.0, 900.0), vec2(900.0, 1600.0), vec2(900.0, 2100.0)]
        {
            let screen = Rect::from_min_size(pos2(0.0, 0.0), size);
            for (width, height, pixel_aspect) in [(256, 224, 8.0 / 7.0), (1488, 896, 1.0)] {
                let games =
                    [GameImage { id: egui::TextureId::User(42), width, height, pixel_aspect }];
                for picture in pictures {
                    for gamepad in [false, true] {
                        let settings = Settings { picture, ..Settings::default() };
                        let mut touch = crate::touch::Touch::new(true, settings, screen);
                        touch.set_status("x1", 1, gamepad, false);
                        let view = View {
                            status: Status::default(),
                            controls: &controls,
                            registers: &registers,
                            recording: false,
                            verify_labels: None,
                            machine: &machine,
                            symbols: None,
                            history: &history,
                            touch: Some(&touch),
                        };
                        let raw = egui::RawInput {
                            screen_rect: Some(screen),
                            time: Some(1.0),
                            ..egui::RawInput::default()
                        };
                        let mut panel = PanelState::default();
                        let mut actions = Vec::new();
                        let output = ctx.run_ui(raw, |ui| {
                            draw(ui, &view, &games, &mut panel, &mut actions, None);
                        });
                        let mesh = image_mesh(&output.shapes, games[0].id);
                        let expected = if picture == Picture::Stretch {
                            screen
                        } else {
                            let whole = !(crate::touch::wanted() && height > layout::MACHINE_LINES);
                            layout::picture_rect(
                                ctx.pixels_per_point(),
                                touch.picture_area().unwrap_or(screen),
                                width,
                                height,
                                pixel_aspect,
                                whole,
                            )
                        };
                        assert_eq!(mesh.calc_bounds(), expected, "{size:?} {picture:?}");
                        assert_eq!(mesh.vertices.len(), 4);
                        for uv in [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(0.0, 1.0), pos2(1.0, 1.0)] {
                            assert!(mesh.vertices.iter().any(|vertex| vertex.uv == uv));
                        }
                        output.drop_without_applying_deltas();
                    }
                }
            }
        }
    }

    #[test]
    fn stretched_screenshots_and_hd_layers_use_the_same_full_panel_rectangle() {
        let ctx = ctx();
        let games = [GameImage {
            id: egui::TextureId::User(42),
            width: 256,
            height: 224,
            pixel_aspect: 8.0 / 7.0,
        }];
        for size in [vec2(2100.0, 900.0), vec2(900.0, 2100.0)] {
            let screen = Rect::from_min_size(pos2(0.0, 0.0), size);
            let raw = egui::RawInput { screen_rect: Some(screen), ..egui::RawInput::default() };
            let output = ctx.run_ui(raw.clone(), |ui| draw_game(ui, &games, None, true));
            assert_eq!(image_mesh(&output.shapes, games[0].id).calc_bounds(), screen);
            output.drop_without_applying_deltas();
            let list = DrawList::new();
            let mut art = hd::Art::empty();
            for menu in [false, true] {
                let output = ctx.run_ui(raw.clone(), |ui| {
                    let layer = Layer { list: &list, art: &mut art, menu };
                    draw_game(ui, &games, Some(layer), true);
                });
                let picture: Option<(Rect, Rect)> =
                    ctx.data(|data| data.get_temp(Id::new(HD_PICTURE)));
                assert_eq!(picture.map(|(rect, _)| rect), Some(screen));
                output.drop_without_applying_deltas();
            }
        }
    }

    #[test]
    fn the_closed_window_draws_the_pill_and_a_click_on_it_opens_the_panel() {
        let ctx = ctx();
        let mut panel = PanelState::default();
        let mut controls = Controls::new(HooksArg::Original, HooksSwitch::Allowed, true, true);
        controls.debug = true;
        let size = vec2(1170.0, 896.0);
        // The first frames measure the pill; anchored areas settle on the second.
        run_frame(&ctx, size, &mut panel, &controls, vec![]);
        run_frame(&ctx, size, &mut panel, &controls, vec![]);
        let pill = ctx.memory(|m| m.area_rect(Id::new("pill"))).expect("the pill is drawn");
        assert!(
            (pill.max.x - (size.x - 12.0)).abs() < 1.0 && (pill.min.y - 12.0).abs() < 1.0,
            "{pill:?}"
        );
        let pos = pill.center();
        run_frame(&ctx, size, &mut panel, &controls, vec![egui::Event::PointerMoved(pos)]);
        let actions = run_frame(&ctx, size, &mut panel, &controls, click_at(pos));
        assert_eq!(actions, [PanelAction::TogglePanel]);
    }

    #[test]
    fn without_the_debug_tools_the_closed_window_draws_no_pill() {
        let ctx = ctx();
        let mut panel = PanelState::default();
        let controls = Controls::new(HooksArg::Original, HooksSwitch::Allowed, true, true);
        assert!(!controls.debug, "the debug tools are off by default");
        let size = vec2(1170.0, 896.0);
        run_frame(&ctx, size, &mut panel, &controls, vec![]);
        run_frame(&ctx, size, &mut panel, &controls, vec![]);
        assert_eq!(ctx.memory(|m| m.area_rect(Id::new("pill"))), None, "no pill");
        // Where the pill would be, a click asks for nothing.
        let pos = pos2(size.x - 60.0, 24.0);
        run_frame(&ctx, size, &mut panel, &controls, vec![egui::Event::PointerMoved(pos)]);
        let actions = run_frame(&ctx, size, &mut panel, &controls, click_at(pos));
        assert_eq!(actions, Vec::<PanelAction>::new());
    }

    /// The texts of the shapes drawn.
    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    #[test]
    fn the_touch_fps_line_is_drawn_only_with_the_debug_tools() {
        let ctx = ctx();
        let controls = Controls::new(HooksArg::Original, HooksSwitch::Allowed, true, true);
        let (machine, _store) = sram_counter_machine();
        let history = FrameHistory::default();
        let registers = machine.cpu_registers();
        let games = [GameImage {
            id: egui::TextureId::default(),
            width: 256,
            height: 224,
            pixel_aspect: 1.0,
        }];
        let line = "60.0 fps \u{b7} sound 42 ms \u{b7} 0 gaps";
        let shown = |debug: bool| {
            // Saved on before: only the debug tools draw it.
            let settings =
                crate::touch::settings::Settings { show_fps: true, ..Default::default() };
            let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(932.0, 430.0));
            let mut touch = crate::touch::Touch::new(true, settings, screen);
            touch.stats = Some(line.to_owned());
            touch.debug = debug;
            let view = View {
                status: Status::default(),
                controls: &controls,
                registers: &registers,
                recording: false,
                verify_labels: None,
                machine: &machine,
                symbols: None,
                history: &history,
                touch: Some(&touch),
            };
            let raw = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(1.0),
                ..egui::RawInput::default()
            };
            let mut panel = PanelState::default();
            let mut actions = Vec::new();
            let output =
                ctx.run_ui(raw, |ui| draw(ui, &view, &games, &mut panel, &mut actions, None));
            let drawn = texts(&output.shapes);
            output.drop_without_applying_deltas();
            drawn
        };
        let off = shown(false);
        assert!(!off.iter().any(|text| text.contains("fps")), "no fps line: {off:?}");
        let on = shown(true);
        assert!(on.iter().any(|text| text == line), "the fps line with the debug tools: {on:?}");
    }

    #[test]
    fn the_pill_shows_the_sound_gaps_and_the_battle_speed_of_the_window_frame() {
        let ctx = ctx();
        let mut controls = Controls::new(HooksArg::Original, HooksSwitch::Allowed, true, true);
        controls.debug = true;
        let (machine, _store) = sram_counter_machine();
        let history = FrameHistory::default();
        let registers = machine.cpu_registers();
        let games = [GameImage {
            id: egui::TextureId::default(),
            width: 256,
            height: 224,
            pixel_aspect: 1.0,
        }];
        let shown = |live: Live| {
            let view = View {
                status: Status { fps: 60.0, live: Some(live), ..Status::default() },
                controls: &controls,
                registers: &registers,
                recording: false,
                verify_labels: None,
                machine: &machine,
                symbols: None,
                history: &history,
                touch: None,
            };
            let mut panel = PanelState::default();
            let mut texts_drawn = Vec::new();
            // The first frame measures the pill; anchored areas settle on the second.
            for _ in 0..2 {
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1170.0, 896.0))),
                    time: Some(1.0),
                    ..egui::RawInput::default()
                };
                let mut actions = Vec::new();
                let output =
                    ctx.run_ui(raw, |ui| draw(ui, &view, &games, &mut panel, &mut actions, None));
                texts_drawn = texts(&output.shapes);
                output.drop_without_applying_deltas();
            }
            texts_drawn
        };
        let on = shown(Live {
            queued_ms: 42.4,
            gaps: 7,
            tempo: Some((3, 2)),
            requested: 2,
            run: 1,
            slow: 4,
            slowest_ms: 312.6,
        });
        assert!(on.iter().any(|text| text == "60 fps"), "{on:?}");
        assert!(
            on.iter().any(|text| text
                == "sound 42 ms \u{b7} 7 gaps \u{b7} battle 1.5x \u{b7} 2/1 frames \u{b7} slow 4 \u{b7} max 313 ms"),
            "{on:?}"
        );
        let off = shown(Live {
            queued_ms: 30.0,
            gaps: 3,
            tempo: None,
            requested: 1,
            run: 1,
            slow: 0,
            slowest_ms: 0.0,
        });
        assert!(
            off.iter().any(|text| text
                == "sound 30 ms \u{b7} 3 gaps \u{b7} battle 1x \u{b7} slow 0 \u{b7} max 0 ms"),
            "{off:?}"
        );
        let double = shown(Live {
            queued_ms: 120.0,
            gaps: 0,
            tempo: Some((2, 1)),
            requested: 2,
            run: 2,
            slow: 0,
            slowest_ms: 0.0,
        });
        assert!(
            double.iter().any(|text| text
                == "sound 120 ms \u{b7} 0 gaps \u{b7} battle 2x \u{b7} 2/2 frames \u{b7} slow 0 \u{b7} max 0 ms"),
            "{double:?}"
        );
    }

    #[test]
    fn the_open_window_draws_every_tab_without_actions_of_its_own() {
        let ctx = ctx();
        let controls =
            Controls::new(HooksArg::Rust, HooksSwitch::Locked("recording"), false, false);
        for tab in Tab::ALL {
            let mut panel = PanelState::default();
            panel.open_tab(tab);
            for size in [vec2(1440.0, 900.0), vec2(700.0, 450.0)] {
                assert_eq!(
                    run_frame(&ctx, size, &mut panel, &controls, vec![]),
                    Vec::<PanelAction>::new()
                );
            }
        }
    }

    #[test]
    fn a_playing_replay_and_missing_means_draw_their_notes_without_actions() {
        let ctx = ctx();
        let controls = Controls::new(HooksArg::Rust, HooksSwitch::Allowed, false, false);
        let (machine, _store) = sram_counter_machine();
        let history = FrameHistory::default();
        let mut panel = PanelState::default();
        panel.open_tab(Tab::Replays);
        panel.replays.playing = Some("battle".to_owned());
        let status = Status { means_missing: true, ..Status::default() };
        let registers = machine.cpu_registers();
        let view = View {
            status,
            controls: &controls,
            registers: &registers,
            recording: false,
            verify_labels: None,
            machine: &machine,
            symbols: None,
            history: &history,
            touch: None,
        };
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1440.0, 900.0))),
            time: Some(1.0),
            ..egui::RawInput::default()
        };
        let mut actions = Vec::new();
        let games = [GameImage {
            id: egui::TextureId::default(),
            width: 256,
            height: 224,
            pixel_aspect: 1.0,
        }];
        let output = ctx.run_ui(raw, |ui| draw(ui, &view, &games, &mut panel, &mut actions, None));
        assert!(output.shapes.len() > 1, "something was drawn");
        output.drop_without_applying_deltas();
        assert!(actions.is_empty(), "drawing asks for nothing: {actions:?}");
    }

    #[test]
    fn a_toast_shows_while_closed_and_its_open_link_asks_for_the_tab() {
        let ctx = ctx();
        let mut panel = PanelState::default();
        let controls = Controls::new(HooksArg::Verify, HooksSwitch::Allowed, true, true);
        panel.push_toast("Divergence in x - saved for repro".to_owned(), Tab::Replays, 0.5);
        let size = vec2(1170.0, 896.0);
        run_frame(&ctx, size, &mut panel, &controls, vec![]);
        run_frame(&ctx, size, &mut panel, &controls, vec![]);
        // "Open" is the last item of the toast: just inside its right margin.
        let toast = ctx.memory(|m| m.area_rect(Id::new("toast"))).expect("the toast is drawn");
        let pos = pos2(toast.max.x - 12.0 - 6.0, toast.center().y);
        run_frame(&ctx, size, &mut panel, &controls, vec![egui::Event::PointerMoved(pos)]);
        let actions = run_frame(&ctx, size, &mut panel, &controls, click_at(pos));
        assert_eq!(actions, [PanelAction::OpenTab(Tab::Replays)]);
    }
}
