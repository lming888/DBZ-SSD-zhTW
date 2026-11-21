//! The draw list: what an HD screen paints, as plain data in logical pixels. The screens build it
//! and the backend of `ssd-app` paints it, so a layout can be checked without a window or a GPU.
use crate::layout::ADVANCE;
use crate::view::Picture;

/// An RGBA colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    /// An opaque colour from its red, green and blue parts.
    #[must_use]
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self([red, green, blue, 255])
    }
}

/// A rectangle in logical pixels, from its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// The rectangle with its top-left corner at (`x`, `y`).
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// The right edge.
    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// The bottom edge.
    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// The rectangle grown by `d` on every side (shrunk for a negative `d`).
    #[must_use]
    pub fn grow(&self, d: f32) -> Self {
        Self::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }

    /// The vertical middle of the rectangle.
    #[must_use]
    pub fn middle_y(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

/// Where a text run lies horizontally, from its `x`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// The run starts at `x`.
    Left,
    /// The run is centred on `x`.
    Centre,
    /// The run ends at `x`.
    Right,
}

/// A part of a card of the hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardPart {
    /// The whole card.
    Whole,
    /// The ball in the top-left corner (the attack value).
    Attack,
    /// The kanji square in the middle.
    Symbol,
    /// The ball in the bottom-right corner (the defence value).
    Defence,
}

/// One step of a draw list.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// The ground of the screen: the field as it was when the menu opened. The app paints the frame
    /// it froze over the game picture (the rect the live frame is drawn in); nothing is painted where
    /// no frame was frozen.
    Backdrop,
    /// A filled rectangle.
    Fill { rect: Rect, color: Color },
    /// A filled disc inside `rect`: a circle when the rectangle is square (the balls of a card).
    Disc { rect: Rect, color: Color },
    /// The outline of a rectangle, drawn inside its bounds.
    Outline { rect: Rect, width: f32, color: Color },
    /// A menu window: cream paper in an orange frame, dark-red lines outside and inside the frame
    /// and a corner cap on each corner. `rect` is the outside of the frame.
    Window { rect: Rect },
    /// A straight line.
    Line { from: (f32, f32), to: (f32, f32), width: f32, color: Color },
    /// A line of text, vertically centred on `y`.
    Text { x: f32, y: f32, text: String, size: f32, color: Color, align: Align },
    /// The orange cursor, a right-pointing triangle with its top-left corner at (`x`, `y`).
    Cursor { x: f32, y: f32 },
    /// A small orange triangle pointing left or right (the cheats' arrows, which the font has no glyph
    /// for), the cursor's size, with its top-left corner at (`x`, `y`).
    Arrow { x: f32, y: f32, left: bool },
    /// A menu window in high contrast: white paper in a black frame with a white line outside it, so it stands off
    /// the field. `rect` is the outside of the frame, as for [`Op::Window`].
    WindowHc { rect: Rect },
    /// A triangle in `rect` pointing right (or left), filled with `fill`, outlined with a wide `edge` line and a
    /// narrower `ring` line on it: the safe cursor, and the arrows in high contrast.
    Pointer { rect: Rect, left: bool, fill: Color, edge: Color, ring: Color },
    /// A picture in a square portrait frame, or an empty box when there is nothing to show (an empty list,
    /// or a character without a corner face). `rect` is the outside of the frame.
    Portrait { rect: Rect, picture: Option<Picture> },
    /// The card of the Cheats screen's preview: the battle's HD card drawn from the pack's card art (the
    /// attack ball with four dots or the Z, the symbol, the defence ball with the numeral seven or the Z)
    /// to fit `rect`, which is the outside of the card's frame. A backend with no card art paints
    /// [`crate::screens::pixel_card`] instead.
    Card { rect: Rect, attack_z: bool, defence_z: bool },
    /// A card of the hand of the Cards screen, or a part of it, drawn from the 4 x 6 tile words the game wrote for it
    /// (`tiles`, row by row) to fit `rect`. A backend with no card art paints [`crate::screens::cards::flat_part`].
    HandCard { rect: Rect, tiles: Vec<u16>, part: CardPart },
    /// A small orange triangle pointing down (the cursor over a card), 18 wide and 16 high, with its top-left corner at
    /// (`x`, `y`).
    CursorDown { x: f32, y: f32 },
    /// The kanji or emblem of a card symbol (0 to 6, a fighter's class) in the HD art, filling `rect`. A backend with no
    /// card art paints nothing there.
    Symbol { rect: Rect, symbol: u8 },
    /// A bar filled to `fraction` (0 to 1) of its well, with a white border. `rect` is the outside.
    Bar { rect: Rect, fraction: f32, color: Color },
}

