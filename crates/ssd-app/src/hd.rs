//! The HD menus in the window (Enhanced mode). The menu driver of `ssd-game` decides what the HD
//! shows and which presses the original menu gets; `ssd-menus` draws the screen as a draw list, and
//! this layer paints it with egui over the game's frame area. The menu logic stays the original's.
//!
//! The names and texts come from the catalogue; the pictures of the objects are decoded from the
//! cached ROM the first time they are drawn and kept as egui textures for the run. The field as it
//! was when the menu opened is kept too: it is the backdrop the HD paints behind its windows.

use crate::canvas::GameView;
use crate::filter::{self, Shape};
use crate::paths;
use crate::theme;
use anyhow::Context;
use ssd_catalogue::{Catalogue, Portrait, RomView, face_portrait, object_portrait};
use ssd_game::Ruleset;
use ssd_game::cheats::BLOCK;
use ssd_game::data as d;
use ssd_game::menu::driver::{Binding, Curtain, Driver, Frame as HdFrame};
use ssd_game::menu::{SAVE_LIST_STEP, SAVE_SCREEN};
use ssd_hd::world::{SharedHd, WideFrame, WideHd, WideRun};
use ssd_menus::bag::Sidecar;
use ssd_menus::cheats::{CheatSidecar, MARKER, MARKER_BYTES, SUSPENDED};
use ssd_menus::layout::{self, palette};
use ssd_menus::mods_state::ModsSidecar;
use ssd_menus::screens::SlotList;
use ssd_menus::{CardPart, Color, DrawList, Key, Language, Op, Picture, Rect};
use ssd_oracle::{Frame, FrameLog, Joypad};
use std::collections::HashMap;
use std::path::PathBuf;

/// The sidecar file of the bags, beside the battery save.
const SIDECAR: &str = "ssd.bag";
/// The sidecar file of the cheats of the save slots, beside the bags.
const CHEATS_SIDECAR: &str = "ssd.cheats";
/// The sidecar file of the mod state of the save slots (the extended flags and the high parts), beside the bags.
const MODS_SIDECAR: &str = "ssd.mods";
/// The gate of the cheats in work RAM: the suspended byte of the cheat block (nonzero while a scripted fight
/// has the battle cheats off).
const GATE: usize = (BLOCK - 0x7E_0000) as usize + SUSPENDED;
/// Where the block's marker starts in work RAM: before a run writes the block, the power-on fill is no gate.
const BLOCK_MARKER: usize = (BLOCK - 0x7E_0000) as usize + MARKER;
/// The frames a notice of the cheats stays on the field: about two seconds of the game's sixty.
const NOTICE_FRAMES: u32 = 120;
/// The notices: a scripted fight suspends the battle cheats, and the map brings them back.
const OFF_NOTICE: &str = "Cheats off for this fight";
const ON_NOTICE: &str = "Cheats back on";

/// The notices of the cheats on the field, from the gate of the cheat block: one comes when a scripted fight
/// suspends the battle cheats, and one when the map brings them back. A notice stays for [`NOTICE_FRAMES`]
/// frames. A gate that stays up through a fight shows its notice once.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CheatNotice {
    /// Whether the battle cheats were suspended at the last frame.
    suspended: bool,
    /// The notice showing, with the frames it has left.
    shown: Option<(&'static str, u32)>,
}

impl CheatNotice {
    /// Moves the notice on by one frame of `wram`.
    pub fn note(&mut self, wram: &[u8]) {
        let live =
            wram.get(BLOCK_MARKER..BLOCK_MARKER + MARKER_BYTES.len()) == Some(&MARKER_BYTES[..]);
        let suspended = live && wram.get(GATE).is_some_and(|&gate| gate != 0);
        if suspended != self.suspended {
            self.shown = Some((if suspended { OFF_NOTICE } else { ON_NOTICE }, NOTICE_FRAMES));
        }
        self.suspended = suspended;
        if let Some((_, frames)) = self.shown.as_mut() {
            *frames = frames.saturating_sub(1);
            if *frames == 0 {
                self.shown = None;
            }
        }
    }

    /// The text of the notice showing, if one is.
    #[must_use]
    pub fn text(&self) -> Option<&'static str> {
        self.shown.map(|(text, _)| text)
    }
}

/// The game mode of the title's "Which slot?" list (the Continue's slot list).
const TITLE_SLOTS_MODE: u8 = 0x0E;

/// The save-slot list the original shows, from work RAM: the title's Continue list by the game mode, and the Save's
/// slot list by the menu's screen and step. Other steps of the Save (its question and its completion) show none.
#[must_use]
pub fn slot_list(wram: &[u8]) -> Option<SlotList> {
    let byte = |address: u32| wram.get((address - 0x7E_0000) as usize).copied();
    if byte(d::GAME_MODE) == Some(TITLE_SLOTS_MODE) {
        return Some(SlotList::Continue);
    }
    (byte(d::MENU_SCREEN) == Some(SAVE_SCREEN) && byte(d::MENU_STEP) == Some(SAVE_LIST_STEP))
        .then_some(SlotList::Save)
}

/// Whether a battery save holds at least one game (a good slot, or one the backup restores).
fn holds_a_game(sram: &[u8]) -> bool {
    ssd_save::SaveFile::from_bytes(sram).is_ok_and(|file| {
        (0..ssd_save::SLOT_COUNT).any(|slot| {
            matches!(
                file.status(slot),
                Ok(ssd_save::SlotStatus::Good | ssd_save::SlotStatus::FromBackup)
            )
        })
    })
}

/// The HD layer of a run: the menu driver, with the names it shows and the textures it paints, and
/// the 16:9 view of the game's frames the menus are drawn over.
pub struct Hd {
    driver: Driver,
    art: Art,
    wide: WideRun,
    /// The cheats' notices on the field.
    notice: CheatNotice,
    /// The names and texts of the menus in English, as the catalogue has them: a translation is made from them.
    labels: ssd_menus::Labels,
    /// The names and texts of the menus in the language of the text, as the driver shows them: the save editor opens
    /// with them.
    shown_labels: ssd_menus::Labels,
    /// The language the PC file held when it was last seen: a change of the Options' row is measured from it, so
    /// another row's change does not undo `--lang`.
    language_saved: Option<Language>,
    /// The menus' words in the language of the text (`None`: English).
    ui_text: Option<std::sync::Arc<ssd_menus::ui_text::UiText>>,
    /// The canonical ROM: the names a translation draws over the tile maps are read from it.
    rom: std::sync::Arc<[u8]>,
    /// The save editor, while it is open (the title's Select opens it, see [`Hd::key`]).
    editor: Option<ssd_menus::saveedit::Editor>,
    /// Select was pressed on the title: the editor opens at the next frame, which has the cartridge save.
    editor_wanted: bool,
    /// Whether the editor may open: not while a replay plays or records (the cartridge save is the replay's).
    editor_allowed: bool,
    /// The level cap of the enabled packs (see [`Hd::set_level_cap`]).
    level_cap: Option<u16>,
}

/// The game mode the boot code sets before it calls the title routine, which runs inside the boot code until the player starts
/// (`docs/re/data/title-chapters.md`, "Boot"): the mode stays `$87` for the whole title, with no party yet (the first record's flags
/// and the party size at `$01BB` are zero, as the boot code clears them), and the main loop changes it as soon as the title returns.
const TITLE_MODE: u8 = 0x87;
const TITLE_PARTY_FLAGS: usize = 0x0B00;
const TITLE_PARTY_COUNT: usize = 0x01BB;

/// Whether the title screen is what the game runs: the title's mode and no party.
#[must_use]
pub fn on_title(wram: &[u8]) -> bool {
    let byte = |at: usize| wram.get(at).copied();
    byte((d::GAME_MODE - 0x7E_0000) as usize) == Some(TITLE_MODE)
        && byte(TITLE_PARTY_FLAGS) == Some(0)
        && byte(TITLE_PARTY_COUNT) == Some(0)
}

/// The editor's key of an HD key (the Start button arrives as [`Hd::close_menu`]).
fn editor_key(key: Key) -> Option<ssd_menus::saveedit::Key> {
    use ssd_menus::saveedit::Key as E;
    Some(match key {
        Key::Up => E::Up,
        Key::Down => E::Down,
        Key::Left => E::Left,
        Key::Right => E::Right,
        Key::Confirm => E::A,
        Key::Cancel => E::B,
        Key::Tab => E::R,
        Key::TabBack => E::L,
        Key::ResetPage => E::Y,
        Key::Apply => E::X,
        Key::ResetAll => return None,
    })
}

impl Hd {
    /// The HD layer for the canonical ROM `rom`: its catalogue gives the names and texts, the bag (of the
    /// ruleset's rules) binds to the saves with the sidecar file of the saves directory, and the cheats of the
    /// ruleset are kept in their sidecar file the same way. The mod state of the saves is kept in its own file.
    ///
    /// # Errors
    ///
    /// The catalogue cannot be read from the ROM (it is not the canonical image), or a sidecar file
    /// exists and cannot be read.
    pub fn new(rom: &[u8], ruleset: Ruleset) -> anyhow::Result<Self> {
        let catalogue = Catalogue::from_rom(rom)?;
        let sidecar = read_sidecar()?;
        let labels = ssd_catalogue::menu_labels(&catalogue);
        let driver = Driver::with_bag(labels.clone(), ruleset.bag(), sidecar);
        let driver =
            if ruleset.cheats() { driver.with_cheats(read_cheats_sidecar()?) } else { driver };
        // Read whatever the ruleset: a Save writes the file either way, and a driver without the entries would
        // write the file with the one slot's entry only.
        let driver = driver.with_mods(read_mods_sidecar()?);
        // The data packs the Data packs screen lists (empty outside an Enhanced run).
        let driver = driver.with_packs(crate::packs::catalog());
        let driver =
            driver.with_options(crate::options::read_slots()?, crate::options::read_settings()?);
        let language_saved = Language::from_values(&driver.options().pc);
        let language = crate::lang::effective(&driver.options().pc);
        let mut hd = Self {
            driver,
            art: Art {
                rom: rom.to_vec(),
                pack: crate::hd_art::pack(rom),
                cards: crate::hd_art::pack(rom).and_then(|pack| {
                    match ssd_hd::CardArt::load(&pack.dir) {
                        Ok(cards) => cards,
                        Err(error) => {
                            log::warn!("the HD card art: {error}");
                            None
                        }
                    }
                }),
                card_textures: HashMap::new(),
                symbol_textures: HashMap::new(),
                hand_textures: HashMap::new(),
                textures: HashMap::new(),
                field: None,
                backdrop_shape: Shape::default(),
                backdrop: None,
            },
            wide: {
                let mut wide = WideRun::new(&crate::hd_art::world(rom));
                wide.set_art(crate::hd_art::pack(rom));
                wide.set_font(crate::hd_art::font(rom));
                wide.set_lightning(crate::hd_art::lightning());
                wide
            },
            notice: CheatNotice::default(),
            shown_labels: labels.clone(),
            labels,
            language_saved,
            ui_text: None,
            rom: std::sync::Arc::from(rom),
            editor: None,
            editor_wanted: false,
            editor_allowed: true,
            level_cap: None,
        };
        hd.set_language(language);
        Ok(hd)
    }

    /// Draws the game's text in `language` from the next frame on.
    pub fn set_language(&mut self, language: Language) {
        let table = crate::lang::translation(language);
        self.shown_labels = match &table {
            Some(table) => ssd_catalogue::translated_labels(&self.labels, table),
            None => self.labels.clone(),
        };
        self.driver.set_labels(self.shown_labels.clone());
        self.wide.set_overlay(crate::lang::overlay(language, &self.rom));
        self.ui_text = crate::lang::ui_text(language);
    }

    /// The PC file now holds `values` (the Options changed a row): a new choice of the Language row takes effect at
    /// once, and wins over `--lang` from then on; a reset row takes the language a run starts in
    /// ([`crate::lang::followed`]). The tables are read again only when the stored language changed.
    pub fn follow_language(&mut self, values: &ssd_menus::options::Values) {
        let saved = Language::from_values(values);
        if saved != self.language_saved {
            self.language_saved = saved;
            self.set_language(crate::lang::followed(values));
        }
    }

    /// The machine's state was replaced (a state loaded): see [`Driver::reset`].
    pub fn reset(&mut self) {
        self.driver.reset();
    }

    /// A replay plays: the bag is bound by the replay's recorded events, not by the saves.
    pub fn set_replaying(&mut self) {
        self.driver.set_binding(Binding::Recorded);
    }

    /// Writes the sidecar file of the bags (after a save changed one).
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save_sidecar(&self) -> anyhow::Result<()> {
        write_beside(&sidecar_path(), &self.driver.sidecar().to_bytes())
    }

