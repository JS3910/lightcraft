//! A person's page in the People view: only cropped faces, never whole photos. First the faces already named, then a
//! "More" section with the unnamed faces that look like them (best first), to be confirmed one at a time or in a
//! user-chosen batch.
//!
//! The engine does the looking (`faces.person`); this draws it. Only the rows on screen ask for a face render.

use std::sync::Arc;

use egui::{Align2, Pos2, Rect, RichText, Sense, pos2, vec2};
use serde_json::{Value, json};

use super::face_tile::{self, Entry, FaceTile, Gallery, Hover, Listing, STRIP_H, TileEvent};
use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

const PAD: f32 = 20.0;
const SECTION_H: f32 = 56.0;

/// One face: which photo, which of its regions, the box to show it by, and (for the "More" faces) how alike.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub photo: u64,
    pub index: usize,
    pub rect: lightcraft_geom::Rect,
    pub score: f32,
}

/// What `faces.person` said about one person.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PersonPage {
    pub name: String,
    pub total: usize,
    pub confirmed: Vec<Face>,
    pub more: Vec<Face>,
    /// Recognition is on, so an empty "More" means "none yet" and not "not looking".
    pub ready: bool,
    pub pending: usize,
}

/// The faces rejected as this person during one visit to their page. The catalog holds each rejection for good, so the
/// engine no longer lists the face in their More; the page keeps its tile where it was, showing a question mark and no
/// new suggestion, until the page is left. "Left" is seen from the frame passes: a page that was not drawn in the
/// previous pass starts a new visit.
#[derive(Clone, Debug, Default)]
pub struct Rejected {
    person: String,
    pass: u64,
    /// The face and the place it had in the More list.
    faces: Vec<(Face, usize)>,
}

impl Rejected {
    /// The page of `person` is drawn in egui pass `pass`: a visit that went by in between, or another person, starts afresh.
    pub fn visit(&mut self, person: &str, pass: u64) {
        if pass > self.pass.saturating_add(1) || self.person.trim().to_lowercase() != person.trim().to_lowercase() {
            self.faces.clear();
        }
        self.person = person.to_string();
        self.pass = pass;
    }