/// The steps of one screen, in the order they are painted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawList {
    ops: Vec<Op>,
    canvas_size: Option<(f32, f32)>,
    /// The texts the screens drew as several runs, or as one run made of parts: [`DrawList::map_text`] asks for the
    /// whole first.
    wholes: Vec<Whole>,
}

/// A text a screen drew as more than the one run the table is asked for.
#[derive(Debug, Clone, PartialEq)]
enum Whole {
    /// The runs at `runs` (indices of steps, in order) are the lines of `text` wrapped to `width` pixels at `size`,
    /// `pitch` apart, at most `most` of them.
    Lines { runs: Vec<usize>, text: String, width: f32, size: f32, pitch: f32, most: usize },
    /// The run at `run` is `parts` joined with `sep`.
    Parts { run: usize, parts: Vec<String>, sep: String },
    /// The runs at `done` and `rest` (none when all of it is written) are `text` being typed, its first `written`
    /// characters and the others, and the steps at `marks` the mark after the last written character.
    Typed { done: usize, rest: Option<usize>, marks: Vec<usize>, text: String, written: usize },
}

impl Whole {
    /// The whole with its runs at the new indices `moved` gives (`None` for a run that is not there any more, which
    /// drops the whole).
    fn moved(&self, moved: impl Fn(usize) -> Option<usize>) -> Option<Self> {
        Some(match self {
            Self::Lines { runs, text, width, size, pitch, most } => Self::Lines {
                runs: runs.iter().map(|&run| moved(run)).collect::<Option<_>>()?,
                text: text.clone(),
                width: *width,
                size: *size,
                pitch: *pitch,
                most: *most,
            },
            Self::Parts { run, parts, sep } => {
                Self::Parts { run: moved(*run)?, parts: parts.clone(), sep: sep.clone() }
            }
            Self::Typed { done, rest, marks, text, written } => Self::Typed {
                done: moved(*done)?,
                rest: match rest {
                    Some(rest) => Some(moved(*rest)?),
                    None => None,
                },
                marks: marks.iter().map(|&mark| moved(mark)).collect::<Option<_>>()?,
                text: text.clone(),
                written: *written,
            },
        })
    }
}

impl DrawList {
    /// An empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty list in a logical canvas other than the default widescreen design.
    #[must_use]
    pub fn sized(width: f32, height: f32) -> Self {
        let size = (width, height);
        Self {
            canvas_size: (size != (crate::layout::WIDTH, crate::layout::HEIGHT)).then_some(size),
            ..Self::default()
        }
    }

    /// The logical canvas the backend fits to the picture.
    #[must_use]
    pub fn canvas_size(&self) -> (f32, f32) {
        self.canvas_size.unwrap_or((crate::layout::WIDTH, crate::layout::HEIGHT))
    }

    /// The steps, in paint order.
    #[must_use]
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Appends a step.
    pub fn push(&mut self, op: Op) {
        self.ops.push(op);
    }

    /// Appends the steps of `other`, in their order.
    pub fn append(&mut self, other: DrawList) {
        let base = self.ops.len();
        self.wholes
            .extend(other.wholes.iter().filter_map(|whole| whole.moved(|run| Some(base + run))));
        self.ops.extend(other.ops);
    }

    /// The last `runs` text runs are the lines of `text` wrapped to `chars` characters (as [`crate::screens::wrap`]
    /// does), `pitch` apart, at most `most` of them: [`DrawList::map_text`] translates `text` whole and wraps the
    /// translation again to the same width. Nothing is marked when there are no such runs.
    pub fn mark_lines(&mut self, text: &str, runs: usize, chars: usize, pitch: f32, most: usize) {
        let at: Vec<usize> = (0..self.ops.len())
            .rev()
            .filter(|&at| matches!(self.ops[at], Op::Text { .. }))
            .take(runs)
            .collect();
        let Some(&last) = at.first() else { return };
        if runs == 0 || at.len() < runs {
            return;
        }
        let Op::Text { size, .. } = self.ops[last] else { return };
        self.wholes.push(Whole::Lines {
            runs: at.into_iter().rev().collect(),
            text: text.to_owned(),
            width: chars as f32 * ADVANCE * size,
            size,
            pitch,
            most,
        });
    }

