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
    /// Unnamed faces in the library (more than are listed when the list was cut).
    pub total: usize,
    pub faces: Vec<UnnamedFace>,
    /// The faces are in confidence order: suggested ones first, then look-alikes next to each other.
    pub ordered: bool,
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
        ordered: v["ordered"] == true,
        ready: v["ready"] == true,
    }
}

/// The subtitle for a non-empty list: it names only the order the list is really in (suggested names first when any face
/// carries one, look-alikes next to each other when ordered, else none).
fn order_sentence(list: &Unnamed) -> &'static str {
    if list.ordered && list.faces.iter().any(|f| f.suggestion.is_some()) {
        "The likeliest names come first, then faces that look alike are next to each other. Click to select, Shift-click for a range; click the name under a face to name it (or the selected ones)."
    } else if list.ordered {
        "Faces that look alike are next to each other. Click to select, Shift-click for a range; click the name under a face to name it (or the selected ones)."
    } else {
        "Click faces to select them, Shift-click for a range; click the name under a face to name it (or the selected ones)."
    }
}

/// The unnamed faces, asked for again only when the catalog changed, or (while the scan runs and learns more faces) at
/// most every few seconds, since arranging them scores every face against the named ones and looks at every pair of the rest.
pub fn faces(app: &mut LightcraftApp, now: f64) -> Arc<Unnamed> {
    let (rev, indexed) = (app.session.catalog.revision, app.caches.faces_indexed);
    if let Some((r, i, at, list)) = &app.caches.unnamed
        && *r == rev
        && (*i == indexed || now - *at < 3.0)
    {
        return list.clone();
    }
    let v = app.session.execute("faces.unnamed", &json!({})).unwrap_or(Value::Null);
    let list = Arc::new(parse(&v));
    // what was selected and has since been named (or removed) is no longer selected
    let present: std::collections::HashSet<(u64, usize)> = list.faces.iter().map(|f| (f.photo, f.index)).collect();
    face_tile::retain_present(app, &Gallery::Unnamed, &present);
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
        crate::i18n::tr(order_sentence(list)).to_string()
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

    #[test]
    fn the_subtitle_names_only_the_order_the_list_is_in() {
        let face = |s: Option<&str>| UnnamedFace {
            photo: 1,
            index: 0,
            view: lightcraft_geom::Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 },
            suggestion: s.map(str::to_string),
        };
        let list = |ordered, faces| Unnamed { total: 2, faces, ordered, ready: true };
        let suggested = order_sentence(&list(true, vec![face(Some("Jane")), face(None)]));
        assert!(suggested.starts_with("The likeliest names come first"), "{suggested}");
        let alike = order_sentence(&list(true, vec![face(None), face(None)]));
        assert!(alike.starts_with("Faces that look alike") && !alike.contains("likeliest"), "{alike}");
        let plain = order_sentence(&list(false, vec![face(None)]));
        assert!(plain.starts_with("Click faces") && !plain.contains("alike"), "{plain}");
    }

    #[test]
    fn the_engines_answer_becomes_a_list_and_odd_entries_are_left_out() {
        let v = json!({
            "total": 9, "ordered": true, "ready": true,
            "faces": [
                {"photo": 3, "index": 1, "rect": {"x0": 0.1, "y0": 0.1, "x1": 0.3, "y1": 0.4}, "view": {"x0": 0.12, "y0": 0.12, "x1": 0.28, "y1": 0.38}, "suggestion": {"name": "Jane Doe", "score": 0.7}},
                {"photo": 4, "index": 0, "rect": {"x0": 0.1, "y0": 0.1, "x1": 0.3, "y1": 0.4}, "suggestion": null},
                {"photo": 5, "index": 0},
                {"index": 0, "rect": {"x0": 0.1, "y0": 0.1, "x1": 0.3, "y1": 0.4}},
            ],
        });
        let list = parse(&v);
        assert_eq!((list.total, list.ordered, list.ready, list.faces.len()), (9, true, true, 2));
        assert_eq!(list.faces[0].suggestion.as_deref(), Some("Jane Doe"));
        assert_eq!(list.faces[0].view.x0, 0.12, "the view box is used when there is one");
        assert_eq!((list.faces[1].view.x0, list.faces[1].suggestion.clone()), (0.1, None), "else the face's own box");
        for bad in [Value::Null, json!([]), json!({"faces": 5, "total": -1})] {
            assert_eq!(parse(&bad), Unnamed::default(), "{bad}");
        }
    }
}