    /// Forget every rejected-in-place face.
    pub fn clear(&mut self) {
        self.faces.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// The face boxes `removed` went from their photos: forget those faces, and move the later ones of the same photos up
    /// (their region indexes shifted), with the places they had in the list.
    pub fn forget(&mut self, removed: &[face_tile::FaceKey]) {
        let gone: Vec<usize> = self.faces.iter().filter(|(f, _)| removed.contains(&(f.photo, f.index))).map(|(_, at)| *at).collect();
        self.faces.retain(|(f, _)| !removed.contains(&(f.photo, f.index)));
        for (face, at) in &mut self.faces {
            if let Some((_, index)) = face_tile::shift_key((face.photo, face.index), removed) {
                face.index = index;
            }
            *at = at.saturating_sub(gone.iter().filter(|g| **g < *at).count());
        }
    }

    pub fn contains(&self, key: (u64, usize)) -> bool {
        self.faces.iter().any(|(f, _)| (f.photo, f.index) == key)
    }

    /// `face`, which was tile number `at` of the More list, was rejected.
    pub fn remember(&mut self, face: &Face, at: usize) {
        if !self.contains((face.photo, face.index)) {
            self.faces.push((face.clone(), at));
        }
    }

    /// `more` as the engine lists it, with the rejected faces back in the places they had.
    pub fn put_back(&self, more: &[Face]) -> Vec<Face> {
        let mut out: Vec<Face> = more.to_vec();
        let mut kept: Vec<&(Face, usize)> =
            self.faces.iter().filter(|(f, _)| !more.iter().any(|m| (m.photo, m.index) == (f.photo, f.index))).collect();
        kept.sort_by_key(|(_, at)| *at);
        for (face, at) in kept {
            out.insert((*at).min(out.len()), face.clone());
        }
        out
    }
}

fn face_of(v: &Value) -> Option<Face> {
    let rect = |r: &Value| Some(lightcraft_geom::Rect { x0: r["x0"].as_f64()?, y0: r["y0"].as_f64()?, x1: r["x1"].as_f64()?, y1: r["y1"].as_f64()? });
    Some(Face {
        photo: v["photo"].as_u64()?,
        index: usize::try_from(v["index"].as_u64()?).ok()?,
        // shown by the detector's box when the scan has one: every face equally close
        rect: rect(&v["view"]).or_else(|| rect(&v["rect"]))?,
        score: v["score"].as_f64().unwrap_or(0.0) as f32,
    })
}

/// The engine's answer as a page (anything that does not make sense is left out).
pub fn parse(v: &Value) -> PersonPage {
    let faces = |k: &str| v[k].as_array().map(|a| a.iter().filter_map(face_of).collect()).unwrap_or_default();
    PersonPage {
        name: v["name"].as_str().unwrap_or("").to_string(),
        total: v["total"].as_u64().map_or(0, |n| usize::try_from(n).unwrap_or(0)),
        confirmed: faces("confirmed"),
        more: faces("more"),
        ready: v["ready"] == true,
        pending: v["pendingPhotos"].as_u64().map_or(0, |n| usize::try_from(n).unwrap_or(0)),
    }
}

/// The page for `name`, asked for again only when the catalog changed, or (while the scan runs) at most once a second
/// as it learns more faces.
fn page_for(app: &mut LightcraftApp, name: &str, now: f64) -> Arc<PersonPage> {
    let (rev, indexed) = (app.session.catalog.revision, app.caches.faces_indexed);
    if let Some((n, r, i, at, page)) = &app.caches.person_page
        && n == name
        && *r == rev
        && (*i == indexed || now - *at < 1.0)
    {
        return page.clone();
    }
    let v = app.session.execute("faces.person", &json!({"name": name})).unwrap_or(Value::Null);
    let page = Arc::new(parse(&v));
    app.caches.person_page = Some((name.to_string(), rev, indexed, now, page.clone()));
    page
}

/// The current, session-visible candidates at or above the selected score. The score, not the tile position, is the
/// boundary: a later tile with an equal score belongs to the batch too.
fn at_or_above(more: &[&Face], score: f32) -> Vec<(u64, usize)> {
    more.iter().filter(|f| f.score >= score).map(|f| (f.photo, f.index)).collect()
}

fn confirm_above(app: &mut LightcraftApp, ctx: &egui::Context, name: &str, more: &[&Face], score: f32) {
    let faces: Vec<Value> = at_or_above(more, score).into_iter().map(|(photo, index)| json!({"photo": photo, "index": index})).collect();
    if !faces.is_empty()
        && let Err(error) = app.run("faces.nameFaces", json!({"faces": faces, "name": name}))
    {
        app.toast(ctx, error);
    }
}

/// The faces of a person's two galleries as the page shows them, with the name each strip carries: the faces rejected during
/// this visit are put back where they were as a lone question mark (no name, so nothing new is offered, and a second
/// reject removes the box).
pub struct Lists {
    pub confirmed: Vec<Face>,
    pub more: Vec<Face>,
    pub confirmed_entries: Vec<Entry>,
    pub more_entries: Vec<Entry>,
}

/// `page` of the person `name` as the galleries show it.
pub fn lists(app: &LightcraftApp, page: &PersonPage, name: &str) -> Lists {
    let confirmed = app.ui.rejected_confirmed.put_back(&page.confirmed);
    let more = app.ui.rejected_faces.put_back(&page.more);
    let entries = |faces: &[Face], rejected: &Rejected| -> Vec<Entry> {
        faces
            .iter()
            .map(|f| Entry { key: (f.photo, f.index), suggestion: (!rejected.contains((f.photo, f.index))).then(|| name.to_string()) })
            .collect()
    };
    Lists {
        confirmed_entries: entries(&confirmed, &app.ui.rejected_confirmed),
        more_entries: entries(&more, &app.ui.rejected_faces),
        confirmed,
        more,
    }
}

/// Reject on a tile (or the Delete key, with any selected face as `face`) of `gallery` of the page of `lists`: the
/// faces whose names were rejected keep their tiles, in place, until the page is left.
pub fn reject(app: &mut LightcraftApp, ctx: &egui::Context, gallery: &Gallery, lists: &Lists, face: face_tile::FaceKey) {
    let (faces, order) = match gallery {
        Gallery::More(_) => (&lists.more, &lists.more_entries),
        _ => (&lists.confirmed, &lists.confirmed_entries),
    };
    if let face_tile::Rejection::Rejected(keys) = face_tile::reject(app, ctx, gallery, order, face) {
        for key in keys {
            let Some(at) = faces.iter().position(|f| (f.photo, f.index) == key) else { continue };
            let Some(f) = faces.get(at) else { continue };
            match gallery {
                Gallery::More(_) => app.ui.rejected_faces.remember(f, at),
                _ => app.ui.rejected_confirmed.remember(f, at),
            }
        }
    }
}

/// The Delete key on the open page: reject the selected faces, as the reject button on one of them does. Whether it did.
pub(crate) fn reject_selected(app: &mut LightcraftApp, ctx: &egui::Context, name: &str) -> bool {
    let Some(gallery) = app.ui.face_gallery.clone() else { return false };
    let (Gallery::Confirmed(n) | Gallery::More(n)) = &gallery else { return false };
    let Some(face) = app.ui.unnamed_selected.iter().min().copied().filter(|_| n == name) else { return false };
    let Some((cached, _, _, _, page)) = &app.caches.person_page else { return false };
    if cached != name {
        return false;
    }
    let lists = lists(app, &page.clone(), name);
    reject(app, ctx, &gallery, &lists, face);
    true
}

/// The page-scoped A key takes precedence over View ▸ Visualize Spots only for a visible More tile. Called before the
/// general shortcut dispatcher, using the previous frame's registered rectangles as the pointer's target.
pub(crate) fn handle_bulk_key(app: &mut LightcraftApp, ctx: &egui::Context) -> bool {
    if app.ui.view != crate::state::ViewMode::People || app.ui.dialog.is_some() || ctx.text_edit_focused() {
        return false;
    }
    let Some(name) = app.ui.person_page.clone() else { return false };
    if !ctx.input(|i| i.modifiers == egui::Modifiers::NONE && i.key_pressed(egui::Key::A)) {
        return false;
    }
    let Some((cached_name, cached_revision, _, _, page)) = &app.caches.person_page else { return false };
    if *cached_name != name || *cached_revision != app.session.catalog.revision {
        return false;
    }
    let page = page.clone();
    let pointer = ctx.pointer_hover_pos();
    let target = page
        .more
        .iter()
        .filter(|f| !app.ui.rejected_faces.contains((f.photo, f.index)))
        .find(|f| {
            let widget = format!("more-face:{}:{}", f.photo, f.index);
            app.widgets.iter().any(|(id, rect)| id == &widget && pointer.is_some_and(|p| rect.contains(p)))
        })
        .or_else(|| {
            page.more.iter().filter(|f| !app.ui.rejected_faces.contains((f.photo, f.index))).find(|f| {
                ctx.memory(|m| {
                    m.has_focus(face_tile::widget_id("more", "face", (f.photo, f.index)))
                        || m.has_focus(face_tile::widget_id("more", "above", (f.photo, f.index)))
                })
            })
        });
    let Some(target) = target else { return false };
    let more: Vec<&Face> = page.more.iter().filter(|f| !app.ui.rejected_faces.contains((f.photo, f.index))).collect();
    confirm_above(app, ctx, &name, &more, target.score);
    true
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, name: &str) {
    let t = Tokens::get(ui.ctx());
    let now = ui.input(|i| i.time);
    let page = page_for(app, name, now);
    let pass = ui.ctx().cumulative_pass_nr();
    app.ui.rejected_faces.visit(name, pass);
    app.ui.rejected_confirmed.visit(name, pass);
    let shown_name = if page.name.is_empty() { name.to_string() } else { page.name.clone() };
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(PAD - 4.0);
        let back = crate::widgets::arrow_button(ui, crate::i18n::tr("People"), crate::icons::Icon::ChevronLeft, true);
        register(ui, "person:back", back.rect);
        if back.clicked() {
            let _ = app.run("view.people", json!({}));
        }
        ui.label(RichText::new(&shown_name).font(t.semibold(15.0)).color(t.text));
        let rename = ui.button(crate::i18n::tr("Rename Person…"));
        register(ui, "person:rename", rename.rect);
        if rename.clicked() {
            super::people::open_rename(app, &shown_name);
        }
        let count = if page.total == 1 { crate::i18n::tr("1 face").to_string() } else { crate::i18n::tr_format!("{n} faces", n = page.total) };
        ui.label(RichText::new(count).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(PAD - 4.0);
            let photos = ui.button(crate::i18n::tr("Show photos")).on_hover_text(crate::i18n::tr("Their photos in the grid"));
            register(ui, "person:photos", photos.rect);
            if photos.clicked() {
                let _ = app.run("library.filter", json!({"person": shown_name}));
                let _ = app.run("view.photoGrid", json!({}));
            }
        });
    });
    ui.add_space(6.0);
    if page.total == 0 && app.ui.rejected_confirmed.is_empty() {
        super::empty_message(
            ui,
            ui.available_rect_before_wrap(),
            "No faces by that name",
            "They may have been renamed or removed. Go back to People", // (empty_message translates both)
        );
        return;
    }
    // the faces rejected on this visit stay where they were, as question marks
    let lists = lists(app, &page, &shown_name);
    let more: Vec<&Face> = lists.more.iter().collect();
    // (what "all above" and the A key may confirm: the faces with a suggestion left)
    let suggested: Vec<&Face> = more.iter().copied().filter(|f| !app.ui.rejected_faces.contains((f.photo, f.index))).collect();
    // what the "More" section says when it has no faces to show
    let recognising = app.caches.faces_active || page.ready;
    // with a model installed the button switches recognition on; without one it opens Settings ▸ Faces
    let step = if recognising { super::faces::Setup::Running } else { super::faces::setup(app, ui.ctx()) };
    let note: Option<String> = if step == super::faces::Setup::TurnOn {
        Some(crate::i18n::tr_format!("Face recognition is off. Turn it on to find more photos of {name}.", name = shown_name))
    } else if !recognising {
        Some(crate::i18n::tr_format!("Face recognition needs a model, in Settings, to find more photos of {name}.", name = shown_name))
    } else if more.is_empty() && page.pending > 0 {
        Some(crate::i18n::tr_format!("Looking through your photos… {n} left", n = page.pending))
    } else if more.is_empty() {
        Some(crate::i18n::tr_format!("No other faces look like {name} yet. Faces appear here as your photos are scanned.", name = shown_name))
    } else {
        None
    };
    let ppp = ui.ctx().pixels_per_point();
    let edge = super::people::tile_edge(app.ui.thumb_size);
    let tile_h = edge + STRIP_H;
    let (confirmed_gallery, more_gallery) = (Gallery::Confirmed(shown_name.clone()), Gallery::More(shown_name.clone()));
    let (confirmed_entries, more_entries) = (&lists.confirmed_entries, &lists.more_entries);
    // what was selected and has since been named, hidden or removed is no longer selected
    face_tile::retain_present(app, &confirmed_gallery, &confirmed_entries.iter().map(|e| e.key).collect());
    face_tile::retain_present(app, &more_gallery, &more_entries.iter().map(|e| e.key).collect());
    let mut hit: Option<(TileEvent, Face, Gallery)> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show_viewport(ui, |ui, viewport| {
        let width = ui.available_width();
        let cols = super::people::columns(width, edge);
        let row_h = tile_h + super::people::TILE_GAP;
        let confirmed_rows = lists.confirmed.len().div_ceil(cols);
        let more_rows = more.len().div_ceil(cols);
        let more_top = PAD + confirmed_rows as f32 * row_h + 12.0;
        let total_h = more_top + SECTION_H + more_rows as f32 * row_h + PAD;
        let (area, _) = ui.allocate_exact_size(vec2(width, total_h), Sense::hover());
        // the scroll area gives the viewport relative to its content; the grids are laid out on screen, so it goes there too
        // (else the first row drawn runs ahead of the scroll offset, and far enough down no tile is drawn at all)
        let layout = Layout { name: &shown_name, viewport: viewport.translate(area.min.to_vec2()), cols, edge, ppp };
        let origin = |top: f32| area.min + vec2(PAD, top);
        let confirmed: Vec<&Face> = lists.confirmed.iter().collect();
        let mut shown = draw_grid(app, ui, &Grid { gallery: &confirmed_gallery, faces: &confirmed, entries: confirmed_entries, origin: origin(PAD), layout: &layout }, &mut hit);
        // the "More" section
        let heading = pos2(area.left() + PAD, area.top() + more_top + 14.0);
        ui.painter().text(heading, Align2::LEFT_CENTER, crate::i18n::tr("More"), t.semibold(15.0), t.text);
        let sub = match &note {
            Some(n) => n.clone(),
            None => crate::i18n::tr_format!(
                "Faces that look like {name}. Click one to select it, use the check to confirm, ↑ for all above or × if it is not them; click a name to give the face to someone else.",
                name = shown_name
            ),
        };
        ui.painter().text(pos2(area.left() + PAD, area.top() + more_top + 36.0), Align2::LEFT_CENTER, sub, t.font(12.0), t.text_dim);
        if !recognising {
            let r = Rect::from_min_size(pos2(area.left() + PAD + 330.0, area.top() + more_top + 8.0), vec2(120.0, 24.0));
            let label = if step == super::faces::Setup::TurnOn { "Turn on" } else { "Open Settings" };
            let b = ui.put(r, egui::Button::new(crate::i18n::tr(label)));
            register(ui, "faces:setup", b.rect);
            if b.clicked() {
                if step == super::faces::Setup::TurnOn {
                    super::faces::take_step(app, ui.ctx(), step);
                } else {
                    let _ = app.run("app.settings", json!({"tab": "faces"}));
                }
            }
        }
        shown += draw_grid(app, ui, &Grid { gallery: &more_gallery, faces: &more, entries: more_entries, origin: origin(more_top + SECTION_H), layout: &layout }, &mut hit);
        // the picture cache keeps every face on screen
        app.renderer.want_variants(shown);
    });
    if let Some((what, f, gallery)) = hit {
        match what {
            TileEvent::Open => {
                let _ = app.run("library.select", json!({"ids": [f.photo], "active": f.photo}));
                let _ = app.run("view.detail", json!({}));
                // Escape comes back to this page; the People button to everyone, with this person next to its title
                app.ui.person_from = Some((Some(shown_name.clone()), f.photo));
                app.ui.last_person = Some(shown_name.clone());
                app.ui.person_page = None;
            }
            TileEvent::Confirm => {
                // on a selected tile the whole selection is confirmed, each face as the person; else this face alone
                face_tile::confirm_own(app, &gallery, &lists.more_entries, (f.photo, f.index));
            }
            TileEvent::ConfirmAbove => confirm_above(app, ui.ctx(), &shown_name, &suggested, f.score),
            TileEvent::Hide => reject(app, ui.ctx(), &gallery, &lists, (f.photo, f.index)),
            _ => {}
        }
    }
}

