//! The question Esc asks before the game ends: "Quit the game?" with No and Yes, in a window of the HD menus' style
//! centred on the picture (the words go through the menus' table, so a translation shows them in its language). While
//! it asks the game is held: no frame runs and no key reaches it, while the window's shortcuts still work. Esc again
//! or No closes it; Yes ends the run the way Esc did before. The arrows (and the d-pad) move the cursor, Enter and the
//! key or button of the SNES A confirm; the mouse moves the cursor over an answer and a click gives it.

use crate::input::PadButton;
use crate::options::controls::Bindings;
use crate::panel::{KeyRoute, PanelAction};
use sdl3::gamepad::Button;
use sdl3::keyboard::Scancode;
use ssd_menus::layout::{HEIGHT, LARGE_TEXT, LINE, WIDTH, palette};
use ssd_menus::{Align, DrawList, Op, Rect};

/// The answer under the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    No,
    Yes,
}

/// A press the question takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// The cursor to No.
    Left,
    /// The cursor to Yes.
    Right,
    /// The cursor to the other answer (up and down).
    Toggle,
    /// The answer under the cursor.
    Confirm,
}

/// How the question ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// No (or Esc again): the game goes on.
    Close,
    /// Yes: the run ends.
    Quit,
}

/// What a key press does, the question open or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The game's joypad gets it (the question is closed).
    Game,
    /// A shortcut of the window.
    Action(PanelAction),
    /// The question opened: the keys held are let go.
    Opened,
    /// The question closed: the game goes on, with no key held.
    Closed,
    /// Yes: the run ends.
    Quit,
    /// Nobody gets it.
    Nothing,
}

/// The window: as wide and high as the save's "Overwrite slot?" question, centred on the logical screen.
const WINDOW: Rect =
    Rect { x: (WIDTH - WINDOW_W) / 2.0, y: (HEIGHT - WINDOW_H) / 2.0, w: WINDOW_W, h: WINDOW_H };
const WINDOW_W: f32 = 520.0;
const WINDOW_H: f32 = 200.0;
/// The line of the question.
const QUESTION_Y: f32 = WINDOW.y + 56.0;
/// The centres of the answers: No on the left, Yes on the right.
const NO_CENTRE: f32 = WIDTH / 2.0 - 120.0;
const YES_CENTRE: f32 = WIDTH / 2.0 + 120.0;
/// The chip behind the answer under the cursor.
const CHIP_TOP: f32 = WINDOW.y + 130.0;
const CHIP_WIDTH: f32 = 120.0;
const CHIP_HEIGHT: f32 = 44.0;
/// The size of the answers.
const ANSWER_TEXT: f32 = 24.0;

/// The question while it is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuitConfirm {
    /// The cursor is on Yes (it opens on No).
    yes: bool,
}

impl QuitConfirm {
    /// The answer under the cursor.
    #[must_use]
    pub fn choice(self) -> Choice {
        if self.yes { Choice::Yes } else { Choice::No }
    }

    /// A press: the cursor moves, or the answer under it is given.
    pub fn press(&mut self, press: Press) -> Option<Outcome> {
        match press {
            Press::Left => self.yes = false,
            Press::Right => self.yes = true,
            Press::Toggle => self.yes = !self.yes,
            Press::Confirm => return Some(self.outcome()),
        }
        None
    }

    /// The mouse at `at` (logical pixels): over an answer, the cursor goes to it.
    pub fn hover(&mut self, at: (f32, f32)) {
        if let Some(choice) = answer_at(at) {
            self.yes = choice == Choice::Yes;
        }
    }

    /// A click at `at` (logical pixels): on an answer, that answer is given.
    pub fn click(&mut self, at: (f32, f32)) -> Option<Outcome> {
        let choice = answer_at(at)?;
        self.yes = choice == Choice::Yes;
        Some(self.outcome())
    }

    fn outcome(self) -> Outcome {
        if self.yes { Outcome::Quit } else { Outcome::Close }
    }

    /// The window in the menus' logical space (English words: the HD layer translates them).
    #[must_use]
    pub fn scene(self) -> DrawList {
        self.scene_sized(WIDTH, HEIGHT)
    }

