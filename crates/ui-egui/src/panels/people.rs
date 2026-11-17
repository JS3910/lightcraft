//! People: a card per person named on faces (read from XMP): a close-up of their face and how many photos they are in
//! (a small number on the picture). A click opens that person's page (`person.rs`): only their cropped faces, and the
//! faces that look like them. Below the people, the faces nobody has named yet (`unnamed.rs`), to be named a group at a
//! time.
//!
//! The thumbnail-size slider in the bottom bar sizes the faces here too (with its own smaller and larger limits), and
//! every face is cut from the same kind of box (the detector's, once the scan has looked at it), so faces are shown
//! equally close. Only the rows on screen ask for a face render (the engine caches them, memory and disk).

use egui::{Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_catalog::Person;
use serde_json::json;

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

const GAP: f32 = 14.0;
pub(super) const PAD: f32 = 20.0;
const HEADER_H: f32 = 44.0;
/// The name under a card's picture.
const NAME_H: f32 = 26.0;
/// The unnamed-faces heading and its hints.
const SECTION_H: f32 = 70.0;

/// `thumb` (the thumbnail slider, 90 to 480) as a position between `low` and `high`.
fn between(thumb: f32, low: f32, high: f32) -> f32 {
    let f = (thumb - 90.0) / (480.0 - 90.0);
    low + if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.4 } * (high - low)
}

/// A person card's face picture (points) for the thumbnail slider's value.
pub fn card_edge(thumb: f32) -> f32 {
    between(thumb, 84.0, 260.0)
}

/// A face tile's picture (points), for the unnamed faces and a person's page.
pub fn tile_edge(thumb: f32) -> f32 {
    between(thumb, 56.0, 180.0)
}

/// How many `edge`-wide cells (with the gap between them) fit across `width`.
pub(super) fn columns(width: f32, edge: f32) -> usize {
    (((width - PAD * 2.0 + GAP) / (edge + GAP)).floor() as usize).max(1)
}

pub(super) const TILE_GAP: f32 = GAP;