/// One of the page's galleries, laid out from `origin` (its top left, in the scroll area) in rows of `cols` tiles.
struct Grid<'a> {
    gallery: &'a Gallery,
    faces: &'a [&'a Face],
    /// The same faces as the shared tile's list, the name on each strip being the person's.
    entries: &'a [Entry],
    origin: Pos2,
    layout: &'a Layout<'a>,
}

/// What the page's galleries have in common.
struct Layout<'a> {
    /// The person's name.
    name: &'a str,
    viewport: Rect,
    cols: usize,
    /// A tile's picture edge, and pixels per point (for the size of the face picture).
    edge: f32,
    ppp: f32,
}

/// Draw the rows of `grid` that are on screen (only they ask for a face render); returns how many tiles that was. Faces
/// pressed on in a way the page acts on after the frame (open the photo, the hover buttons) are left in `hit`.
fn draw_grid<'a>(app: &mut LightcraftApp, ui: &mut egui::Ui, grid: &Grid<'a>, hit: &mut Option<(TileEvent, Face, Gallery)>) -> usize {
    let tile_h = grid.layout.edge + STRIP_H;
    let row_h = tile_h + super::people::TILE_GAP;
    let col_w = grid.layout.edge + super::people::TILE_GAP;
    let rows = grid.faces.len().div_ceil(grid.layout.cols);
    let cell = |i: usize| {
        Rect::from_min_size(
            grid.origin + vec2((i % grid.layout.cols) as f32 * col_w, (i / grid.layout.cols) as f32 * row_h),
            vec2(grid.layout.edge, tile_h),
        )
    };
    // Tab moved the name box to a tile that may be off screen
    if let Some(i) = app.ui.unnamed_scroll.and_then(|key| grid.entries.iter().position(|e| e.key == key)) {
        app.ui.unnamed_scroll = None;
        ui.scroll_to_rect(cell(i).expand2(vec2(0.0, super::people::TILE_GAP)), None);
    }
    let first = (((grid.layout.viewport.top() - grid.origin.y) / row_h).floor().max(0.0) as usize).min(rows);
    let last = (((grid.layout.viewport.bottom() - grid.origin.y) / row_h).ceil().max(0.0) as usize).min(rows);
    let mut shown = 0;
    for i in (first * grid.layout.cols)..(last * grid.layout.cols).min(grid.faces.len()) {
        let Some(f) = grid.faces.get(i) else { break };
        shown += 1;
        if let Some(event) = draw_tile(app, ui, grid, i, cell(i), f) {
            *hit = Some((event, (*f).clone(), grid.gallery.clone()));
        }
    }
    shown
}

