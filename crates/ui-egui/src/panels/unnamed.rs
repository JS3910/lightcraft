//! The unnamed faces in the People view: every face nobody has named, as cropped pictures, the likeliest names first, then
//! look-alikes next to each other (once recognition has looked at them), each a tile (`face_tile.rs`) with a name strip below the picture. Click
//! faces to select them (Shift-click selects a range); a click on a strip opens the name box on that tile, which names the
//! selected faces together when the tile is selected, else just that face. A face the named ones recognise carries the
//! name they suggest on its strip.
//!
//! The engine does the looking (`faces.unnamed`) and the naming (`faces.nameFaces`); this draws and selects.

use std::sync::Arc;

use egui::{Align2, Rect, pos2, vec2};
use serde_json::{Value, json};

use super::face_tile::{self, Entry, FaceKey, FaceTile, Gallery, Hover, Listing, TileEvent};
use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// One face of a stack: which photo and region, and its own box.
#[derive(Clone, Debug, PartialEq)]
pub struct StackFace {
    pub photo: u64,
    pub index: usize,
    pub rect: lightcraft_geom::Rect,
    /// The box to show it by (as for a single face: the detector's, when the scan embedded the face).
    pub view: lightcraft_geom::Rect,
}

/// One entry of the list, one tile: an unnamed face (which photo and region, the box to show it by, and a suggested name), or a
/// stack of faces that are virtually certain to be the same person, shown by its representative face (the face fields
/// are the representative's).
#[derive(Clone, Debug, PartialEq)]
pub struct UnnamedFace {
    pub photo: u64,
    pub index: usize,
    /// The face's own box, which only an edit changes: a tile is told apart by it, since `view` changes when the scan
    /// embeds the face.
    pub rect: lightcraft_geom::Rect,
    pub view: lightcraft_geom::Rect,
    pub suggestion: Option<String>,
    /// A stack: all its faces, the most typical first (the representative is the first), at least two. Empty for a single face.
    pub stack: Vec<StackFace>,
}

impl UnnamedFace {
    /// Whether the entry is a stack.
    pub fn is_stack(&self) -> bool {
        !self.stack.is_empty()
    }

    /// How many faces the entry holds: those of the stack, or one.
    pub fn count(&self) -> usize {
        self.stack.len().max(1)
    }

    /// Every face the entry holds, each with its own box and the box to show it by: the faces of the stack, or the face itself.
    pub fn members(&self) -> Vec<StackFace> {
        if self.is_stack() { self.stack.clone() } else { vec![StackFace { photo: self.photo, index: self.index, rect: self.rect, view: self.view }] }
    }

    /// Every face the entry holds, as photo and region.
    pub fn keys(&self) -> Vec<FaceKey> {
        self.members().iter().map(|m| (m.photo, m.index)).collect()
    }
}

/// What `faces.unnamed` said.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unnamed {
    /// Unnamed faces in the People view's photos (more than are listed when the list was cut).
    pub total: usize,
    pub faces: Vec<UnnamedFace>,
    /// Some of the faces that suggest nothing were put next to the ones they look like.
    pub look_alike: bool,
    /// Recognition is running.
    pub ready: bool,
    /// The engine is still putting the faces in order (on a thread of its own): the list is the best order it has now.
    pub ordering: bool,
    /// The list is the final order.
    pub complete: bool,
    /// The order is not final and nothing is working on it: the run was cancelled, died or could not be started.
    pub stopped: bool,
}

impl Unnamed {
    /// How many faces the list shows: those in stacks count too.
    pub fn face_count(&self) -> usize {
        self.faces.iter().map(UnnamedFace::count).sum()
    }

    /// Every face of the list, in stacks too.
    pub fn all_keys(&self) -> impl Iterator<Item = FaceKey> + '_ {
        self.faces.iter().flat_map(UnnamedFace::keys)
    }
}

fn rect_of(v: &Value) -> Option<lightcraft_geom::Rect> {
    Some(lightcraft_geom::Rect { x0: v["x0"].as_f64()?, y0: v["y0"].as_f64()?, x1: v["x1"].as_f64()?, y1: v["y1"].as_f64()? })
}

fn face_of(v: &Value) -> Option<UnnamedFace> {
    Some(UnnamedFace {
        photo: v["photo"].as_u64()?,
        index: usize::try_from(v["index"].as_u64()?).ok()?,
        rect: rect_of(&v["rect"]).or_else(|| rect_of(&v["view"]))?,
        view: rect_of(&v["view"]).or_else(|| rect_of(&v["rect"]))?,
        suggestion: v["suggestion"]["name"].as_str().map(str::to_string),
        stack: stack_of(v),
    })
}

/// The faces of the stack an entry stands for (none when it is a single face, or its stack has fewer than two faces we can read).
fn stack_of(v: &Value) -> Vec<StackFace> {
    let faces: Vec<StackFace> = v["stack"]["faces"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    // `[photo, index, x0, y0, x1, y1, view x0, y0, x1, y1]` (the view box is the box when the engine says none)
                    let n = f.as_array()?;
                    let num = |i: usize| n.get(i).and_then(Value::as_f64);
                    let rect = lightcraft_geom::Rect { x0: num(2)?, y0: num(3)?, x1: num(4)?, y1: num(5)? };
                    let view = match (num(6), num(7), num(8), num(9)) {
                        (Some(x0), Some(y0), Some(x1), Some(y1)) => lightcraft_geom::Rect { x0, y0, x1, y1 },
                        _ => rect,
                    };
                    Some(StackFace { photo: n.first()?.as_u64()?, index: usize::try_from(n.get(1)?.as_u64()?).ok()?, rect, view })
                })
                .collect()
        })
        .unwrap_or_default();
    if faces.len() >= 2 { faces } else { Vec::new() }
}

/// The engine's answer as a list (anything that does not make sense is left out).
pub fn parse(v: &Value) -> Unnamed {
    Unnamed {
        total: v["total"].as_u64().map_or(0, |n| usize::try_from(n).unwrap_or(0)),
        faces: v["faces"].as_array().map(|a| a.iter().filter_map(face_of).collect()).unwrap_or_default(),
        look_alike: v["lookAlike"] == true,
        ready: v["ready"] == true,
        ordering: v["ordering"] == true,
        complete: v["complete"] == true,
        stopped: v["stopped"] == true,
    }
}

/// A list that is only a status: its order is not complete (still being worked out, or stopped) and no list was on screen to
/// keep places in, so the section says so instead of showing tiles that would jump when the order comes in.
pub fn waiting(list: &Unnamed) -> bool {
    !list.complete && list.faces.is_empty() && list.total > 0
}

