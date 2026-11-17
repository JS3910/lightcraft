//! The face tile shared by the People view's unnamed faces and a person's page (its confirmed faces and its "More"
//! faces): a cropped face picture with a name strip below it that is always drawn (a name, or the suggested name and a
//! question mark, or a lone question mark), the selection of faces, the rule for what a naming action applies to, the
//! name box opened on a tile, and optional hover buttons (confirm, all above, hide) a gallery switches on.
//!
//! Selection: a click on the picture selects (Shift-click a range), a double click opens the photo, a click on the strip
//! opens the name box. The selection is `app.ui.unnamed_selected` with the range anchor `app.ui.unnamed_anchor`, and it
//! belongs to one [`Gallery`] at a time (`app.ui.face_gallery`): selecting in another gallery replaces it, and a gallery
//! only ever sees a selection that is its own ([`selection`]).
//!
//! What naming applies to ([`apply_to`]): a box on a selected tile acts on every selected face, a box on any other tile on
//! that face alone (and leaves the selection as it is). Either way it is one command, so one undo step
//! ([`name_each`], `faces.nameFaces` with a name per face).
//!
//! Hover buttons ([`Hover`], `FaceTile::hover`): a gallery that gives its tiles a `Hover` gets, on hover, a check
//! (confirm), an up arrow (all above) and a cross (hide), each only when switched on. The tile reports a press as
//! [`TileEvent::Confirm`], [`TileEvent::ConfirmAbove`] or [`TileEvent::Hide`]; what they do is the gallery's business.
//!
//! Widget ids: `{prefix}-face:{photo}:{index}` the picture, `{prefix}-name:{photo}:{index}` the strip,
//! `{prefix}-face-confirm|above|hide:{photo}:{index}` the buttons; the open box is `field:tileName` with its list
//! `tileName:pick:{name}`.

use std::collections::HashSet;

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::name_box::{self, NameBox, NameBoxState, Outcome, Step};
use crate::state::NameEdit;
use crate::theme::Tokens;
use crate::widgets::register;

/// The name strip under a tile's picture (points).
pub const STRIP_H: f32 = 22.0;

/// A face: its photo and the index of its region.
pub type FaceKey = (u64, usize);

/// A list of face tiles the selection can live in. The same face can be in two (an unnamed face is in the Unnamed faces
/// and in somebody's More), so a selection says which one it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gallery {
    /// The People view's unnamed faces.
    Unnamed,
    /// The faces already named as this person, on their page.
    Confirmed(String),
    /// The faces that look like this person, on their page.
    More(String),
}

/// The hover buttons a gallery switches on for its tiles, and what their tooltips say.
#[derive(Clone, Debug)]
pub struct Hover<'a> {
    pub confirm: bool,
    pub above: bool,
    pub hide: bool,
    /// Who the face would be confirmed as.
    pub person: &'a str,
    /// How alike the person the face looks.
    pub score: f32,
    /// How many shown faces "all above" would name.
    pub above_count: usize,
}

/// What the tile draws.
pub struct FaceTile<'a> {
    pub key: FaceKey,
    pub view: lightcraft_geom::Rect,
    /// The name on the strip: the face's own when `named`, else the name the named faces suggest, if any.
    pub suggestion: Option<&'a str>,
    /// The strip shows the face's own name (no question mark).
    pub named: bool,
    /// Widget id prefix (`unnamed`, `more`, `person`).
    pub prefix: &'a str,
    /// The hover buttons, when this gallery has them.
    pub hover: Option<Hover<'a>>,
}

/// What happened to the tile this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileEvent {
    None,
    /// A click on the picture.
    Click {
        shift: bool,
    },
    /// A double click on the picture.
    Open,
    /// A click on the strip: open the name box.
    Name,
    /// The check button: confirm this face.
    Confirm,
    /// The up-arrow button: confirm this face and every one at least as alike.
    ConfirmAbove,
    /// The cross button: hide this face.
    Hide,
}

/// One entry of the list a tile belongs to, in the order shown: the face and the name on its strip (its own name in a
/// gallery of named faces, else the name suggested for it).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: FaceKey,
    pub suggestion: Option<String>,
}