/// `text` shortened with "…" until it is no wider than `max_w` in `font` (measured, not guessed).
pub(super) fn fit(painter: &egui::Painter, text: &str, font: egui::FontId, max_w: f32) -> String {
    let width = |s: &str| painter.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x;
    if width(text) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    (1..chars.len()).rev().map(|n| chars[..n].iter().collect::<String>() + "…").find(|s| width(s) <= max_w).unwrap_or_else(|| "…".to_string())
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    if let Some(name) = app.ui.person_page.clone() {
        return super::person::show(app, ui, &name);
    }
    let t = Tokens::get(ui.ctx());
    let now = ui.input(|i| i.time);
    let thumb = app.ui.thumb_size;
    let people = app.caches.people(&app.session.catalog, &app.session.filter);
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    let title =
        ui.painter().text(pos2(head.left() + PAD, head.center().y), Align2::LEFT_CENTER, crate::i18n::tr("Named People"), t.semibold(15.0), t.text);
    // the person page last left (a face opened from it, or its back button): one click returns to it
    if let Some(last) = app.ui.last_person.clone().filter(|n| people.iter().any(|p| &p.name == n)) {
        let label = fit(ui.painter(), &last, t.font(13.0), 224.0);
        let width = ui.painter().layout_no_wrap(label.clone(), t.font(13.0), Color32::WHITE).size().x + 44.0;
        let r = Rect::from_min_size(pos2(title.right() + 14.0, head.center().y - 12.0), vec2(width, 24.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        let b = crate::widgets::arrow_button(&mut child, RichText::new(label).font(t.font(13.0)), crate::icons::Icon::ChevronRight, false);
        register(ui, "people:last", b.rect);
        if b.on_hover_text(crate::i18n::tr("Back to this person")).clicked() {
            let _ = app.run("view.person", json!({"name": last}));
        }
    }
    ui.painter().text(pos2(head.right() - PAD, head.center().y), Align2::RIGHT_CENTER, people.len().to_string(), t.font(13.0), t.text_dim);
    // until face recognition is set up: what it takes, and a button that does the next step
    super::faces::setup_banner(app, ui);
    // the filters narrowing the list (a date, a keyword…), removable here
    let chips = lightcraft_engine::filter_chips(&app.session.filter, &app.session.catalog);
    super::chips::show(app, ui, &chips);
    // Esc with no name box open drops the selection
    clear_selection_on_escape(app, ui.ctx());
    let unnamed = super::unnamed::faces(app, now);
    app.ui.rejected_unnamed.visit("", ui.ctx().cumulative_pass_nr());
    if people.is_empty() && unnamed.faces.is_empty() {
        let (title, body) = if chips.is_empty() {
            ("No people yet", "Face names written to XMP by Lightroom and other apps show up here, and faces found in your photos can be named.")
        } else {
            ("No named people in these photos", "Remove a filter above, or choose Clear all")
        };
        super::empty_message(ui, ui.available_rect_before_wrap(), title, body);
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let active = app.session.filter.person.clone();
    let (edge, tile) = (card_edge(thumb), tile_edge(thumb));
    egui::ScrollArea::vertical().auto_shrink(false).show_viewport(ui, |ui, viewport| {
        let width = ui.available_width();
        let cols = columns(width, edge);
        let row_h = edge + NAME_H + GAP;
        let rows = people.len().div_ceil(cols);
        let named_h = if people.is_empty() { 40.0 } else { PAD + rows as f32 * row_h };
        let (tile_cols, tile_row) = (columns(width, tile), tile + super::face_tile::STRIP_H + GAP);
        let tile_rows = unnamed.faces.len().div_ceil(tile_cols);
        let section_top = named_h + 6.0;
        let (area, _) = ui.allocate_exact_size(vec2(width, section_top + SECTION_H + tile_rows as f32 * tile_row + PAD), Sense::hover());
        let visible = |top: f32, rows: usize, row_h: f32| {
            let first = ((viewport.top() - top) / row_h).floor().max(0.0) as usize;
            let last = (((viewport.bottom() - top) / row_h).ceil().max(0.0) as usize).min(rows);
            first..last
        };
        if people.is_empty() {
            ui.painter().text(
                pos2(area.left() + PAD, area.top() + 24.0),
                Align2::LEFT_CENTER,
                "No named people yet. Name the faces below to start.",
                t.font(13.0),
                t.text_dim,
            );
        }
        let mut shown = 0;
        for row in visible(PAD, rows, row_h) {
            for col in 0..cols {
                let Some(person) = people.get(row * cols + col) else { break };
                shown += 1;
                let min = area.min + vec2(PAD + col as f32 * (edge + GAP), PAD + row as f32 * row_h);
                let selected = active.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&person.name));
                card(app, ui, person, Rect::from_min_size(min, vec2(edge, edge + NAME_H)), edge, ppp, selected);
            }
        }
        // the faces nobody has named
        let top = area.top() + section_top;
        ui.painter().line_segment([pos2(area.left() + PAD, top), pos2(area.right() - PAD, top)], Stroke::new(1.0, t.button_border));
        super::unnamed::section_header(app, ui, &unnamed, pos2(area.left() + PAD, top + 14.0), area.right() - PAD);
        let grid_top = section_top + SECTION_H;
        // Tab moved the name box to a tile that may be off screen
        if let Some(key) = app.ui.unnamed_scroll.take()
            && let Some(i) = unnamed.faces.iter().position(|f| (f.photo, f.index) == key)
        {
            let min = area.min + vec2(PAD + (i % tile_cols) as f32 * (tile + GAP), grid_top + (i / tile_cols) as f32 * tile_row);
            ui.scroll_to_rect(Rect::from_min_size(min, vec2(tile, tile_row)), None);
        }
        for row in visible(grid_top, tile_rows, tile_row) {
            for col in 0..tile_cols {
                let i = row * tile_cols + col;
                let Some(face) = unnamed.faces.get(i) else { break };
                shown += 1;
                let min = area.min + vec2(PAD + col as f32 * (tile + GAP), grid_top + row as f32 * tile_row);
                let r = Rect::from_min_size(min, vec2(tile, tile + super::face_tile::STRIP_H));
                if let Some(to) = super::unnamed::tile(app, ui, &unnamed, i, r, face, ppp) {
                    app.ui.unnamed_scroll = Some(to);
                }
            }
        }
        // the picture cache keeps all of them (a fixed budget would evict and re-request the same few every frame)
        app.renderer.want_variants(shown);
    });
}

/// The Delete key (or the key labelled Backspace) in the People view rejects the selected faces, as the reject button on one
/// of them does: their names first, then their face boxes. It waits while a text field, a name box or a dialog has the
/// keyboard. The key is used up in the People view, selection or not.
pub(crate) fn handle_delete_key(app: &mut LightcraftApp, ctx: &egui::Context) -> bool {
    use egui::Key;
    if app.ui.view != crate::state::ViewMode::People || app.ui.dialog.is_some() || app.ui.name_edit.is_some() || app.ui.card_name.is_some() {
        return false;
    }
    if ctx.egui_wants_keyboard_input()
        || !ctx.input(|i| i.modifiers == egui::Modifiers::NONE && (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)))
    {
        return false;
    }
    let _ = match app.ui.person_page.clone() {
        Some(name) => super::person::reject_selected(app, ctx, &name),
        None => super::unnamed::reject_selected(app, ctx),
    };
    // with nothing selected the key does nothing here: it must not reach the photo delete behind the view
    true
}

