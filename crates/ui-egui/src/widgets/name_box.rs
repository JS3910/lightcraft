//! The name box: one component for naming a face (the loupe today; the People view's tiles and cards next).
//!
//! It draws the shared text field with the text selected on opening, completes what is typed from the people already
//! named (the completed part selected, so the next key replaces it and Backspace removes it), lists up to seven
//! matching names below, and reports how the edit ended ([`Outcome`]). What an ending means (naming a face, moving on
//! to the next one, closing) is the caller's; the rules for which names are offered, what the box completes to and
//! which spelling is stored are the pure functions in [`rules`].
//!
//! Keys: Enter saves (the listed name picked with Up / Down, else the text), Esc cancels and is the only cancel, Tab and
//! Shift-Tab save typed text and move on without confirming an untouched suggestion. A press outside saves changed text
//! and closes an untouched box without effect.
//!
//! Widget ids (`spec.id` = `faceName` in the loupe): `field:{id}` the text field, `{id}:pick:{name}` a listed name,
//! `faces:setup` the offer to set recognition up.

use egui::{Key, Modifiers, Pos2, RichText};

use crate::panels::faces::Setup;
use crate::text_field::TextField;
use crate::theme::Tokens;
use crate::widgets::register;

pub mod rules;
pub use rules::Offer;

/// What the box is for this frame: its widget id, the closest people to the face, everyone named, the name the face has.
pub struct NameBox<'a> {
    pub id: &'a str,
    /// The closest few people the engine returned for the face, best first, with their similarity.
    pub candidates: &'a [(String, f32)],
    /// Every named person (the names completion and the list draw on).
    pub people: &'a [String],
    /// The name the face has now (the spelling rule keeps a capitalization change on a face that has the name already).
    pub current: Option<&'a str>,
    /// While recognition is not set up the box offers the next step.
    pub setup: Setup,
}

/// Where Tab (next) and Shift-Tab (previous) go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Next,
    Previous,
}

/// How the box ended this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Open,
    /// Enter, a click on a listed name, or a press outside after changing the text. The name is already in the stored
    /// spelling; empty means "remove the name".
    Save(String),
    /// Tab or Shift-Tab: `save` has the typed text (stored spelling), or nothing when the text was left untouched.
    Move {
        step: Step,
        save: Option<String>,
    },
    /// Esc: nothing changes.
    Cancel,
    /// A press outside an untouched box: close, name nothing.
    Untouched,
    /// The offer to set face recognition up was pressed.
    Setup,
}

/// What the box remembers between frames. Make one with [`NameBoxState::open`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameBoxState {
    /// What the field shows.
    pub text: String,
    /// The part the user typed: `text` less the completed remainder.
    typed: String,
    /// The text has been changed by the user.
    edited: bool,
    /// The listed name chosen with the arrow keys.
    picked: Option<usize>,
    /// Frames drawn, to tell the opening one.
    age: u32,
    /// The egui pass the box was last drawn in: a box not drawn for a while has lost its face (see [`Self::is_stale`]).
    drawn: u64,
    /// The egui pass the box was opened in: the box is not yet drawn for real in it (the press that opened it may still
    /// take the focus away from the field).
    opened: u64,
}

impl NameBoxState {
    /// A box opening with `text` (the name or the suggestion, possibly empty) selected, in egui pass `pass`
    /// (`ctx.cumulative_pass_nr()`).
    pub fn open(text: impl Into<String>, pass: u64) -> Self {
        let text = text.into();
        NameBoxState { typed: text.clone(), text, edited: false, picked: None, age: 0, drawn: pass, opened: pass }
    }

    /// The user changed the text or chose a listed name.
    pub fn touched(&self) -> bool {
        self.edited || self.picked.is_some()
    }

    /// The box has not been drawn for a few passes: whatever it was for is no longer shown (another photo, another
    /// view), so it should close without effect.
    pub fn is_stale(&self, pass: u64) -> bool {
        self.drawn.saturating_add(1) < pass
    }

    /// The text that drives the list: nothing before the first edit, so the list shows the closest people.
    fn query(&self) -> &str {
        if self.edited { &self.typed } else { "" }
    }
}