/// The faces an action on `face` applies to: all selected faces (in list order) when `face` is selected, else `face` alone.
pub fn apply_to(selected: &HashSet<FaceKey>, order: &[Entry], face: FaceKey) -> Vec<FaceKey> {
    if selected.contains(&face) {
        let list: Vec<FaceKey> = order.iter().map(|e| e.key).filter(|k| selected.contains(k)).collect();
        if !list.is_empty() {
            return list;
        }
    }
    vec![face]
}

/// The strip's text: the face's own name, or the suggestion and a question mark, or a lone question mark.
pub fn strip_label(text: Option<&str>, named: bool) -> String {
    match (text, named) {
        (Some(s), true) => s.to_string(),
        (Some(s), false) => format!("{s}?"),
        (None, _) => "?".to_string(),
    }
}

/// The selected faces of `gallery`: the selection when it is in `gallery`, else none.
pub fn selection(app: &LightcraftApp, gallery: &Gallery) -> HashSet<FaceKey> {
    if app.ui.face_gallery.as_ref() == Some(gallery) { app.ui.unnamed_selected.clone() } else { HashSet::new() }
}

/// Is `key` selected in `gallery`?
pub fn is_selected(app: &LightcraftApp, gallery: &Gallery, key: FaceKey) -> bool {
    app.ui.face_gallery.as_ref() == Some(gallery) && app.ui.unnamed_selected.contains(&key)
}

/// Make `gallery` the one the selection is in; a selection that was in another gallery is dropped.
fn claim(app: &mut LightcraftApp, gallery: &Gallery) {
    if app.ui.face_gallery.as_ref() != Some(gallery) {
        app.ui.unnamed_selected.clear();
        app.ui.unnamed_anchor = None;
        app.ui.face_gallery = Some(gallery.clone());
    }
}

/// Is something selected in a gallery of the person page that is open?
pub fn selection_on_page(app: &LightcraftApp) -> bool {
    let open = app.ui.person_page.as_deref();
    let here = matches!(&app.ui.face_gallery, Some(Gallery::Confirmed(n) | Gallery::More(n)) if Some(n.as_str()) == open);
    here && !app.ui.unnamed_selected.is_empty()
}

/// Drop the selection (in whichever gallery it is).
pub fn clear_selection(app: &mut LightcraftApp) {
    app.ui.unnamed_selected.clear();
    app.ui.unnamed_anchor = None;
    app.ui.face_gallery = None;
}

/// Select exactly `keys` in `gallery`.
pub fn select_all(app: &mut LightcraftApp, gallery: &Gallery, keys: impl IntoIterator<Item = FaceKey>) {
    claim(app, gallery);
    app.ui.unnamed_selected = keys.into_iter().collect();
}

/// Drop what is selected in `gallery` and is no longer in `present` (named, removed, hidden).
pub fn retain_present(app: &mut LightcraftApp, gallery: &Gallery, present: &HashSet<FaceKey>) {
    if app.ui.face_gallery.as_ref() == Some(gallery) {
        app.ui.unnamed_selected.retain(|k| present.contains(k));
    }
}

/// A click on face number `i` of `order`, a list of `gallery`: select it (or deselect it), or with Shift every face from
/// the last one clicked.
pub fn click(app: &mut LightcraftApp, gallery: &Gallery, order: &[Entry], i: usize, shift: bool) {
    let Some(f) = order.get(i) else { return };
    claim(app, gallery);
    match app.ui.unnamed_anchor.filter(|_| shift) {
        Some(a) => {
            for e in order.get(a.min(i)..=a.max(i)).unwrap_or_default() {
                app.ui.unnamed_selected.insert(e.key);
            }
        }
        None => {
            if !app.ui.unnamed_selected.remove(&f.key) {
                app.ui.unnamed_selected.insert(f.key);
            }
            app.ui.unnamed_anchor = Some(i);
        }
    }
}

