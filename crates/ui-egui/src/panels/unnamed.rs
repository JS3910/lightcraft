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

/// One unnamed face: which photo and region, the box to show it by, and a suggested name.
#[derive(Clone, Debug, PartialEq)]
pub struct UnnamedFace {
    pub photo: u64,
    pub index: usize,
    pub view: lightcraft_geom::Rect,
    pub suggestion: Option<String>,
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
}

fn rect_of(v: &Value) -> Option<lightcraft_geom::Rect> {
    Some(lightcraft_geom::Rect { x0: v["x0"].as_f64()?, y0: v["y0"].as_f64()?, x1: v["x1"].as_f64()?, y1: v["y1"].as_f64()? })
}

fn face_of(v: &Value) -> Option<UnnamedFace> {
    Some(UnnamedFace {
        photo: v["photo"].as_u64()?,
        index: usize::try_from(v["index"].as_u64()?).ok()?,
        view: rect_of(&v["view"]).or_else(|| rect_of(&v["rect"]))?,
        suggestion: v["suggestion"]["name"].as_str().map(str::to_string),
    })
}

/// The engine's answer as a list (anything that does not make sense is left out).
pub fn parse(v: &Value) -> Unnamed {
    Unnamed {
        total: v["total"].as_u64().map_or(0, |n| usize::try_from(n).unwrap_or(0)),
        faces: v["faces"].as_array().map(|a| a.iter().filter_map(face_of).collect()).unwrap_or_default(),
        look_alike: v["lookAlike"] == true,
        ready: v["ready"] == true,
    }
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
    if lead.is_empty() { hint.to_string() } else { format!("{lead} {hint}") }
}

/// While the scan runs and learns more faces, the list is asked for again after this many seconds at most (arranging it
/// works out a suggestion for every new face and the look-alike order of those without one).
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
    // A face is told apart by its photo and its box, not by its region index: removing a box (or undoing that) renumbers the
    // later regions of the photo, and a tile must stay with its face, not with a slot.
    let key = |f: &UnnamedFace| (f.photo, [f.view.x0.to_bits(), f.view.y0.to_bits(), f.view.x1.to_bits(), f.view.y1.to_bits()]);
    let start = shown.above.min(old.faces.len());
    let end = start.saturating_add(shown.on_screen).min(old.faces.len());
    let mut latest: std::collections::HashMap<_, &UnnamedFace> = new.faces.iter().map(|f| (key(f), f)).collect();
    // the faces in view, in their order, as far as the new list still has them
    let kept: Vec<UnnamedFace> = old.faces.get(start..end).unwrap_or_default().iter().filter_map(|f| latest.remove(&key(f)).cloned()).collect();
    let in_view: std::collections::HashSet<_> = kept.iter().map(key).collect();
    let mut rest = new.faces.iter().filter(|f| !in_view.contains(&key(f))).cloned().collect::<Vec<_>>();
    let below = rest.split_off(start.min(rest.len()));
    let mut faces = rest;
    faces.extend(kept);
    faces.extend(below);
    Unnamed { faces, ..new }
}

/// The unnamed faces of the People view's photos, asked for again when the catalog changed, or (while the scan runs and
/// learns more faces) at most every [`SCAN_REFRESH`] seconds. What a new list puts on the screen is [`keep_places`]; a
/// list is sorted afresh (nothing kept) when there is none, which is when the People view is opened and when the source
/// or a filter changes ([`super::people::follow_view`] drops it then).
pub fn faces(app: &mut LightcraftApp, now: f64) -> Arc<Unnamed> {
    let (rev, indexed) = (app.session.catalog.revision, app.caches.faces_indexed);
    if let Some((r, i, at, list)) = &app.caches.unnamed
        && *r == rev
        && (*i == indexed || now - *at < SCAN_REFRESH)
    {
        return list.clone();
    }
    let v = app.session.execute("faces.unnamed", &json!({"scope": "visible"})).unwrap_or(Value::Null);
    let asked = parse(&v);
    let list = Arc::new(match (&app.caches.unnamed, app.caches.unnamed_shown) {
        (Some((_, _, _, old)), Some(shown)) => keep_places(old, asked, shown),
        _ => asked,
    });
    // what was selected, rejected in place or being named and has since been named (or removed, or left the view with its
    // photo) is no longer so
    let present: std::collections::HashSet<(u64, usize)> = list.faces.iter().map(|f| (f.photo, f.index)).collect();
    face_tile::retain_present(app, &Gallery::Unnamed, &present);
    app.ui.rejected_unnamed.retain_present(&present);
    if app.ui.name_edit.as_ref().is_some_and(|e| !present.contains(&(e.photo, e.index))) {
        app.ui.name_edit = None;
    }
    app.caches.unnamed = Some((rev, indexed, now, list.clone()));
    list
}