    /// The last text run is `parts` joined with `sep`: when the table has no row for the run, [`DrawList::map_text`]
    /// translates each part and joins them again.
    pub fn mark_parts(&mut self, parts: &[&str], sep: &str) {
        if let Some(run) =
            (0..self.ops.len()).rev().find(|&at| matches!(self.ops[at], Op::Text { .. }))
        {
            let parts = parts.iter().map(|&part| part.to_owned()).collect();
            self.wholes.push(Whole::Parts { run, parts, sep: sep.to_owned() });
        }
    }

    /// The text run at step `done` is the first `written` characters of `text`, the run at `rest` (if any) the others,
    /// and the steps from `marks` the mark between them (fills, placed after the last written character): when the table
    /// has `text`, [`DrawList::map_text`] writes as large a share of the translation, the mark and the rest moved after
    /// it. Nothing is marked when `done` is not a text run.
    pub fn mark_typed(
        &mut self,
        text: &str,
        written: usize,
        done: usize,
        rest: Option<usize>,
        marks: std::ops::Range<usize>,
    ) {
        if matches!(self.ops.get(done), Some(Op::Text { .. })) {
            self.wholes.push(Whole::Typed {
                done,
                rest,
                marks: marks.collect(),
                text: text.to_owned(),
                written,
            });
        }
    }

    /// The marks of `from` ([`DrawList::mark_lines`], [`DrawList::mark_parts`], [`DrawList::mark_typed`]) on this list, which has the run of
    /// step `at` of `from` at step `moved[at]` (`None`: not kept).
    pub(crate) fn keep_marks(&mut self, from: &DrawList, moved: &[Option<usize>]) {
        let wholes = from
            .wholes
            .iter()
            .filter_map(|whole| whole.moved(|run| moved.get(run).copied().flatten()));
        self.wholes.extend(wholes);
    }

    /// Appends a filled rectangle.
    pub fn fill(&mut self, rect: Rect, color: Color) {
        self.push(Op::Fill { rect, color });
    }

    /// Appends a text run.
    pub fn text(&mut self, x: f32, y: f32, text: &str, size: f32, color: Color, align: Align) {
        self.push(Op::Text { x, y, text: text.to_owned(), size, color, align });
    }

    /// Replaces the text of each run with what `translate` gives for it (a run it gives `None` for keeps its own).
    ///
    /// A text marked as wrapped lines ([`DrawList::mark_lines`]) is asked for whole first: when `translate` has it, its
    /// lines give way to the translation wrapped to the same width ([`wrap_wide`]), the first where the first line was
    /// and the others `pitch` apart. A run marked as joined parts ([`DrawList::mark_parts`]) that `translate` has no
    /// translation for is the translations of its parts (or their English) joined again. The marks are used up.
    pub fn map_text(&mut self, translate: impl Fn(&str) -> Option<String>) {
        // What each step becomes: `None` as it is (its text translated), `Some` these steps instead.
        let mut instead: Vec<Option<Vec<Op>>> = vec![None; self.ops.len()];
        for whole in std::mem::take(&mut self.wholes) {
            match whole {
                Whole::Lines { runs, text, width, size, pitch, most } => {
                    let Some(translated) = translate(&text) else { continue };
                    let Some(Op::Text { x, y, size: drawn, color, align, .. }) =
                        self.ops.get(runs[0]).cloned()
                    else {
                        continue;
                    };
                    let lines = wrap_wide(&translated, width, size);
                    let new = lines.into_iter().take(most).enumerate().map(|(line, text)| {
                        Op::Text { x, y: y + pitch * line as f32, text, size: drawn, color, align }
                    });
                    instead[runs[0]] = Some(new.collect());
                    for &run in &runs[1..] {
                        instead[run] = Some(Vec::new());
                    }
                }
                Whole::Parts { run, parts, sep } => {
                    let Some(Op::Text { text, .. }) = self.ops.get(run) else { continue };
                    if translate(text).is_some() {
                        continue;
                    }
                    let parts: Vec<String> = parts
                        .iter()
                        .map(|part| translate(part).unwrap_or_else(|| part.clone()))
                        .collect();
                    let mut op = self.ops[run].clone();
                    if let Op::Text { text, .. } = &mut op {
                        *text = parts.join(&sep);
                    }
                    instead[run] = Some(vec![op]);
                }
                Whole::Typed { done, rest, marks, text, written } => {
                    let Some(translated) = translate(&text) else { continue };
                    let Some(Op::Text { x, y, text: shown, size, color, align }) =
                        self.ops.get(done).cloned()
                    else {
                        continue;
                    };
                    // As large a share of the translation as of the English: both end together
                    let chars: Vec<char> = translated.chars().collect();
                    let total = text.chars().count().max(1);
                    let typed = ((written * chars.len() + total / 2) / total).min(chars.len());
                    let (now, later): (String, String) =
                        (chars[..typed].iter().collect(), chars[typed..].iter().collect());
                    let wide = |text: &str| text.chars().map(|c| advance(c, size)).sum::<f32>();
                    let shift = wide(now.trim_end()) - wide(shown.trim_end());
                    instead[done] = Some(vec![Op::Text { x, y, text: now, size, color, align }]);
                    for mark in marks {
                        if let Some(Op::Fill { rect, color }) = self.ops.get(mark).cloned() {
                            let rect = Rect::new(rect.x + shift, rect.y, rect.w, rect.h);
                            instead[mark] = Some(vec![Op::Fill { rect, color }]);
                        }
                    }
                    if let Some(rest) = rest
                        && let Some(Op::Text { x, y, size, color, align, .. }) =
                            self.ops.get(rest).cloned()
                    {
                        let later = later.trim_start().to_owned();
                        instead[rest] = Some(if later.is_empty() {
                            Vec::new()
                        } else {
                            vec![Op::Text { x: x + shift, y, text: later, size, color, align }]
                        });
                    }
                }
            }
        }
        let ops = std::mem::take(&mut self.ops);
        for (mut op, instead) in ops.into_iter().zip(instead) {
            if let Some(new) = instead {
                self.ops.extend(new);
                continue;
            }
            if let Op::Text { text, .. } = &mut op
                && let Some(translated) = translate(text)
            {
                *text = translated;
            }
            self.ops.push(op);
        }
    }