    /// Writes the sidecar file of the cheats (after a save changed one).
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save_cheats_sidecar(&self) -> anyhow::Result<()> {
        write_beside(&cheats_sidecar_path(), &self.driver.cheat_sidecar().to_bytes())
    }

    /// Writes the sidecar file of the mod state (after a save wrote one).
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save_mods_sidecar(&self) -> anyhow::Result<()> {
        write_beside(&mods_sidecar_path(), &self.driver.mods_sidecar().to_bytes())
    }

    /// Writes the options of the save slots (after a save changed one).
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save_options_sidecar(&self) -> anyhow::Result<()> {
        crate::options::write_slots(self.driver.option_slots())
    }

    /// The cheats' notices, from the gate of the cheat block (see [`CheatNotice::note`]), once a frame.
    pub fn note_cheats(&mut self, wram: &[u8]) {
        self.notice.note(wram);
    }

    /// The HD's picture over the field while no menu is on screen: the cheats' notice, while one shows, the
    /// CHEATS badges of the save-slot list the original shows (see [`slot_list`]), for the slots saved with a cheat on,
    /// and the count of the stack under the shop's Sell cursor (see [`Driver::sell_count`]).
    #[must_use]
    pub fn overlay(&self, wram: &[u8], sram: Option<&[u8]>) -> Option<DrawList> {
        let mut list = DrawList::new();
        if let Some(text) = self.notice.text() {
            list.append(ssd_menus::screens::notice(text));
        }
        // The HD Save screen has its own badges: the original's slot list runs unseen under it.
        let hd_save = self.driver.screen() == Some(ssd_menus::view::Screen::Save);
        if let (Some(which), Some(sram), false) = (slot_list(wram), sram, hd_save) {
            list.append(ssd_menus::screens::slot_badges(which, self.driver.marked_slots(sram)));
        }
        if let Some(count) = self.driver.sell_count(wram) {
            list.append(ssd_menus::screens::sell_count(count));
        }
        if self.editor.is_none()
            && self.editor_allowed
            && on_title(wram)
            && sram.is_some_and(holds_a_game)
        {
            list.append(ssd_menus::saveedit::title_strip());
        }
        (!list.ops().is_empty()).then(|| {
            crate::options::access::restyle(&self.driver.options().pc, self.translated(list, None))
        })
    }

    /// `list` with the menus' words in the language of the text, on the screen `context` (see [`UiText::get_in`]).
    ///
    /// [`UiText::get_in`]: ssd_menus::ui_text::UiText::get_in
    fn translated(&self, mut list: DrawList, context: Option<&str>) -> DrawList {
        if let Some(table) = &self.ui_text {
            list.map_text(|text| table.get_in(context, text));
        }
        list
    }

    /// The Item screen shows `tab` of the bag.
    pub fn set_tab(&mut self, tab: ssd_menus::view::Tab) {
        self.driver.set_tab(tab);
    }

    /// The HD's frame: the joypad and the host events of the frame, given the work RAM and the
    /// battery save (see [`Driver::frame`]).
    pub fn frame(&mut self, wram: &[u8], sram: Option<&[u8]>, live: Joypad) -> HdFrame {
        if std::mem::take(&mut self.editor_wanted)
            && let Some(sram) = sram
        {
            match ssd_menus::saveedit::Editor::open(
                sram,
                &self.shown_labels,
                self.driver.sidecar(),
                self.driver.marked_slots(sram),
            ) {
                Ok(mut editor) => {
                    editor.set_progression(self.level_cap, self.driver.mods_sidecar());
                    self.editor = Some(editor);
                }
                Err(error) => log::warn!("the save editor does not open: {error}"),
            }
        }
        if let Some(editor) = self.editor.as_mut() {
            if let Some(sram) = sram {
                editor.observe(sram);
            }
            editor.tick();
            editor.held(i8::from(live.right) - i8::from(live.left));
            if editor.finished() {
                self.editor = None;
            }
            // The game sees no button while the editor is open, nor on the frame that closes it.
            return HdFrame {
                pad: Joypad::default(),
                events: Vec::new(),
                saved: None,
                saved_cheats: None,
                saved_mods: None,
                saved_options: false,
                settings: None,
                video: None,
            };
        }
        self.driver.frame(wram, sram, live)
    }

    /// Whether the save editor may open now: not while a replay plays or records.
    pub fn allow_editor(&mut self, allowed: bool) {
        self.editor_allowed = allowed;
    }

    /// Tells the HD screens which level cap the enabled packs give (`ssd_game::mods::level_cap_in` on the machine's image): the
    /// Status screen reaches the levels past the game's own with it, and the save editor, when it opens, sets them.
    pub fn set_level_cap(&mut self, cap: Option<u16>) {
        self.level_cap = cap;
        self.driver.set_level_cap(cap);
    }

    /// The write the editor made, once: the host stores `commit.sram` in the machine's cartridge save (the editor's copy is
    /// ahead of it), and the sidecar entries of the slot are re-keyed here. Returns the commit for the host's write.
    pub fn take_editor_commit(&mut self) -> Option<ssd_menus::saveedit::Commit> {
        let commit = self.editor.as_mut()?.take_commit()?;
        commit.rekey(&mut self.driver.sidecars_mut());
        Some(commit)
    }

    /// A replay's recorded application event, applied to the bag (see [`Driver::replay_event`]).
    ///
    /// # Errors
    ///
    /// The payload is not what the event holds.
    pub fn replay_event(&mut self, tag: u8, payload: &[u8], wram: &[u8]) -> anyhow::Result<()> {
        self.driver.replay_event(tag, payload, wram).map(|_| ()).map_err(anyhow::Error::msg)
    }

    /// The driver of the menus: the curtain, the presses and the HD's screens.
    #[must_use]
    pub fn driver(&self) -> &Driver {
        &self.driver
    }

    /// The options of this PC the menus hold, to change from outside (the touch controls' layout).
    pub fn pc_options_mut(&mut self) -> &mut ssd_menus::options::Values {
        self.driver.pc_options_mut()
    }

    /// Whether a choice of the HD runs its presses now (see [`Driver::busy`]).
    #[must_use]
    pub fn busy(&self) -> bool {
        self.driver.busy()
    }

    /// Whether the running HD plan should run at [`crate::app::HD_BURST`] speed (see [`Driver::bursts`]).
    #[must_use]
    pub fn bursts(&self) -> bool {
        self.driver.bursts()
    }

    /// Whether the field is free: no menu has the curtain and no press is under way. Then the picture the window shows
    /// is the game's own and may be drawn a frame late.
    #[must_use]
    pub fn field_free(&self) -> bool {
        self.driver.curtain() == Curtain::Down && !self.driver.busy()
    }

    /// Opens the Cheats screen over a battle (see [`Driver::open_battle_cheats`]).
    pub fn open_battle_cheats(&mut self) -> bool {
        self.driver.open_battle_cheats()
    }

    /// A navigation key of the HD on the menu of `wram` (see [`Driver::key`]).
    pub fn key(&mut self, key: Key, wram: &[u8]) {
        if let Some(editor) = self.editor.as_mut() {
            if let Some(key) = editor_key(key) {
                editor.key(key);
            }
            return;
        }
        // Select on the title opens the save editor (nothing else of the HD lives there).
        if key == Key::ResetAll
            && self.editor_allowed
            && self.driver.curtain() == Curtain::Down
            && on_title(wram)
        {
            self.editor_wanted = true;
            return;
        }
        self.driver.key(key, wram);
    }

    /// Start on the Cards screen closes the menu (see [`Driver::close_menu`]).
    pub fn close_menu(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            editor.key(ssd_menus::saveedit::Key::Start);
            return;
        }
        self.driver.close_menu();
    }

    /// Whether the Controls page waits for a key or button, and for which device (see [`Driver::listening`]).
    #[must_use]
    pub fn listening(&self) -> Option<ssd_menus::options::controls::Device> {
        self.driver.listening()
    }

    /// A raw press for the key box that waits for one (see [`Driver::capture`]).
    pub fn capture(&mut self, capture: ssd_menus::options::controls::Capture) {
        self.driver.capture(capture);
    }

    /// The setup a button of the Options' Game page asked for, once (Android restarts the game into it).
    #[cfg(target_os = "android")]
    pub fn take_setup_request(&mut self) -> Option<ssd_menus::options::Opens> {
        self.driver.take_setup_request()
    }

    /// The request the Data packs screen made, once (see [`Driver::take_mods_request`]).
    pub fn take_mods_request(&mut self) -> Option<ssd_menus::packs::Request> {
        self.driver.take_mods_request()
    }

    /// Whether Y is held this frame (see [`Driver::set_moving`]).
    pub fn set_y_held(&mut self, held: bool) {
        self.driver.set_moving(held);
    }

    /// The HD menus' part of a save state (see [`Driver::session_bytes`]).
    pub fn session_bytes(&self) -> Option<Vec<u8>> {
        self.driver.session_bytes()
    }

    /// Puts back the HD menus' part of a state just loaded into `machine` (see [`Driver::restore_session`]).
    ///
    /// # Errors
    ///
    /// The bytes are not a session, or its writes do not fit in work RAM.
    pub fn restore_session(
        &mut self,
        bytes: &[u8],
        machine: &mut ssd_oracle::Machine,
    ) -> anyhow::Result<()> {
        let events =
            self.driver.restore_session(bytes, machine.wram()).map_err(anyhow::Error::msg)?;
        ssd_oracle::apply_events(machine, &events)?;
        Ok(())
    }

    /// A pack installed while the run goes on joins the Data packs screen's list (see [`Driver::add_pack`]).
    pub fn add_pack(&mut self, info: ssd_menus::packs::PackInfo) {
        self.driver.add_pack(info);
    }

    /// `list` as the HD's own windows over the field are shown: its words in the language of the text, in the
    /// Accessibility page's style (Esc's question, see [`crate::quit_confirm`]).
    #[must_use]
    pub fn dressed(&self, list: DrawList) -> DrawList {
        crate::options::access::restyle(&self.driver.options().pc, self.translated(list, None))
    }

    /// A line the Data packs screen shows until the next key (see [`Driver::set_mods_alert`]).
    pub fn set_mods_alert(&mut self, text: String) {
        self.driver.set_mods_alert(text);
    }

    /// The player's press on the field opens the menu (see [`Driver::open`]).
    pub fn open(&mut self) {
        self.driver.open();
    }

    /// The HD's picture of the menu now, if the curtain is up.
    #[must_use]
    pub fn scene(&self, wram: &[u8]) -> Option<DrawList> {
        if let Some(editor) = self.editor.as_ref() {
            return Some(self.translated(ssd_menus::saveedit::scene(editor), None));
        }
        let context = match self.driver.screen() {
            Some(ssd_menus::view::Screen::Options) => Some("options"),
            _ => None,
        };
        self.driver.scene(wram).map(|list| self.translated(list, context))
    }

    /// The 16:9 view of the frame that just ran (see [`WideRun::note`]); `None` when the frame has no
    /// render log or the view refuses it, and the window shows the PPU's own frame.
    pub fn note_frame(&mut self, wram: &[u8], log: Option<&FrameLog>) -> Option<WideFrame> {
        self.wide.note(wram, log)
    }

    /// Presentation belongs to the renderer, not to the emulated game's menu state.
    pub fn prepare_view(&mut self, wram: &[u8], narrow: bool) {
        self.wide.set_hide_field_ui(narrow && self.driver.travel_transition(wram));
    }

    /// The 16:9 view of the frame that just ran as the Enhanced picture (see [`WideRun::note_hd_shared`]): a frame whose inputs
    /// are those of the picture before is not drawn again.
    pub fn note_frame_hd_shared(
        &mut self,
        wram: &[u8],
        log: Option<&FrameLog>,
        options: ssd_hd::HdOptions,
    ) -> Option<SharedHd> {
        self.wide.note_hd_shared(wram, log, options)
    }

    /// The frames since Goku's first Super Saiyan transformation started, while it goes on (`None`: it does not), for the
    /// frame noted next: the lightning of `art-override\ssj-lightning\` is drawn over its Enhanced picture (see
    /// [`WideRun::set_ssj_lightning`]).
    pub fn set_ssj_lightning(&mut self, frames: Option<u32>) {
        self.wide.set_ssj_lightning(frames);
    }

    /// A frame that ran and is not drawn (a fast-forward's frame the window never shows): the 16:9
    /// view only follows its work RAM, so the frame drawn next starts from the right state.
    pub fn skip_frame(&mut self, wram: &[u8]) {
        self.wide.skip(wram);
    }

    /// The Enhanced picture for a fast-forward, which shows its pictures a window frame late:
    /// the frame is drawn in the background and the picture returned is the one drawn for the
    /// call before (see [`WideRun::redraw_hd_pipelined`]).
    pub fn redraw_frame_hd_pipelined(
        &mut self,
        log: Option<&FrameLog>,
        options: ssd_hd::HdOptions,
    ) -> Option<WideHd> {
        self.wide.redraw_hd_pipelined(log, options)
    }

    /// The Enhanced picture for a run at its own speed that shows its pictures a window frame late: as
    /// [`Hd::redraw_frame_hd_pipelined`], but a picture still being drawn is waited for, so none is dropped (see
    /// [`WideRun::redraw_hd_next`]).
    pub fn redraw_frame_hd_next(
        &mut self,
        log: Option<&FrameLog>,
        options: ssd_hd::HdOptions,
    ) -> Option<WideHd> {
        self.wide.redraw_hd_next(log, options)
    }

    /// Reads the pack's art on a thread of its own (see [`WideRun::set_art_async`]).
    pub fn set_art_async(&mut self, on: bool) {
        self.wide.set_art_async(on);
    }

    /// What the Enhanced pictures drawn since the last call cost, and whether the last was drawn in the background
    /// (see [`WideRun::take_cost`]).
    pub fn take_draw_cost(&mut self) -> Option<(ssd_hd::DrawCost, bool)> {
        self.wide.take_cost()
    }

    /// Whether a picture is being drawn in the background (or is done and not taken).
    #[must_use]
    pub fn drawing(&self) -> bool {
        self.wide.drawing()
    }

    /// Waits for the picture being drawn in the background and returns it: the picture of the last frame handed over.
    pub fn finish_drawing(&mut self) -> Option<WideHd> {
        self.wide.finish_pipelined()
    }

    /// Starts the thread that draws the pictures in the background now, and returns its id for the Performance Hint
    /// session (Android; `None` elsewhere): the session has to name the threads that work for the frame.
    pub fn start_draw_worker(&mut self) -> Option<i32> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.wide.set_worker_start(std::sync::Arc::new(move || {
            // (the window loop is waiting for this on the other end; a send to a gone receiver is nothing)
            let _ = tx.send(crate::perf_hint::current_thread());
        }));
        self.wide.start_worker();
        rx.recv().ok().flatten()
    }

    /// The 16:9 view of the frame last skipped ([`Hd::skip_frame`]), after all.
    pub fn redraw_frame(&mut self, log: Option<&FrameLog>) -> Option<WideFrame> {
        self.wide.redraw(log)
    }

    /// The Enhanced picture of the frame last skipped, sharing its picture (see [`WideRun::redraw_hd_shared`]).
    pub fn redraw_frame_hd_shared(
        &mut self,
        log: Option<&FrameLog>,
        options: ssd_hd::HdOptions,
    ) -> Option<SharedHd> {
        self.wide.redraw_hd_shared(log, options)
    }

    /// [`Hd::note_field`] for an Enhanced picture: the backdrop is the picture as it is drawn (kept, not copied).
    pub fn note_field_picture(&mut self, picture: &SharedHd) {
        if self.driver.curtain() == Curtain::Down && !self.driver.busy() {
            self.art.note_field_shared(picture);
        }
    }

    /// Keeps the frame just rendered as the backdrop, while the field is free and no menu press is
    /// under way: the menu opens over it, and the field does not move while the menu is open.
    pub fn note_field(&mut self, frame: &Frame<'_>) {
        if self.driver.curtain() == Curtain::Down && !self.driver.busy() {
            self.art.note_field(frame);
        }
    }

    /// The textures for painting, with the same last pass on the backdrop as on the live picture.
    #[must_use]
    pub fn art(&mut self, shape: Shape) -> &mut Art {
        self.art.backdrop_shape = shape;
        &mut self.art
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        tests::presentation_hd()
    }
}