/// One tile of `grid`: the event for the page to act on after the frame, if any (selecting, opening the name box and the
/// box itself are handled here).
fn draw_tile<'a>(app: &mut LightcraftApp, ui: &mut egui::Ui, grid: &Grid<'a>, i: usize, r: Rect, f: &Face) -> Option<TileEvent> {
    let key = (f.photo, f.index);
    let more = matches!(grid.gallery, Gallery::More(_));
    // a face rejected on this visit is a lone question mark: nothing to confirm, only the cross (which removes the box)
    let rejected = if more { app.ui.rejected_faces.contains(key) } else { app.ui.rejected_confirmed.contains(key) };
    // a confirmed face has the cross alone
    let hover = Some(Hover {
        confirm: more && !rejected,
        above: more && !rejected,
        hide: true,
        removes: rejected,
        person: grid.layout.name,
        score: f.score,
        above_count: grid.faces.iter().filter(|c| c.score >= f.score && !app.ui.rejected_faces.contains((c.photo, c.index))).count(),
        positional: false,
    });
    let suggestion = (!rejected).then_some(grid.layout.name);
    let spec = FaceTile { key, view: f.rect, suggestion, named: !more, prefix: if more { "more" } else { "person" }, hover };
    let selected = face_tile::is_selected(app, grid.gallery, key);
    let mut out = None;
    match face_tile::show(app, ui, r, &spec, selected, grid.layout.ppp) {
        TileEvent::Name => {
            if let Some(e) = grid.entries.get(i) {
                face_tile::open(app, ui.ctx(), e);
            }
        }
        TileEvent::Click { shift } => face_tile::click(app, grid.gallery, grid.entries, i, shift),
        TileEvent::None => {}
        event => out = Some(event),
    }
    if app.ui.name_edit.as_ref().is_some_and(|e| (e.photo, e.index) == key) {
        let list = Listing { gallery: grid.gallery, order: grid.entries, at: i };
        if let Some(to) = face_tile::name_box_for(app, ui.ctx(), r, &list) {
            app.ui.unnamed_scroll = Some(to);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_above_uses_score_including_later_ties_and_only_visible_faces() {
        let face = |photo, score| Face { photo, index: 0, rect: lightcraft_geom::Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }, score };
        // The tie is deliberately after a lower-score tile: an index slice would miss it.
        let candidates = [face(1, 0.95), face(2, 0.80), face(3, 0.70), face(4, 0.80), face(5, 0.60)];
        let dismissed = (2_u64, 0_usize);
        let shown: Vec<&Face> = candidates.iter().filter(|f| (f.photo, f.index) != dismissed).collect();
        assert_eq!(at_or_above(&shown, 0.80), vec![(1, 0), (4, 0)], "the dismissed face is excluded and the late tie included");
        assert_eq!(at_or_above(&shown, 0.95), vec![(1, 0)], "the highest choice names only itself");
        assert_eq!(at_or_above(&shown, 0.60), vec![(1, 0), (3, 0), (4, 0), (5, 0)], "the lowest choice names the full displayed range");
    }

    #[test]
    fn a_rejected_face_goes_back_where_it_was_until_the_page_is_left() {
        let face = |photo| Face { photo, index: 0, rect: lightcraft_geom::Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }, score: 0.5 };
        let mut rejected = Rejected::default();
        rejected.visit("Ann", 10);
        rejected.remember(&face(3), 2);
        rejected.remember(&face(3), 0);
        let engine = [face(1), face(2), face(4)];
        let shown: Vec<u64> = rejected.put_back(&engine).iter().map(|f| f.photo).collect();
        assert_eq!(shown, [1, 2, 3, 4], "in the place it had, once");
        // the engine still listing it (a page asked for before the catalog changed) is not a second tile
        assert_eq!(rejected.put_back(&[face(1), face(3)]).len(), 2);
        // a place past the end is the end
        rejected.remember(&face(9), 50);
        assert_eq!(rejected.put_back(&[]).iter().map(|f| f.photo).collect::<Vec<_>>(), [3, 9]);
        // the next pass is the same visit; one that went by without the page, or another person, starts a new one
        rejected.visit(" ann ", 11);
        assert!(rejected.contains((3, 0)));
        rejected.visit("Ann", 14);
        assert!(!rejected.contains((3, 0)), "the page was not drawn in between");
        rejected.remember(&face(3), 0);
        rejected.visit("Bob", 15);
        assert!(!rejected.contains((3, 0)));
    }

    #[test]
    fn the_engines_answer_becomes_a_page_and_odd_entries_are_left_out() {
        let v = json!({
            "name": "Jane Doe", "total": 2, "ready": true, "pendingPhotos": 7,
            "confirmed": [
                {"photo": 3, "index": 0, "rect": {"x0": 0.1, "y0": 0.2, "x1": 0.3, "y1": 0.5}},
                {"photo": 4, "index": 1},
                {"photo": "x", "index": 0, "rect": {"x0": 0.1, "y0": 0.2, "x1": 0.3, "y1": 0.5}},
            ],
            "more": [{"photo": 9, "index": 2, "score": 0.5, "rect": {"x0": 0.0, "y0": 0.0, "x1": 1.0, "y1": 1.0}}],
        });
        let page = parse(&v);
        assert_eq!((page.name.as_str(), page.total, page.ready, page.pending), ("Jane Doe", 2, true, 7));
        assert_eq!(page.confirmed.len(), 1, "a face without a box or a photo is dropped");
        assert_eq!((page.confirmed[0].photo, page.confirmed[0].index), (3, 0));
        assert_eq!((page.more.len(), page.more[0].score), (1, 0.5));
        // nothing, or nonsense, is an empty page and not a crash
        for bad in [Value::Null, json!([]), json!({"confirmed": "x", "more": 5, "total": -1})] {
            assert_eq!(parse(&bad), PersonPage::default(), "{bad}");
        }
    }

    #[test]
    fn forgetting_removed_faces_moves_the_later_ones_of_their_photos_up_with_their_places() {
        let face = |photo, index| Face { photo, index, rect: lightcraft_geom::Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }, score: 0.5 };
        let mut rejected = Rejected::default();
        rejected.visit("Ann", 1);
        rejected.remember(&face(1, 0), 0);
        rejected.remember(&face(1, 1), 1);
        rejected.remember(&face(2, 0), 2);
        rejected.forget(&[(1, 0)]);
        assert!(!rejected.contains((1, 1)) && rejected.contains((1, 0)) && rejected.contains((2, 0)));
        let shown = rejected.put_back(&[]);
        assert_eq!(shown.iter().map(|f| (f.photo, f.index)).collect::<Vec<_>>(), [(1, 0), (2, 0)]);
        assert_eq!(
            rejected.put_back(&[face(7, 0), face(8, 0)]).iter().map(|f| f.photo).collect::<Vec<_>>(),
            [1, 2, 7, 8],
            "places moved up with the removed tile"
        );
    }
}