    /// The same question centred on the field HUD's canvas.
    #[must_use]
    pub fn scene_sized(self, width: f32, height: f32) -> DrawList {
        let mut list = DrawList::sized(width, height);
        let dx = (width - WIDTH) / 2.0;
        let dy = (height - HEIGHT) / 2.0;
        let window = Rect::new(WINDOW.x + dx, WINDOW.y + dy, WINDOW.w, WINDOW.h);
        list.push(Op::Window { rect: window });
        list.text(
            width / 2.0,
            QUESTION_Y + dy,
            "Quit the game?",
            LARGE_TEXT,
            palette::BLACK,
            Align::Centre,
        );
        for (choice, centre, label) in
            [(Choice::No, NO_CENTRE, "No"), (Choice::Yes, YES_CENTRE, "Yes")]
        {
            let on = self.choice() == choice;
            if on {
                let mut chip = chip(centre);
                chip.x += dx;
                chip.y += dy;
                list.fill(chip, palette::ORANGE);
                list.push(Op::Outline { rect: chip, width: LINE, color: palette::DARK_RED });
            }
            let color = if on { palette::CREAM } else { palette::BLACK };
            list.text(
                centre + dx,
                chip(centre).middle_y() + dy,
                label,
                ANSWER_TEXT,
                color,
                Align::Centre,
            );
        }
        list
    }
}

/// The chip of the answer centred at `centre`.
fn chip(centre: f32) -> Rect {
    Rect::new(centre - CHIP_WIDTH / 2.0, CHIP_TOP, CHIP_WIDTH, CHIP_HEIGHT)
}

/// The answer whose chip holds `at`.
fn answer_at((x, y): (f32, f32)) -> Option<Choice> {
    [(Choice::No, NO_CENTRE), (Choice::Yes, YES_CENTRE)].into_iter().find_map(|(choice, centre)| {
        let rect = chip(centre);
        (x >= rect.x && x <= rect.right() && y >= rect.y && y <= rect.bottom()).then_some(choice)
    })
}

/// Where a key press goes, with the question `asking` (opened and closed here). `route` is where the window sends the
/// key ([`crate::panel::route_key`]), `press` what it is to the question ([`key_press`], [`button_press`]). Closed,
/// the Quit key opens the question and the others go where they went; open, the Quit key closes it, a shortcut still
/// works, and a game key is the question's or nobody's.
pub fn key(asking: &mut Option<QuitConfirm>, route: KeyRoute, press: Option<Press>) -> Step {
    match (asking.as_mut(), route) {
        (_, KeyRoute::TextField) => Step::Nothing,
        (_, KeyRoute::Action(action)) => Step::Action(action),
        (None, KeyRoute::Quit) => {
            *asking = Some(QuitConfirm::default());
            Step::Opened
        }
        (None, KeyRoute::Game) => Step::Game,
        (Some(_), KeyRoute::Quit) => settle(asking, Outcome::Close),
        (Some(question), KeyRoute::Game) => match press.and_then(|press| question.press(press)) {
            Some(outcome) => settle(asking, outcome),
            None => Step::Nothing,
        },
    }
}

/// The question ends with `outcome` (a key's or a click's).
pub fn settle(asking: &mut Option<QuitConfirm>, outcome: Outcome) -> Step {
    *asking = None;
    match outcome {
        Outcome::Close => Step::Closed,
        Outcome::Quit => Step::Quit,
    }
}

/// What a key is to the question: the arrows, Enter, and the keys of the player's map for the d-pad and the SNES A.
#[must_use]
pub fn key_press(bindings: &Bindings, sc: Scancode) -> Option<Press> {
    match sc {
        Scancode::Left => Some(Press::Left),
        Scancode::Right => Some(Press::Right),
        Scancode::Up | Scancode::Down => Some(Press::Toggle),
        Scancode::Return | Scancode::KpEnter => Some(Press::Confirm),
        _ => bindings.key_button(sc).and_then(pad_press),
    }
}

/// What a gamepad button is to the question: the d-pad, and the buttons of the player's map for the SNES A.
#[must_use]
pub fn button_press(bindings: &Bindings, button: Button) -> Option<Press> {
    match button {
        Button::DPadLeft => Some(Press::Left),
        Button::DPadRight => Some(Press::Right),
        Button::DPadUp | Button::DPadDown => Some(Press::Toggle),
        _ => bindings.pad_button(button).and_then(pad_press),
    }
}