/// The frame the menu opens over: the Enhanced picture it was drawn as (kept by reference, not copied), or the
/// PPU's frame as RGBA pixels. `generation` names it (serials of [`ssd_hd::world::next_serial`]).
struct FieldImage {
    generation: u64,
    source: FieldSource,
}

enum FieldSource {
    Pixels { width: u32, height: u32, rgba: Vec<u8> },
    Picture(SharedHd),
}

/// The pictures of the objects, and the backdrop, as egui textures decoded on first use.
pub struct Art {
    rom: Vec<u8>,
    /// The HD pack whose art stands for the pictures of the objects, when there is one.
    pack: Option<std::sync::Arc<ssd_hd::pack::Pack>>,
    /// The art of the battle's cards, when the pack has it: the Cheats screen's preview is drawn from it.
    cards: Option<ssd_hd::CardArt>,
    /// The preview card as a texture, by its Z corners (attack, defence), made on first use.
    card_textures: HashMap<(bool, bool), Option<egui::TextureHandle>>,
    /// The class kanji of the Status screen as textures, by symbol number, made on first use from the card art.
    symbol_textures: HashMap<u8, Option<egui::TextureHandle>>,
    /// The cards of the Cards screen's hand (and their parts) as textures, by their tile words and the part, made on
    /// first use.
    hand_textures: HashMap<(Vec<u16>, CardPart), Option<egui::TextureHandle>>,
    textures: HashMap<Picture, Option<egui::TextureHandle>>,
    field: Option<FieldImage>,
    backdrop_shape: Shape,
    /// The texture of the field at a generation and video shape, uploaded when it is first painted.
    backdrop: Option<(u64, Shape, egui::TextureHandle)>,
}