/// The subtitle for a non-empty list: it names only the order the list is really in (the likeliest names first when any
/// face carries a suggestion, look-alikes next to each other when some faces have none to go by), then how to use the tiles.
fn order_sentence(list: &Unnamed) -> String {
    let suggested = list.faces.iter().any(|f| f.suggestion.is_some());
    let lead = match (suggested, list.look_alike) {
        (true, true) => crate::i18n::tr("The likeliest names come first, then faces that look alike are next to each other."),
        (true, false) => crate::i18n::tr("The likeliest names come first."),
        (false, true) => crate::i18n::tr("Faces that look alike are next to each other."),
        (false, false) => "",
    };
    let hint = crate::i18n::tr("Click faces to select them, Shift-click for a range; click the name under a face to name it (or the selected ones).");
    let lead = if lead.is_empty() { String::new() } else { format!("{lead} ") };
    if list.faces.iter().any(UnnamedFace::is_stack) {
        let stacks = crate::i18n::tr("A tile with a number is a stack of faces that are almost certainly the same person: naming it names them all.");
        format!("{lead}{stacks} {hint}")
    } else {
        format!("{lead}{hint}")
    }
}

/// One cell of the grid: an entry of the list (a face or a stack), or a face of a stack that is open, which follows the
/// stack's own cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Entry(usize),
    Member { entry: usize, face: usize },
}

/// Whether the entry is a stack that was opened in place (it holds one of the faces `open` remembers).
pub fn is_open(f: &UnnamedFace, open: &std::collections::HashSet<FaceKey>) -> bool {
    f.is_stack() && f.stack.iter().any(|m| open.contains(&(m.photo, m.index)))
}

/// The cells of the grid, in order: every entry, and after an open stack its faces.
pub fn cells(list: &Unnamed, open: &std::collections::HashSet<FaceKey>) -> Vec<Cell> {
    let mut out = Vec::with_capacity(list.faces.len());
    for (i, f) in list.faces.iter().enumerate() {
        out.push(Cell::Entry(i));
        if is_open(f, open) {
            out.extend((0..f.stack.len()).map(|face| Cell::Member { entry: i, face }));
        }
    }
    out
}

/// What of the list lies on the screen when the cells `first..end` do: how many entries are wholly above them and how many are
/// at least partly in them (an open stack with its faces is one entry).
pub fn shown_cells(cells: &[Cell], first: usize, end: usize) -> Shown {
    let entry_of = |c: &Cell| match c {
        Cell::Entry(i) | Cell::Member { entry: i, .. } => *i,
    };
    let seen = |range: std::ops::Range<usize>| -> std::collections::BTreeSet<usize> {
        cells.get(range.start.min(cells.len())..range.end.min(cells.len())).unwrap_or_default().iter().map(entry_of).collect()
    };
    let above = seen(0..first);
    let on = seen(first..end);
    // (an entry cut by the top edge is in view, not above)
    let above = above.iter().filter(|i| !on.contains(i)).count();
    Shown { above, on_screen: on.len() }
}

/// Open the stack of `f` in place, or close it when it is open.
fn toggle(app: &mut LightcraftApp, f: &UnnamedFace) {
    if is_open(f, &app.ui.unnamed_open) {
        for k in f.keys() {
            app.ui.unnamed_open.remove(&k);
        }
    } else {
        app.ui.unnamed_open.extend(f.keys());
    }
}

/// One face of the open stack `list.faces[entry]` (number `face` in it), in `r`.
pub fn member_tile(app: &mut LightcraftApp, ui: &mut egui::Ui, (entry, face): (usize, usize), list: &Unnamed, r: Rect, ppp: f32) {
    let Some(m) = list.faces.get(entry).and_then(|f| f.stack.get(face)) else { return };
    let key = (m.photo, m.index);
    let spec = FaceTile { key, view: m.view, suggestion: None, named: false, prefix: "unnamed-member", hover: None, count: 0, open: false };
    // (a face of a stack is the stack for selecting: its tile is selected when the stack is, and a click selects or deselects all of it)
    let selected = list.faces.get(entry).is_some_and(|f| f.keys().into_iter().all(|k| face_tile::is_selected(app, &Gallery::Unnamed, k)));
    match face_tile::show(app, ui, r, &spec, selected, ppp) {
        TileEvent::Click { shift } => face_tile::click(app, &Gallery::Unnamed, &entries(list, &app.ui.rejected_unnamed), entry, shift),

        TileEvent::Open => {
            let _ = app.run("library.select", json!({"ids": [m.photo], "active": m.photo}));
            let _ = app.run("view.detail", json!({}));
            app.ui.person_from = Some((None, m.photo));
        }
        _ => {}
    }
}

/// While the scan runs and learns more faces, the list is asked for again after this many seconds at most (arranging it
/// works out a suggestion for every new face and the look-alike order of those without one). This holds when a result
/// came in meanwhile too: asking again at once would start the next run at once, and the scan would have a run going
/// all the time.
const SCAN_REFRESH: f64 = 10.0;

/// Where the list lies on the screen, as last drawn: how many tiles are wholly above the window and how many are at
/// least partly in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    pub above: usize,
    pub on_screen: usize,
}

/// A new list `new` (the engine's order) as it is put on the screen over `old`, the list shown now, of which `shown` lies
/// on the screen:
/// - the faces at least partly in view keep their places and their order (with what the new list says of them);
/// - the same number of faces as before stays above them;
/// - every other place, above and below, is filled from the new order, leaving out the faces kept in view.
///
/// So a face that left (named, or its box removed) just disappears and the tiles after it move up one place, a face the scan
/// found takes its sorted place out of sight, and what is out of sight is sorted again as confidences change.
pub fn keep_places(old: &Unnamed, new: Unnamed, shown: Shown) -> Unnamed {
    keep_places_held(old, new, shown, &|_| false)
}