/// Name faces, each with its own name, as one undo step. An empty name leaves the face alone, or with `clearing` removes
/// its name.
pub fn name_each(app: &mut LightcraftApp, names: &[(FaceKey, String)], clearing: bool) {
    let list: Vec<Value> = names
        .iter()
        .filter(|(_, n)| clearing || !n.trim().is_empty())
        .map(|((photo, index), n)| json!({"photo": photo, "index": index, "name": n}))
        .collect();
    if !list.is_empty() {
        let _ = app.run("faces.nameFaces", json!({"faces": list}));
    }
}

/// The tile's egui id for one of its parts (`face`, `name`, a button): keyboard focus and the page's A key look it up.
pub fn widget_id(prefix: &str, part: &str, key: FaceKey) -> egui::Id {
    egui::Id::unique((prefix, part, key.0, key.1))
}

/// Draw the tile in `r` (the picture on top, the strip below) and report what was done to it.
pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect, tile: &FaceTile, selected: bool, ppp: f32) -> TileEvent {
    let t = Tokens::get(ui.ctx());
    let (photo, index) = tile.key;
    let pic = Rect::from_min_size(r.min, vec2(r.width(), (r.height() - STRIP_H).max(1.0)));
    let strip = Rect::from_min_max(pos2(r.left(), pic.bottom()), r.right_bottom());
    let resp = ui.interact(pic, widget_id(tile.prefix, "face", tile.key), Sense::click());
    crate::widgets::announce_selected(&resp, crate::i18n::tr("Face"), selected);
    register(ui, format!("{}-face:{photo}:{index}", tile.prefix), pic);
    let name = ui.interact(strip, widget_id(tile.prefix, "name", tile.key), Sense::click());
    register(ui, format!("{}-name:{photo}:{index}", tile.prefix), strip);
    // the hover buttons are added after the picture, so they win where they overlap it
    let buttons = tile.hover.as_ref().map(|h| buttons_for(ui, tile, h, pic));
    let hovered = resp.hovered() || buttons.as_ref().is_some_and(Buttons::hovered);
    let editing = app.ui.name_edit.as_ref().is_some_and(|e| (e.photo, e.index) == tile.key);
    paint_picture(app, ui, tile, pic, ppp);
    paint_selection(ui, pic, selected, hovered);
    if let (Some(b), Some(h)) = (&buttons, &tile.hover) {
        if hovered {
            paint_buttons(ui, b);
        }
        explain_buttons(&resp, b, h);
    }
    // the strip: always there, never dependent on hover
    let fill = if editing || name.hovered() { t.button_border } else { t.canvas };
    let p = ui.painter();
    p.rect_filled(strip, 0.0, fill);
    let text = strip_label(tile.suggestion, tile.named);
    let label = super::people::fit(p, &text, t.font(11.5), strip.width() - 10.0);
    let color = if tile.suggestion.is_some() { t.text } else { t.text_dim };
    p.text(pos2(strip.left() + 5.0, strip.center().y), Align2::LEFT_CENTER, label, t.font(11.5), color);
    let name = name.on_hover_text(crate::i18n::tr("Click to name this face"));
    if let Some(event) = buttons.as_ref().and_then(Buttons::pressed) {
        return event;
    }
    if name.clicked() {
        return TileEvent::Name;
    }
    if resp.double_clicked() {
        return TileEvent::Open;
    }
    if resp.clicked() {
        return TileEvent::Click { shift: ui.input(|i| i.modifiers.shift) };
    }
    TileEvent::None
}

/// One hover button: where it sits and how it was interacted with.
type Button = Option<(Rect, egui::Response)>;

/// The hover buttons of one tile.
struct Buttons {
    confirm: Button,
    above: Button,
    hide: Button,
}

impl Buttons {
    fn hovered(&self) -> bool {
        [&self.confirm, &self.above, &self.hide].into_iter().flatten().any(|(_, r)| r.hovered())
    }

    /// The button pressed this frame, if any.
    fn pressed(&self) -> Option<TileEvent> {
        let clicked = |b: &Button| b.as_ref().is_some_and(|(_, r)| r.clicked());
        if clicked(&self.hide) {
            Some(TileEvent::Hide)
        } else if clicked(&self.above) {
            Some(TileEvent::ConfirmAbove)
        } else if clicked(&self.confirm) {
            Some(TileEvent::Confirm)
        } else {
            None
        }
    }
}