impl Art {
    /// An art with no ROM and no pack: it paints what needs no picture (the first-launch screens).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            rom: Vec::new(),
            pack: None,
            cards: None,
            card_textures: HashMap::new(),
            symbol_textures: HashMap::new(),
            hand_textures: HashMap::new(),
            textures: HashMap::new(),
            field: None,
            backdrop_shape: Shape::default(),
            backdrop: None,
        }
    }

    /// Copies the frame as the backdrop (a new generation each time).
    fn note_field(&mut self, frame: &Frame<'_>) {
        let mut rgba = Vec::with_capacity(frame.pixels.len() * 4);
        for color in frame.pixels {
            rgba.extend_from_slice(&[color.r, color.g, color.b, 255]);
        }
        let source =
            FieldSource::Pixels { width: frame.size.width, height: frame.size.height, rgba };
        self.keep_field(ssd_hd::world::next_serial(), source);
    }

    /// Keeps an Enhanced picture as the backdrop: nothing is copied, and a picture that is the backdrop already
    /// is not kept again (its texture stays). A picture that lacks art still being read (the art read on its own
    /// thread) does not take the place of a backdrop: the menu would keep it, the original's pixels where the art goes,
    /// as long as it is open. The next picture with its art does.
    fn note_field_shared(&mut self, picture: &SharedHd) {
        if self.field.as_ref().is_some_and(|field| field.generation == picture.serial) {
            return;
        }
        if picture.picture.art_pending && self.field.is_some() {
            return;
        }
        self.keep_field(picture.serial, FieldSource::Picture(picture.clone()));
    }

    fn keep_field(&mut self, generation: u64, source: FieldSource) {
        self.field = Some(FieldImage { generation, source });
    }

    fn menu_canvas(&self, picture: egui::Rect, stretch: bool) -> egui::Rect {
        if stretch && self.backdrop_shape.crop {
            egui::Rect::from_min_size(
                picture.min,
                egui::vec2(picture.width(), picture.width() * 3.0 / 4.0),
            )
        } else {
            picture
        }
    }

    /// The texture of the backdrop, uploaded if the field or video shape changed; `None` before any field was
    /// copied. The 4:3 crop and filters match the live picture before it is painted in the same rect.
    fn backdrop(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        let field = self.field.as_ref()?;
        let (width, height, rgba, logical) = match &field.source {
            FieldSource::Pixels { width, height, rgba } => (*width, *height, rgba.as_slice(), None),
            FieldSource::Picture(shared) => {
                let picture = &shared.picture.picture;
                (
                    picture.width,
                    picture.height,
                    picture.rgba.as_slice(),
                    Some([
                        picture.logical.x,
                        picture.logical.y,
                        picture.logical.width,
                        picture.logical.height,
                    ]),
                )
            }
        };
        if width == 0 || height == 0 {
            return None;
        }
        let stale = self.backdrop.as_ref().is_none_or(|(generation, shape, _)| {
            *generation != field.generation || *shape != self.backdrop_shape
        });
        if stale {
            let view = GameView {
                rgba,
                width,
                height,
                pixel_aspect: 1.0,
                smooth: false,
                logical,
                serial: None,
            };
            let mut buffer = Vec::new();
            let view = filter::shape(view, self.backdrop_shape, &mut buffer);
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [view.width as usize, view.height as usize],
                view.rgba,
            );
            let texture = ctx.load_texture("field backdrop", image, egui::TextureOptions::NEAREST);
            self.backdrop = Some((field.generation, self.backdrop_shape, texture));
        }
        self.backdrop.as_ref().map(|(_, _, texture)| texture.id())
    }

    /// The texture of the preview card with the Z corners given and its size in pixels, drawn from the pack's
    /// card art on first use; `None` when the pack has no card art (or lacks a tile of the card).
    fn card(
        &mut self,
        ctx: &egui::Context,
        attack_z: bool,
        defence_z: bool,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        let cards = self.cards.as_ref()?;
        let entry = self.card_textures.entry((attack_z, defence_z)).or_insert_with(|| {
            let card = cards.preview(attack_z, defence_z)?;
            let image =
                egui::ColorImage::from_rgba_unmultiplied([card.width, card.height], &card.rgba);
            Some(ctx.load_texture(
                format!("preview card {attack_z} {defence_z}"),
                image,
                egui::TextureOptions::LINEAR,
            ))
        });
        entry.as_ref().map(|texture| (texture.id(), texture.size_vec2()))
    }

    /// The texture of the card symbol `symbol` (a class kanji), from the pack's card art on first use; `None` when the pack
    /// has no card art or no picture of it.
    fn symbol(&mut self, ctx: &egui::Context, symbol: u8) -> Option<egui::TextureId> {
        let cards = self.cards.as_ref();
        let entry = self.symbol_textures.entry(symbol).or_insert_with(|| {
            let (side, rgba) = cards?.symbol(symbol)?;
            let image = egui::ColorImage::from_rgba_unmultiplied([side, side], rgba);
            Some(ctx.load_texture(format!("symbol {symbol}"), image, egui::TextureOptions::LINEAR))
        });
        entry.as_ref().map(egui::TextureHandle::id)
    }

    /// The texture of `part` of the card of the Cards screen's hand with the tile words `tiles`, drawn from the pack's card
    /// art on first use; `None` when the pack has no card art or none of the part's tiles has art.
    fn hand_card(
        &mut self,
        ctx: &egui::Context,
        tiles: &[u16],
        part: CardPart,
    ) -> Option<egui::TextureId> {
        let cards = self.cards.as_ref()?;
        // A hand is five cards: a long run of dealt hands does not pile textures up
        if self.hand_textures.len() > 96 {
            self.hand_textures.clear();
        }
        let entry = self.hand_textures.entry((tiles.to_vec(), part)).or_insert_with(|| {
            let card = cards.compose(tiles, hd_part(part))?;
            let image =
                egui::ColorImage::from_rgba_unmultiplied([card.width, card.height], &card.rgba);
            Some(ctx.load_texture("hand card", image, egui::TextureOptions::LINEAR))
        });
        entry.as_ref().map(egui::TextureHandle::id)
    }

    /// The texture of `picture` (an object's, or a corner face), decoded on first use; `None` when the ROM has no
    /// picture for it.
    fn texture(&mut self, ctx: &egui::Context, picture: Picture) -> Option<egui::TextureId> {
        let rom = &self.rom;
        let pack = self.pack.as_deref();
        let entry = self.textures.entry(picture).or_insert_with(|| {
            let view = RomView::new(rom).ok()?;
            let (portrait, name) = match picture {
                Picture::Object(object) => {
                    (object_portrait(view, object).ok()?, format!("object {object}"))
                }
                Picture::Face(face) => {
                    (face_portrait(view, face).ok()?, format!("face {face:02X}"))
                }
            };
            // The approved art of the picture itself (found by its characters), when the pack
            // has it: never another picture's art
            if let Some(image) = pack.and_then(|pack| hd_picture(pack, &portrait)) {
                return Some(ctx.load_texture(
                    format!("{name} (HD)"),
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [ssd_catalogue::art::SIDE; 2],
                &rgba(&portrait),
            );
            Some(ctx.load_texture(name, image, egui::TextureOptions::NEAREST))
        });
        entry.as_ref().map(egui::TextureHandle::id)
    }
}

/// The HD art of the picture `portrait` (the object's own half of its set, in the object's
/// palette), as an egui image: the pack's art for those very characters, recoloured to the
/// object's palette when it was made for another.
fn hd_picture(pack: &ssd_hd::pack::Pack, portrait: &Portrait) -> Option<egui::ColorImage> {
    let row: [[u8; 3]; 16] = std::array::from_fn(|index| snes_rgb(portrait.palette[index]));
    let picture = pack.menu_picture(&portrait.characters, &row)?;
    Some(egui::ColorImage::from_rgba_unmultiplied([picture.width, picture.height], &picture.rgba))
}

/// The sidecar file's path: `ssd.bag` beside the battery save (the saves directory, or a chapter's folder).
fn sidecar_path() -> PathBuf {
    crate::chapter::battery_dir().join(SIDECAR)
}

/// The sidecar file of the cheats' path: `ssd.cheats` beside the battery save.
fn cheats_sidecar_path() -> PathBuf {
    crate::chapter::battery_dir().join(CHEATS_SIDECAR)
}

/// The sidecar file of the mod state's path: `ssd.mods` beside the battery save.
fn mods_sidecar_path() -> PathBuf {
    crate::chapter::battery_dir().join(MODS_SIDECAR)
}

/// Writes `bytes` to `path`: beside the file and renamed over it, so a crash never leaves half a file. The writer
/// of the player's sidecar and option files: a run that does not persist writes nothing (see [`paths::persists`]).
pub(crate) fn write_beside(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    if !paths::persists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    let partial = PathBuf::from(name);
    std::fs::write(&partial, bytes).with_context(|| format!("writing {}", partial.display()))?;
    std::fs::rename(&partial, path).with_context(|| format!("replacing {}", path.display()))
}

/// The cheats' sidecar: empty when the file does not exist yet. A file that is not a cheats file (damaged,
/// or of another version) is kept aside as `ssd.cheats.bad`, and the cheats of every save start off.
fn read_cheats_sidecar() -> anyhow::Result<CheatSidecar> {
    let path = cheats_sidecar_path();
    match std::fs::read(&path) {
        Ok(bytes) => Ok(CheatSidecar::from_bytes(&bytes).unwrap_or_else(|error| {
            keep_damaged(&path, &path.with_extension("cheats.bad"), &error);
            CheatSidecar::empty()
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(CheatSidecar::empty()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// The mod state's sidecar: empty when the file does not exist yet, as for the saves made before it existed. A file
/// that is not a mod state file (damaged, or of another version) is kept aside as `ssd.mods.bad`, and the mod state of
/// every save starts neutral.
fn read_mods_sidecar() -> anyhow::Result<ModsSidecar> {
    let path = mods_sidecar_path();
    match std::fs::read(&path) {
        Ok(bytes) => Ok(ModsSidecar::from_bytes(&bytes).unwrap_or_else(|error| {
            keep_damaged(&path, &path.with_extension("mods.bad"), &error);
            ModsSidecar::empty()
        })),
        // A chapter that starts fresh carries the sidecar of its slot, kept in memory by a run that does not persist.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(crate::chapter::fresh("ssd.mods")
                .and_then(|bytes| ModsSidecar::from_bytes(bytes).ok())
                .unwrap_or_else(ModsSidecar::empty))
        }
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// The sidecar of the bags: empty when the file does not exist yet. A file that is not a bag file
/// (damaged, or of another version) is kept aside as `ssd.bag.bad` and the run starts with none:
/// every bag is then rebuilt from the original's list, and the run is not refused.
fn read_sidecar() -> anyhow::Result<Sidecar> {
    let path = sidecar_path();
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Sidecar::from_bytes(&bytes).unwrap_or_else(|error| {
            keep_damaged(&path, &path.with_extension("bag.bad"), &error);
            Sidecar::empty()
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Sidecar::empty()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// A damaged sidecar, read as none: a run that persists keeps the file aside as `aside`; a run that does not persist
/// leaves it as it is, since it writes nothing.
fn keep_damaged(path: &std::path::Path, aside: &std::path::Path, error: impl std::fmt::Display) {
    if !paths::persists() {
        log::warn!("{}: {error}; read as none, the file is left as it is", path.display());
        return;
    }
    log::warn!("{}: {error}; kept as {}", path.display(), aside.display());
    if let Err(error) = std::fs::rename(path, aside) {
        log::warn!("{}: {error}", aside.display());
    }
}

/// The picture as RGBA pixels: index 0 transparent, the other colours as the SNES shows them (each
/// 5-bit channel widened to 8 bits by repeating its top bits).
#[must_use]
pub fn rgba(portrait: &Portrait) -> Vec<u8> {
    let mut out = Vec::with_capacity(portrait.pixels.len() * 4);
    for &index in &portrait.pixels {
        if index == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        let [red, green, blue] = snes_rgb(portrait.palette[usize::from(index)]);
        out.extend_from_slice(&[red, green, blue, 255]);
    }
    out
}

/// A 15-bit SNES colour (`0BBBBBGGGGGRRRRR`) as 8-bit red, green and blue.
#[must_use]
pub fn snes_rgb(colour: u16) -> [u8; 3] {
    let channel = |shift: u16| {
        let value = ((colour >> shift) & 0x1F) as u8;
        (value << 3) | (value >> 2)
    };
    [channel(0), channel(5), channel(10)]
}

/// The navigation keys of the HD that were pressed between two frames of the host's joypad: the
/// d-pad, A to confirm, B to cancel, the shoulders to change the bag's tab or the options page (L goes back), Y and
/// Select to reset an options page or all of them (the keyboard and the
/// gamepad alike). A held button counts once, when it goes down.
#[must_use]
pub fn keys_pressed(previous: Joypad, now: Joypad) -> Vec<Key> {
    [
        (now.up && !previous.up, Key::Up),
        (now.down && !previous.down, Key::Down),
        (now.left && !previous.left, Key::Left),
        (now.right && !previous.right, Key::Right),
        (now.a && !previous.a, Key::Confirm),
        (now.b && !previous.b, Key::Cancel),
        (now.l && !previous.l, Key::TabBack),
        (now.y && !previous.y, Key::ResetPage),
        (now.select && !previous.select, Key::ResetAll),
        (now.x && !previous.x, Key::Apply),
        (now.r && !previous.r, Key::Tab),
    ]
    .into_iter()
    .filter_map(|(pressed, key)| pressed.then_some(key))
    .collect()
}

/// The egui colour of a draw list colour.
fn colour(color: Color) -> egui::Color32 {
    let [red, green, blue, alpha] = color.0;
    egui::Color32::from_rgba_unmultiplied(red, green, blue, alpha)
}

/// Where a logical point lies in the picture the menu is painted over: the logical space is scaled
/// uniformly to fit the picture and centred in it, so it never leaves the picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Points per logical pixel.
    pub scale: f32,
    /// The point of the logical origin.
    pub origin: egui::Pos2,
}

impl Fit {
    /// The fit of the logical space in `picture`.
    #[cfg(test)]
    #[must_use]
    pub fn new(picture: egui::Rect) -> Self {
        Self::sized(picture, layout::WIDTH, layout::HEIGHT)
    }

    /// The fit of a screen's own logical canvas.
    #[must_use]
    pub fn for_list(picture: egui::Rect, list: &DrawList) -> Self {
        let (width, height) = list.canvas_size();
        Self::sized(picture, width, height)
    }

    /// The fit of a logical space of `width` x `height` in `picture` (the first-launch screens have their own).
    #[must_use]
    pub fn sized(picture: egui::Rect, width: f32, height: f32) -> Self {
        let scale = (picture.width() / width).min(picture.height() / height).max(0.0);
        let size = egui::vec2(width, height) * scale;
        Self { scale, origin: picture.center() - size / 2.0 }
    }

    /// The point of a logical point.
    #[must_use]
    pub fn point(&self, x: f32, y: f32) -> egui::Pos2 {
        self.origin + egui::vec2(x, y) * self.scale
    }

    /// The egui rectangle of a logical rectangle.
    #[must_use]
    pub fn rect(&self, rect: Rect) -> egui::Rect {
        egui::Rect::from_min_size(
            self.point(rect.x, rect.y),
            egui::vec2(rect.w, rect.h) * self.scale,
        )
    }
}

/// Paints a draw list over `picture`, the rect the game's frame is drawn in, with the textures of
/// `art`. The backdrop covers the picture; the logical layout is fitted to it (see [`Fit`]). The
/// painting is clipped to the picture as well, so nothing can be drawn over the letterbox.
pub fn paint(ui: &egui::Ui, picture: egui::Rect, list: &DrawList, art: &mut Art) {
    paint_fit(ui, picture, &Fit::for_list(picture, list), list, art);
}

/// Draws a stretched 4:3 menu in the same display space as the game, including when a 4:3 CRT receives a
/// widescreen framebuffer. Text must be tessellated before projection so its glyphs follow the portraits.
/// Returns the canvas whose coordinates the menu's pointer uses.
pub fn paint_menu(
    ui: &egui::Ui,
    picture: egui::Rect,
    list: &DrawList,
    art: &mut Art,
    stretch: bool,
) -> egui::Rect {
    let canvas = art.menu_canvas(picture, stretch);
    if canvas == picture {
        paint(ui, picture, list, art);
        return canvas;
    }
    let layer = egui::LayerId::new(egui::Order::Background, ui.id().with("hd-menu-projection"));
    let painter = egui::Painter::new(ui.ctx().clone(), layer, canvas);
    paint_with(ui, &painter, canvas, &Fit::for_list(canvas, list), list, art);
    let recorded = ui.ctx().graphics_mut(|graphics| std::mem::take(graphics.entry(layer)));
    let shapes = recorded.all_entries().cloned().collect();
    let projection = egui::emath::RectTransform::from_to(canvas, picture);
    for primitive in ui.ctx().tessellate(shapes, ui.ctx().pixels_per_point()) {
        let egui::epaint::Primitive::Mesh(mut mesh) = primitive.primitive else {
            unreachable!("HD draw lists do not contain paint callbacks");
        };
        for vertex in &mut mesh.vertices {
            vertex.pos = projection * vertex.pos;
        }
        let clip = projection.transform_rect(primitive.clip_rect).intersect(picture);
        ui.painter().with_clip_rect(clip).add(mesh);
    }
    canvas
}

/// [`paint`] with the fit of the list's own logical space.
pub fn paint_fit(ui: &egui::Ui, picture: egui::Rect, fit: &Fit, list: &DrawList, art: &mut Art) {
    let painter = ui.painter_at(picture);
    paint_with(ui, &painter, picture, fit, list, art);
}

fn paint_with(
    ui: &egui::Ui,
    painter: &egui::Painter,
    picture: egui::Rect,
    fit: &Fit,
    list: &DrawList,
    art: &mut Art,
) {
    let fit = *fit;
    for op in list.ops() {
        match op {
            Op::Backdrop => {
                if let Some(id) = art.backdrop(ui.ctx()) {
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    painter.image(id, picture, uv, egui::Color32::WHITE);
                }
            }
            Op::Fill { rect, color } => {
                painter.rect_filled(fit.rect(*rect), 0.0, colour(*color));
            }
            Op::Disc { rect, color } => {
                let area = fit.rect(*rect);
                painter.circle_filled(area.center(), area.width() / 2.0, colour(*color));
            }
            Op::Outline { rect, width, color } => {
                let stroke = egui::Stroke::new(width * fit.scale, colour(*color));
                painter.rect_stroke(fit.rect(*rect), 0.0, stroke, egui::StrokeKind::Inside);
            }
            Op::Window { rect } => window(painter, &fit, *rect),
            Op::WindowHc { rect } => window_contrast(painter, &fit, *rect),
            Op::Pointer { rect, left, fill, edge, ring } => {
                let (tip, base) =
                    if *left { (rect.x, rect.right()) } else { (rect.right(), rect.x) };
                let points = [(base, rect.y), (tip, rect.middle_y()), (base, rect.bottom())]
                    .map(|(x, y)| fit.point(x, y));
                for (colour_of, width) in [(edge, POINTER_EDGE), (ring, POINTER_RING)] {
                    let stroke = egui::Stroke::new(width * fit.scale, colour(*colour_of));
                    painter.add(egui::Shape::convex_polygon(
                        points.to_vec(),
                        colour(*fill),
                        stroke,
                    ));
                }
            }
            Op::Line { from, to, width, color } => {
                let stroke = egui::Stroke::new(width * fit.scale, colour(*color));
                painter.line_segment([fit.point(from.0, from.1), fit.point(to.0, to.1)], stroke);
            }
            Op::Text { x, y, text, size, color, align } => {
                let anchor = match align {
                    ssd_menus::Align::Left => egui::Align2::LEFT_CENTER,
                    ssd_menus::Align::Centre => egui::Align2::CENTER_CENTER,
                    ssd_menus::Align::Right => egui::Align2::RIGHT_CENTER,
                };
                let font = theme::game(size * fit.scale);
                painter.text(fit.point(*x, *y), anchor, text, font, colour(*color));
            }
            Op::Cursor { x, y } => {
                let points =
                    [(0.0, 0.0), (layout::CURSOR_WIDTH, 9.0), (0.0, layout::CURSOR_HEIGHT)]
                        .map(|(dx, dy)| fit.point(x + dx, y + dy));
                let stroke = egui::Stroke::new(2.0 * fit.scale, colour(palette::DARK_RED));
                painter.add(egui::Shape::convex_polygon(
                    points.to_vec(),
                    colour(palette::ORANGE),
                    stroke,
                ));
            }
            Op::Arrow { x, y, left } => {
                let (tip, base) =
                    if *left { (0.0, layout::CURSOR_WIDTH) } else { (layout::CURSOR_WIDTH, 0.0) };
                let points = [(base, 0.0), (tip, 9.0), (base, layout::CURSOR_HEIGHT)]
                    .map(|(dx, dy)| fit.point(x + dx, y + dy));
                let stroke = egui::Stroke::new(2.0 * fit.scale, colour(palette::DARK_RED));
                painter.add(egui::Shape::convex_polygon(
                    points.to_vec(),
                    colour(palette::ORANGE),
                    stroke,
                ));
            }
            Op::Portrait { rect, picture } => portrait(ui, painter, &fit, art, *rect, *picture),
            Op::Card { rect, attack_z, defence_z } => {
                match art.card(ui.ctx(), *attack_z, *defence_z) {
                    Some((id, size)) => {
                        // The card's art keeps its proportions, centred in the rect
                        let area = fit.rect(*rect);
                        let scale = (area.width() / size.x).min(area.height() / size.y);
                        let drawn = egui::Rect::from_center_size(area.center(), size * scale);
                        let uv =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        painter.image(id, drawn, uv, egui::Color32::WHITE);
                    }
                    None => paint_with(
                        ui,
                        painter,
                        picture,
                        &fit,
                        &ssd_menus::screens::pixel_card(*rect, *attack_z, *defence_z),
                        art,
                    ),
                }
            }
            Op::Symbol { rect, symbol } => {
                if let Some(id) = art.symbol(ui.ctx(), *symbol) {
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    painter.image(id, fit.rect(*rect), uv, egui::Color32::WHITE);
                }
            }
            Op::HandCard { rect, tiles, part } => match art.hand_card(ui.ctx(), tiles, *part) {
                Some(id) => {
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    painter.image(id, fit.rect(*rect), uv, egui::Color32::WHITE);
                }
                None => paint_with(
                    ui,
                    painter,
                    picture,
                    &fit,
                    &ssd_menus::screens::cards::flat_part(*rect, *part),
                    art,
                ),
            },
            Op::CursorDown { x, y } => {
                let points = [(0.0, 0.0), (18.0, 0.0), (9.0, 16.0)]
                    .map(|(dx, dy)| fit.point(x + dx, y + dy));
                let stroke = egui::Stroke::new(2.0 * fit.scale, colour(palette::DARK_RED));
                painter.add(egui::Shape::convex_polygon(
                    points.to_vec(),
                    colour(palette::ORANGE),
                    stroke,
                ));
            }
            Op::Bar { rect, fraction, color } => {
                painter.rect_filled(fit.rect(*rect), 0.0, colour(palette::WHITE));
                let well = rect.grow(-layout::BAR_BORDER);
                painter.rect_filled(fit.rect(well), 0.0, colour(palette::BLACK));
                let filled = Rect::new(well.x, well.y, well.w * fraction.clamp(0.0, 1.0), well.h);
                painter.rect_filled(fit.rect(filled), 0.0, colour(*color));
            }
        }
    }
}

/// The card art's part of a draw list's [`CardPart`].
fn hd_part(part: CardPart) -> ssd_hd::CardPart {
    match part {
        CardPart::Whole => ssd_hd::CardPart::WHOLE,
        CardPart::Attack => ssd_hd::CardPart::ATTACK,
        CardPart::Symbol => ssd_hd::CardPart::SYMBOL,
        CardPart::Defence => ssd_hd::CardPart::DEFENCE,
    }
}

/// The stroke widths of a [`Op::Pointer`]: the wide outline and the narrower line on it.
const POINTER_EDGE: f32 = 5.0;
const POINTER_RING: f32 = 2.0;

/// A menu window in high contrast: a white line outside a black frame, and white paper.
fn window_contrast(painter: &egui::Painter, fit: &Fit, rect: Rect) {
    let black = colour(palette::BLACK);
    let white = colour(palette::WHITE);
    painter.rect_filled(fit.rect(rect.grow(layout::LINE)), 0.0, white);
    painter.rect_filled(fit.rect(rect), 0.0, black);
    painter.rect_filled(fit.rect(rect.grow(-layout::FRAME)), 0.0, white);
}

/// A menu window: the dark-red line outside the orange frame, the frame, the dark-red line inside
/// it, the cream paper, and a corner cap on each corner.
fn window(painter: &egui::Painter, fit: &Fit, rect: Rect) {
    let dark = colour(palette::DARK_RED);
    painter.rect_filled(fit.rect(rect.grow(layout::LINE)), 0.0, dark);
    painter.rect_filled(fit.rect(rect), 0.0, colour(palette::ORANGE));
    painter.rect_filled(fit.rect(rect.grow(-layout::FRAME)), 0.0, dark);
    painter.rect_filled(
        fit.rect(rect.grow(-(layout::FRAME + layout::LINE))),
        0.0,
        colour(palette::CREAM),
    );
    let overhang = layout::CAP_OVERHANG;
    let corners = [
        (rect.x - overhang, rect.y - overhang),
        (rect.right() - layout::CAP + overhang, rect.y - overhang),
        (rect.x - overhang, rect.bottom() - layout::CAP + overhang),
        (rect.right() - layout::CAP + overhang, rect.bottom() - layout::CAP + overhang),
    ];
    for (x, y) in corners {
        let cap = Rect::new(x, y, layout::CAP, layout::CAP);
        painter.rect_filled(fit.rect(cap), 0.0, dark);
        painter.rect_filled(fit.rect(cap.grow(-layout::LINE)), 0.0, colour(palette::CAP));
    }
}

/// A square portrait frame: white line, black border, and the picture of the object at the largest
/// whole scale that fits (or the empty box with its label).
fn portrait(
    ui: &egui::Ui,
    painter: &egui::Painter,
    fit: &Fit,
    art: &mut Art,
    rect: Rect,
    picture: Option<Picture>,
) {
    painter.rect_filled(fit.rect(rect.grow(2.0)), 0.0, colour(palette::WHITE));
    painter.rect_filled(fit.rect(rect), 0.0, colour(palette::BLACK));
    let inner = rect.grow(-layout::PORTRAIT_FRAME);
    let texture = picture.and_then(|picture| art.texture(ui.ctx(), picture));
    // Nothing to show (an empty list, or a face with no picture): the box stays empty, never a
    // stand-in text
    if let Some(id) = texture {
        // A box under two pictures wide (the favoured row of the Cards screen) shows its picture the box's size
        let whole = (inner.w / layout::PICTURE).floor().max(1.0);
        let side = if whole < 2.0 { inner.w.min(inner.h) } else { layout::PICTURE * whole };
        let picture = Rect::new(
            inner.x + (inner.w - side) / 2.0,
            inner.y + (inner.h - side) / 2.0,
            side,
            side,
        );
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(id, fit.rect(picture), uv, egui::Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_menus::Style;

    pub(super) fn presentation_hd() -> Hd {
        let rom = crate::test_support::sram_counter_rom();
        let labels = ssd_menus::Labels::default();
        Hd {
            driver: Driver::new(labels.clone()),
            art: Art::empty(),
            wide: WideRun::new(&rom),
            notice: CheatNotice::default(),
            shown_labels: labels.clone(),
            labels,
            language_saved: None,
            ui_text: None,
            rom: std::sync::Arc::from(rom),
            editor: None,
            editor_wanted: false,
            editor_allowed: false,
            level_cap: None,
        }
    }

    fn field_ui_log() -> FrameLog {
        let regs = ssd_oracle::LineRegs {
            brightness: 15,
            bg_mode: 1,
            mode_1_bg3_priority: true,
            main_bg: [true, true, true, false],
            bg_map_base: [0x6800, 0x6000, 0x2C00, 0],
            bg_char_base: [0x4000, 0x2000, 0x5000, 0],
            ..Default::default()
        };
        let mut log = FrameLog {
            complete: true,
            height: 224,
            h_hi_res: false,
            v_hi_res: false,
            entries: (1..=224)
                .map(|scanline| ssd_oracle::LineEntry { scanline, regs, ..Default::default() })
                .collect(),
            cgram_start: vec![0; 256],
            cgram_writes: Vec::new(),
            vram: vec![0; 32768],
            vram_writes: Vec::new(),
            oam_low: vec![0; 512],
            oam_high: vec![0; 32],
            oam_writes: Vec::new(),
        };
        // A red map, one green BG1 portrait tile and one blue BG3 window tile.
        log.cgram_start[1..4].copy_from_slice(&[0x001F, 0x03E0, 0x7C00]);
        log.vram[0x2000..0x2008].fill(0x00FF);
        log.vram[0x4010..0x4018].fill(0xFF00);
        log.vram[0x5008..0x5010].fill(0xFFFF);
        log.vram[0x6800] = 0x2001;
        log.vram[0x2C02] = 0x2001;
        log
    }

    #[test]
    fn prepare_view_hides_native_field_ui_only_in_4_3_and_restores_it_in_16_9() {
        let log = field_ui_log();
        let options = ssd_hd::HdOptions { scale: 2, look: ssd_hd::Look::Pixel };
        let pixel = |shared: &SharedHd, x: usize| {
            let picture = &shared.picture.picture;
            let x = usize::try_from(picture.logical.x).unwrap() + x * picture.scale as usize;
            let y = usize::try_from(picture.logical.y).unwrap() + 2 * picture.scale as usize;
            let at = (y * picture.width as usize + x) * 4;
            <[u8; 4]>::try_from(&picture.rgba[at..at + 4]).unwrap()
        };
        for screen in [1, 5] {
            let mut hd = presentation_hd();
            let mut wram = vec![0; 0x2_0000];
            wram[(d::FIELD_STATE - 0x7E_0000) as usize] = 1;
            wram[(d::MENU_SCREEN - 0x7E_0000) as usize] = screen;
            hd.frame(&wram, None, Joypad::default());
            assert!(hd.driver.travel_transition(&wram));
            for narrow in [false, true, false, true, false] {
                hd.prepare_view(&wram, narrow);
                let shown = hd.note_frame_hd_shared(&wram, Some(&log), options).unwrap();
                let map = [255, 0, 0, 255];
                assert_eq!(
                    pixel(&shown, 2),
                    if narrow { map } else { [0, 255, 0, 255] },
                    "screen {screen}, 4:3 {narrow}: the portrait"
                );
                assert_eq!(
                    pixel(&shown, 18),
                    if narrow { map } else { [0, 0, 255, 255] },
                    "screen {screen}, 4:3 {narrow}: the window"
                );
                assert_eq!(pixel(&shown, 40), map, "the map is unchanged");
            }
            wram[(d::FIELD_STATE - 0x7E_0000) as usize] = 0;
            hd.frame(&wram, None, Joypad::default());
            hd.prepare_view(&wram, true);
            let restored = hd.note_frame_hd_shared(&wram, Some(&log), options).unwrap();
            assert_eq!(pixel(&restored, 2), [0, 255, 0, 255], "the transition ended");
        }
    }

    #[test]
    fn prepare_view_does_not_hide_native_ui_for_other_menu_screens_or_battles() {
        let log = field_ui_log();
        let options = ssd_hd::HdOptions { scale: 2, look: ssd_hd::Look::Pixel };
        for (screen, battle) in
            [(0, false), (2, false), (4, false), (6, false), (13, false), (1, true), (5, true)]
        {
            let mut hd = presentation_hd();
            let mut wram = vec![0; 0x2_0000];
            wram[(d::FIELD_STATE - 0x7E_0000) as usize] = 1;
            wram[(d::MENU_SCREEN - 0x7E_0000) as usize] = screen;
            wram[(d::CONTEXT_61 - 0x7E_0000) as usize] = u8::from(battle);
            hd.frame(&wram, None, Joypad::default());
            assert!(!hd.driver.travel_transition(&wram));
            hd.prepare_view(&wram, false);
            let wide = hd.note_frame_hd_shared(&wram, Some(&log), options).unwrap();
            hd.prepare_view(&wram, true);
            let narrow = hd.note_frame_hd_shared(&wram, Some(&log), options).unwrap();
            assert_eq!(
                wide.picture.picture.rgba, narrow.picture.picture.rgba,
                "screen {screen}, battle {battle}"
            );
        }
    }

    /// A 16:9 Enhanced picture of one colour, lacking art still being read (`pending`) or not.
    fn wide_picture(colour: u8, pending: bool) -> SharedHd {
        let rect = ssd_hd::Rect { x: 0, y: 0, width: 4, height: 2 };
        SharedHd::new(WideHd {
            picture: ssd_hd::HdFrame {
                rgba: vec![colour; 4 * 2 * 4],
                width: 4,
                height: 2,
                scale: 1,
                logical: rect,
            },
            scene: ssd_hd::world::Scene::Field,
            logical: ssd_hd::world::Rect { x: 0, y: 0, width: 4, height: 2 },
            hud: ssd_hd::world::Rect { x: 0, y: 0, width: 4, height: 2 },
            stats: ssd_hd::world::WideStats::default(),
            art_pending: pending,
        })
    }

    /// The colour of the backdrop's first pixel.
    fn backdrop_colour(art: &Art) -> Option<u8> {
        match &art.field.as_ref()?.source {
            FieldSource::Picture(shared) => Some(shared.picture.picture.rgba[0]),
            FieldSource::Pixels { rgba, .. } => Some(rgba[0]),
        }
    }

    #[test]
    fn a_picture_that_lacks_art_being_read_takes_the_place_of_no_backdrop_and_the_next_with_its_art_does()
     {
        let mut art = Art::empty();
        // With no backdrop yet, the picture is the backdrop, art or not
        art.note_field_shared(&wide_picture(1, true));
        assert_eq!(backdrop_colour(&art), Some(1));
        art.note_field_shared(&wide_picture(2, false));
        assert_eq!(backdrop_colour(&art), Some(2));
        // A picture that lacks art leaves the backdrop drawn with all its art
        art.note_field_shared(&wide_picture(3, true));
        assert_eq!(backdrop_colour(&art), Some(2));
        // The next picture with its art takes its place
        art.note_field_shared(&wide_picture(4, false));
        assert_eq!(backdrop_colour(&art), Some(4));
    }

    fn patterned_field(scale: u32) -> SharedHd {
        let (width, height) = (372 * scale, 224 * scale);
        let rgba = (0..width * height)
            .flat_map(|at| {
                [(at % width % 251) as u8, (at / width % 251) as u8, (at % width / 251) as u8, 255]
            })
            .collect();
        SharedHd::new(WideHd {
            picture: ssd_hd::HdFrame {
                rgba,
                width,
                height,
                scale,
                logical: ssd_hd::Rect { x: 58 * scale, y: 0, width: 256 * scale, height },
            },
            scene: ssd_hd::world::Scene::Field,
            logical: ssd_hd::world::Rect { x: 58, y: 0, width: 256, height: 224 },
            hud: ssd_hd::world::Rect { x: 58, y: 0, width: 256, height: 224 },
            stats: ssd_hd::world::WideStats::default(),
            art_pending: false,
        })
    }

    fn painted_backdrop(
        ctx: &egui::Context,
        art: &mut Art,
        picture: egui::Rect,
    ) -> (egui::TextureId, Option<std::sync::Arc<egui::ColorImage>>) {
        let mut list = DrawList::new();
        list.push(Op::Backdrop);
        let raw = egui::RawInput { screen_rect: Some(picture), ..Default::default() };
        let output = ctx.run_ui(raw, |ui| paint(ui, picture, &list, art));
        let id = art.backdrop.as_ref().expect("the backdrop was uploaded").2.id();
        let mesh = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == id => Some(mesh),
                _ => None,
            })
            .expect("the backdrop was painted");
        assert_eq!(mesh.calc_bounds(), picture);
        for uv in
            [egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0), egui::pos2(0.0, 1.0), egui::pos2(1.0, 1.0)]
        {
            assert!(mesh.vertices.iter().any(|vertex| vertex.uv == uv));
        }
        let image =
            output.textures_delta.set.get(&id).and_then(|deltas| deltas.last()).map(|delta| {
                let egui::epaint::image::ImageData::Color(image) = &delta.image;
                std::sync::Arc::clone(image)
            });
        output.drop_without_applying_deltas();
        (id, image)
    }

    #[test]
    fn the_menu_backdrop_matches_the_live_crop_and_filters_even_when_only_options_change() {
        let ctx = egui::Context::default();
        let (machine, _store) = crate::test_support::sram_counter_machine();
        let frame = machine.frame();
        let shapes = [
            Shape::default(),
            Shape { crop: false, filters: filter::Filters { scanlines: 2, crt: 0 } },
            Shape { crop: false, filters: filter::Filters { scanlines: 0, crt: 2 } },
            Shape { crop: false, filters: filter::Filters { scanlines: 1, crt: 2 } },
            Shape { crop: true, filters: filter::Filters::default() },
            Shape { crop: true, filters: filter::Filters { scanlines: 2, crt: 1 } },
            Shape { crop: true, filters: filter::Filters { scanlines: 1, crt: 2 } },
            Shape::default(),
        ];
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0));
        for scale in [1, 2] {
            let shared = patterned_field(scale);
            let mut art = Art::empty();
            art.note_field_shared(&shared);
            for shape in shapes {
                art.backdrop_shape = shape;
                let mut buffer = Vec::new();
                let live = filter::shape(
                    GameView::enhanced(&shared.picture.picture, &frame, false),
                    shape,
                    &mut buffer,
                );
                let picture = crate::layout::picture_rect(
                    1.0,
                    area,
                    live.width,
                    live.height,
                    live.pixel_aspect,
                    false,
                );
                let (id, image) = painted_backdrop(&ctx, &mut art, picture);
                let image = image.expect("a new video shape refreshes the frozen field");
                let width = if shape.crop { 256 } else { 372 } * scale;
                assert_eq!(image.size, [width as usize, (224 * scale) as usize]);
                let expected = egui::ColorImage::from_rgba_unmultiplied(
                    [live.width as usize, live.height as usize],
                    live.rgba,
                );
                assert_eq!(image.pixels, expected.pixels, "{shape:?} at {scale}x");
                assert_eq!(art.field.as_ref().unwrap().generation, shared.serial);
                for size in [egui::vec2(2100.0, 900.0), egui::vec2(900.0, 2100.0)] {
                    let stretched = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                    let (cached, uploaded) = painted_backdrop(&ctx, &mut art, stretched);
                    assert_eq!(cached, id, "the destination does not change the source crop");
                    assert!(uploaded.is_none(), "unchanged field and video shape stay cached");
                }
            }
        }
    }

    #[test]
    fn a_new_field_refreshes_the_backdrop_and_a_classic_frame_is_not_cropped() {
        let ctx = egui::Context::default();
        let mut art = Art::empty();
        art.backdrop_shape.crop = true;
        let picture = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut previous = None;
        for colour in [20, 80] {
            art.keep_field(
                ssd_hd::world::next_serial(),
                FieldSource::Pixels { width: 8, height: 6, rgba: vec![colour; 8 * 6 * 4] },
            );
            let (id, image) = painted_backdrop(&ctx, &mut art, picture);
            let image = image.expect("a new field refreshes the texture");
            assert_eq!(image.size, [8, 6], "a Classic frame has no extra map to crop");
            assert_eq!(
                image.pixels[0],
                egui::Color32::from_rgba_unmultiplied(colour, colour, colour, colour)
            );
            assert_ne!(previous, Some(id));
            previous = Some(id);
        }
    }

    #[test]
    fn the_cards_screen_names_each_kanji_as_the_battles_tooltip_does() {
        use ssd_menus::screens::cards::Card;
        let words = ssd_hd::symbol_words();
        for (symbol, expected) in words.into_iter().enumerate() {
            let card = Card { attack: 1, defence: 1, symbol: symbol as u8, tiles: Vec::new() };
            assert_eq!(card.words(), expected, "symbol {symbol}");
        }
    }

    #[test]
    fn a_run_that_does_not_persist_writes_no_sidecar_and_leaves_a_damaged_one_as_it_is() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let bag = sidecar_path();
        paths::set_persistent(false);
        write_beside(&bag, b"bags").unwrap();
        assert!(!bag.exists() && !paths::saves_dir().exists(), "no sidecar, no saves folder");
        // A damaged sidecar is read as none, and the file stays where it is.
        std::fs::create_dir_all(paths::saves_dir()).unwrap();
        std::fs::write(&bag, b"garbage").unwrap();
        assert!(read_sidecar().is_ok());
        assert_eq!(std::fs::read(&bag).unwrap(), b"garbage", "the damaged file is unchanged");
        assert!(!bag.with_extension("bag.bad").exists());
        // A windowed run writes the sidecar, and keeps a damaged one aside.
        paths::set_persistent(true);
        write_beside(&bag, b"bags").unwrap();
        assert_eq!(std::fs::read(&bag).unwrap(), b"bags");
        std::fs::write(&bag, b"garbage").unwrap();
        read_sidecar().unwrap();
        assert!(bag.with_extension("bag.bad").exists(), "the damaged file was moved aside");
        assert!(!bag.exists());
        paths::set_persistent(false);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn the_colours_widen_five_bits_to_eight_by_repeating_their_top_bits() {
        assert_eq!(snes_rgb(0x0000), [0, 0, 0]);
        assert_eq!(snes_rgb(0x7FFF), [255, 255, 255]);
        // 26 (0b11010) widens to 0b11010110 = 214, as the console shows it.
        assert_eq!(snes_rgb(26 << 5), [0, 214, 0]);
        assert_eq!(snes_rgb(0x001F), [255, 0, 0]);
    }

    #[test]
    fn index_zero_is_transparent_and_the_others_take_the_palette() {
        let mut palette = [0_u16; 16];
        palette[1] = 0x001F;
        let portrait = Portrait { half: 0, characters: Vec::new(), pixels: vec![0, 1], palette };
        let pixels = rgba(&portrait);
        assert_eq!(pixels, [0, 0, 0, 0, 255, 0, 0, 255]);
    }

    #[test]
    fn the_scene_fits_the_area_and_keeps_its_proportions() {
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1440.0, 900.0));
        let fit = Fit::new(area);
        assert!((fit.scale - 1.125).abs() < 1e-6, "width is the limit: 1440 / 1280");
        assert_eq!(fit.point(0.0, 0.0), egui::pos2(0.0, 45.0), "centred vertically");
        let square = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 720.0));
        assert!((Fit::new(square).scale - 0.5).abs() < 1e-6);
    }

    #[test]
    fn stretch_aspect_4_3_fullscreen_keeps_portraits_square_on_a_4_3_display() {
        let ctx = measuring_context();
        let mut values = ssd_menus::options::Values::new();
        values.set("video.aspect", 1);
        values.set("video.display", 1);
        let video = crate::options::video::VideoSettings::from_values(&values);
        assert!(video.narrow && video.fullscreen);
        let mut list = DrawList::sized(960.0, 720.0);
        list.push(Op::Backdrop);
        for x in [32.0, 824.0] {
            list.push(Op::Portrait {
                rect: Rect::new(x, 28.0, 104.0, 104.0),
                picture: Some(Picture::Face(0)),
            });
        }
        let games = [crate::canvas::GameImage {
            id: egui::TextureId::User(42),
            width: 256,
            height: 224,
            pixel_aspect: 8.0 / 7.0,
        }];
        let display = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 720.0));
        let mut expected = None;
        // A 4:3 CRT may receive a 16:9 HDMI signal; its physical shape is not the framebuffer's shape.
        for size in
            [egui::vec2(960.0, 720.0), egui::vec2(1280.0, 720.0), egui::vec2(1920.0, 1080.0)]
        {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let raw = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut art = Art::empty();
            art.backdrop_shape = video.shape();
            art.note_field_shared(&patterned_field(1));
            let texture = ctx.load_texture(
                "square portrait",
                egui::ColorImage::from_rgba_unmultiplied([32, 32], &vec![255; 32 * 32 * 4]),
                egui::TextureOptions::NEAREST,
            );
            let portrait_id = texture.id();
            art.textures.insert(Picture::Face(0), Some(texture));
            let output = ctx.run_ui(raw.clone(), |ui| crate::ui::draw_game(ui, &games, None, true));
            assert!(output.shapes.iter().any(|clipped| matches!(
                &clipped.shape,
                egui::Shape::Mesh(mesh) if mesh.texture_id == games[0].id && mesh.calc_bounds() == screen
            )), "the live game still fills the panel");
            output.drop_without_applying_deltas();
            let output = ctx.run_ui(raw.clone(), |ui| {
                let layer = crate::ui::Layer { list: &list, art: &mut art, menu: true };
                crate::ui::draw_game(ui, &games, Some(layer), true);
            });
            let physical = egui::emath::RectTransform::from_to(screen, display);
            let portraits: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Mesh(mesh) if mesh.texture_id == portrait_id => {
                        Some(physical.transform_rect(mesh.calc_bounds()))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(portraits.len(), 2);
            for portrait in &portraits {
                assert!(
                    (portrait.width() - portrait.height()).abs() < 0.01,
                    "{size:?}: {portrait:?}"
                );
                assert!(portrait.top() < 40.0, "no 16:9 vertical inset: {portrait:?}");
            }
            assert!(portraits[0].left() < 40.0 && portraits[1].right() > 920.0);
            match &expected {
                Some(previous) => {
                    assert_eq!(&portraits, previous, "the physical layout is signal-independent");
                }
                None => expected = Some(portraits),
            }
            let id = art.backdrop.as_ref().unwrap().2.id();
            assert!(
                output.shapes.iter().any(|clipped| matches!(
                    &clipped.shape,
                    egui::Shape::Mesh(mesh) if mesh.texture_id == id && mesh.calc_bounds() == screen
                )),
                "the game backdrop still fills the same panel"
            );
            assert_eq!(art.backdrop.as_ref().unwrap().2.size(), [256, 224]);
            output.drop_without_applying_deltas();
            assert_eq!(art.menu_canvas(screen, false), screen, "Stretch off is unchanged");
            art.backdrop_shape.crop = false;
            assert_eq!(art.menu_canvas(screen, true), screen, "16:9 is unchanged");
        }
    }

    #[test]
    fn the_4_3_projection_includes_text_vertices_and_preserves_their_uvs() {
        fn meshes(ctx: &egui::Context, mut output: egui::FullOutput) -> Vec<egui::epaint::Mesh> {
            let shapes = std::mem::take(&mut output.shapes);
            let primitives = ctx.tessellate(shapes, output.pixels_per_point);
            output.drop_without_applying_deltas();
            primitives
                .into_iter()
                .map(|primitive| {
                    let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                        panic!("text should tessellate into a mesh");
                    };
                    mesh
                })
                .collect()
        }

        let ctx = measuring_context();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
        let mut art = Art::empty();
        art.backdrop_shape.crop = true;
        let canvas = art.menu_canvas(screen, true);
        let mut list = DrawList::new();
        list.text(300.0, 300.0, "MENU", 30.0, palette::BLACK, ssd_menus::Align::Left);
        let output =
            ctx.run_ui(egui::RawInput { screen_rect: Some(canvas), ..Default::default() }, |ui| {
                paint(ui, canvas, &list, &mut art);
            });
        let before = meshes(&ctx, output);
        let output =
            ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                paint_menu(ui, screen, &list, &mut art, true);
            });
        let after = meshes(&ctx, output);
        assert_eq!((before.len(), after.len()), (1, 1));
        assert_eq!(before[0].texture_id, after[0].texture_id);
        assert_eq!(before[0].vertices.len(), after[0].vertices.len());
        let projection = egui::emath::RectTransform::from_to(canvas, screen);
        for (source, target) in before[0].vertices.iter().zip(&after[0].vertices) {
            assert!((projection * source.pos - target.pos).length() < 0.001);
            assert_eq!(source.uv, target.uv);
            assert_eq!(source.color, target.color);
        }
    }

    #[test]
    fn the_quit_question_pointer_inverts_the_4_3_menu_projection() {
        let ctx = measuring_context();
        let screen = egui::Rect::from_min_size(egui::pos2(24.0, 40.0), egui::vec2(1280.0, 720.0));
        let mut art = Art::empty();
        art.backdrop_shape.crop = true;
        let mut question = crate::quit_confirm::QuitConfirm::default();
        let list = question.scene();
        let (x, y) = list
            .ops()
            .iter()
            .find_map(|op| match op {
                Op::Text { x, y, text, .. } if text == "Yes" => Some((*x, *y)),
                _ => None,
            })
            .unwrap();
        let canvas = art.menu_canvas(screen, true);
        let projection = egui::emath::RectTransform::from_to(canvas, screen);
        // Near the top of the answer, not its centre: the old unprojected hit test misses this point.
        let at = projection * Fit::new(canvas).point(x, y - 20.0);
        let games = [crate::canvas::GameImage {
            id: egui::TextureId::User(42),
            width: 256,
            height: 224,
            pixel_aspect: 8.0 / 7.0,
        }];
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut answer = None;
        for events in [vec![egui::Event::PointerMoved(at)], vec![button(true), button(false)]] {
            let output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(screen), events, ..Default::default() },
                |ui| {
                    let layer = crate::ui::Layer { list: &list, art: &mut art, menu: true };
                    crate::ui::draw_game(ui, &games, Some(layer), true);
                    answer = crate::ui::quit_confirm_pointer(ui, &mut question);
                },
            );
            output.drop_without_applying_deltas();
        }
        assert_eq!(answer, Some(crate::quit_confirm::Outcome::Quit));
    }

    #[test]
    fn the_4_3_hud_fills_the_canvas_and_its_quit_question_remains_clickable() {
        let ctx = measuring_context();
        let mut question = crate::quit_confirm::QuitConfirm::default();
        let list = question.scene_sized(960.0, 720.0);
        for size in [egui::vec2(960.0, 720.0), egui::vec2(1280.0, 720.0)] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let mut art = Art::empty();
            art.backdrop_shape.crop = true;
            let canvas = art.menu_canvas(screen, true);
            let fit = Fit::for_list(canvas, &list);
            assert_eq!(fit.rect(Rect::new(0.0, 0.0, 960.0, 720.0)), canvas);
            let projection = egui::emath::RectTransform::from_to(canvas, screen);
            let at = projection * fit.point(600.0, 390.0);
            let mut answer = None;
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events: vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        },
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    let layer = crate::ui::Layer { list: &list, art: &mut art, menu: true };
                    crate::ui::draw_game(ui, &[], Some(layer), true);
                    answer = crate::ui::quit_confirm_pointer(ui, &mut question);
                },
            );
            output.drop_without_applying_deltas();
            assert_eq!(answer, Some(crate::quit_confirm::Outcome::Quit), "{size:?}");
        }
    }

    /// The egui context the menus' text is measured with: the theme's fonts, laid out once. The
    /// first pass uploads the font atlas, which nothing paints here, so its delta is cleared (epaint
    /// asserts in debug builds that a delta is handled before it is dropped).
    fn measuring_context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx
    }

    /// The boxes, in points, of what `list` paints over `fit`'s picture: a window with its frame and
    /// its corner caps, a portrait with its white line, the bars, the cursors and arrows, the text
    /// runs (measured in the font they are painted in). The backdrop is the picture itself: left out.
    fn painted_boxes(ctx: &egui::Context, fit: &Fit, list: &DrawList) -> Vec<egui::Rect> {
        let logical = |rect: Rect| fit.rect(rect);
        let mut boxes = Vec::new();
        for op in list.ops() {
            match op {
                Op::Backdrop => {}
                Op::Fill { rect, .. }
                | Op::Disc { rect, .. }
                | Op::Outline { rect, .. }
                | Op::Bar { rect, .. }
                | Op::Card { rect, .. }
                | Op::HandCard { rect, .. }
                | Op::Symbol { rect, .. } => {
                    boxes.push(logical(*rect));
                }
                Op::Window { rect } | Op::WindowHc { rect } => {
                    boxes.push(logical(rect.grow(layout::CAP_OVERHANG.max(layout::LINE))));
                }
                Op::Portrait { rect, .. } => boxes.push(logical(rect.grow(2.0))),
                Op::CursorDown { x, y } => {
                    boxes.push(logical(Rect::new(*x, *y, 18.0, 16.0)));
                }
                Op::Pointer { rect, .. } => boxes.push(logical(rect.grow(POINTER_EDGE / 2.0))),
                Op::Cursor { x, y } | Op::Arrow { x, y, .. } => {
                    boxes.push(logical(Rect::new(
                        *x,
                        *y,
                        layout::CURSOR_WIDTH,
                        layout::CURSOR_HEIGHT,
                    )));
                }
                Op::Line { from, to, width, .. } => {
                    let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
                    let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
                    boxes.push(logical(Rect::new(x0, y0, x1 - x0, y1 - y0).grow(width / 2.0)));
                }
                Op::Text { x, y, text, size, align, .. } => {
                    boxes.push(text_box(ctx, fit, (*x, *y), text, *size, *align));
                }
            }
        }
        boxes
    }

    /// The box, in points, of a text run of `text` at `size` logical pixels, anchored at `at` in logical pixels,
    /// measured in the font it is painted in.
    fn text_box(
        ctx: &egui::Context,
        fit: &Fit,
        at: (f32, f32),
        text: &str,
        size: f32,
        align: ssd_menus::Align,
    ) -> egui::Rect {
        let galley = ctx.fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                text.to_owned(),
                theme::game(size * fit.scale),
                egui::Color32::WHITE,
            )
        });
        let anchor = match align {
            ssd_menus::Align::Left => egui::Align2::LEFT_CENTER,
            ssd_menus::Align::Centre => egui::Align2::CENTER_CENTER,
            ssd_menus::Align::Right => egui::Align2::RIGHT_CENTER,
        };
        anchor.anchor_size(fit.point(at.0, at.1), galley.size())
    }

    /// The screens of the menus in their worst cases: every command and row, the longest names, a
    /// party of nine (the most the records hold), a full bag of 24 stacks, and each sheet.
    fn every_screen() -> Vec<DrawList> {
        let plain = plain_screens();
        let styles = [
            Style { large_text: true, ..Style::default() },
            Style { contrast: true, safe_cursor: true, ..Style::default() },
            Style { large_text: true, contrast: true, safe_cursor: true },
        ];
        let mut screens = plain.clone();
        for style in styles {
            screens.extend(plain.iter().map(|list| style.apply(list.clone())));
        }
        screens
    }

    /// An Order view of `count` members of the longest names.
    fn order_view(
        count: usize,
        cursor: usize,
        held: Option<usize>,
        changed: bool,
    ) -> ssd_menus::screens::OrderView {
        use ssd_menus::screens::{OrderMember, OrderView};
        OrderView {
            members: (0..count)
                .map(|i| OrderMember { name: format!("Tsumuri Maima {i}"), face: Some(28) })
                .collect(),
            cursor,
            held,
            changed,
            can_undo: changed,
        }
    }

    /// A Status view of `count` members of the worst figures: the longest name, six-digit figures, five techniques.
    fn status_view(count: usize, cursor: usize) -> ssd_menus::screens::StatusView {
        use ssd_menus::screens::{StatusMember, StatusView, Technique};
        let member = |i: usize| StatusMember {
            name: format!("Tien Shinhan {i}"),
            level: 99,
            hp: 9999,
            max_hp: 9999,
            ki: 9999,
            max_ki: 9999,
            bp: 16_777_215,
            pla: 99,
            exp: 16_777_215,
            to_next: 16_777_215,
            class: 2,
            face: Some(28),
            techniques: (0..5)
                .map(|n| Technique { name: format!("Dragon Thunder {n}"), cost: 255 })
                .collect(),
        };
        StatusView { members: (0..count).map(member).collect(), cursor }
    }

    /// The screens in the design as drawn: see [`every_screen`].
    fn plain_screens() -> Vec<DrawList> {
        use ssd_menus::cheats::{Cheats, Row, Setting, ZMode, ZTarget};
        use ssd_menus::view::{
            BagView, Member, MenuView, Screen, Selected, Sheet, Slot, StackView, Tab,
        };
        let member = |name: &str, face| Member {
            name: name.into(),
            level: 99,
            hp: 999,
            max_hp: 999,
            ki: 999,
            max_ki: 999,
            bp: 16_777_215,
            face,
        };
        let party: Vec<Member> =
            (0..9).map(|i| member(&format!("Tien Shinhan {i}"), Some(28))).collect();
        let base = MenuView {
            screen: Screen::Command,
            bag: None,
            town: false,
            flying: false,
            cursor: 0,
            page: 0,
            zeni: 999,
            party: party.clone(),
            items: Vec::new(),
            selected: None,
        };
        let mut screens = Vec::new();
        for cursor in 0..5 {
            let view = MenuView { cursor, ..base.clone() };
            screens.push(ssd_menus::screens::command(&view));
            screens.push(ssd_menus::screens::scene_aspect(&view, true));
        }
        for cursor in 0..4 {
            let view = MenuView { town: true, cursor, ..base.clone() };
            screens.push(ssd_menus::screens::command(&view));
            screens.push(ssd_menus::screens::scene_aspect(&view, true));
        }
        for cursor in 0..3 {
            let view = MenuView { flying: true, cursor, ..base.clone() };
            screens.push(ssd_menus::screens::command(&view));
            screens.push(ssd_menus::screens::scene_aspect(&view, true));
        }
        for cursor in 0..7 {
            let view = MenuView { screen: Screen::Menu, cursor, ..base.clone() };
            screens.push(ssd_menus::screens::scene(&view));
            screens.push(ssd_menus::screens::scene_aspect(&view, true));
        }
        let items: Vec<Slot> = (0..24)
            .map(|i| Slot { object: i, attribute: 0xC0, name: "Dragon Radar Plus".into() })
            .collect();
        let stacks: Vec<StackView> = (0..24)
            .map(|i| StackView {
                id: i,
                object: i as u8,
                name: "Dragon Radar Plus".into(),
                count: 99,
                usable: i % 2 == 0,
                droppable: true,
                ally: i % 3 == 0,
            })
            .collect();
        let selected = Selected {
            object: 2,
            name: "Dragon Radar Plus".into(),
            price: Some(255),
            help: vec!["Restores 25".into(), "HP and KI at once.".into(), "A third line.".into()],
            battle: true,
            field: true,
        };
        let bag = |sheet, notice| BagView {
            tab: Tab::All,
            stacks: stacks.clone(),
            used: 24,
            slots: 60,
            full: true,
            usable_here: true,
            droppable_here: true,
            sheet,
            choice: 1,
            notice,
        };
        for sheet in [Sheet::Browse, Sheet::Choose, Sheet::Confirm] {
            for page in 0..3 {
                for cursor in [0, 7] {
                    let view = MenuView {
                        screen: Screen::Items,
                        bag: Some(bag(sheet, Some("Bag full: nothing more can be taken."))),
                        items: items.clone(),
                        page,
                        cursor,
                        selected: Some(selected.clone()),
                        ..base.clone()
                    };
                    screens.push(ssd_menus::screens::items(&view));
                }
            }
        }
        screens.push(ssd_menus::screens::items(&MenuView {
            screen: Screen::Items,
            items,
            selected: None,
            ..base
        }));
        // The Status screen: a full party of the longest names and figures with the cursor on each member, and a short one.
        for cursor in 0..9 {
            screens.push(ssd_menus::screens::status_screen(&status_view(9, cursor)));
        }
        screens.push(ssd_menus::screens::status_screen(&status_view(1, 0)));
        // The Order screen: the longest names, the cursor on each slot and on Apply, a member held at each slot, a
        // changed order, and a party of nine (the most the records hold).
        for (count, changed) in [(5, false), (5, true), (9, true)] {
            for cursor in 0..=count {
                for held in [None, Some(cursor).filter(|&slot| slot < count)] {
                    screens.push(ssd_menus::screens::order_screen(&order_view(
                        count, cursor, held, changed,
                    )));
                }
            }
        }
        // The cheats screen: each row under the cursor, with nothing on and with every setting on (the longest
        // On list, the longest names), and the notice of a scripted fight.
        let names: Vec<String> = (0..12).map(|id| format!("Tsumuri Maima {id}")).collect();
        let mut busy = Cheats::neutral();
        for setting in Setting::ALL {
            busy.set_index(setting, setting.steps().len() - 1);
        }
        busy.set_random_battles_off(true);
        busy.set_z_mode(ZMode::Both);
        busy.set_z_target(ZTarget::Fighter(11));
        busy.set_zeni(true);
        busy.set_endless(true);
        for row in Row::ALL {
            for cheats in [Cheats::neutral(), busy] {
                let view = ssd_menus::view::CheatsView {
                    cheats,
                    row,
                    names: names.clone(),
                    spawner: None,
                };
                screens.push(ssd_menus::screens::cheats_screen(&view));
            }
        }
        // The Options screen: each page with the cursor on each of its rows (the two questions lie over the page they ask about, which
        // the text check cannot take).
        for category in ssd_menus::options::Category::ALL {
            for row in 0..category.rows().len().max(1) {
                let view = ssd_menus::view::OptionsView {
                    category,
                    row,
                    values: vec![1; category.rows().len()],
                    confirm: None,
                    video: None,
                    controls: None,
                };
                screens.push(ssd_menus::screens::options_screen(&view));
            }
        }
        screens.push(ssd_menus::screens::notice("Cheats off for this fight"));
        for which in [SlotList::Continue, SlotList::Save] {
            screens.push(ssd_menus::screens::slot_badges(which, [true; 3]));
        }
        screens
    }

    #[test]
    fn every_element_of_every_hd_screen_lies_inside_the_game_picture() {
        let ctx = measuring_context();
        let screens = every_screen();
        // A picture of the Classic frame (256 x 224 at 8:7), a square one, a widescreen one, at the
        // sizes the window takes with the panel open and closed, and at display scales of 1.25 and 1.5.
        let areas = [
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1040.0, 800.0)),
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 720.0)),
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0)),
            egui::Rect::from_min_size(egui::pos2(24.0, 40.0), egui::vec2(2560.0, 1360.0)),
        ];
        // ... the Classic frame (256 x 239 at 8:7), and the 16:9 frame of the widescreen view
        // (372 x 239 at 8:7), both in the live picture's rect.
        let frames = [
            (256, 224, 8.0 / 7.0),
            (256, 239, 8.0 / 7.0),
            (372, 239, 8.0 / 7.0),
            (256, 224, 1.0),
            (320, 180, 1.0),
        ];
        for ppp in [1.0, 1.25, 1.5] {
            for area in areas {
                for ((width, height, aspect), whole) in
                    frames.into_iter().flat_map(|frame| [(frame, true), (frame, false)])
                {
                    let picture =
                        crate::layout::picture_rect(ppp, area, width, height, aspect, whole);
                    assert!(area.contains_rect(picture), "{picture:?} in {area:?}");
                    for (index, list) in screens.iter().enumerate() {
                        let fit = Fit::for_list(picture, list);
                        let (width, height) = list.canvas_size();
                        let canvas = fit.rect(Rect::new(0.0, 0.0, width, height));
                        assert!(
                            picture.expand(0.01).contains_rect(canvas),
                            "the logical space fits"
                        );
                        for boxed in painted_boxes(&ctx, &fit, list) {
                            assert!(
                                picture.expand(0.01).contains_rect(boxed),
                                "screen {index} at {ppp} ppp, {width}x{height} in {area:?}: \
                                 {boxed:?} is outside the picture {picture:?}"
                            );
                            assert!(
                                canvas.expand(0.01).contains_rect(boxed),
                                "screen {index}: {boxed:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn no_text_leaves_its_window_and_no_two_runs_share_their_capitals() {
        let ctx = measuring_context();
        let screens = every_screen();
        // The logical space at one point a pixel, so the sizes are the design's.
        let fit = Fit::new(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(layout::WIDTH, layout::HEIGHT),
        ));
        for (index, list) in screens.iter().enumerate() {
            let windows: Vec<Rect> = list
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::Window { rect } | Op::WindowHc { rect } => Some(*rect),
                    _ => None,
                })
                .collect();
            // Each run with its capitals' band: the cap height centred on the run's box, which two runs must not share.
            let mut runs: Vec<(String, egui::Rect)> = Vec::new();
            for op in list.ops() {
                let Op::Text { x, y, text, size, align, .. } = op else { continue };
                let boxed = text_box(&ctx, &fit, (*x, *y), text, *size, *align);
                // The innermost window the run starts in: the run lies inside its cream paper.
                let innermost = windows
                    .iter()
                    .filter(|w| w.x <= *x && *x <= w.right() && w.y <= *y && *y <= w.bottom())
                    .min_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)));
                if let Some(window) = innermost {
                    let paper = fit.rect(window.grow(-(layout::FRAME + layout::LINE)));
                    assert!(
                        paper.expand(0.01).contains_rect(boxed),
                        "screen {index}: {text:?} at {boxed:?} leaves its window's paper {paper:?}"
                    );
                }
                let cap = 0.7 * size * fit.scale;
                let band =
                    egui::Rect::from_center_size(boxed.center(), egui::vec2(boxed.width(), cap));
                runs.push((text.clone(), band));
            }
            for (i, (a_text, a_band)) in runs.iter().enumerate() {
                for (b_text, b_band) in &runs[i + 1..] {
                    // A shadow is the same text drawn twice, so two equal runs may share their place.
                    if a_text == b_text {
                        continue;
                    }
                    let shared = a_band.intersect(*b_band);
                    assert!(
                        shared.width() <= 0.5 || shared.height() <= 0.5,
                        "screen {index}: {a_text:?} at {a_band:?} and {b_text:?} at {b_band:?} share {shared:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_scripted_fight_shows_the_off_notice_and_the_map_the_on_notice_for_two_seconds() {
        let mut wram = vec![0_u8; 0x2_0000];
        let mut notice = CheatNotice::default();
        notice.note(&wram);
        assert_eq!(notice.text(), None, "no cheat block: no notice");
        wram[GATE] = 1;
        notice.note(&wram);
        assert_eq!(
            notice.text(),
            None,
            "a gate byte without the block's marker is the power-on fill"
        );
        wram[BLOCK_MARKER..BLOCK_MARKER + MARKER_BYTES.len()].copy_from_slice(&MARKER_BYTES);
        notice.note(&wram);
        assert_eq!(notice.text(), Some(OFF_NOTICE), "the fight suspends the cheats");
        for _ in 1..NOTICE_FRAMES - 1 {
            notice.note(&wram);
        }
        assert_eq!(notice.text(), Some(OFF_NOTICE), "it shows for two seconds");
        notice.note(&wram);
        assert_eq!(notice.text(), None, "then it goes");
        for _ in 0..NOTICE_FRAMES {
            notice.note(&wram);
        }
        assert_eq!(notice.text(), None, "a gate that stays up shows its notice once");
        wram[GATE] = 0;
        notice.note(&wram);
        assert_eq!(notice.text(), Some(ON_NOTICE), "the map brings the cheats back");
    }

    #[test]
    fn the_original_slot_lists_are_found_from_the_game_mode_and_the_menu_step() {
        let mut wram = vec![0_u8; 0x2_0000];
        assert_eq!(slot_list(&wram), None);
        wram[0x24] = TITLE_SLOTS_MODE;
        assert_eq!(slot_list(&wram), Some(SlotList::Continue), "the title's Which slot?");
        wram[0x24] = 0x87;
        assert_eq!(slot_list(&wram), None, "the title's menu is not a slot list");
        let screen = (d::MENU_SCREEN - 0x7E_0000) as usize;
        let step = (d::MENU_STEP - 0x7E_0000) as usize;
        wram[screen] = SAVE_SCREEN;
        assert_eq!(slot_list(&wram), None, "the Save's step 0 is its question");
        wram[step] = SAVE_LIST_STEP;
        assert_eq!(slot_list(&wram), Some(SlotList::Save), "the Save's Which slot?");
        wram[screen] = 4;
        assert_eq!(slot_list(&wram), None, "the Menu list is no slot list");
    }

    #[test]
    fn the_font_of_the_menus_is_registered_in_the_theme() {
        let _ = theme::game(32.0);
    }

    #[test]
    fn a_button_is_a_key_when_it_goes_down_and_the_menu_reads_the_d_pad_and_a_b() {
        let none = Joypad::default();
        let down = Joypad { down: true, ..Joypad::default() };
        let down_and_a = Joypad { down: true, a: true, ..Joypad::default() };
        assert_eq!(keys_pressed(none, down), [Key::Down]);
        assert_eq!(keys_pressed(down, down), [], "held, not pressed again");
        assert_eq!(keys_pressed(down, down_and_a), [Key::Confirm]);
        let b = Joypad { b: true, ..Joypad::default() };
        assert_eq!(keys_pressed(none, b), [Key::Cancel]);
        assert_eq!(keys_pressed(none, Joypad { start: true, ..Joypad::default() }), []);
        let l = Joypad { l: true, ..Joypad::default() };
        let r = Joypad { r: true, ..Joypad::default() };
        assert_eq!(
            (keys_pressed(none, l), keys_pressed(none, r)),
            (vec![Key::TabBack], vec![Key::Tab])
        );
        let y = Joypad { y: true, ..Joypad::default() };
        let select = Joypad { select: true, ..Joypad::default() };
        assert_eq!(keys_pressed(none, y), [Key::ResetPage]);
        assert_eq!(keys_pressed(none, select), [Key::ResetAll]);
        let cross = Joypad { x: true, ..Joypad::default() };
        assert_eq!(keys_pressed(none, cross), [Key::Apply], "X applies the data packs");
        assert_eq!(keys_pressed(cross, cross), [], "a held X counts once");
    }
}