/// [`keep_places`], and a stack in view that `held` says is to stay is kept as it is even when the new list lists its faces one
/// by one (what the engine answers between a change to every face of a stack, a rejected suggestion, and the run that stacks
/// them again), provided it lists all of them and no stack of its own.
pub fn keep_places_held(old: &Unnamed, new: Unnamed, shown: Shown, held: &dyn Fn(&UnnamedFace) -> bool) -> Unnamed {
    // A face is told apart by its photo and its own box, not by its region index: removing a box (or undoing that) renumbers
    // the later regions of the photo, and a tile must stay with its face, not with a slot. Not by the box it is shown by
    // either: that changes when the scan embeds the face, which is just when the list is asked for again. (Two faces of
    // a photo with the very same box are told apart by which of them it is, so neither tile is lost.)
    //
    // A stack is told apart by the faces it holds: it is the stack in the new list that holds one of the faces of the one in
    // view (so a stack that grew, or whose most typical face changed, keeps its place, and several tiles that became one stack
    // leave one tile, in the place of the first of them).
    type BoxKey = (u64, [u64; 4], usize);
    let keys = |faces: &[UnnamedFace]| -> Vec<Vec<BoxKey>> {
        let mut seen: std::collections::HashMap<(u64, [u64; 4]), usize> = std::collections::HashMap::new();
        let mut box_key = |photo: u64, r: &lightcraft_geom::Rect| -> BoxKey {
            let key = (photo, [r.x0.to_bits(), r.y0.to_bits(), r.x1.to_bits(), r.y1.to_bits()]);
            let n = seen.entry(key).or_insert(0);
            let ord = *n;
            *n += 1;
            (key.0, key.1, ord)
        };
        faces.iter().map(|f| f.members().iter().map(|m| box_key(m.photo, &m.rect)).collect()).collect()
    };
    let (old_keys, new_keys) = (keys(&old.faces), keys(&new.faces));
    let start = shown.above.min(old.faces.len());
    let end = start.saturating_add(shown.on_screen).min(old.faces.len());
    let holder: std::collections::HashMap<BoxKey, usize> = new_keys.iter().enumerate().flat_map(|(i, ks)| ks.iter().map(move |k| (*k, i))).collect();
    // the entries in view, in their order, as far as the new list still has them. An entry that held several faces stays the
    // entry that holds most of them (a face that leaves a stack, its name rejected or its box moved, becomes a tile of its own, and the stack keeps its place)
    let mut taken: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut kept: Vec<UnnamedFace> = Vec::new();
    for (n, ks) in old_keys.get(start..end).unwrap_or_default().iter().enumerate() {
        let was = old.faces.get(start + n);
        if let Some(was) = was.filter(|w| w.is_stack() && held(w)) {
            // the faces of the stack the new list still has (one that was named, or lost its box, is gone), each as a tile of its own
            let alive: Vec<(usize, usize)> =
                ks.iter().enumerate().filter_map(|(m, k)| Some((m, holder.get(k).copied().filter(|i| !taken.contains(i))?))).collect();
            let singles = alive.iter().all(|(_, i)| new.faces.get(*i).is_some_and(|f| !f.is_stack()));
            let distinct: std::collections::HashSet<_> = alive.iter().map(|(_, i)| i).collect();
            if alive.len() >= 2 && singles && distinct.len() == alive.len() {
                let faces: Vec<StackFace> = alive.iter().filter_map(|(m, _)| was.stack.get(*m).cloned()).collect();
                // (shown by the first face left, with what the engine now says of it)
                let head = alive.first().and_then(|(_, i)| new.faces.get(*i));
                if let (Some(head), Some(first)) = (head, faces.first()) {
                    taken.extend(alive.iter().map(|(_, i)| *i));
                    kept.push(UnnamedFace { photo: first.photo, index: first.index, rect: first.rect, stack: faces.clone(), ..head.clone() });
                    continue;
                }
            }
        }
        // (an entry that held several faces stays the entry that holds most of them: a face that left it becomes a
        // tile of its own, and the stack keeps its place)
        let mut share: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        for i in ks.iter().filter_map(|k| holder.get(k)).filter(|i| !taken.contains(*i)) {
            *share.entry(*i).or_default() += 1;
        }
        if let Some(i) = share.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|(i, _)| i) {
            taken.insert(i);
            kept.extend(new.faces.get(i).cloned());
        }
    }
    let mut rest: Vec<UnnamedFace> = new.faces.iter().enumerate().filter(|(i, _)| !taken.contains(i)).map(|(_, f)| f.clone()).collect();
    let below = rest.split_off(start.min(rest.len()));
    let mut faces = rest;
    faces.extend(kept);
    faces.extend(below);
    Unnamed { faces, ..new }
}

/// The list to show when the engine answered `asked` while `old` was the list on screen (none: no list yet; a [`waiting`]
/// list, and a list with no faces in it (there are no tiles to keep in place), count as none):
/// - a list on screen keeps the tiles in view in their places ([`keep_places`]), whether the answer is the final order or
///   the best order there is while the rest is worked out;
/// - with no list on screen the answer is shown when its order is final (sorted afresh), and while it is not, only as a
///   status: the tiles would jump when the order came in.
#[cfg(test)]
fn place(old: Option<&Unnamed>, shown: Option<Shown>, asked: Unnamed) -> Unnamed {
    place_held(old, shown, asked, &|_| false)
}

/// [`place`], keeping the stacks `held` names as they are ([`keep_places_held`]).
fn place_held(old: Option<&Unnamed>, shown: Option<Shown>, mut asked: Unnamed, held: &dyn Fn(&UnnamedFace) -> bool) -> Unnamed {
    match old.filter(|old| !waiting(old) && !old.faces.is_empty()) {
        Some(old) => match shown {
            Some(shown) => keep_places_held(old, asked, shown, held),
            None => asked,
        },
        None if !asked.complete => {
            asked.faces.clear();
            asked
        }
        None => asked,
    }
}

/// The unnamed faces of the People view's photos, asked for again when the catalog changed, when the engine took in a
/// result of the thread that puts them in order (`faces.pump` counts them), or (while the scan runs and learns more faces)
/// at most every [`SCAN_REFRESH`] seconds. The question does not wait for the order: it answers at once with the best order
/// there is and says when more is coming, so a click never waits for the scoring.
///
/// What a new list puts on the screen is [`keep_places`]; a list is sorted afresh (nothing kept) when there is none, which
/// is when the People view is opened and when the source or a filter changes ([`super::people::follow_view`] drops it
/// then). With no list on screen and an order that is not complete yet, there are no tiles, only a status ([`waiting`]):
/// tiles shown now would jump when the order comes in.
pub fn faces(app: &mut LightcraftApp, now: f64) -> Arc<Unnamed> {
    let (rev, indexed, arranged) = (app.session.catalog.revision, app.caches.faces_indexed, app.caches.faces_arranged);
    let restart = std::mem::take(&mut app.caches.unnamed_restart);
    if !restart
        && let Some((r, i, at, list)) = &app.caches.unnamed
        && *r == rev
        // (a result that came in is a reason to ask again, unless a list is on screen and the scan found faces meanwhile: then
        // the throttle holds. A status asks at once: there is nothing on screen to hold, and the count of faces the scan
        // learned is not what the list waits for)
        && if waiting(list) || *i == indexed { app.caches.unnamed_arranged == arranged } else { now - *at < SCAN_REFRESH }
    {
        return list.clone();
    }
    let v = app.session.execute("faces.unnamed", &json!({"scope": "visible", "wait": false, "restart": restart})).unwrap_or(Value::Null);
    let old = app.caches.unnamed.as_ref().map(|(_, _, _, old)| old.as_ref());
    let asked = parse(&v);
    if asked.ordering {
        // A run is going to finish a little later. Nothing else would look at its result soon: the scan's pump looks every
        // second or so when it has nothing else to do, and the window asks for a frame every few seconds. So the next frame
        // pumps, and (the caller asks for frames while the order is not complete) every 50 ms after it.
        app.caches.faces_ordering = true;
        app.caches.faces_next_pump = 0.0;
    }
    // a stack whose suggestion was rejected in this visit stays one tile in its place, a question mark, until the run that stacks
    // its faces again has finished
    let rejected = app.ui.rejected_unnamed.clone();
    let held = |f: &UnnamedFace| f.keys().iter().all(|k| rejected.contains(*k));
    let list = Arc::new(place_held(old, app.caches.unnamed_shown, asked, &held));
    app.caches.unnamed_arranged = arranged;
    // what was selected, rejected in place or being named and has since been named (or removed, or left the view with its
    // photo) is no longer so
    let present: std::collections::HashSet<(u64, usize)> = list.all_keys().collect();
    face_tile::retain_present(app, &Gallery::Unnamed, &present);
    app.ui.rejected_unnamed.retain_present(&present);
    if app.ui.name_edit.as_ref().is_some_and(|e| !present.contains(&(e.photo, e.index))) {
        app.ui.name_edit = None;
    }
    app.caches.unnamed = Some((rev, indexed, now, list.clone()));
    list
}