    /// The steps that are text runs, in paint order (for tests and for reading a screen back).
    pub fn texts(&self) -> impl Iterator<Item = (f32, f32, &str)> {
        self.ops.iter().filter_map(|op| match op {
            Op::Text { x, y, text, .. } => Some((*x, *y, text.as_str())),
            _ => None,
        })
    }
}

/// The advance of `c` in pixels at `size`: Share Tech Mono's for ASCII, a full em for the rest (the CJK face's
/// ideographs and full-width marks, which are as wide as they are high).
fn advance(c: char, size: f32) -> f32 {
    if c.is_ascii() { ADVANCE * size } else { size }
}

/// The marks a line does not start with (they hang at the end of the line before, past the width).
const NO_START: &str = "。，、；：！？」』）》〉…．,.;:!?)";
/// The marks a line does not end with (they go down to the next line with what follows them).
const NO_END: &str = "「『（《〈(";

/// `text` (a translation, Chinese with some ASCII) in lines of at most `width` pixels at `size`: a line breaks
/// between any two non-ASCII characters and at the spaces, never inside an ASCII word, never before a closing mark
/// (which may hang past the width) or after an opening one. A piece wider than a line has a line of its own.
#[must_use]
pub fn wrap_wide(text: &str, width: f32, size: f32) -> Vec<String> {
    // The pieces: an ASCII word, a non-ASCII character, or a space (a place to break, dropped at a break).
    let mut pieces: Vec<String> = Vec::new();
    for c in text.chars() {
        let word = c.is_ascii() && !c.is_whitespace();
        match pieces.last_mut() {
            Some(last) if word && last.chars().all(|l| l.is_ascii() && !l.is_whitespace()) => {
                last.push(c);
            }
            _ => pieces.push(c.to_string()),
        }
    }
    let wide = |piece: &str| piece.chars().map(|c| advance(c, size)).sum::<f32>();
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut space = false;
    for piece in pieces {
        if piece.chars().all(char::is_whitespace) {
            space = !line.is_empty();
            continue;
        }
        let gap = if space { advance(' ', size) } else { 0.0 };
        let hangs = piece.chars().count() == 1 && NO_START.contains(piece.as_str());
        if line.is_empty() || hangs || wide(&line) + gap + wide(&piece) <= width {
            if space {
                line.push(' ');
            }
            line.push_str(&piece);
        } else {
            // An opening mark at the end of the line goes down with the piece after it.
            let mut next = String::new();
            if let Some(last) = line.chars().last()
                && NO_END.contains(last)
                && line.chars().count() > 1
            {
                line.pop();
                next.push(last);
            }
            lines.push(std::mem::take(&mut line));
            next.push_str(&piece);
            line = next;
        }
        space = false;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::palette;

    fn texts(list: &DrawList) -> Vec<&str> {
        list.texts().map(|(_, _, text)| text).collect()
    }

    #[test]
    fn a_translation_breaks_between_characters_at_its_width_and_never_inside_an_ascii_word() {
        // 16 px: a character is 16 px, an ASCII letter 8.64 px.
        assert_eq!(
            wrap_wide("無。遊戲照原版進行。", 7.0 * 16.0, 16.0),
            ["無。遊戲照原版", "進行。"]
        );
        assert_eq!(wrap_wide("遊戲發下的卡牌", 120.0, 16.0), ["遊戲發下的卡牌"]);
        assert_eq!(
            wrap_wide("按住 Select 與 Start", 5.0 * 16.0, 16.0),
            ["按住", "Select 與", "Start"]
        );
        assert_eq!(wrap_wide("", 100.0, 16.0), Vec::<String>::new());
    }

    #[test]
    fn a_closing_mark_hangs_at_the_end_of_the_line_and_an_opening_one_goes_down() {
        assert_eq!(
            wrap_wide("一二三。四", 3.0 * 16.0, 16.0),
            ["一二三。", "四"],
            "the full stop hangs"
        );
        assert_eq!(
            wrap_wide("一二「三」", 3.0 * 16.0, 16.0),
            ["一二", "「三」"],
            "the bracket goes with 三"
        );
    }

    #[test]
    fn marked_lines_give_way_to_the_whole_translation_wrapped_again() {
        let mut list = DrawList::new();
        list.push(Op::Window { rect: Rect::new(0.0, 0.0, 200.0, 100.0) });
        list.text(10.0, 20.0, "None. The game", 16.0, palette::BLACK, Align::Left);
        list.text(10.0, 42.0, "plays as the", 16.0, palette::BLACK, Align::Left);
        list.text(10.0, 64.0, "original.", 16.0, palette::BLACK, Align::Left);
        list.mark_lines("None. The game plays as the original.", 3, 14, 22.0, usize::MAX);
        list.text(10.0, 90.0, "Back", 16.0, palette::BLACK, Align::Left);
        list.map_text(|text| match text {
            "None. The game plays as the original." => Some("無。遊戲照原版進行。".into()),
            "Back" => Some("返回".into()),
            _ => None,
        });
        assert_eq!(
            list.texts().collect::<Vec<_>>(),
            [(10.0, 20.0, "無。遊戲照原版"), (10.0, 42.0, "進行。"), (10.0, 90.0, "返回")]
        );
        assert!(matches!(list.ops()[0], Op::Window { .. }), "the other steps stay in their place");
    }

    #[test]
    fn a_translation_with_more_lines_than_the_english_keeps_to_the_most_it_may_have() {
        let mut list = DrawList::new();
        list.text(0.0, 10.0, "Short", 16.0, palette::BLACK, Align::Left);
        list.mark_lines("Short", 1, 5, 26.0, 2);
        list.map_text(|_| Some("一二三四五六七八九十".into()));
        let lines = list.texts().collect::<Vec<_>>();
        // 5 characters of English are 43.2 px: two CJK characters a line, two lines at most.
        assert_eq!(lines, [(0.0, 10.0, "一二"), (0.0, 36.0, "三四")]);
    }

    #[test]
    fn appended_lists_keep_their_marks_at_their_new_place() {
        let mut part = DrawList::new();
        part.text(0.0, 0.0, "A  Open   B  Back", 16.0, palette::BLACK, Align::Right);
        part.mark_parts(&["A  Open", "B  Back"], "   ");
        let mut list = DrawList::new();
        list.text(0.0, 0.0, "Talk", 16.0, palette::BLACK, Align::Left);
        list.append(part);
        list.map_text(|text| (text == "B  Back").then(|| "B  返回".to_owned()));
        assert_eq!(texts(&list), ["Talk", "A  Open   B  返回"]);
    }

    #[test]
    fn nothing_is_marked_without_the_runs_and_the_marks_are_used_up() {
        let mut list = DrawList::new();
        list.mark_lines("", 0, 10, 20.0, 3);
        list.mark_parts(&["A"], " ");
        assert_eq!(list, DrawList::new());
        list.text(0.0, 0.0, "one", 16.0, palette::BLACK, Align::Left);
        list.mark_lines("one two", 2, 10, 20.0, 3);
        assert_eq!(list.wholes.len(), 0, "two runs asked for, one there");
        list.mark_lines("one", 1, 10, 20.0, 3);
        list.map_text(|_| None);
        assert_eq!(list.wholes, Vec::new());
    }
}
