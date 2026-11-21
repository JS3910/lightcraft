//! Naming in place on the People view's unnamed faces, driven through the control channel: the name strip under every
//! tile, the name box a click on it opens, what the box names when faces are selected, and Tab / Esc.

use std::sync::Arc;
use std::time::Duration;

use lightcraft_catalog::{Op, PhotoId};
use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

pub(super) fn region(x: f64, name: Option<&str>) -> lightcraft_meta::Region {
    lightcraft_meta::Region {
        rect: lightcraft_geom::Rect { x0: x, y0: 0.2, x1: x + 0.2, y1: 0.6 },
        kind: lightcraft_meta::RegionKind::Face,
        name: name.map(str::to_string),
        description: None,
        ..Default::default()
    }
}

pub(super) fn set_regions(h: &mut Headless, id: PhotoId, regions: Vec<lightcraft_meta::Region>) {
    let mut meta = h.app.session.catalog.photo(id).unwrap().meta.clone();
    meta.regions = regions;
    h.app.session.commit("setup", Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
}

pub(super) fn step(h: &mut Headless) {
    h.step();
    h.step();
}

fn ok(h: &mut Headless, method: &str, params: serde_json::Value) {
    let r = h.request(method, params.clone(), T);
    assert_eq!(r["ok"], true, "{method} {params}: {r}");
    step(h);
}

fn key(h: &mut Headless, key: &str) {
    ok(h, "ui.key", json!({"key": key}));
}

/// The People view over a demo library: photo 0 has Ann Lee and one unnamed face, photos 1 and 2 only unnamed faces
/// (four unnamed faces in all: (p0,1), (p1,0), (p1,1), (p2,0)).
pub(super) fn people() -> (Headless, [PhotoId; 3]) {
    people_in([1400.0, 900.0])
}

/// [`people`] in a window of `size`.
pub(super) fn people_in(size: [f32; 2]) -> (Headless, [PhotoId; 3]) {
    let services = Services { png: None, write_shared: Some(Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, size, 1.0);
    h.settle(T);
    let ids: Vec<PhotoId> = h.app.session.catalog.photos().map(|p| p.id).take(3).collect();
    set_regions(&mut h, ids[0], vec![region(0.1, Some("Ann Lee")), region(0.5, None)]);
    set_regions(&mut h, ids[1], vec![region(0.3, None), region(0.6, None)]);
    set_regions(&mut h, ids[2], vec![region(0.3, None)]);
    ok(&mut h, "ui.set", json!({"view": "people"}));
    h.settle(T);
    step(&mut h);
    (h, [ids[0], ids[1], ids[2]])
}

pub(super) fn picture(id: PhotoId, i: usize) -> String {
    format!("unnamed-face:{}:{i}", id.0)
}

pub(super) fn strip(id: PhotoId, i: usize) -> String {
    format!("unnamed-name:{}:{i}", id.0)
}

fn names(h: &Headless, id: PhotoId) -> Vec<Option<String>> {
    h.app.session.catalog.photo(id).unwrap().meta.regions.iter().map(|r| r.name.clone()).collect()
}

fn n(s: &str) -> Option<String> {
    Some(s.to_string())
}

/// Give faces a suggested name as if recognition had looked at them (the list is cached for the current catalog).
pub(super) fn suggest(h: &mut Headless, who: &[(PhotoId, usize, &str)]) {
    let mut list = (*crate::panels::unnamed::faces(&mut h.app, 0.0)).clone();
    for f in &mut list.faces {
        f.suggestion = who.iter().find(|(p, i, _)| p.0 == f.photo && *i == f.index).map(|(_, _, s)| s.to_string());
    }
    let (rev, indexed) = (h.app.session.catalog.revision, h.app.caches.faces_indexed);
    h.app.caches.unnamed = Some((rev, indexed, f64::MAX, Arc::new(list)));
    step(h);
}

/// The people a face looks most like, as if recognition had ranked them: what its name box lists before anything is typed.
fn closest(h: &mut Headless, id: PhotoId, index: usize, who: &[(&str, f32)]) {
    let mut hints = crate::panels::faces::Hints::default();
    let candidates = who.iter().map(|(n, s)| (n.to_string(), *s)).collect();
    hints.by_index.insert(index, crate::panels::faces::Hint { suggestion: None, candidates });
    h.app.caches.faces_active = true;
    h.app.caches.faces_next_pump = f64::MAX;
    h.app.caches.face_hints = Some((id.0, h.app.session.catalog.revision, h.app.caches.faces_indexed, Arc::new(hints)));
    step(h);
}

fn selected(h: &Headless) -> usize {
    h.app.ui.unnamed_selected.len()
}

fn shown(h: &Headless) -> String {
    h.app.ui.name_edit.as_ref().map(|e| e.input.text.clone()).unwrap_or_default()
}

fn widgets(h: &mut Headless, filter: &str) -> Vec<String> {
    let r = h.request("ui.widgets", json!({"filter": filter}), T);
    r["result"].as_array().into_iter().flatten().filter_map(|w| w["id"].as_str().map(str::to_string)).collect()
}

#[test]
fn every_unnamed_tile_has_a_name_strip_and_the_naming_bar_is_gone() {
    let (mut h, [p0, p1, p2]) = people();
    suggest(&mut h, &[(p1, 0, "Ann Lee")]);
    let strips = widgets(&mut h, "unnamed-name:");
    for (id, i) in [(p0, 1), (p1, 0), (p1, 1), (p2, 0)] {
        assert!(strips.contains(&strip(id, i)), "{strips:?}");
    }
    assert!(widgets(&mut h, "unnamed:name").is_empty() && widgets(&mut h, "unnamed:clear").is_empty());
    assert!(widgets(&mut h, "field:unnamedName").is_empty());
    assert!(widgets(&mut h, "unnamed-accept:").is_empty(), "the overlay strip on the picture is gone");
    assert_eq!(widgets(&mut h, "unnamed:selectAll").len(), 1, "Select all stays");
}

#[test]
fn a_click_on_a_suggested_name_opens_the_box_with_the_name_selected_instead_of_naming() {
    let (mut h, [_, p1, _]) = people();
    suggest(&mut h, &[(p1, 0, "Ann Lee")]);
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p1, 0)}));
    assert_eq!(shown(&h), "Ann Lee");
    assert_eq!(names(&h, p1), [None, None], "nothing is named yet");
    assert_eq!(h.app.session.undo.len(), undo);
    key(&mut h, "enter");
    assert_eq!(names(&h, p1), [n("Ann Lee"), None]);
    assert!(h.app.ui.name_edit.is_none());
}