/// What the section says under its heading: a status while the faces are being put in order and there is no list to show,
/// else why there are no faces, else how the list is ordered and how to use it.
pub(crate) fn subtitle(list: &Unnamed, pending: u64) -> String {
    if waiting(list) && list.stopped {
        crate::i18n::tr("Putting the faces in order was stopped.").to_string()
    } else if waiting(list) {
        crate::i18n::tr("Putting the faces in order…").to_string()
    } else if !list.ready && list.faces.is_empty() {
        crate::i18n::tr("Turn on face recognition in Settings to find the faces in your photos.").to_string()
    } else if list.faces.is_empty() && pending > 0 {
        crate::i18n::tr_format!("Looking through your photos… {n} left", n = pending)
    } else if list.faces.is_empty() {
        crate::i18n::tr("No unnamed faces. Faces found in your photos appear here until you name them.").to_string()
    } else if list.stopped {
        format!("{} {}", crate::i18n::tr("Putting the faces in order was stopped, so this order is not final."), order_sentence(list))
    } else {
        order_sentence(list).to_string()
    }
}

/// What the heading counts: the faces, or "n of N" when the list was cut. While the order is worked out there are no tiles to
/// count and "0 of 40000" would read as an error, so it is the total alone.
fn count_text(list: &Unnamed) -> String {
    if !waiting(list) && list.total > list.face_count() {
        crate::i18n::tr_format!("{shown} of {total}", shown = list.face_count(), total = list.total)
    } else {
        list.total.to_string()
    }
}

/// The heading of the section, at `at`, and what it says about where the faces come from.
pub fn section_header(app: &mut LightcraftApp, ui: &mut egui::Ui, list: &Unnamed, at: egui::Pos2, right: f32) {
    let t = Tokens::get(ui.ctx());
    let count = count_text(list);
    let p = ui.painter();
    p.text(at, Align2::LEFT_CENTER, crate::i18n::tr("Unnamed faces"), t.semibold(15.0), t.text);
    p.text(pos2(right, at.y), Align2::RIGHT_CENTER, count, t.font(13.0), t.text_dim);
    let sub = subtitle(list, app.caches.faces_pending);
    let sub_at = p.text(pos2(at.x, at.y + 24.0), Align2::LEFT_CENTER, sub, t.font(12.0), t.text_dim);
    if waiting(list) {
        register(ui, "unnamed:status", sub_at);
    }
    if list.stopped && list.ready {
        // nothing starts the run again by itself until a face or a name changes: offer it
        let r = if list.faces.is_empty() {
            Rect::from_min_size(pos2(at.x, at.y + 38.0), vec2(150.0, 22.0))
        } else {
            Rect::from_min_size(pos2(right - 290.0, at.y + 12.0), vec2(150.0, 22.0))
        };
        let b = ui.put(r, egui::Button::new(crate::i18n::tr("Put in order again")));
        register(ui, "unnamed:restart", b.rect);
        if b.clicked() {
            app.caches.unnamed_restart = true;
        }
    }
    if !list.ready && list.faces.is_empty() {
        let r = Rect::from_min_size(pos2(at.x, at.y + 38.0), vec2(120.0, 22.0));
        let b = ui.put(r, egui::Button::new(crate::i18n::tr("Open Settings")));
        register(ui, "unnamed:openSettings", b.rect);
        if b.clicked() {
            let _ = app.run("app.settings", json!({"tab": "faces"}));
        }
    } else if !list.faces.is_empty() {
        let r = Rect::from_min_size(pos2(right - 130.0, at.y + 12.0), vec2(130.0, 22.0));
        // (a stack is its faces: they are all selected with it)
        let all_faces = || list.all_keys();
        let all = all_faces().next().is_some() && face_tile::selection(app, &Gallery::Unnamed).len() == all_faces().count();
        let b = ui.put(r, egui::Button::new(crate::i18n::tr(if all { "Select none" } else { "Select all" })));
        register(ui, "unnamed:selectAll", b.rect);
        if b.clicked() {
            if all {
                face_tile::clear_selection(app);
            } else {
                face_tile::select_all(app, &Gallery::Unnamed, all_faces());
            }
        }
    }
}

/// The list as the name box and the selection see it: the faces in the order shown, with their suggestions.
pub fn entries(list: &Unnamed, rejected: &super::person::Rejected) -> Vec<Entry> {
    list.faces.iter().map(|f| entry(f, rejected)).collect()
}

/// One tile of the list as the name box and the selection see it: its face (a stack's first), the name on its strip, and for a
/// stack the faces it holds.
fn entry(f: &UnnamedFace, rejected: &super::person::Rejected) -> Entry {
    Entry { key: (f.photo, f.index), suggestion: shown(f, rejected).map(str::to_string), members: if f.is_stack() { f.keys() } else { Vec::new() } }
}

/// The name on a face's strip: its suggestion, unless it was rejected during this visit (nothing new is offered straight
/// after a rejection; a stack's suggestion was rejected when any of its faces was).
fn shown<'a>(f: &'a UnnamedFace, rejected: &super::person::Rejected) -> Option<&'a str> {
    f.suggestion.as_deref().filter(|_| !f.keys().iter().any(|k| rejected.contains(*k)))
}

/// Reject on a tile, or the Delete key with any selected face as `face`: the faces whose names were rejected stay in place
/// as a question mark until the view is left.
pub fn reject(app: &mut LightcraftApp, ctx: &egui::Context, list: &Unnamed, face: FaceKey) {
    let order = entries(list, &app.ui.rejected_unnamed);
    if let face_tile::Rejection::Rejected(keys) = face_tile::reject(app, ctx, &Gallery::Unnamed, &order, face) {
        for (at, f) in list.faces.iter().enumerate().filter(|(_, f)| f.keys().iter().any(|k| keys.contains(k))) {
            // (every face of a stack is remembered: the stack's head may change while it stays in place)
            for m in f.members() {
                app.ui.rejected_unnamed.remember(&super::person::Face { photo: m.photo, index: m.index, rect: m.view, score: 0.0 }, at);
            }
        }
    }
}