/// Draws the box at `at` (a floating area) and returns how it ended.
pub fn show(ctx: &egui::Context, at: Pos2, state: &mut NameBoxState, spec: &NameBox) -> Outcome {
    let t = Tokens::get(ctx);
    let opening = state.age == 0;
    let early = ctx.cumulative_pass_nr() == state.opened;
    state.drawn = ctx.cumulative_pass_nr();
    // keys are read before the field sees them; the frame that opens the box ignores them (they were meant for what
    // came before: Tab that moved here, the Enter that named the last face)
    let (down, up) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::ArrowDown), i.consume_key(Modifiers::NONE, Key::ArrowUp)));
    let (escape, enter, tab) =
        ctx.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Enter), i.key_pressed(Key::Tab).then_some(i.modifiers.shift)));
    let listed = rules::offered(state.query(), spec.candidates, spec.people);
    if down && !listed.is_empty() {
        state.picked = Some(state.picked.map_or(0, |i| (i + 1).min(listed.len() - 1)));
    }
    if up {
        state.picked = state.picked.and_then(|i| i.checked_sub(1));
    }

    let mut clicked: Option<Outcome> = None;
    // a box that is only being measured (its first pass) is not yet the box the user sees: it keeps the opening frame
    let mut real = false;
    let field = format!("field:{}", spec.id);
    let shown =
        egui::Area::new(egui::Id::unique(("name-box", spec.id))).order(egui::Order::Foreground).fixed_pos(at).constrain(true).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(230.0);
                let before = Before::of(ui.ctx(), &field, &state.text);
                let r = TextField::singleline(&field, &mut state.text)
                    .hint(crate::i18n::tr("Name"))
                    .width(220.0)
                    .begin_edit(opening && !ui.is_sizing_pass() && !early)
                    .show(ui);
                real = !ui.is_sizing_pass() && !early;
                track(ui.ctx(), state, spec, &field, before, r.response.changed());
                let listed = rules::offered(state.query(), spec.candidates, spec.people);
                for (i, offer) in listed.iter().enumerate() {
                    let label = match offer.score {
                        Some(s) => format!("{}   {:.0}%", offer.name, (s * 100.0).max(0.0)),
                        None => offer.name.clone(),
                    };
                    let button = egui::Button::new(RichText::new(label).color(t.text_label))
                        .frame(false)
                        .selected(state.picked == Some(i))
                        .min_size(egui::vec2(220.0, 0.0));
                    let b = ui.add(button);
                    register(ui, format!("{}:pick:{}", spec.id, offer.name), b.rect);
                    if b.clicked() {
                        clicked = Some(Outcome::Save(rules::stored_spelling(&offer.name, spec.people, spec.current)));
                    }
                }
                if let Some(label) = spec.setup.short() {
                    let b = ui.add(egui::Button::new(RichText::new(crate::i18n::tr(label)).font(t.font(12.0)).color(t.accent)).frame(false));
                    register(ui, "faces:setup", b.rect);
                    if b.clicked() {
                        clicked = Some(Outcome::Setup);
                    }
                }
                ui.label(RichText::new(crate::i18n::tr("Enter to confirm, Esc to cancel")).font(t.font(11.0)).color(t.text_dim));
            });
        });
    if real {
        state.age = state.age.saturating_add(1);
    }
    if let Some(outcome) = clicked {
        return outcome;
    }
    if opening {
        return Outcome::Open;
    }
    if escape {
        return Outcome::Cancel;
    }
    if let Some(shift) = tab {
        let step = if shift { Step::Previous } else { Step::Next };
        return Outcome::Move { step, save: state.touched().then(|| chosen(state, spec)) };
    }
    if enter {
        return Outcome::Save(chosen(state, spec));
    }
    // a press anywhere else closes it (not the press that opened it, which is the frame before)
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !shown.response.rect.contains(p)));
    match (outside, state.touched()) {
        (true, true) => Outcome::Save(chosen(state, spec)),
        (true, false) => Outcome::Untouched,
        _ => Outcome::Open,
    }
}

/// What saving the box now stores: the listed name chosen with the arrow keys, else the text, in the stored spelling.
fn chosen(state: &NameBoxState, spec: &NameBox) -> String {
    let picked = state.picked.and_then(|i| rules::offered(state.query(), spec.candidates, spec.people).into_iter().nth(i));
    rules::stored_spelling(picked.as_ref().map_or(state.text.as_str(), |o| o.name.as_str()), spec.people, spec.current)
}

/// After the field was drawn: follow the typed part, and complete it when the text grew with the caret at its end.
fn track(ctx: &egui::Context, state: &mut NameBoxState, spec: &NameBox, field: &str, before: Before, changed: bool) {
    let id = crate::text_field::id(field);
    let range = egui::text_edit::TextEditState::load(ctx, id).and_then(|s| s.cursor.char_range());
    let len = state.text.chars().count();
    if changed {
        state.edited = true;
        state.picked = None;
        let grew = len > before.kept;
        let at_end = range.is_some_and(|r| r.is_empty() && r.primary.index == egui::text::CharIndex(len));
        state.typed = state.text.clone();
        if grew && at_end {
            let offers = rules::offered(&state.typed, spec.candidates, spec.people);
            if let Some(full) = rules::completion(&state.typed, &offers) {
                let (from, full) = (len, full.to_string());
                let to = full.chars().count();
                state.text = full;
                let mut s = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
                let at = egui::text::CCursor::new;
                s.cursor.set_char_range(Some(egui::text::CCursorRange::two(at(from), at(to))));
                s.store(ctx, id);
            }
        }
    } else if ctx.memory(|m| m.has_focus(id)) && range.is_some_and(|r| r.is_empty()) {
        // the caret moved or the completion was accepted: what is shown is what was typed
        state.typed = state.text.clone();
    }
}

/// What the field held before this frame's input: how many characters stay when the selection is replaced (the text
/// less what was selected), so replacing a selected name with a letter counts as typing, not as shrinking.
#[derive(Clone, Copy)]
struct Before {
    kept: usize,
}

impl Before {
    fn of(ctx: &egui::Context, field: &str, text: &str) -> Self {
        let selected = egui::text_edit::TextEditState::load(ctx, crate::text_field::id(field))
            .and_then(|s| s.cursor.char_range())
            .map_or(0, |r| r.primary.index.0.abs_diff(r.secondary.index.0));
        Before { kept: text.chars().count().saturating_sub(selected) }
    }
}