#[test]
fn a_tile_without_a_suggestion_opens_an_empty_box() {
    let (mut h, [_, p1, _]) = people();
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p1, 1)}));
    assert_eq!(shown(&h), "");
    assert!(h.app.ui.name_edit.is_some());
}

#[test]
fn a_click_on_a_listed_name_names_a_face_that_has_no_suggestion() {
    let (mut h, [_, p1, _]) = people();
    closest(&mut h, p1, 1, &[("Ann Lee", 0.4)]);
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p1, 1)}));
    ok(&mut h, "ui.clickWidget", json!({"id": "tileName:pick:Ann Lee"}));
    assert_eq!(names(&h, p1), [None, n("Ann Lee")]);
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(h.app.session.undo.len(), undo + 1);
}

#[test]
fn a_click_on_a_listed_name_names_the_face_that_and_not_its_suggestion() {
    let (mut h, [_, p1, _]) = people();
    suggest(&mut h, &[(p1, 0, "Bob Ray")]);
    closest(&mut h, p1, 0, &[("Bob Ray", 0.8), ("Ann Lee", 0.4)]);
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p1, 0)}));
    assert_eq!(shown(&h), "Bob Ray");
    ok(&mut h, "ui.clickWidget", json!({"id": "tileName:pick:Ann Lee"}));
    assert_eq!(names(&h, p1), [n("Ann Lee"), None], "the name clicked, not the one the box opened with");
}