/// Esc with no name box open drops the selected unnamed faces (with a box open, Esc only closes the box).
fn clear_selection_on_escape(app: &mut LightcraftApp, ctx: &egui::Context) {
    if app.ui.name_edit.is_none() && app.ui.dialog.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        super::face_tile::clear_selection(app);
    }
}

/// The badge on a person's picture: the full number of photos (the app has no digit-grouping helper, so counts elsewhere are plain too).
fn badge_count(count: usize) -> String {
    count.to_string()
}

/// "6110 photos", for what assistive tools announce.
fn photo_count_text(count: usize) -> String {
    crate::i18n::tr_format!("{n} photo{}", if count == 1 { "" } else { "s" }, n = count)
}

fn card(app: &mut LightcraftApp, ui: &mut egui::Ui, person: &Person, r: Rect, edge: f32, ppp: f32, selected: bool) {
    let t = Tokens::get(ui.ctx());
    let face = Rect::from_min_size(r.min, vec2(edge, edge));
    let name_rect = Rect::from_min_max(pos2(r.left(), face.bottom()), r.max);
    let resp = ui.interact(face, egui::Id::unique(("person-card", &person.name)), Sense::click());
    crate::widgets::announce(&resp, format!("{}, {}", person.name, photo_count_text(person.count)));
    register(ui, format!("person:{}", person.name), face);
    // the name under the picture: a click edits it in place
    let name_resp = ui.interact(name_rect, egui::Id::unique(("person-card-name", &person.name)), Sense::click());
    register(ui, format!("personName:{}", person.name), name_rect);
    let view = app.session.face_view(person.photo, person.face);
    let p = ui.painter();
    p.rect_filled(face, 3.0, t.canvas);
    if let Some(job) = app.session.face_job(person.photo, view, (edge * ppp).ceil() as usize)
        && let Some(tex) = app.renderer.variant(job)
    {
        p.image(tex.id(), face, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    // how many photos they are in, on the picture
    let g = p.layout_no_wrap(badge_count(person.count), t.font(11.5), Color32::WHITE);
    let pill = Rect::from_min_size(
        pos2((face.right() - g.size().x - 14.0).max(face.left() + 2.0), face.bottom() - g.size().y - 11.0),
        g.size() + vec2(10.0, 6.0),
    );
    p.rect_filled(pill, 4.0, Color32::from_black_alpha(175));
    p.galley(pill.min + vec2(5.0, 3.0), g, Color32::WHITE);
    if selected {
        super::grid::selection_ring(p, &t, face, lightcraft_engine::SelectionState::Active);
    } else if resp.hovered() {
        p.rect_stroke(face, 3.0, Stroke::new(1.0, t.text_dim), StrokeKind::Outside);
    }
    // a long name must not run past the card
    let name = fit(p, &person.name, t.semibold(13.0), edge);
    p.text(pos2(r.left() + 2.0, face.bottom() + 14.0), Align2::LEFT_CENTER, name, t.semibold(13.0), t.text);
    let resp = resp.on_hover_text(crate::i18n::tr_format!("{} — show their photos", person.name));
    let name_resp = name_resp.on_hover_text(crate::i18n::tr("Rename"));
    let (mut rename, mut editing) = (false, app.ui.card_name.as_ref().is_some_and(|e| e.from == person.name));
    for r in [&resp, &name_resp] {
        crate::widgets::context_menu(r, |ui| {
            let r = ui.button(crate::i18n::tr("Rename Person…"));
            register(ui, "personMenu:rename", r.rect);
            rename |= r.clicked();
        });
    }
    if rename {
        open_rename(app, &person.name);
    }
    if name_resp.clicked() && !editing {
        // one name box at a time: a box open on an unnamed tile closes
        app.ui.name_edit = None;
        let input = crate::name_box::NameBoxState::open(person.name.clone(), ui.ctx().cumulative_pass_nr());
        app.ui.card_name = Some(crate::state::CardNameEdit { from: person.name.clone(), input });
        editing = true;
    }
    if editing {
        name_box(app, ui, person, pos2(r.left(), name_rect.bottom() + 2.0));
    }
    if resp.clicked() {
        let _ = app.run("view.person", json!({"name": person.name}));
    }
}

/// The name box under a card being renamed. It offers the people's names only: a card has no face to rank them by.
fn name_box(app: &mut LightcraftApp, ui: &egui::Ui, person: &Person, at: egui::Pos2) {
    let people = app.caches.person_names(&app.session.catalog);
    let Some(edit) = app.ui.card_name.as_mut() else { return };
    let spec = crate::name_box::NameBox {
        id: "cardName",
        candidates: &[],
        people: &people,
        current: Some(&person.name),
        rejected: &[],
        setup: super::faces::Setup::Running,
    };
    use crate::name_box::Outcome;
    match crate::name_box::show(ui.ctx(), at, &mut edit.input, &spec) {
        Outcome::Open => {}
        Outcome::Save(name) => {
            app.ui.card_name = None;
            rename_to(app, &person.name, &name);
        }
        Outcome::Move { save, .. } => {
            app.ui.card_name = None;
            if let Some(name) = save {
                rename_to(app, &person.name, &name);
            }
        }
        Outcome::Cancel | Outcome::Untouched | Outcome::Setup => app.ui.card_name = None,
    }
}

/// A name typed for a person: a new name renames every face; another person's asks before combining. An empty name or
/// the name the card shows changes nothing.
fn rename_to(app: &mut LightcraftApp, from: &str, to: &str) {
    let to = to.trim();
    if to.is_empty() || to == from {
        return;
    }
    let other = from.to_lowercase() != to.to_lowercase() && app.session.catalog.people().iter().any(|p| p.name.to_lowercase() == to.to_lowercase());
    if other {
        app.ui.dialog = Some(crate::state::Dialog::CombinePeople { from: from.to_string(), to: to.to_string() });
    } else {
        let _ = app.run("faces.renamePerson", json!({"from": from, "to": to}));
    }
}

/// The Rename Person dialog, prefilled with the name the card shows.
pub(crate) fn open_rename(app: &mut LightcraftApp, name: &str) {
    app.ui.dialog = Some(crate::state::Dialog::RenamePerson { from: name.to_string(), to: name.to_string(), merge: false });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_badge_shows_the_full_count_not_a_capped_one() {
        assert_eq!((badge_count(6110), badge_count(1), badge_count(999_999)), ("6110".to_string(), "1".to_string(), "999999".to_string()));
        assert_eq!(photo_count_text(6110), "6110 photos");
        assert_eq!(photo_count_text(1), "1 photo");
    }

    #[test]
    fn faces_follow_the_thumbnail_slider_within_their_own_limits() {
        assert_eq!((card_edge(90.0), card_edge(480.0)), (84.0, 260.0));
        assert_eq!((tile_edge(90.0), tile_edge(480.0)), (56.0, 180.0));
        // between, growing with the slider; beyond its ends, held at the limits; nonsense, a middle size
        assert!(card_edge(220.0) > card_edge(150.0) && tile_edge(300.0) > tile_edge(220.0));
        assert_eq!((card_edge(-5.0), card_edge(10_000.0)), (84.0, 260.0));
        assert!(card_edge(f32::NAN) > 84.0 && card_edge(f32::NAN) < 260.0);
        // there is always at least one column, whatever the width
        assert_eq!((columns(0.0, 260.0), columns(-50.0, 260.0), columns(f32::NAN, 100.0)), (1, 1, 1));
        assert!(columns(1400.0, 84.0) > columns(1400.0, 260.0));
    }
}