/// The heading of the section, at `at`, and what it says about where the faces come from.
pub fn section_header(app: &mut LightcraftApp, ui: &mut egui::Ui, list: &Unnamed, at: egui::Pos2, right: f32) {
    let t = Tokens::get(ui.ctx());
    let count = if list.total > list.faces.len() {
        crate::i18n::tr_format!("{shown} of {total}", shown = list.faces.len(), total = list.total)
    } else {
        list.total.to_string()
    };
    let p = ui.painter();
    p.text(at, Align2::LEFT_CENTER, crate::i18n::tr("Unnamed faces"), t.semibold(15.0), t.text);
    p.text(pos2(right, at.y), Align2::RIGHT_CENTER, count, t.font(13.0), t.text_dim);
    let sub = if !list.ready && list.faces.is_empty() {
        crate::i18n::tr("Turn on face recognition in Settings to find the faces in your photos.").to_string()
    } else if list.faces.is_empty() && app.caches.faces_pending > 0 {
        crate::i18n::tr_format!("Looking through your photos… {n} left", n = app.caches.faces_pending)
    } else if list.faces.is_empty() {
        crate::i18n::tr("No unnamed faces. Faces found in your photos appear here until you name them.").to_string()
    } else {
        order_sentence(list).to_string()
    };
    p.text(pos2(at.x, at.y + 24.0), Align2::LEFT_CENTER, sub, t.font(12.0), t.text_dim);
    if !list.ready && list.faces.is_empty() {
        let r = Rect::from_min_size(pos2(at.x, at.y + 38.0), vec2(120.0, 22.0));
        let b = ui.put(r, egui::Button::new(crate::i18n::tr("Open Settings")));
        register(ui, "unnamed:openSettings", b.rect);
        if b.clicked() {
            let _ = app.run("app.settings", json!({"tab": "faces"}));
        }
    } else if !list.faces.is_empty() {
        let r = Rect::from_min_size(pos2(right - 130.0, at.y + 12.0), vec2(130.0, 22.0));
        let all = !list.faces.is_empty() && face_tile::selection(app, &Gallery::Unnamed).len() == list.faces.len();
        let b = ui.put(r, egui::Button::new(crate::i18n::tr(if all { "Select none" } else { "Select all" })));
        register(ui, "unnamed:selectAll", b.rect);
        if b.clicked() {
            if all {
                face_tile::clear_selection(app);
            } else {
                face_tile::select_all(app, &Gallery::Unnamed, list.faces.iter().map(|f| (f.photo, f.index)));
            }
        }
    }
}

/// The list as the name box and the selection see it: the faces in the order shown, with their suggestions.
pub fn entries(list: &Unnamed, rejected: &super::person::Rejected) -> Vec<Entry> {
    list.faces.iter().map(|f| Entry { key: (f.photo, f.index), suggestion: shown(f, rejected).map(str::to_string) }).collect()
}

/// The name on a face's strip: its suggestion, unless it was rejected during this visit (nothing new is offered straight
/// after a rejection).
fn shown<'a>(f: &'a UnnamedFace, rejected: &super::person::Rejected) -> Option<&'a str> {
    f.suggestion.as_deref().filter(|_| !rejected.contains((f.photo, f.index)))
}

/// Reject on a tile, or the Delete key with any selected face as `face`: the faces whose names were rejected stay in place
/// as a question mark until the view is left.
pub fn reject(app: &mut LightcraftApp, ctx: &egui::Context, list: &Unnamed, face: FaceKey) {
    let order = entries(list, &app.ui.rejected_unnamed);
    if let face_tile::Rejection::Rejected(keys) = face_tile::reject(app, ctx, &Gallery::Unnamed, &order, face) {
        for f in list.faces.iter().filter(|f| keys.contains(&(f.photo, f.index))) {
            let at = list.faces.iter().position(|o| (o.photo, o.index) == (f.photo, f.index)).unwrap_or(0);
            app.ui.rejected_unnamed.remember(&super::person::Face { photo: f.photo, index: f.index, rect: f.view, score: 0.0 }, at);
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

/// One face tile, number `i` of `list`, in `r` (the picture and the name strip). Returns the face a Tab moved the name box
/// to, so the view can scroll to it.
pub fn tile(app: &mut LightcraftApp, ui: &mut egui::Ui, list: &Unnamed, i: usize, r: Rect, f: &UnnamedFace, ppp: f32) -> Option<FaceKey> {
    let key = (f.photo, f.index);
    let selected = face_tile::is_selected(app, &Gallery::Unnamed, key);
    let suggestion = shown(f, &app.ui.rejected_unnamed);
    // a face with a suggestion can be confirmed; reject is on every tile (and removes the box when there is nothing to reject)
    let hover = Some(Hover {
        confirm: suggestion.is_some(),
        above: suggestion.is_some(),
        hide: true,
        removes: suggestion.is_none(),
        person: suggestion.unwrap_or_default(),
        score: 0.0,
        above_count: list.faces.iter().take(i + 1).filter(|c| shown(c, &app.ui.rejected_unnamed).is_some()).count(),
        positional: true,
    });
    let spec = FaceTile { key, view: f.view, suggestion, named: false, prefix: "unnamed", hover };
    match face_tile::show(app, ui, r, &spec, selected, ppp) {
        TileEvent::Name => face_tile::open(app, ui.ctx(), &Entry { key, suggestion: suggestion.map(str::to_string) }),
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
        UnnamedFace { photo, index, view: lightcraft_geom::Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }, suggestion: suggestion.map(str::to_string) }
    }

    #[test]
    fn the_subtitle_names_only_the_order_the_list_is_in() {
        let list = |look_alike, faces| Unnamed { total: 2, faces, look_alike, ready: true };
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
        Unnamed { total: photos.len(), faces: photos.iter().map(|p| face(*p, 0, None)).collect(), look_alike: false, ready: true }
    }

    fn order(l: &Unnamed) -> Vec<u64> {
        l.faces.iter().map(|f| f.photo).collect()
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
}