#[test]
fn a_click_on_a_listed_name_names_every_selected_face() {
    let (mut h, [p0, p1, p2]) = people();
    suggest(&mut h, &[(p1, 0, "Bob Ray")]);
    closest(&mut h, p2, 0, &[("Ann Lee", 0.4)]);
    for (id, i) in [(p1, 0), (p2, 0)] {
        ok(&mut h, "ui.clickWidget", json!({"id": picture(id, i)}));
    }
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p2, 0)}));
    ok(&mut h, "ui.clickWidget", json!({"id": "tileName:pick:Ann Lee"}));
    assert_eq!((names(&h, p0), names(&h, p1), names(&h, p2)), (vec![n("Ann Lee"), None], vec![n("Ann Lee"), None], vec![n("Ann Lee")]));
    assert_eq!(h.app.session.undo.len(), undo + 1, "one step for all of them");
}

#[test]
fn typing_in_a_selected_tiles_box_names_every_selected_face_in_one_undo_step() {
    let (mut h, [p0, p1, p2]) = people();
    for (id, i) in [(p1, 0), (p2, 0), (p0, 1)] {
        ok(&mut h, "ui.clickWidget", json!({"id": picture(id, i)}));
    }
    assert_eq!(selected(&h), 3);
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p2, 0)}));
    ok(&mut h, "ui.text", json!({"text": "Zed Q"}));
    key(&mut h, "enter");
    assert_eq!((names(&h, p0), names(&h, p1), names(&h, p2)), (vec![n("Ann Lee"), n("Zed Q")], vec![n("Zed Q"), None], vec![n("Zed Q")]));
    assert_eq!(h.app.session.undo.len(), undo + 1, "one step for all of them");
    ok(&mut h, "engine.execute", json!({"command": "edit.undo"}));
    assert_eq!((names(&h, p1), names(&h, p2)), (vec![None, None], vec![None]));
}

#[test]
fn enter_on_an_untouched_box_gives_each_selected_face_its_own_suggestion_in_one_step() {
    let (mut h, [p0, p1, p2]) = people();
    suggest(&mut h, &[(p1, 0, "Bob Ray"), (p2, 0, "Cy Dee"), (p0, 1, "Ann Lee")]);
    // (p1,1) has no suggestion; (p0,1) has one but is not selected
    for (id, i) in [(p1, 0), (p2, 0), (p1, 1)] {
        ok(&mut h, "ui.clickWidget", json!({"id": picture(id, i)}));
    }
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p1, 0)}));
    assert_eq!(shown(&h), "Bob Ray");
    key(&mut h, "enter");
    assert_eq!(names(&h, p1), [n("Bob Ray"), None], "a selected face without a suggestion is left alone");
    assert_eq!(names(&h, p2), [n("Cy Dee")], "each face gets its own suggestion");
    assert_eq!(names(&h, p0), [n("Ann Lee"), None], "a face outside the selection is not touched");
    assert_eq!(h.app.session.undo.len(), undo + 1);
}

#[test]
fn a_box_on_an_unselected_tile_names_only_that_face_and_keeps_the_selection() {
    let (mut h, [p0, p1, p2]) = people();
    for (id, i) in [(p1, 0), (p2, 0)] {
        ok(&mut h, "ui.clickWidget", json!({"id": picture(id, i)}));
    }
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p0, 1)}));
    ok(&mut h, "ui.text", json!({"text": "Solo"}));
    key(&mut h, "enter");
    assert_eq!((names(&h, p0), names(&h, p1), names(&h, p2)), (vec![n("Ann Lee"), n("Solo")], vec![None, None], vec![None]));
    assert_eq!(selected(&h), 2, "the selection is as it was");
    assert_eq!(h.app.session.undo.len(), undo + 1);
}

