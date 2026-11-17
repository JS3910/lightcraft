//! The face tile shared by the People view's unnamed faces (and, next, a person's page): a cropped face picture with a
//! name strip below it that is always drawn (the suggested name and a question mark, or a lone question mark), the
//! selection of faces, the rule for what a naming action applies to, and the name box opened on a tile.
//!
//! Selection: a click on the picture selects (Shift-click a range), a double click opens the photo, a click on the strip
//! opens the name box. The selection is `app.ui.unnamed_selected` with the range anchor `app.ui.unnamed_anchor`.
//!
//! What naming applies to ([`apply_to`]): a box on a selected tile acts on every selected face, a box on any other tile on
//! that face alone (and leaves the selection as it is). Either way it is one command, so one undo step
//! ([`name_each`], `faces.nameFaces` with a name per face).
//!
//! Widget ids: `{prefix}-face:{photo}:{index}` the picture, `{prefix}-name:{photo}:{index}` the strip; the open box is
//! `field:tileName` with its list `tileName:pick:{name}`.

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

/// What the tile draws.
pub struct FaceTile<'a> {
    pub key: FaceKey,
    pub view: lightcraft_geom::Rect,
    /// The name the named faces suggest, if any.
    pub suggestion: Option<&'a str>,
    /// Widget id prefix (`unnamed`).
    pub prefix: &'a str,
    /// Position in its list, for the widget ids and the name box's neighbours.
    pub position: usize,
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
}

/// One entry of the list a tile belongs to, in the order shown: the face and the name suggested for it.
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

/// The name strip's text: the suggestion and a question mark, or a lone question mark.
pub fn strip_text(suggestion: Option<&str>) -> String {
    suggestion.map_or_else(|| "?".to_string(), |s| format!("{s}?"))
}