/// The Delete key in the People view: reject the selected unnamed faces. Whether there were any.
pub(crate) fn reject_selected(app: &mut LightcraftApp, ctx: &egui::Context) -> bool {
    let Some(face) = app.ui.unnamed_selected.iter().min().copied().filter(|_| app.ui.face_gallery == Some(Gallery::Unnamed)) else { return false };
    let Some((_, _, _, list)) = app.caches.unnamed.clone() else { return false };
    reject(app, ctx, &list, face);
    true
}

/// How many faces the up arrow of tile `i` names: those with a suggestion shown, from the first tile through `i`, the faces of
/// a stack counted one by one ("?" tiles and stacks are skipped).
pub fn suggested_through(list: &Unnamed, i: usize, rejected: &super::person::Rejected) -> usize {
    list.faces.iter().take(i + 1).filter(|c| shown(c, rejected).is_some()).map(UnnamedFace::count).sum()
}

/// One face tile, number `i` of `list`, in `r` (the picture and the name strip). Returns the face a Tab moved the name box
/// to, so the view can scroll to it.
pub fn tile(app: &mut LightcraftApp, ui: &mut egui::Ui, list: &Unnamed, i: usize, r: Rect, f: &UnnamedFace, ppp: f32) -> Option<FaceKey> {
    let key = (f.photo, f.index);
    // (a stack is selected when all its faces are)
    let selected = f.keys().into_iter().all(|k| face_tile::is_selected(app, &Gallery::Unnamed, k));
    let suggestion = shown(f, &app.ui.rejected_unnamed);
    // a face with a suggestion can be confirmed; reject is on every tile (and removes the box when there is nothing to reject)
    let stack = f.is_stack();
    // (a stack with no suggestion has none of them: a cross on it would remove the boxes of many faces nobody looked at)
    let hover = (!stack || suggestion.is_some()).then(|| Hover {
        confirm: suggestion.is_some(),
        above: suggestion.is_some(),
        hide: true,
        removes: suggestion.is_none(),
        person: suggestion.unwrap_or_default(),
        score: 0.0,
        above_count: suggested_through(list, i, &app.ui.rejected_unnamed),
        positional: true,
    });
    let open = is_open(f, &app.ui.unnamed_open);
    let spec = FaceTile { key, view: f.view, suggestion, named: false, prefix: "unnamed", hover, count: if stack { f.count() } else { 0 }, open };
    match face_tile::show(app, ui, r, &spec, selected, ppp) {
        TileEvent::Name => face_tile::open(app, ui.ctx(), &entry(f, &app.ui.rejected_unnamed)),
        TileEvent::Click { shift } => face_tile::click(app, &Gallery::Unnamed, &entries(list, &app.ui.rejected_unnamed), i, shift),
        // a double click opens the photo, as in the grid; its second click must not undo the first one's selection
        TileEvent::Open => {
            let _ = app.run("library.select", json!({"ids": [f.photo], "active": f.photo}));
            let _ = app.run("view.detail", json!({}));
            // Escape comes back to the faces
            app.ui.person_from = Some((None, f.photo));
        }
        TileEvent::Confirm => face_tile::confirm_own(app, &Gallery::Unnamed, &entries(list, &app.ui.rejected_unnamed), key),
        TileEvent::ConfirmAbove => face_tile::confirm_through(app, &entries(list, &app.ui.rejected_unnamed), i),
        TileEvent::Hide => reject(app, ui.ctx(), list, key),
        TileEvent::Badge => toggle(app, f),
        TileEvent::None => {}
    }
    if app.ui.name_edit.as_ref().is_some_and(|e| (e.photo, e.index) == key) {
        let order = entries(list, &app.ui.rejected_unnamed);
        return face_tile::name_box_for(app, ui.ctx(), r, &Listing { gallery: &Gallery::Unnamed, order: &order, at: i });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(photo: u64, index: usize, suggestion: Option<&str>) -> UnnamedFace {
        let whole = lightcraft_geom::Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };
        UnnamedFace { photo, index, rect: whole, view: whole, suggestion: suggestion.map(str::to_string), stack: Vec::new() }
    }

    #[test]
    fn the_subtitle_names_only_the_order_the_list_is_in() {
        let list = |look_alike, faces| Unnamed { total: 2, faces, look_alike, ready: true, complete: true, ..Default::default() };
        let both = order_sentence(&list(true, vec![face(1, 0, Some("Jane")), face(1, 1, None), face(1, 2, None)]));
        assert!(both.starts_with("The likeliest names come first, then faces that look alike"), "{both}");
        let only_suggested = order_sentence(&list(false, vec![face(1, 0, Some("Jane")), face(1, 1, Some("Bob"))]));
        assert!(only_suggested.starts_with("The likeliest names come first.") && !only_suggested.contains("alike"), "{only_suggested}");
        let alike = order_sentence(&list(true, vec![face(1, 0, None), face(1, 1, None)]));
        assert!(alike.starts_with("Faces that look alike") && !alike.contains("likeliest"), "{alike}");
        let plain = order_sentence(&list(false, vec![face(1, 0, None)]));
        assert!(plain.starts_with("Click faces") && !plain.contains("alike") && !plain.contains("likeliest"), "{plain}");
        assert!(only_suggested.ends_with("name it (or the selected ones)."), "every sentence ends with how to use the tiles: {only_suggested}");
    }

    #[test]
    fn the_heading_counts_the_faces_and_while_waiting_only_the_total() {
        let cut = Unnamed { total: 40000, faces: vec![face(1, 0, None), face(1, 1, None)], ready: true, ..Default::default() };
        assert_eq!(count_text(&cut), "2 of 40000");
        assert_eq!(count_text(&Unnamed { total: 2, ..cut.clone() }), "2");
        let waiting = Unnamed { total: 40000, faces: Vec::new(), ordering: true, ready: true, ..Default::default() };
        assert_eq!(count_text(&waiting), "40000", "not \"0 of 40000\"");
    }

    #[test]
    fn the_engines_answer_becomes_a_list_and_odd_entries_are_left_out() {
        let v = json!({
            "total": 9, "ordered": true, "lookAlike": true, "ready": true,
            "faces": [
                {"photo": 3, "index": 1, "rect": {"x0": 0.1, "y0": 0.1, "x1": 0.3, "y1": 0.4}, "view": {"x0": 0.12, "y0": 0.12, "x1": 0.28, "y1": 0.38}, "suggestion": {"name": "Jane Doe", "score": 0.7}},
                {"photo": 4, "index": 0, "rect": {"x0": 0.1, "y0": 0.1, "x1": 0.3, "y1": 0.4}, "suggestion": null},
                {"photo": 5, "index": 0},
                {"index": 0, "rect": {"x0": 0.1, "y0": 0.1, "x1": 0.3, "y1": 0.4}},
            ],
        });
        let list = parse(&v);
        assert_eq!((list.total, list.look_alike, list.ready, list.faces.len()), (9, true, true, 2));
        assert_eq!(list.faces[0].suggestion.as_deref(), Some("Jane Doe"));
        assert_eq!(list.faces[0].view.x0, 0.12, "the view box is used when there is one");
        assert_eq!((list.faces[1].view.x0, list.faces[1].suggestion.clone()), (0.1, None), "else the face's own box");
        for bad in [Value::Null, json!([]), json!({"faces": 5, "total": -1})] {
            assert_eq!(parse(&bad), Unnamed::default(), "{bad}");
        }
    }

    /// A list of faces `(photo, 0)` for the photos given.
    fn list(photos: &[u64]) -> Unnamed {
        Unnamed {
            total: photos.len(),
            faces: photos.iter().map(|p| face(*p, 0, None)).collect(),
            look_alike: false,
            ready: true,
            ordering: false,
            complete: true,
            stopped: false,
        }
    }

    fn order(l: &Unnamed) -> Vec<u64> {
        l.faces.iter().map(|f| f.photo).collect()
    }

    /// `list(photos)` the engine says is still being put in order.
    fn ordering(photos: &[u64]) -> Unnamed {
        Unnamed { ordering: true, complete: false, ..list(photos) }
    }

    #[test]
    fn with_no_list_on_screen_an_order_that_is_not_final_is_a_status_and_never_tiles() {
        let now = place(None, None, ordering(&[3, 1, 2]));
        assert!(waiting(&now) && now.faces.is_empty() && now.total == 3, "{now:?}");
        assert_eq!(subtitle(&now, 0), "Putting the faces in order…");
        // still waiting for the next answer: a status list is not a list on screen
        let now = place(Some(&now), Some(Shown { above: 0, on_screen: 0 }), ordering(&[3, 1, 2]));
        assert!(waiting(&now), "{now:?}");
        // the final order: tiles, sorted afresh
        let now = place(Some(&now), None, list(&[3, 1, 2]));
        assert!(!waiting(&now) && order(&now) == [3, 1, 2], "{now:?}");
        // nothing to show is not a status
        assert!(!waiting(&place(None, None, Unnamed { ordering: true, complete: false, ..list(&[]) })));
    }

    #[test]
    fn a_stopped_order_with_no_list_on_screen_is_a_status_that_says_so() {
        let stopped = Unnamed { stopped: true, ordering: false, ..ordering(&[3, 1, 2]) };
        let now = place(None, None, stopped);
        assert!(waiting(&now) && now.faces.is_empty(), "{now:?}");
        assert_eq!(subtitle(&now, 0), "Putting the faces in order was stopped.");
    }

    #[test]
    fn a_list_with_no_faces_on_screen_is_no_list_to_keep_places_in() {
        // the scan had found nothing when the view opened; the first faces it finds arrive with an order still being worked out
        let empty = list(&[]);
        let now = place(Some(&empty), Some(Shown { above: 0, on_screen: 0 }), ordering(&[3, 1, 2]));
        assert!(waiting(&now) && now.faces.is_empty(), "no tiles in a half-done order that would then be pinned there: {now:?}");
        let now = place(Some(&empty), Some(Shown::default()), list(&[3, 1, 2]));
        assert_eq!(order(&now), [3, 1, 2], "the complete order is shown as it is");
    }

    #[test]
    fn with_a_list_on_screen_the_best_order_so_far_is_merged_keeping_the_tiles_in_view() {
        let old = list(&[1, 2, 3, 4, 5, 6]);
        // 3 is named meanwhile; the order of the rest is still the old one: 3's tile goes, the tiles after it move up
        let now = place(Some(&old), Some(Shown { above: 1, on_screen: 3 }), ordering(&[1, 2, 4, 5, 6]));
        assert!(now.ordering && !waiting(&now), "{now:?}");
        assert_eq!(order(&now), [1, 2, 4, 5, 6]);
        // the final order arrives while 2 4 5 are in view
        let done = place(Some(&now), Some(Shown { above: 1, on_screen: 3 }), list(&[6, 5, 4, 2, 1]));
        assert_eq!(order(&done), [6, 2, 4, 5, 1], "the faces in view kept their places: {done:?}");
    }

    #[test]
    fn two_faces_with_one_box_both_keep_their_tiles() {
        let mut old = list(&[1, 1, 2]);
        old.faces[1].index = 1;
        let new = old.clone();
        let now = keep_places(&old, new, Shown { above: 0, on_screen: 3 });
        assert_eq!(now.faces.len(), 3, "neither tile of the shared box is lost: {now:?}");
    }

    #[test]
    fn faces_in_view_keep_their_places_and_everything_else_takes_the_new_order() {
        // 1..=8 shown, 3 4 5 in view (two tiles above them); the engine now orders them 8 7 6 5 4 3 2 1
        let old = list(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let now = keep_places(&old, list(&[8, 7, 6, 5, 4, 3, 2, 1]), Shown { above: 2, on_screen: 3 });
        // two places above (the first two of the new order that are not in view), then 3 4 5 as they were, then the rest
        assert_eq!(order(&now), [8, 7, 3, 4, 5, 6, 2, 1]);
    }

    #[test]
    fn a_face_the_scan_shows_by_another_box_keeps_its_place() {
        // the scan embedded the faces: each is now shown by the detector's box, though its own box is the same
        let old = list(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let mut new = list(&[8, 7, 6, 5, 4, 3, 2, 1]);
        for f in &mut new.faces {
            f.view = lightcraft_geom::Rect { x0: 0.1, y0: 0.1, x1: 0.9, y1: 0.9 };
        }
        let now = keep_places(&old, new, Shown { above: 2, on_screen: 3 });
        assert_eq!(order(&now), [8, 7, 3, 4, 5, 6, 2, 1], "the faces in view stayed, whatever box they are shown by now");
    }

    #[test]
    fn a_face_that_leaves_closes_its_gap_without_re_sorting() {
        let old = list(&[1, 2, 3, 4, 5, 6, 7, 8]);
        // 4 was named (it is gone from the engine's list); 3 4 5 were in view
        let now = keep_places(&old, list(&[1, 2, 3, 5, 6, 7, 8]), Shown { above: 2, on_screen: 3 });
        assert_eq!(order(&now), [1, 2, 3, 5, 6, 7, 8], "the tiles after it moved up one place");
        // the same with the order changed out of sight: what is in view and after it still just moves up
        let now = keep_places(&old, list(&[1, 2, 3, 5, 8, 7, 6]), Shown { above: 2, on_screen: 3 });
        assert_eq!(order(&now), [1, 2, 3, 5, 8, 7, 6]);
        // another face in view leaves: nothing above it or in view before it moves
        let now = keep_places(&old, list(&[1, 2, 3, 4, 6, 7, 8]), Shown { above: 2, on_screen: 4 });
        assert_eq!(order(&now), [1, 2, 3, 4, 6, 7, 8]);
    }

    #[test]
    fn a_face_the_scan_found_takes_its_sorted_place_never_inside_the_faces_in_view() {
        let old = list(&[1, 2, 3, 4, 5, 6]);
        // 0 sorts first; 3 4 are in view
        let now = keep_places(&old, list(&[0, 1, 2, 3, 4, 5, 6]), Shown { above: 2, on_screen: 2 });
        assert_eq!(order(&now), [0, 1, 3, 4, 2, 5, 6], "two faces still above, then the faces in view as they were");
        // one that sorts into the middle of those in view
        let now = keep_places(&old, list(&[1, 2, 3, 35, 4, 5, 6]), Shown { above: 2, on_screen: 2 });
        assert_eq!(order(&now), [1, 2, 3, 4, 35, 5, 6]);
    }

    #[test]
    fn with_nothing_in_view_the_whole_list_takes_the_new_order() {
        let old = list(&[1, 2, 3]);
        let now = keep_places(&old, list(&[3, 2, 1]), Shown::default());
        assert_eq!(order(&now), [3, 2, 1]);
        // a list that scrolled past is not an error either
        let now = keep_places(&old, list(&[3, 2, 1]), Shown { above: 99, on_screen: 99 });
        assert_eq!(order(&now), [3, 2, 1]);
    }

    #[test]
    fn a_rejected_suggestion_keeps_its_tile_where_it_is_however_the_new_order_sorts_it() {
        // faces 1..=6 with a suggestion each, 2 3 4 in view; 3's name was rejected, so it now sorts with the look-alikes at the
        // end, and 6 (below the screen) moves up into the order without it
        let mut old = list(&[1, 2, 3, 4, 5, 6]);
        for f in &mut old.faces {
            f.suggestion = Some("Jane".into());
        }
        let mut new = list(&[1, 2, 4, 5, 6, 3]);
        for f in new.faces.iter_mut().filter(|f| f.photo != 3) {
            f.suggestion = Some("Jane".into());
        }
        let now = keep_places(&old, new, Shown { above: 1, on_screen: 3 });
        assert_eq!(order(&now), [1, 2, 3, 4, 5, 6], "nothing in view moved");
        assert_eq!(now.faces[2].suggestion, None, "the rejected tile is a question mark in its place");
    }

    #[test]
    fn what_the_new_list_says_of_a_face_in_view_is_used_in_its_old_place() {
        let old = list(&[1, 2, 3]);
        let mut new = list(&[3, 2, 1]);
        new.faces[1].suggestion = Some("Jane".into());
        new.total = 7;
        let now = keep_places(&old, new, Shown { above: 0, on_screen: 3 });
        assert_eq!((order(&now), now.faces[1].suggestion.as_deref(), now.total), (vec![1, 2, 3], Some("Jane"), 7));
    }

    /// A stack of the faces `(photo, 0)` for the photos given, shown by the first.
    fn stack(photos: &[u64]) -> UnnamedFace {
        let mut head = face(photos[0], 0, None);
        head.stack = photos.iter().map(|p| StackFace { photo: *p, index: 0, rect: head.rect, view: head.view }).collect();
        head
    }

    fn entries_of(l: &Unnamed) -> Vec<Vec<u64>> {
        l.faces.iter().map(|f| f.keys().iter().map(|k| k.0).collect()).collect()
    }

    #[test]
    fn the_engines_answer_may_hold_stacks_and_a_stack_of_fewer_than_two_faces_is_a_single_face() {
        let rect = json!({"x0": 0.1, "y0": 0.1, "x1": 0.3, "y1": 0.4});
        let v = json!({"total": 5, "ready": true, "faces": [
            {"photo": 1, "index": 0, "rect": rect, "stack": {"count": 3, "faces": [
                [1, 0, 0.1, 0.1, 0.3, 0.4], [2, 1, 0.1, 0.1, 0.3, 0.4], [3, 0, 0.1, 0.1, 0.3, 0.4]]}},
            {"photo": 4, "index": 0, "rect": rect, "stack": {"count": 1, "faces": [[4, 0, 0.1, 0.1, 0.3, 0.4]]}},
            {"photo": 5, "index": 0, "rect": rect},
        ]});
        let l = parse(&v);
        assert_eq!(entries_of(&l), [vec![1, 2, 3], vec![4], vec![5]]);
        assert_eq!((l.faces[0].count(), l.faces[1].count(), l.face_count(), l.faces[0].is_stack(), l.faces[1].is_stack()), (3, 1, 5, true, false));
        assert_eq!(l.all_keys().collect::<Vec<_>>(), [(1, 0), (2, 1), (3, 0), (4, 0), (5, 0)], "the faces in stacks are in the list too");
    }

    #[test]
    fn a_face_of_a_stack_is_shown_by_the_box_the_engine_gives_it_as_a_single_face_is() {
        let rect = json!({"x0": 0.1, "y0": 0.1, "x1": 0.3, "y1": 0.4});
        // (`[photo, index, box, view box]`; the view box is left out of the last, which then shows by its box)
        let v = json!({"total": 3, "ready": true, "faces": [
            {"photo": 1, "index": 0, "rect": rect, "stack": {"count": 3, "faces": [
                [1, 0, 0.1, 0.1, 0.3, 0.4, 0.05, 0.05, 0.35, 0.45], [2, 1, 0.2, 0.2, 0.4, 0.5, 0.15, 0.15, 0.45, 0.55],
                [3, 0, 0.1, 0.1, 0.3, 0.4], [3, "x"], {"photo": 9, "index": 0, "rect": rect}]}},
        ]});
        let l = parse(&v);
        assert_eq!(entries_of(&l), [vec![1, 2, 3]], "a face that cannot be read is left out");
        let s = &l.faces[0].stack;
        assert_eq!((s[0].view.x0, s[0].rect.x0), (0.05, 0.1), "the view is not the raw box");
        assert_eq!((s[1].view.x0, s[1].rect.x0), (0.15, 0.2));
        assert_eq!(s[2].view, s[2].rect, "an engine that says no view leaves the box");
    }

    #[test]
    fn faces_that_became_a_stack_leave_one_tile_in_the_place_of_the_first_of_them() {
        let old = list(&[1, 2, 3, 4, 5, 6]);
        // 3 4 5 are in view and now one stack; the engine sorts the stack first
        let new = Unnamed { faces: vec![stack(&[3, 4, 5]), face(1, 0, None), face(2, 0, None), face(6, 0, None)], total: 6, ..list(&[]) };
        let now = keep_places(&old, new, Shown { above: 2, on_screen: 3 });
        assert_eq!(entries_of(&now), [vec![1], vec![2], vec![3, 4, 5], vec![6]], "one tile where 3 was, and what came after it moved up");
    }

    #[test]
    fn a_stack_that_grows_or_changes_its_most_typical_face_keeps_its_place() {
        let old = Unnamed { faces: vec![face(1, 0, None), stack(&[3, 4]), face(2, 0, None)], total: 4, ..list(&[]) };
        // 7 joined and 4 is the most typical now
        let new = Unnamed { faces: vec![stack(&[4, 3, 7]), face(2, 0, None), face(1, 0, None)], total: 5, ..list(&[]) };
        let now = keep_places(&old, new, Shown { above: 0, on_screen: 3 });
        assert_eq!(entries_of(&now), [vec![1], vec![4, 3, 7], vec![2]]);
    }

    #[test]
    fn a_stack_that_lost_a_face_keeps_its_place_and_the_face_taken_out_is_a_tile_out_of_the_way() {
        let old = Unnamed { faces: vec![face(1, 0, None), stack(&[3, 4, 5]), face(2, 0, None)], total: 5, ..list(&[]) };
        // 3 was taken out of the stack: the engine lists it as a face of its own, last for now
        let new = Unnamed { faces: vec![stack(&[4, 5]), face(1, 0, None), face(2, 0, None), face(3, 0, None)], total: 5, ..list(&[]) };
        let now = keep_places(&old, new, Shown { above: 0, on_screen: 3 });
        assert_eq!(entries_of(&now), [vec![1], vec![4, 5], vec![2], vec![3]], "the stack is where it was; no other tile moved");
    }

    #[test]
    fn the_entries_on_screen_are_counted_from_the_cells_and_an_open_stack_is_one_entry() {
        let c = [Cell::Entry(0), Cell::Entry(1), Cell::Member { entry: 1, face: 0 }, Cell::Member { entry: 1, face: 1 }, Cell::Entry(2)];
        // the screen starts at the stack's second face: the stack is in view, only the entry before it is above
        assert_eq!(shown_cells(&c, 3, 5), Shown { above: 1, on_screen: 2 });
        assert_eq!(shown_cells(&c, 0, 2), Shown { above: 0, on_screen: 2 });
        assert_eq!(shown_cells(&c, 4, 5), Shown { above: 2, on_screen: 1 });
    }

    #[test]
    fn an_open_stack_is_followed_by_its_faces_in_the_cells() {
        let l = Unnamed { faces: vec![face(1, 0, None), stack(&[3, 4]), face(2, 0, None)], total: 4, ..list(&[]) };
        let none = std::collections::HashSet::new();
        assert_eq!(cells(&l, &none), [Cell::Entry(0), Cell::Entry(1), Cell::Entry(2)]);
        let open: std::collections::HashSet<FaceKey> = [(4, 0)].into_iter().collect();
        assert_eq!(
            cells(&l, &open),
            [Cell::Entry(0), Cell::Entry(1), Cell::Member { entry: 1, face: 0 }, Cell::Member { entry: 1, face: 1 }, Cell::Entry(2)]
        );
    }

    #[test]
    fn a_question_mark_stack_stays_one_stack_while_the_scan_adds_a_face_or_one_of_its_faces_goes() {
        let old = Unnamed { faces: vec![face(1, 0, None), stack(&[3, 4, 5]), face(2, 0, None)], total: 5, ..list(&[]) };
        let shown = Shown { above: 0, on_screen: 3 };
        let held = |_: &UnnamedFace| true;
        // the suggestion was rejected for all three: until the run stacks them again the engine lists them one by one, and the
        // scan found a face (7) meanwhile
        let singles = Unnamed {
            faces: vec![face(1, 0, None), face(3, 0, None), face(4, 0, None), face(5, 0, None), face(7, 0, None), face(2, 0, None)],
            total: 6,
            ..list(&[])
        };
        let now = keep_places_held(&old, singles, shown, &held);
        assert_eq!(entries_of(&now), [vec![1], vec![3, 4, 5], vec![2], vec![7]], "{:?}", entries_of(&now));
        // one of the faces was named (or its box removed) meanwhile: the others are still the stack, in its place
        let one_gone = Unnamed { faces: vec![face(1, 0, None), face(3, 0, None), face(5, 0, None), face(2, 0, None)], total: 4, ..list(&[]) };
        let now = keep_places_held(&old, one_gone, shown, &held);
        assert_eq!(entries_of(&now), [vec![1], vec![3, 5], vec![2]], "{:?}", entries_of(&now));
        assert_eq!(now.faces[1].count(), 2);
        // the stack's first face is the one that went: the stack is shown by the next one
        let head_gone = Unnamed { faces: vec![face(1, 0, None), face(4, 0, None), face(5, 0, None), face(2, 0, None)], total: 4, ..list(&[]) };
        let now = keep_places_held(&old, head_gone, shown, &held);
        assert_eq!(entries_of(&now), [vec![1], vec![4, 5], vec![2]]);
        assert_eq!((now.faces[1].photo, now.faces[1].is_stack()), (4, true), "the head is a face the stack still holds");
        // the engine stacks all of them, with the new face, again: that stack is the one in place
        let again = Unnamed { faces: vec![stack(&[3, 4, 5, 7]), face(1, 0, None), face(2, 0, None)], total: 5, ..list(&[]) };
        let now = keep_places_held(&old, again, shown, &held);
        assert_eq!(entries_of(&now), [vec![1], vec![3, 4, 5, 7], vec![2]]);
    }

    #[test]
    fn a_stack_that_was_named_closes_its_gap() {
        let old = Unnamed { faces: vec![face(1, 0, None), stack(&[3, 4]), face(2, 0, None)], total: 4, ..list(&[]) };
        let new = Unnamed { faces: vec![face(1, 0, None), face(2, 0, None)], total: 2, ..list(&[]) };
        let now = keep_places(&old, new, Shown { above: 0, on_screen: 3 });
        assert_eq!(entries_of(&now), [vec![1], vec![2]]);
    }

    #[test]
    fn the_subtitle_mentions_stacks_only_when_the_list_has_some() {
        let with = Unnamed { faces: vec![stack(&[1, 2]), face(3, 0, None)], total: 3, ready: true, complete: true, ..Default::default() };
        let without = Unnamed { faces: vec![face(3, 0, None)], ..with.clone() };
        assert!(subtitle(&with, 0).contains("stack of faces that are almost certainly the same person"), "{}", subtitle(&with, 0));
        assert!(!subtitle(&without, 0).contains("stack"));
    }
}