#[test]
fn tab_and_shift_tab_move_the_box_between_tiles_without_confirming_a_suggestion() {
    let (mut h, [p0, p1, p2]) = people();
    suggest(&mut h, &[(p0, 1, "Ann Lee"), (p1, 0, "Bob Ray"), (p1, 1, "Cy Dee"), (p2, 0, "Di Fay")]);
    let order: Vec<(u64, usize)> = h.app.caches.unnamed.as_ref().unwrap().3.faces.iter().map(|f| (f.photo, f.index)).collect();
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": format!("unnamed-name:{}:{}", order[1].0, order[1].1)}));
    let at = |h: &Headless| h.app.ui.name_edit.as_ref().map(|e| (e.photo, e.index));
    assert_eq!(at(&h), Some(order[1]));
    key(&mut h, "tab");
    assert_eq!(at(&h), Some(order[2]), "Tab goes to the next tile");
    ok(&mut h, "ui.key", json!({"key": "tab", "shift": true}));
    assert_eq!(at(&h), Some(order[1]), "Shift-Tab goes back");
    ok(&mut h, "ui.key", json!({"key": "tab", "shift": true}));
    assert_eq!(at(&h), Some(order[0]));
    assert_eq!(h.app.session.undo.len(), undo, "nothing was confirmed on the way");
    assert!(h.app.session.catalog.people().iter().all(|p| p.name == "Ann Lee"));
    // typed text is saved on the way
    ok(&mut h, "ui.text", json!({"text": "Typed"}));
    key(&mut h, "tab");
    assert_eq!(at(&h), Some(order[1]));
    assert_eq!(h.app.session.undo.len(), undo + 1);
}

#[test]
fn escape_closes_the_box_and_only_then_clears_the_selection() {
    let (mut h, [_, p1, p2]) = people();
    ok(&mut h, "ui.clickWidget", json!({"id": picture(p1, 0)}));
    ok(&mut h, "ui.clickWidget", json!({"id": picture(p2, 0)}));
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p1, 0)}));
    key(&mut h, "escape");
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(selected(&h), 2, "the first Esc only closed the box");
    key(&mut h, "escape");
    assert_eq!(selected(&h), 0, "with no box open Esc clears the selection");
}

#[test]
fn select_all_selects_every_unnamed_face_and_a_box_on_one_names_them_all() {
    let (mut h, [p0, p1, p2]) = people();
    ok(&mut h, "ui.clickWidget", json!({"id": "unnamed:selectAll"}));
    assert_eq!(selected(&h), 4);
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p0, 1)}));
    ok(&mut h, "ui.text", json!({"text": "Everyone"}));
    key(&mut h, "enter");
    assert_eq!((names(&h, p1), names(&h, p2)), (vec![n("Everyone"), n("Everyone")], vec![n("Everyone")]));
}

#[test]
fn only_one_name_box_is_open_in_the_people_view() {
    let (mut h, [_, p1, _]) = people();
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p1, 0)}));
    assert!(h.app.ui.name_edit.is_some() && h.app.ui.card_name.is_none());
    ok(&mut h, "ui.clickWidget", json!({"id": "personName:Ann Lee"}));
    assert!(h.app.ui.card_name.is_some() && h.app.ui.name_edit.is_none(), "the card's box replaced the tile's");
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p1, 1)}));
    assert!(h.app.ui.name_edit.is_some() && h.app.ui.card_name.is_none(), "and the tile's replaced the card's");
}

/// The faces in the order the view shows them.
fn order(h: &Headless) -> Vec<(u64, usize)> {
    h.app.caches.unnamed.as_ref().unwrap().3.faces.iter().map(|f| (f.photo, f.index)).collect()
}

fn name_of(h: &Headless, face: (u64, usize)) -> Option<String> {
    names(h, PhotoId(face.0)).get(face.1).cloned().flatten()
}

fn button(kind: &str, face: (u64, usize)) -> String {
    format!("unnamed-face-{kind}:{}:{}", face.0, face.1)
}