/// What a SNES button is to the question.
fn pad_press(button: PadButton) -> Option<Press> {
    match button {
        PadButton::Left => Some(Press::Left),
        PadButton::Right => Some(Press::Right),
        PadButton::Up | PadButton::Down => Some(Press::Toggle),
        PadButton::A => Some(Press::Confirm),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::PadButton;

    fn open() -> Option<QuitConfirm> {
        let mut asking = None;
        assert_eq!(key(&mut asking, KeyRoute::Quit, None), Step::Opened);
        asking
    }

    #[test]
    fn esc_opens_the_question_with_the_cursor_on_no() {
        let asking = open();
        assert_eq!(asking.map(QuitConfirm::choice), Some(Choice::No));
    }

    #[test]
    fn esc_again_closes_it_and_the_game_goes_on() {
        let mut asking = open();
        assert_eq!(key(&mut asking, KeyRoute::Quit, None), Step::Closed);
        assert!(asking.is_none());
    }

    #[test]
    fn confirming_no_closes_and_yes_quits() {
        let mut asking = open();
        assert_eq!(key(&mut asking, KeyRoute::Game, Some(Press::Confirm)), Step::Closed);
        assert!(asking.is_none());
        let mut asking = open();
        assert_eq!(key(&mut asking, KeyRoute::Game, Some(Press::Right)), Step::Nothing);
        assert_eq!(asking.map(QuitConfirm::choice), Some(Choice::Yes));
        assert_eq!(key(&mut asking, KeyRoute::Game, Some(Press::Confirm)), Step::Quit);
    }

    #[test]
    fn the_arrows_move_between_the_two_answers() {
        let mut asking = open();
        key(&mut asking, KeyRoute::Game, Some(Press::Toggle));
        assert_eq!(asking.map(QuitConfirm::choice), Some(Choice::Yes));
        key(&mut asking, KeyRoute::Game, Some(Press::Toggle));
        assert_eq!(asking.map(QuitConfirm::choice), Some(Choice::No));
        key(&mut asking, KeyRoute::Game, Some(Press::Right));
        key(&mut asking, KeyRoute::Game, Some(Press::Left));
        assert_eq!(asking.map(QuitConfirm::choice), Some(Choice::No));
    }

    #[test]
    fn while_it_asks_the_game_gets_no_key_but_the_shortcuts_still_work() {
        let mut asking = open();
        // A game key that is not one of the question's (B, Start...): nobody gets it.
        assert_eq!(key(&mut asking, KeyRoute::Game, None), Step::Nothing);
        assert_eq!(key(&mut asking, KeyRoute::Game, Some(Press::Left)), Step::Nothing);
        assert!(asking.is_some(), "still asking");
        let shot = KeyRoute::Action(PanelAction::Screenshot);
        assert_eq!(key(&mut asking, shot, None), Step::Action(PanelAction::Screenshot));
        assert_eq!(key(&mut asking, KeyRoute::TextField, Some(Press::Confirm)), Step::Nothing);
        assert!(asking.is_some());
    }

    #[test]
    fn closed_the_keys_go_where_they_went() {
        let mut asking = None;
        assert_eq!(key(&mut asking, KeyRoute::Game, Some(Press::Confirm)), Step::Game);
        let shot = KeyRoute::Action(PanelAction::Screenshot);
        assert_eq!(key(&mut asking, shot, None), Step::Action(PanelAction::Screenshot));
        assert_eq!(key(&mut asking, KeyRoute::TextField, None), Step::Nothing);
        assert!(asking.is_none());
    }

    #[test]
    fn enter_and_the_a_button_confirm_and_the_arrows_move() {
        let bindings = Bindings::default();
        assert_eq!(key_press(&bindings, Scancode::Return), Some(Press::Confirm));
        assert_eq!(key_press(&bindings, Scancode::KpEnter), Some(Press::Confirm));
        assert_eq!(key_press(&bindings, Scancode::Left), Some(Press::Left));
        assert_eq!(key_press(&bindings, Scancode::Right), Some(Press::Right));
        assert_eq!(key_press(&bindings, Scancode::Up), Some(Press::Toggle));
        assert_eq!(key_press(&bindings, Scancode::F12), None);
        // The key the player's map makes the SNES A confirms (X by default); the one of B (Z) does not.
        assert_eq!(bindings.key_button(Scancode::X), Some(PadButton::A));
        assert_eq!(key_press(&bindings, Scancode::X), Some(Press::Confirm));
        assert_eq!(bindings.key_button(Scancode::Z), Some(PadButton::B));
        assert_eq!(key_press(&bindings, Scancode::Z), None);
        assert_eq!(button_press(&bindings, Button::DPadLeft), Some(Press::Left));
        assert_eq!(button_press(&bindings, Button::DPadRight), Some(Press::Right));
        assert_eq!(button_press(&bindings, Button::East), Some(Press::Confirm), "the game's A");
        assert_eq!(button_press(&bindings, Button::South), None, "the game's B");
    }

    #[test]
    fn the_window_is_centred_with_the_question_and_its_answers() {
        let list = QuitConfirm::default().scene();
        let window = list
            .ops()
            .iter()
            .find_map(|op| match op {
                Op::Window { rect } => Some(*rect),
                _ => None,
            })
            .expect("a window");
        assert!((window.x + window.w / 2.0 - WIDTH / 2.0).abs() < 0.01, "centred across");
        assert!((window.y + window.h / 2.0 - HEIGHT / 2.0).abs() < 0.01, "centred down");
        let texts: Vec<&str> = list.texts().map(|(_, _, text)| text).collect();
        assert_eq!(texts, ["Quit the game?", "No", "Yes"]);
        // No is left of Yes.
        let xs: Vec<f32> = list.texts().map(|(x, _, _)| x).collect();
        assert!(xs[1] < xs[2]);
    }

    #[test]
    fn the_chip_follows_the_cursor() {
        let mut q = QuitConfirm::default();
        let on_no = q.scene();
        q.press(Press::Right);
        let on_yes = q.scene();
        assert_ne!(on_no, on_yes);
        let chip = |list: &DrawList| {
            list.ops().iter().find_map(|op| match op {
                Op::Fill { rect, .. } => Some(rect.x + rect.w / 2.0),
                _ => None,
            })
        };
        assert_eq!(chip(&on_no), Some(NO_CENTRE));
        assert_eq!(chip(&on_yes), Some(YES_CENTRE));
    }

    #[test]
    fn the_mouse_moves_the_cursor_and_a_click_answers() {
        let mut q = QuitConfirm::default();
        let yes = (YES_CENTRE, CHIP_TOP + CHIP_HEIGHT / 2.0);
        let no = (NO_CENTRE, CHIP_TOP + CHIP_HEIGHT / 2.0);
        q.hover(yes);
        assert_eq!(q.choice(), Choice::Yes);
        q.hover((WIDTH / 2.0, 0.0));
        assert_eq!(q.choice(), Choice::Yes, "off the answers the cursor stays");
        q.hover(no);
        assert_eq!(q.choice(), Choice::No);
        assert_eq!(q.click((0.0, 0.0)), None);
        assert_eq!(q.click(yes), Some(Outcome::Quit));
        assert_eq!(q.click(no), Some(Outcome::Close));
    }

    #[test]
    fn the_taiwan_table_has_the_question_and_its_answers() {
        let table = ssd_menus::ui_text::UiText::parse(include_str!(
            "../../../translations/zh-tw/ui-zh-tw.tsv"
        ));
        let mut list = QuitConfirm::default().scene();
        list.map_text(|text| table.get_in(None, text));
        let texts: Vec<&str> = list.texts().map(|(_, _, text)| text).collect();
        assert_eq!(texts, ["要離開遊戲嗎？", "否", "是"]);
    }

    #[test]
    fn a_click_settles_as_a_key_does() {
        let mut asking = open();
        assert_eq!(settle(&mut asking, Outcome::Close), Step::Closed);
        assert!(asking.is_none());
        let mut asking = open();
        assert_eq!(settle(&mut asking, Outcome::Quit), Step::Quit);
    }
}