/// A click on face number `i` of `order`: select it (or deselect it), or with Shift every face from the last one clicked.
pub fn click(app: &mut LightcraftApp, order: &[Entry], i: usize, shift: bool) {
    let Some(f) = order.get(i) else { return };
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

/// Name faces, each with its own name (empty: leave alone), as one undo step.
pub fn name_each(app: &mut LightcraftApp, names: &[(FaceKey, String)]) {
    let list: Vec<Value> =
        names.iter().filter(|(_, n)| !n.trim().is_empty()).map(|((photo, index), n)| json!({"photo": photo, "index": index, "name": n})).collect();
    if !list.is_empty() {
        let _ = app.run("faces.nameFaces", json!({"faces": list}));
    }
}

/// Draw the tile in `r` (the picture on top, the strip below) and report what was done to it.
pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect, tile: &FaceTile, selected: bool, ppp: f32) -> TileEvent {
    let t = Tokens::get(ui.ctx());
    let (photo, index) = tile.key;
    let pic = Rect::from_min_size(r.min, vec2(r.width(), (r.height() - STRIP_H).max(1.0)));
    let strip = Rect::from_min_max(pos2(r.left(), pic.bottom()), r.right_bottom());
    let resp = ui.interact(pic, egui::Id::unique((tile.prefix, "face", photo, index)), Sense::click());
    crate::widgets::announce_selected(&resp, crate::i18n::tr("Unnamed face"), selected);
    register(ui, format!("{}-face:{photo}:{index}", tile.prefix), pic);
    let name = ui.interact(strip, egui::Id::unique((tile.prefix, "name", photo, index)), Sense::click());
    register(ui, format!("{}-name:{photo}:{index}", tile.prefix), strip);
    let editing = app.ui.name_edit.as_ref().is_some_and(|e| (e.photo, e.index) == tile.key);
    let p = ui.painter();
    p.rect_filled(pic, 3.0, t.canvas);
    if let Some(job) = app.session.face_job(lightcraft_catalog::PhotoId(photo), tile.view, (pic.width() * ppp).ceil() as usize)
        && let Some(tex) = app.renderer.variant(job)
    {
        p.image(tex.id(), pic, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    if selected {
        p.rect_filled(pic, 3.0, Color32::from_rgba_unmultiplied(70, 130, 255, 55));
        p.rect_stroke(pic, 3.0, Stroke::new(2.0, Color32::from_rgb(110, 160, 255)), StrokeKind::Inside);
        let c = pos2(pic.left() + 13.0, pic.top() + 13.0);
        p.circle_filled(c, 9.0, Color32::from_rgb(70, 130, 255));
        p.line_segment([pos2(c.x - 4.0, c.y), pos2(c.x - 1.0, c.y + 3.5)], Stroke::new(1.8, Color32::WHITE));
        p.line_segment([pos2(c.x - 1.0, c.y + 3.5), pos2(c.x + 4.5, c.y - 3.5)], Stroke::new(1.8, Color32::WHITE));
    } else if resp.hovered() {
        p.rect_stroke(pic, 3.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    }
    // the strip: always there, never dependent on hover
    let fill = if editing || name.hovered() { t.button_border } else { t.canvas };
    p.rect_filled(strip, 0.0, fill);
    let text = strip_text(tile.suggestion);
    let label = super::people::fit(p, &text, t.font(11.5), strip.width() - 10.0);
    let color = if tile.suggestion.is_some() { t.text } else { t.text_dim };
    p.text(pos2(strip.left() + 5.0, strip.center().y), Align2::LEFT_CENTER, label, t.font(11.5), color);
    let name = name.on_hover_text(crate::i18n::tr("Click to name this face"));
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

/// Open the name box on `entry`: its suggestion selected, or empty.
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

/// Draws the name box if it is open on `key`, and applies how it ended (`order` is the list the tile belongs to, `at` the
/// tile's position in it). Returns the face a Tab moved to, so the caller can scroll to it.
pub fn name_box_for(app: &mut LightcraftApp, ctx: &egui::Context, tile: Rect, order: &[Entry], at: usize) -> Option<FaceKey> {
    let entry = order.get(at)?;
    let mut input = {
        let e = app.ui.name_edit.as_ref().filter(|e| (e.photo, e.index) == entry.key)?;
        e.input.clone()
    };
    let hints = super::faces::hints_for(app, entry.key.0);
    let candidates = hints.as_deref().and_then(|h| h.by_index.get(&entry.key.1)).map(|h| h.candidates.clone()).unwrap_or_default();
    let people = app.caches.person_names(&app.session.catalog);
    let setup = super::faces::setup(app, ctx);
    let spec = NameBox { id: "tileName", candidates: &candidates, people: &people, current: None, setup };
    let outcome = name_box::show(ctx, box_pos(tile), &mut input, &spec);
    let touched = input.touched();
    if let Some(e) = app.ui.name_edit.as_mut().filter(|e| (e.photo, e.index) == entry.key) {
        e.input = input;
    }
    finish(app, ctx, order, at, outcome, touched)
}

/// What an ended name box means: close it, name the faces it applies to, on Tab open the neighbour's box.
fn finish(app: &mut LightcraftApp, ctx: &egui::Context, order: &[Entry], at: usize, outcome: Outcome, touched: bool) -> Option<FaceKey> {
    let entry = order.get(at)?;
    let targets = apply_to(&app.ui.unnamed_selected, order, entry.key);
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
            name_targets(app, order, &targets, touched.then_some(name.as_str()));
            None
        }
        Outcome::Move { step, save } => {
            let named = save.is_some();
            let from = at;
            let skip: Vec<FaceKey> = if named { targets.clone() } else { Vec::new() };
            if let Some(name) = save.as_deref() {
                name_targets(app, order, &targets, Some(name));
            }
            let to = neighbour(order, from, step, &skip);
            app.ui.name_edit = None;
            let e = to.and_then(|i| order.get(i))?;
            open(app, ctx, e);
            Some(e.key)
        }
    }
}

/// Name `targets`: all with `name` when given, else each with its own suggestion (faces without one are left alone).
fn name_targets(app: &mut LightcraftApp, order: &[Entry], targets: &[FaceKey], name: Option<&str>) {
    let names: Vec<(FaceKey, String)> = targets
        .iter()
        .map(|k| {
            let own = || order.iter().find(|e| e.key == *k).and_then(|e| e.suggestion.clone()).unwrap_or_default();
            (*k, name.map_or_else(own, str::to_string))
        })
        .collect();
    name_each(app, &names);
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
    fn the_strip_says_the_suggestion_with_a_question_mark_or_just_the_mark() {
        assert_eq!(strip_text(Some("Ann")), "Ann?");
        assert_eq!(strip_text(None), "?");
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