/// Suggest `Name<i>` for every face of the list whose position is not in `except`.
fn suggest_all_but(h: &mut Headless, except: &[usize]) -> Vec<(u64, usize)> {
    let faces = order(h);
    let who: Vec<(PhotoId, usize, String)> =
        faces.iter().enumerate().filter(|(i, _)| !except.contains(i)).map(|(i, f)| (PhotoId(f.0), f.1, format!("Name{i}"))).collect();
    let who: Vec<(PhotoId, usize, &str)> = who.iter().map(|(p, i, s)| (*p, *i, s.as_str())).collect();
    suggest(h, &who);
    faces
}

#[test]
fn confirm_and_all_above_are_on_suggested_unnamed_tiles_only_and_reject_on_every_one() {
    let (mut h, _) = people();
    let faces = suggest_all_but(&mut h, &[1]);
    for (i, f) in faces.iter().enumerate() {
        for kind in ["confirm", "above"] {
            assert_eq!(widgets(&mut h, &button(kind, *f)).len(), usize::from(i != 1), "{kind} on tile {i}");
        }
        assert_eq!(widgets(&mut h, &button("hide", *f)).len(), 1, "reject on every tile, {i}");
    }
}

#[test]
fn confirm_names_the_face_with_its_suggestion_in_one_undo_step_and_keeps_the_selection() {
    let (mut h, _) = people();
    let faces = suggest_all_but(&mut h, &[]);
    ok(&mut h, "ui.clickWidget", json!({"id": format!("unnamed-face:{}:{}", faces[3].0, faces[3].1)}));
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": button("confirm", faces[1])}));
    assert_eq!(name_of(&h, faces[1]).as_deref(), Some("Name1"));
    assert_eq!(faces.iter().filter(|f| name_of(&h, **f).is_some()).count(), 1, "only that face");
    assert_eq!(h.app.session.undo.len(), undo + 1);
    assert_eq!(name_of(&h, faces[3]), None);
    assert_eq!(selected(&h), 1, "an unselected tile's button leaves the selection as it is");
    ok(&mut h, "engine.execute", json!({"command": "edit.undo"}));
    assert_eq!(name_of(&h, faces[1]), None);
}

#[test]
fn confirm_on_a_selected_tile_confirms_every_selected_face_with_a_suggestion_in_one_step() {
    let (mut h, _) = people();
    let faces = suggest_all_but(&mut h, &[2]);
    for i in [0, 2, 3] {
        ok(&mut h, "ui.clickWidget", json!({"id": format!("unnamed-face:{}:{}", faces[i].0, faces[i].1)}));
    }
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": button("confirm", faces[3])}));
    let got: Vec<Option<String>> = faces.iter().map(|f| name_of(&h, *f)).collect();
    assert_eq!(
        got,
        [Some("Name0".to_string()), None, None, Some("Name3".to_string())],
        "each selected face got its own suggestion; the face without one stays"
    );
    assert_eq!(h.app.session.undo.len(), undo + 1);
}

#[test]
fn all_above_names_every_suggested_face_from_the_first_tile_through_this_one() {
    let (mut h, _) = people();
    let faces = suggest_all_but(&mut h, &[1]);
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": button("above", faces[2])}));
    let got: Vec<Option<String>> = faces.iter().map(|f| name_of(&h, *f)).collect();
    assert_eq!(got.iter().filter(|n| n.is_some()).count(), 2);
    assert_eq!(name_of(&h, faces[0]).as_deref(), Some("Name0"));
    assert_eq!(name_of(&h, faces[1]), None, "no suggestion: skipped");
    assert_eq!(name_of(&h, faces[2]).as_deref(), Some("Name2"), "this tile's own suggestion");
    assert_eq!(name_of(&h, faces[3]), None, "below this tile: untouched");
    assert_eq!(h.app.session.undo.len(), undo + 1);
    ok(&mut h, "engine.execute", json!({"command": "edit.undo"}));
    assert_eq!(name_of(&h, faces[0]), None);
    assert_eq!(name_of(&h, faces[2]), None);
}