/// Interact with and register the buttons `hover` switches on: hide (top right), all above (next to it), confirm (bottom
/// right). The top left stays free for the selection's check.
fn buttons_for(ui: &mut egui::Ui, tile: &FaceTile, hover: &Hover, pic: Rect) -> Buttons {
    let (photo, index) = tile.key;
    let one = |on: bool, part: &str, rect: Rect| -> Button {
        on.then(|| {
            let response = ui.interact(rect, widget_id(tile.prefix, part, tile.key), Sense::click());
            register(ui, format!("{}-face-{part}:{photo}:{index}", tile.prefix), rect);
            (rect, response)
        })
    };
    let hide = Rect::from_min_size(pos2(pic.right() - 24.0, pic.top() + 4.0), vec2(20.0, 20.0));
    let above = Rect::from_min_size(pos2(pic.right() - 52.0, pic.top() + 4.0), vec2(24.0, 20.0));
    let confirm = Rect::from_min_size(pos2(pic.right() - 28.0, pic.bottom() - 28.0), vec2(24.0, 24.0));
    Buttons { hide: one(hover.hide, "hide", hide), above: one(hover.above, "above", above), confirm: one(hover.confirm, "confirm", confirm) }
}

fn explain_buttons(picture: &egui::Response, b: &Buttons, hover: &Hover) {
    let person = hover.person;
    if hover.confirm {
        let _ = picture.clone().on_hover_text(format!("Looks like {person} (similarity {:.2}). Use the check to confirm.", hover.score));
    }
    if let Some((_, r)) = &b.above {
        let _ = r.clone().on_hover_text(format!(
            "All above: confirm {} faces as {person} (this face and every shown face with equal or higher similarity). Press A.",
            hover.above_count
        ));
    }
    if let Some((_, r)) = &b.hide {
        let _ = r.clone().on_hover_text(crate::i18n::tr("Hide this face for now"));
    }
    if let Some((_, r)) = &b.confirm {
        let _ = r.clone().on_hover_text(format!("Confirm as {person}"));
    }
}

fn paint_picture(app: &mut LightcraftApp, ui: &egui::Ui, tile: &FaceTile, pic: Rect, ppp: f32) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    p.rect_filled(pic, 3.0, t.canvas);
    if let Some(job) = app.session.face_job(lightcraft_catalog::PhotoId(tile.key.0), tile.view, (pic.width() * ppp).ceil() as usize)
        && let Some(tex) = app.renderer.variant(job)
    {
        p.image(tex.id(), pic, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

fn paint_selection(ui: &egui::Ui, pic: Rect, selected: bool, hovered: bool) {
    let p = ui.painter();
    if selected {
        p.rect_filled(pic, 3.0, Color32::from_rgba_unmultiplied(70, 130, 255, 55));
        p.rect_stroke(pic, 3.0, Stroke::new(2.0, Color32::from_rgb(110, 160, 255)), StrokeKind::Inside);
        let c = pos2(pic.left() + 13.0, pic.top() + 13.0);
        p.circle_filled(c, 9.0, Color32::from_rgb(70, 130, 255));
        p.line_segment([pos2(c.x - 4.0, c.y), pos2(c.x - 1.0, c.y + 3.5)], Stroke::new(1.8, Color32::WHITE));
        p.line_segment([pos2(c.x - 1.0, c.y + 3.5), pos2(c.x + 4.5, c.y - 3.5)], Stroke::new(1.8, Color32::WHITE));
    } else if hovered {
        p.rect_stroke(pic, 3.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    }
}

fn paint_buttons(ui: &egui::Ui, b: &Buttons) {
    let p = ui.painter();
    let dark = Color32::from_rgba_unmultiplied(30, 30, 30, 220);
    let white = |w: f32| Stroke::new(w, Color32::WHITE);
    if let Some((rect, _)) = &b.confirm {
        let c = rect.center();
        p.circle_filled(c, 12.0, dark);
        let check = [pos2(c.x - 5.0, c.y), pos2(c.x - 1.5, c.y + 4.0), pos2(c.x + 5.5, c.y - 4.0)];
        p.line_segment([check[0], check[1]], white(2.0));
        p.line_segment([check[1], check[2]], white(2.0));
    }
    if let Some((rect, _)) = &b.hide {
        let xc: Pos2 = rect.center();
        p.circle_filled(xc, 10.0, dark);
        p.line_segment([pos2(xc.x - 3.5, xc.y - 3.5), pos2(xc.x + 3.5, xc.y + 3.5)], white(1.6));
        p.line_segment([pos2(xc.x - 3.5, xc.y + 3.5), pos2(xc.x + 3.5, xc.y - 3.5)], white(1.6));
    }
    if let Some((rect, _)) = &b.above {
        p.rect_filled(*rect, 4.0, dark);
        let ac = rect.center();
        p.line_segment([pos2(ac.x, ac.y + 5.0), pos2(ac.x, ac.y - 5.0)], white(2.0));
        p.line_segment([pos2(ac.x - 4.0, ac.y - 1.0), pos2(ac.x, ac.y - 5.0)], white(2.0));
        p.line_segment([pos2(ac.x, ac.y - 5.0), pos2(ac.x + 4.0, ac.y - 1.0)], white(2.0));
    }
}

/// Open the name box on `entry`: its name or suggestion selected, or empty.
pub fn open(app: &mut LightcraftApp, ctx: &egui::Context, entry: &Entry) {
    let input = NameBoxState::open(entry.suggestion.clone().unwrap_or_default(), ctx.cumulative_pass_nr());
    // one name box at a time: a box open on a Named People card closes
    app.ui.card_name = None;
    app.ui.name_edit = Some(NameEdit { photo: entry.key.0, index: entry.key.1, current: None, input });
}

/// Where the box goes: just under the tile's picture, at the strip.
pub fn box_pos(tile: Rect) -> Pos2 {
    pos2(tile.left(), tile.bottom() - STRIP_H + 2.0)
}

/// The list a tile belongs to, as the name box needs it.
pub struct Listing<'a> {
    pub gallery: &'a Gallery,
    /// The faces in the order shown.
    pub order: &'a [Entry],
    /// The tile's position in `order`.
    pub at: usize,
}

/// Draws the name box if it is open on the tile, and applies how it ended. Returns the face a Tab moved to, so the caller
/// can scroll to it.
pub fn name_box_for(app: &mut LightcraftApp, ctx: &egui::Context, tile: Rect, list: &Listing) -> Option<FaceKey> {
    let entry = list.order.get(list.at)?;
    let mut input = {
        let e = app.ui.name_edit.as_ref().filter(|e| (e.photo, e.index) == entry.key)?;
        e.input.clone()
    };
    let hints = super::faces::hints_for(app, entry.key.0);
    let candidates = hints.as_deref().and_then(|h| h.by_index.get(&entry.key.1)).map(|h| h.candidates.clone()).unwrap_or_default();
    let people = app.caches.person_names(&app.session.catalog);
    let setup = super::faces::setup(app, ctx);
    // a named face keeps its spelling when the same name is typed again
    let current = matches!(list.gallery, Gallery::Confirmed(_)).then_some(entry.suggestion.as_deref()).flatten();
    let spec = NameBox { id: "tileName", candidates: &candidates, people: &people, current, setup };
    let outcome = name_box::show(ctx, box_pos(tile), &mut input, &spec);
    let touched = input.touched();
    if let Some(e) = app.ui.name_edit.as_mut().filter(|e| (e.photo, e.index) == entry.key) {
        e.input = input;
    }
    finish(app, ctx, list, outcome, touched)
}

/// What an ended name box means: close it, name the faces it applies to, on Tab open the neighbour's box.
fn finish(app: &mut LightcraftApp, ctx: &egui::Context, list: &Listing, outcome: Outcome, touched: bool) -> Option<FaceKey> {
    let entry = list.order.get(list.at)?;
    let targets = apply_to(&selection(app, list.gallery), list.order, entry.key);
    match outcome {
        Outcome::Open => None,
        Outcome::Cancel | Outcome::Untouched => {
            app.ui.name_edit = None;
            None
        }
        Outcome::Setup => {
            app.ui.name_edit = None;
            let setup = super::faces::setup(app, ctx);
            super::faces::take_step(app, ctx, setup);
            None
        }
        Outcome::Save(name) => {
            app.ui.name_edit = None;
            name_targets(app, list, &targets, touched.then_some(name.as_str()));
            None
        }
        Outcome::Move { step, save } => {
            let skip: Vec<FaceKey> = if save.is_some() { targets.clone() } else { Vec::new() };
            if let Some(name) = save.as_deref() {
                name_targets(app, list, &targets, Some(name));
            }
            let to = neighbour(list.order, list.at, step, &skip);
            app.ui.name_edit = None;
            let e = to.and_then(|i| list.order.get(i))?;
            open(app, ctx, e);
            Some(e.key)
        }
    }
}

/// Name `targets`: all with `name` when given, else each with its own name or suggestion (faces without one are left
/// alone). On a gallery of named faces a face that already has the name is left alone and an empty name removes it.
fn name_targets(app: &mut LightcraftApp, list: &Listing, targets: &[FaceKey], name: Option<&str>) {
    let confirmed = matches!(list.gallery, Gallery::Confirmed(_));
    let names: Vec<(FaceKey, String)> = targets
        .iter()
        .filter_map(|k| {
            let own = list.order.iter().find(|e| e.key == *k).and_then(|e| e.suggestion.clone()).unwrap_or_default();
            let new = name.map_or_else(|| own.clone(), str::to_string);
            (!(confirmed && new == own)).then_some((*k, new))
        })
        .collect();
    name_each(app, &names, confirmed);
}

/// The tile Tab (or Shift-Tab) leads to from position `at`, going round, skipping `skip` (faces about to be named).
pub fn neighbour(order: &[Entry], at: usize, step: Step, skip: &[FaceKey]) -> Option<usize> {
    let n = order.len();
    (1..n)
        .map(|d| if step == Step::Next { (at + d) % n } else { (at + n - d % n) % n })
        .find(|i| order.get(*i).is_some_and(|e| !skip.contains(&e.key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(n: usize) -> Vec<Entry> {
        (0..n).map(|i| Entry { key: (1, i), suggestion: None }).collect()
    }

    #[test]
    fn a_selected_face_acts_on_every_selected_face_in_list_order_and_any_other_on_itself() {
        let o = order(5);
        let sel: HashSet<FaceKey> = [(1, 3), (1, 1)].into_iter().collect();
        assert_eq!(apply_to(&sel, &o, (1, 3)), vec![(1, 1), (1, 3)]);
        assert_eq!(apply_to(&sel, &o, (1, 2)), vec![(1, 2)], "outside the selection: that face alone");
        assert_eq!(apply_to(&HashSet::new(), &o, (1, 4)), vec![(1, 4)]);
        // a selected face that is not in the list still acts on itself
        let gone: HashSet<FaceKey> = [(9, 9)].into_iter().collect();
        assert_eq!(apply_to(&gone, &o, (9, 9)), vec![(9, 9)]);
    }

    #[test]
    fn the_strip_says_a_name_or_a_suggestion_with_a_question_mark_or_just_the_mark() {
        assert_eq!(strip_label(Some("Ann"), false), "Ann?");
        assert_eq!(strip_label(Some("Ann"), true), "Ann");
        assert_eq!(strip_label(None, false), "?");
    }

    #[test]
    fn tab_goes_round_and_skips_faces_being_named() {
        let o = order(4);
        assert_eq!(neighbour(&o, 3, Step::Next, &[]), Some(0));
        assert_eq!(neighbour(&o, 0, Step::Previous, &[]), Some(3));
        assert_eq!(neighbour(&o, 1, Step::Next, &[(1, 2)]), Some(3));
        assert_eq!(neighbour(&o, 0, Step::Next, &[(1, 1), (1, 2), (1, 3)]), None);
        assert_eq!(neighbour(&order(1), 0, Step::Next, &[]), None);
        assert_eq!(neighbour(&[], 0, Step::Next, &[]), None);
    }
}
