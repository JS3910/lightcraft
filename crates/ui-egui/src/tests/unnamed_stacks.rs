//! Stacks in the Unnamed faces grid: a tile with a count for faces that are virtually certain to be the same person, driven
//! through the control channel. The list is injected the way `unnamed_tiles` injects suggestions (no recognition runs here;
//! the engine's side is tested in `face_arrange_bg`).

use std::sync::Arc;

use lightcraft_catalog::PhotoId;
use serde_json::json;

const T: std::time::Duration = std::time::Duration::from_secs(20);

fn ok(h: &mut Headless, method: &str, params: serde_json::Value) {
    let r = h.request(method, params.clone(), T);
    assert_eq!(r["ok"], true, "{method} {params}: {r}");
    h.step();
    h.step();
}

fn done(h: &mut Headless) {
    h.settle(T);
    h.step();
}

use super::unnamed_tiles::{people, picture, strip};
use crate::headless::Headless;
use crate::panels::unnamed::{StackFace, Unnamed};

/// Make the faces `members` (photo, index) one stack in the list on screen, as if the engine had stacked them: the first is the
/// representative. The faces leave the list as tiles of their own.
fn stack_up(h: &mut Headless, members: &[(PhotoId, usize)]) {
    let list = (*crate::panels::unnamed::faces(&mut h.app, 0.0)).clone();
    let at = |p: PhotoId, i: usize| list.faces.iter().find(|f| f.photo == p.0 && f.index == i).cloned().expect("a face of the list");
    let first = at(members[0].0, members[0].1);
    let mut head = first.clone();
    head.stack = members.iter().map(|(p, i)| StackFace { photo: p.0, index: *i, rect: at(*p, *i).rect, view: at(*p, *i).view }).collect();
    let mut faces: Vec<_> = list.faces.iter().filter(|f| !members.iter().any(|(p, i)| p.0 == f.photo && *i == f.index)).cloned().collect();
    faces.insert(0, head);
    let list = Unnamed { faces, ..list };
    let (rev, indexed) = (h.app.session.catalog.revision, h.app.caches.faces_indexed);
    h.app.caches.unnamed = Some((rev, indexed, f64::MAX, Arc::new(list)));
    h.step();
    h.step();
}

fn widgets(h: &mut Headless, filter: &str) -> Vec<String> {
    let r = h.request("ui.widgets", json!({"filter": filter}), std::time::Duration::from_secs(20));
    r["result"].as_array().into_iter().flatten().filter_map(|w| w["id"].as_str().map(str::to_string)).collect()
}

fn names(h: &Headless, id: PhotoId) -> Vec<Option<String>> {
    h.app.session.catalog.photo(id).unwrap().meta.regions.iter().map(|r| r.name.clone()).collect()
}

fn count_id(id: PhotoId, i: usize) -> String {
    format!("unnamed-stack-count:{}:{i}", id.0)
}

#[test]
fn a_stack_is_one_tile_with_a_count_and_its_faces_have_no_tiles_of_their_own() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    let tiles = widgets(&mut h, "unnamed-face:");
    assert_eq!(tiles.len(), 2, "the stack and the one face left: {tiles:?}");
    assert!(tiles.contains(&picture(p1, 0)) && tiles.contains(&picture(p0, 1)), "{tiles:?}");
    assert_eq!(widgets(&mut h, "unnamed-stack-count:"), [count_id(p1, 0)], "the count badge is on the stack only");
    // every id on the screen is there once (a duplicate would be detected, not collapsed)
    let all = widgets(&mut h, "unnamed");
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(all.len(), unique.len(), "{all:?}");
    assert_eq!(h.app.caches.unnamed.as_ref().unwrap().3.face_count(), 4, "the heading counts the faces, not the tiles");
}

#[test]
fn a_stack_has_no_cross_and_no_tick() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    for kind in ["hide", "confirm", "above"] {
        assert!(widgets(&mut h, &format!("unnamed-face-{kind}:{}:0", p1.0)).is_empty(), "{kind} on the stack");
        assert_eq!(widgets(&mut h, &format!("unnamed-face-{kind}:{}:1", p0.0)).len(), usize::from(kind == "hide"), "{kind} on the single face");
    }
}

#[test]
fn typing_a_name_on_a_stack_names_every_face_in_it_in_one_undo_step() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p1, 0)}));
    ok(&mut h, "ui.text", json!({"text": "Zed Q"}));
    ok(&mut h, "ui.key", json!({"key": "enter"}));
    let some = |s: &str| Some(s.to_string());
    assert_eq!((names(&h, p1), names(&h, p2)), (vec![some("Zed Q"), some("Zed Q")], vec![some("Zed Q")]));
    assert_eq!(names(&h, p0), [some("Ann Lee"), None], "a face outside the stack is left alone");
    assert_eq!(h.app.session.undo.len(), undo + 1, "one step");
    done(&mut h);
    assert!(widgets(&mut h, "unnamed-stack-count:").is_empty(), "the stack has left the grid");
    ok(&mut h, "engine.execute", json!({"command": "edit.undo"}));
    assert_eq!((names(&h, p1), names(&h, p2)), (vec![None, None], vec![None]), "one undo puts every face back");
}

#[test]
fn the_delete_key_never_removes_the_boxes_of_a_stack() {
    let (mut h, [_, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    // what a selection holding the stack's representative would act on
    h.app.ui.face_gallery = Some(crate::panels::face_tile::Gallery::Unnamed);
    h.app.ui.unnamed_selected = [(p1.0, 0)].into_iter().collect();
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.key", json!({"key": "delete"}));
    assert_eq!(h.app.session.catalog.photo(p1).unwrap().meta.regions.len(), 2);
    assert_eq!(h.app.session.catalog.photo(p2).unwrap().meta.regions.len(), 1);
    assert_eq!(h.app.session.undo.len(), undo, "nothing was done");
}

#[test]
fn the_subtitle_says_what_a_stack_is_only_when_there_is_one() {
    let (mut h, [_, p1, p2]) = people();
    let before = crate::panels::unnamed::subtitle(&h.app.caches.unnamed.as_ref().unwrap().3, 0);
    assert!(!before.contains("stack"), "{before}");
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    let after = crate::panels::unnamed::subtitle(&h.app.caches.unnamed.as_ref().unwrap().3, 0);
    assert!(after.contains("stack of faces") && after.contains("naming it names them all"), "{after}");
}

/// The widgets matching `filter`, with their rectangles (x, y, w, h).
fn rects(h: &mut Headless, filter: &str) -> Vec<(String, [f64; 4])> {
    let r = h.request("ui.widgets", json!({"filter": filter}), T);
    r["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let a = w["rect"].as_array()?;
            Some((w["id"].as_str()?.to_string(), [a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?, a[3].as_f64()?]))
        })
        .collect()
}

fn rect_of(h: &mut Headless, id: &str) -> [f64; 4] {
    rects(h, id).into_iter().find(|(i, _)| i == id).map(|(_, r)| r).unwrap_or_else(|| panic!("{id} is not on screen"))
}

/// Every face tile of the grid, the faces of an opened stack included, in reading order.
fn reading(h: &mut Headless) -> Vec<String> {
    let mut v = rects(h, "unnamed-face:");
    v.extend(rects(h, "unnamed-member-face:"));
    v.sort_by(|a, b| (a.1[1], a.1[0]).partial_cmp(&(b.1[1], b.1[0])).unwrap_or(std::cmp::Ordering::Equal));
    v.into_iter().map(|(id, _)| id).collect()
}

fn member(id: PhotoId, i: usize) -> String {
    format!("unnamed-member-face:{}:{i}", id.0)
}

#[test]
fn clicking_the_count_opens_the_stack_after_its_tile_on_a_tinted_background_and_clicking_it_again_closes_it() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    let (stack_before, single_before) = (rect_of(&mut h, &picture(p1, 0)), rect_of(&mut h, &picture(p0, 1)));
    assert!(widgets(&mut h, "unnamed-member-face:").is_empty() && widgets(&mut h, "unnamed-tint").is_empty(), "closed: no faces, no tint");
    ok(&mut h, "ui.clickWidget", json!({"id": count_id(p1, 0)}));
    assert_eq!(
        reading(&mut h),
        [picture(p1, 0), member(p1, 1), member(p2, 0), picture(p0, 1)],
        "the stack's tile is its first face, the others follow it, then the rest of the grid: three tiles for three faces"
    );
    assert_eq!(widgets(&mut h, "unnamed-tint").len(), 3, "each face sits on the tint, the stack's tile is one of them");
    assert_eq!(widgets(&mut h, "unnamed-stack-count:"), [count_id(p1, 0)], "the count stays on the first face, and closes the stack");
    assert_eq!(rect_of(&mut h, &picture(p1, 0)), stack_before, "the stack's own tile did not move");
    assert_ne!(rect_of(&mut h, &picture(p0, 1)), single_before, "the tile after it moved on, behind the faces");
    // every id on the screen is there once, the faces of the stack with ids of their own
    let all = widgets(&mut h, "unnamed");
    assert_eq!(all.len(), all.iter().collect::<std::collections::HashSet<_>>().len(), "{all:?}");
    ok(&mut h, "ui.clickWidget", json!({"id": count_id(p1, 0)}));
    assert_eq!(reading(&mut h), [picture(p1, 0), picture(p0, 1)]);
    assert!(widgets(&mut h, "unnamed-tint").is_empty());
    assert_eq!(rect_of(&mut h, &picture(p0, 1)), single_before, "closing puts the tiles back where they were");
    assert_eq!(rect_of(&mut h, &picture(p1, 0)), stack_before);
}
// ---- stacks with a suggested name ----

/// [`stack_up`] for faces that are all suggested `name`: the stack carries it.
fn stack_up_suggested(h: &mut Headless, members: &[(PhotoId, usize)], name: &str) {
    stack_up(h, members);
    let mut list = (*crate::panels::unnamed::faces(&mut h.app, 0.0)).clone();
    if let Some(head) = list.faces.iter_mut().find(|f| f.is_stack()) {
        head.suggestion = Some(name.to_string());
    }
    let (rev, indexed) = (h.app.session.catalog.revision, h.app.caches.faces_indexed);
    h.app.caches.unnamed = Some((rev, indexed, f64::MAX, Arc::new(list)));
    h.step();
    h.step();
}

fn rejected(h: &Headless, id: PhotoId) -> Vec<Vec<String>> {
    h.app.session.catalog.photo(id).unwrap().meta.regions.iter().map(|r| r.rejected.clone()).collect()
}

#[test]
fn a_stack_with_a_suggestion_says_it_with_a_question_mark_and_has_the_tick_the_arrow_and_the_cross() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Ann Lee");
    for kind in ["hide", "confirm", "above"] {
        assert_eq!(widgets(&mut h, &format!("unnamed-face-{kind}:{}:0", p1.0)).len(), 1, "{kind} on the stack");
        assert_eq!(widgets(&mut h, &format!("unnamed-face-{kind}:{}:1", p0.0)).len(), usize::from(kind == "hide"), "{kind} on the single face");
    }
    assert_eq!(widgets(&mut h, "unnamed-stack-count:"), [count_id(p1, 0)]);
    let list = h.app.caches.unnamed.as_ref().unwrap().3.clone();
    let shown = crate::panels::unnamed::entries(&list, &h.app.ui.rejected_unnamed);
    assert_eq!(shown[0].suggestion.as_deref(), Some("Ann Lee"), "the strip says it, with its question mark");
}

#[test]
fn the_tick_on_a_stack_names_every_face_of_it_as_the_suggestion_in_one_undo_step() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Ann Lee");
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": format!("unnamed-face-confirm:{}:0", p1.0)}));
    let ann = |s: &str| Some(s.to_string());
    assert_eq!((names(&h, p1), names(&h, p2)), (vec![ann("Ann Lee"), ann("Ann Lee")], vec![ann("Ann Lee")]));
    assert_eq!(names(&h, p0), [ann("Ann Lee"), None], "a face outside the stack is left alone");
    assert_eq!(h.app.session.undo.len(), undo + 1, "one step");
    done(&mut h);
    assert!(widgets(&mut h, "unnamed-stack-count:").is_empty(), "the stack has left the grid");
    ok(&mut h, "engine.execute", json!({"command": "edit.undo"}));
    assert_eq!((names(&h, p1), names(&h, p2)), (vec![None, None], vec![None]), "one undo puts every face back");
}

#[test]
fn the_cross_on_a_stack_rejects_the_suggestion_for_every_face_in_one_step_and_the_stack_stays_in_place_as_a_question_mark() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Ann Lee");
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": format!("unnamed-face-hide:{}:0", p1.0)}));
    let ann = vec!["Ann Lee".to_string()];
    assert_eq!(
        (rejected(&h, p1), rejected(&h, p2)),
        (vec![ann.clone(), ann.clone()], vec![ann.clone()]),
        "stored on each face, as a single rejection is"
    );
    assert_eq!(rejected(&h, p0)[1], Vec::<String>::new(), "a face outside the stack is left alone");
    assert_eq!(names(&h, p1), [None, None], "nothing is named");
    assert_eq!(h.app.session.undo.len(), undo + 1, "one step");
    done(&mut h);
    // still one tile with its count, at the same place, and its strip is a question mark
    assert_eq!(widgets(&mut h, "unnamed-stack-count:"), [count_id(p1, 0)], "the stack stays in place");
    let list = h.app.caches.unnamed.as_ref().unwrap().3.clone();
    assert!(list.faces[0].is_stack() && list.faces[0].count() == 3, "{:?}", list.faces);
    let shown = crate::panels::unnamed::entries(&list, &h.app.ui.rejected_unnamed);
    assert_eq!(shown[0].suggestion, None, "a question mark, not the name again");
    assert_eq!(widgets(&mut h, "unnamed-face-hide:").len(), 1, "no cross on it now: it has no suggestion (only the single face has one)");
    ok(&mut h, "engine.execute", json!({"command": "edit.undo"}));
    assert_eq!(rejected(&h, p1), vec![Vec::<String>::new(), Vec::new()], "one undo takes the rejection back from every face");
}

#[test]
fn the_delete_key_on_a_stack_with_a_suggestion_does_what_its_cross_does() {
    let (mut h, [_, p1, p2]) = people();
    stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Ann Lee");
    ok(&mut h, "ui.clickWidget", json!({"id": picture(p1, 0)}));
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.key", json!({"key": "delete"}));
    let ann = vec!["Ann Lee".to_string()];
    assert_eq!((rejected(&h, p1), rejected(&h, p2)), (vec![ann.clone(), ann.clone()], vec![ann]));
    assert_eq!(h.app.session.catalog.photo(p1).unwrap().meta.regions.len(), 2, "no box was removed");
    assert_eq!(h.app.session.undo.len(), undo + 1);
}

#[test]
fn the_arrow_on_a_tile_after_a_stack_names_every_face_of_the_stack_too() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Bob Ray");
    // the face after the stack is suggested a name of its own
    let mut list = (*crate::panels::unnamed::faces(&mut h.app, 0.0)).clone();
    for f in list.faces.iter_mut().filter(|f| !f.is_stack()) {
        f.suggestion = Some("Cy Dee".into());
    }
    let (rev, indexed) = (h.app.session.catalog.revision, h.app.caches.faces_indexed);
    h.app.caches.unnamed = Some((rev, indexed, f64::MAX, Arc::new(list)));
    h.step();
    h.step();
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": format!("unnamed-face-above:{}:1", p0.0)}));
    let some = |s: &str| Some(s.to_string());
    assert_eq!((names(&h, p1), names(&h, p2)), (vec![some("Bob Ray"), some("Bob Ray")], vec![some("Bob Ray")]));
    assert_eq!(names(&h, p0), [some("Ann Lee"), some("Cy Dee")]);
    assert_eq!(h.app.session.undo.len(), undo + 1, "one step");
}

fn selected(h: &Headless) -> Vec<(u64, usize)> {
    let mut s: Vec<_> = h.app.ui.unnamed_selected.iter().copied().collect();
    s.sort_unstable();
    s
}

#[test]
fn clicking_a_stack_selects_all_its_faces_and_clicking_it_again_deselects_them() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    ok(&mut h, "ui.clickWidget", json!({"id": picture(p1, 0)}));
    assert_eq!(selected(&h), [(p1.0, 0), (p1.0, 1), (p2.0, 0)]);
    // (another tile in between: two clicks in a row on one tile are a double click, which opens the photo)
    ok(&mut h, "ui.clickWidget", json!({"id": picture(p0, 1)}));
    ok(&mut h, "ui.clickWidget", json!({"id": picture(p1, 0)}));
    assert_eq!(selected(&h), [(p0.0, 1)], "the stack was deselected as a whole");
}

#[test]
fn shift_clicking_selects_the_faces_of_every_stack_in_the_range() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    // the list is the stack, then (p0, 1): a click on the last tile, then a shift-click on the stack
    ok(&mut h, "ui.clickWidget", json!({"id": picture(p0, 1)}));
    let list = crate::panels::unnamed::faces(&mut h.app, 0.0);
    let order = crate::panels::unnamed::entries(&list, &h.app.ui.rejected_unnamed);
    crate::panels::face_tile::click(&mut h.app, &crate::panels::face_tile::Gallery::Unnamed, &order, 0, true);
    assert_eq!(selected(&h), [(p0.0, 1), (p1.0, 0), (p1.0, 1), (p2.0, 0)]);
}

#[test]
fn select_all_includes_the_stacks_and_select_none_clears_them() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    ok(&mut h, "ui.clickWidget", json!({"id": "unnamed:selectAll"}));
    assert_eq!(selected(&h), [(p0.0, 1), (p1.0, 0), (p1.0, 1), (p2.0, 0)]);
    ok(&mut h, "ui.clickWidget", json!({"id": "unnamed:selectAll"}));
    assert!(selected(&h).is_empty(), "the button said Select none");
}

#[test]
fn a_name_typed_on_a_face_selected_with_a_stack_names_the_stack_too_in_one_step() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    ok(&mut h, "ui.clickWidget", json!({"id": "unnamed:selectAll"}));
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": strip(p0, 1)}));
    ok(&mut h, "ui.text", json!({"text": "Everyone"}));
    ok(&mut h, "ui.key", json!({"key": "enter"}));
    let some = |s: &str| Some(s.to_string());
    assert_eq!((names(&h, p1), names(&h, p2)), (vec![some("Everyone"), some("Everyone")], vec![some("Everyone")]));
    assert_eq!(names(&h, p0), [some("Ann Lee"), some("Everyone")]);
    assert_eq!(h.app.session.undo.len(), undo + 1, "one step");
}

#[test]
fn the_delete_key_on_a_selection_with_a_stack_without_a_suggestion_removes_only_the_single_faces() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    ok(&mut h, "ui.clickWidget", json!({"id": "unnamed:selectAll"}));
    ok(&mut h, "ui.key", json!({"key": "delete"}));
    assert_eq!(h.app.session.catalog.photo(p0).unwrap().meta.regions.len(), 1, "the single face's box is gone");
    assert_eq!((h.app.session.catalog.photo(p1).unwrap().meta.regions.len(), h.app.session.catalog.photo(p2).unwrap().meta.regions.len()), (2, 1));
}

#[test]
fn the_arrow_skips_a_stack_with_no_suggestion_and_the_faces_it_names_are_counted_not_the_tiles() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    let mut list = (*crate::panels::unnamed::faces(&mut h.app, 0.0)).clone();
    for f in list.faces.iter_mut().filter(|f| !f.is_stack()) {
        f.suggestion = Some("Cy Dee".into());
    }
    assert_eq!(crate::panels::unnamed::suggested_through(&list, 1, &h.app.ui.rejected_unnamed), 1, "the stack is a '?': skipped");
    let (rev, indexed) = (h.app.session.catalog.revision, h.app.caches.faces_indexed);
    h.app.caches.unnamed = Some((rev, indexed, f64::MAX, Arc::new(list)));
    h.step();
    h.step();
    ok(&mut h, "ui.clickWidget", json!({"id": format!("unnamed-face-above:{}:1", p0.0)}));
    assert_eq!((names(&h, p1), names(&h, p2)), (vec![None, None], vec![None]));
    assert_eq!(names(&h, p0)[1], Some("Cy Dee".to_string()));
}

#[test]
fn the_arrows_count_is_faces_not_tiles() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Bob Ray");
    let mut list = (*crate::panels::unnamed::faces(&mut h.app, 0.0)).clone();
    for f in list.faces.iter_mut().filter(|f| !f.is_stack()) {
        f.suggestion = Some("Cy Dee".into());
    }
    let rejected = &h.app.ui.rejected_unnamed;
    assert_eq!(crate::panels::unnamed::suggested_through(&list, 0, rejected), 3, "the stack alone is three faces");
    assert_eq!(crate::panels::unnamed::suggested_through(&list, 1, rejected), 4);
    let _ = p0;
}

#[test]
fn clicking_a_face_of_an_open_stack_selects_the_whole_stack_as_clicking_the_stack_does() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    ok(&mut h, "ui.clickWidget", json!({"id": count_id(p1, 0)}));
    ok(&mut h, "ui.clickWidget", json!({"id": member(p1, 1)}));
    assert_eq!(selected(&h), [(p1.0, 0), (p1.0, 1), (p2.0, 0)], "one face of the stack is all of it");
    // (another tile in between: two clicks in a row on one tile are a double click, which opens the photo)
    ok(&mut h, "ui.clickWidget", json!({"id": picture(p0, 1)}));
    ok(&mut h, "ui.clickWidget", json!({"id": member(p2, 0)}));
    assert_eq!(selected(&h), [(p0.0, 1)], "and clicking it again deselects all of it");
}

// ---- the tiles on screen keep their places while a stack forms, grows and is named (the real engine) ----

/// The face tiles of the grid in reading order, with their rectangles.
fn grid(h: &mut Headless) -> Vec<(String, [f64; 4])> {
    let mut v = rects(h, "unnamed-face:");
    v.sort_by(|a, b| (a.1[1], a.1[0]).partial_cmp(&(b.1[1], b.1[0])).unwrap_or(std::cmp::Ordering::Equal));
    v
}

/// Ask the engine again (as the window does when the scan found something) and draw frames until the list is final and `done`.
fn look_again(h: &mut Headless, what: &str, done: impl Fn(&Unnamed) -> bool) {
    h.app.caches.unnamed_restart = true;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        h.step();
        let list = h.app.caches.unnamed.as_ref().map(|c| c.3.clone());
        if list.as_ref().is_some_and(|l| l.complete && !l.ordering && done(l)) {
            break;
        }
        assert!(std::time::Instant::now() < until, "{what}: the list never got there: {list:?}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.step();
    h.step();
}

fn stack_len(l: &Unnamed) -> usize {
    l.faces.iter().map(|f| f.stack.len()).max().unwrap_or(0)
}

/// `ids` with the tiles of `gone` left out, and `at` (when it is in `ids`) called "STACK".
fn without(ids: &[String], gone: &[String], at: &str) -> Vec<String> {
    ids.iter().filter(|i| !gone.contains(i) || *i == at).map(|i| if i == at { "STACK".to_string() } else { i.clone() }).collect()
}

#[test]
fn the_tiles_on_screen_keep_their_places_when_a_stack_forms_grows_and_is_named() {
    use super::unnamed_ordering::{recognising, unit};
    use super::unnamed_tiles::{region, set_regions};
    let (mut h, [p0, p1, p2], _hold, dir) = recognising("places");
    // nine unnamed faces that are alike nobody (and not each other), one that looks like Ann Lee
    set_regions(&mut h, p0, vec![region(0.1, Some("Ann Lee")), region(0.5, None)]);
    set_regions(&mut h, p1, (1..=5).map(|i| region(0.1 * i as f64, None)).collect());
    set_regions(&mut h, p2, (1..=3).map(|i| region(0.1 * i as f64, None)).collect());
    let put = |h: &mut Headless, id: PhotoId, x: f64, v: Vec<f32>| h.app.session.test_face_embedding(id, &region(x, None).rect, v);
    for (n, p) in [p1, p2].into_iter().enumerate() {
        // (dimensions below 16: the stamp of a face, which tells the run it changed, is made of the first 16 numbers)
        for i in 1..=(5 - 2 * n) {
            put(&mut h, p, 0.1 * i as f64, unit(&[(4 + 5 * n + i, 1.0)]));
        }
    }
    ok(&mut h, "ui.set", json!({"view": "people"}));
    look_again(&mut h, "first", |l| l.face_count() == 9 && stack_len(l) == 0);
    let before = grid(&mut h);
    assert_eq!(before.len(), 9, "every face is on screen: {before:?}");
    let ids = |g: &[(String, [f64; 4])]| g.iter().map(|(i, _)| i.clone()).collect::<Vec<_>>();
    let tile = |p: PhotoId, i: usize| picture(p, i);

    // --- a stack forms: three faces of two photos become alike
    let members = [(p1, 1usize), (p1, 3), (p2, 1)];
    put(&mut h, p1, 0.2, unit(&[(13, 1.0), (14, 0.1)]));
    put(&mut h, p1, 0.4, unit(&[(13, 1.0), (14, -0.1)]));
    put(&mut h, p2, 0.2, unit(&[(13, 1.0)]));
    look_again(&mut h, "forms", |l| stack_len(l) == 3);
    let member_tiles: Vec<String> = members.iter().map(|(p, i)| tile(*p, *i)).collect();
    let first = ids(&before).into_iter().find(|i| member_tiles.contains(i)).expect("a member was on screen");
    let formed = grid(&mut h);
    let head = widgets(&mut h, "unnamed-stack-count:").pop().expect("a count badge").replace("unnamed-stack-count:", "unnamed-face:");
    assert_eq!(
        formed.iter().map(|(i, _)| if *i == head { "STACK".to_string() } else { i.clone() }).collect::<Vec<_>>(),
        without(&ids(&before), &member_tiles, &first),
        "one tile where the first of them was, every other tile in its order"
    );
    let at = ids(&before).iter().position(|i| *i == first).unwrap();
    assert_eq!(formed[..at], before[..at], "the tiles before it did not move");

    // --- it grows: another face, the last on screen that is not in it, becomes one of them
    let last = ids(&formed).into_iter().rev().find(|i| *i != head).expect("a tile after the stack");
    let (photo, index) = {
        let mut parts = last.rsplit(':');
        let index: usize = parts.next().unwrap().parse().unwrap();
        (PhotoId(parts.next().unwrap().parse().unwrap()), index)
    };
    let x = h.app.session.catalog.photo(photo).unwrap().meta.regions[index].rect.x0;
    put(&mut h, photo, x, unit(&[(13, 1.0), (14, 0.05)]));
    look_again(&mut h, "grows", |l| stack_len(l) == 4);
    let grown = grid(&mut h);
    let head = widgets(&mut h, "unnamed-stack-count:").pop().expect("a count badge").replace("unnamed-stack-count:", "unnamed-face:");
    assert_eq!(
        grown.iter().map(|(i, _)| if *i == head { "STACK".to_string() } else { i.clone() }).collect::<Vec<_>>(),
        without(
            &ids(&formed).iter().map(|i| if *i == head { "STACK".into() } else { i.clone() }).collect::<Vec<_>>(),
            std::slice::from_ref(&last),
            "STACK"
        ),
        "the stack kept its place and the tile of the face that joined it is gone"
    );
    assert_eq!(grown[..at], formed[..at], "the tiles before it did not move");

    // --- it is named: the stack leaves and the tiles after it move up one place
    let stack_at = ids(&grown).iter().position(|i| *i == head).unwrap();
    ok(&mut h, "ui.clickWidget", json!({"id": head.replace("unnamed-face:", "unnamed-name:")}));
    ok(&mut h, "ui.text", json!({"text": "Zed Q"}));
    ok(&mut h, "ui.key", json!({"key": "enter"}));
    done(&mut h);
    let named = grid(&mut h);
    let mut expected = ids(&grown);
    expected.remove(stack_at);
    assert_eq!(ids(&named), expected, "every other tile kept its order");
    assert_eq!(named[..stack_at], grown[..stack_at], "the tiles before it did not move");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- an open stack's faces are single faces ----

fn painted_count(h: &Headless, text: &str) -> usize {
    h.painted_text().iter().filter(|t| t.as_str() == text).count()
}

fn open_the_stack(h: &mut Headless, head: (PhotoId, usize)) {
    ok(h, "ui.clickWidget", json!({"id": count_id(head.0, head.1)}));
}

/// The id of the part `kind` (`face`, `name`, `face-hide`, `face-confirm`, `face-above`) of the face `(id, i)` of the open stack the
/// faces of which are `(p1, 0)` (its first, the stack's own tile), `(p1, 1)` and `(p2, 0)`.
fn part(kind: &str, id: PhotoId, i: usize, first: bool) -> String {
    format!("unnamed{}-{kind}:{}:{i}", if first { "" } else { "-member" }, id.0)
}

#[test]
fn every_face_of_an_open_stack_has_the_cross_and_a_suggested_one_the_tick_and_the_arrow_as_a_single_face_has() {
    let (mut h, [_, p1, p2]) = people();
    stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Ann Lee");
    open_the_stack(&mut h, (p1, 0));
    for (id, i, first) in [(p1, 0, true), (p1, 1, false), (p2, 0, false)] {
        for kind in ["face-hide", "face-confirm", "face-above"] {
            assert_eq!(widgets(&mut h, &part(kind, id, i, first)).len(), 1, "{kind} on face {id:?} {i}");
        }
    }
    // a stack without a suggestion: the cross only, as on a single face with none
    let (mut h, [_, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    open_the_stack(&mut h, (p1, 0));
    for (id, i, first) in [(p1, 0, true), (p1, 1, false), (p2, 0, false)] {
        assert_eq!(widgets(&mut h, &part("face-hide", id, i, first)).len(), 1, "the cross on face {id:?} {i}");
        for kind in ["face-confirm", "face-above"] {
            assert!(widgets(&mut h, &part(kind, id, i, first)).is_empty(), "{kind} on face {id:?} {i}");
        }
    }
    let all = widgets(&mut h, "unnamed");
    assert_eq!(all.len(), all.iter().collect::<std::collections::HashSet<_>>().len(), "every id once: {all:?}");
}

#[test]
fn each_face_of_an_open_stack_shows_the_stacks_suggestion_and_a_question_mark_once_it_was_rejected() {
    let (mut h, [_, p1, p2]) = people();
    stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Ann Lee");
    assert_eq!(painted_count(&h, "Ann Lee?"), 1, "closed: the stack's tile says it");
    open_the_stack(&mut h, (p1, 0));
    assert_eq!(painted_count(&h, "Ann Lee?"), 3, "open: each face says it, the first one too: {:?}", h.painted_text());
    // reject it for the stack (its cross while it is closed), then open it again
    open_the_stack(&mut h, (p1, 0));
    ok(&mut h, "ui.clickWidget", json!({"id": part("face-hide", p1, 0, true)}));
    done(&mut h);
    open_the_stack(&mut h, (p1, 0));
    assert_eq!(painted_count(&h, "Ann Lee?"), 0, "rejected in this visit: {:?}", h.painted_text());
    assert_eq!(painted_count(&h, "?"), 4, "a question mark on each of the three faces and the single face: {:?}", h.painted_text());
}

#[test]
fn the_cross_on_a_face_of_an_open_stack_does_what_it_does_on_a_single_face() {
    // with a suggestion: it rejects the name for that face alone
    let (mut h, [_, p1, p2]) = people();
    stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Ann Lee");
    open_the_stack(&mut h, (p1, 0));
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": part("face-hide", p1, 1, false)}));
    let ann = vec!["Ann Lee".to_string()];
    assert_eq!((rejected(&h, p1), rejected(&h, p2)), (vec![Vec::new(), ann], vec![Vec::new()]), "that face only");
    assert_eq!(h.app.session.undo.len(), undo + 1);
    // the first face's cross too, and it does not take the others with it
    ok(&mut h, "ui.clickWidget", json!({"id": part("face-hide", p1, 0, true)}));
    assert_eq!(rejected(&h, p2), [Vec::<String>::new()], "the face of the other photo is untouched");
    assert_eq!(rejected(&h, p1)[0], ["Ann Lee"]);
    // without a suggestion: it removes that face's box, in one undo step
    let (mut h, [_, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    open_the_stack(&mut h, (p1, 0));
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": part("face-hide", p2, 0, false)}));
    assert_eq!((h.app.session.catalog.photo(p1).unwrap().meta.regions.len(), h.app.session.catalog.photo(p2).unwrap().meta.regions.len()), (2, 0));
    assert_eq!(h.app.session.undo.len(), undo + 1, "one step");
    // and the first face (the stack's own tile) the same
    ok(&mut h, "ui.clickWidget", json!({"id": part("face-hide", p1, 0, true)}));
    assert_eq!(h.app.session.catalog.photo(p1).unwrap().meta.regions.len(), 1, "only the first face's box went");
    assert_eq!(h.app.session.undo.len(), undo + 2);
}

#[test]
fn the_tick_on_a_face_of_an_open_stack_names_that_face_alone() {
    let some = |s: &str| Some(s.to_string());
    // (the list is injected, so each tick is on a stack of its own)
    for (id, first) in [(1usize, false), (0, true)] {
        let (mut h, [_, p1, p2]) = people();
        stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Ann Lee");
        open_the_stack(&mut h, (p1, 0));
        let undo = h.app.session.undo.len();
        ok(&mut h, "ui.clickWidget", json!({"id": part("face-confirm", p1, id, first)}));
        let want = if first { vec![some("Ann Lee"), None] } else { vec![None, some("Ann Lee")] };
        assert_eq!((names(&h, p1), names(&h, p2)), (want, vec![None]), "face {id} of the photo, alone");
        assert_eq!(h.app.session.undo.len(), undo + 1);
    }
}

#[test]
fn delete_on_a_selected_open_stack_removes_all_its_boxes_in_one_undo_step_and_on_a_closed_one_says_why_it_does_nothing() {
    let (mut h, [p0, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    // closed: nothing is removed, and the user is told why
    ok(&mut h, "ui.clickWidget", json!({"id": picture(p1, 0)}));
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.key", json!({"key": "delete"}));
    assert_eq!(h.app.session.undo.len(), undo, "nothing was done");
    let toast = h.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert!(toast.contains("open the stack"), "it says why: {toast:?}");
    // open: the faces are on screen, so Delete removes every one of them
    open_the_stack(&mut h, (p1, 0));
    assert_eq!(selected(&h), [(p1.0, 0), (p1.0, 1), (p2.0, 0)], "still selected");
    ok(&mut h, "ui.key", json!({"key": "delete"}));
    assert_eq!(h.app.session.undo.len(), undo + 1, "one step");
    assert_eq!(
        (
            h.app.session.catalog.photo(p0).unwrap().meta.regions.len(),
            h.app.session.catalog.photo(p1).unwrap().meta.regions.len(),
            h.app.session.catalog.photo(p2).unwrap().meta.regions.len()
        ),
        (2, 0, 0)
    );
    ok(&mut h, "engine.execute", json!({"command": "edit.undo"}));
    assert_eq!(
        (h.app.session.catalog.photo(p1).unwrap().meta.regions.len(), h.app.session.catalog.photo(p2).unwrap().meta.regions.len()),
        (2, 1),
        "one undo brings them all back"
    );
}

#[test]
fn delete_on_a_selected_open_stack_with_a_suggestion_rejects_it_for_every_face_as_for_single_faces() {
    let (mut h, [_, p1, p2]) = people();
    stack_up_suggested(&mut h, &[(p1, 0), (p1, 1), (p2, 0)], "Ann Lee");
    open_the_stack(&mut h, (p1, 0));
    ok(&mut h, "ui.clickWidget", json!({"id": part("face", p1, 1, false)}));
    ok(&mut h, "ui.key", json!({"key": "delete"}));
    let ann = vec!["Ann Lee".to_string()];
    assert_eq!((rejected(&h, p1), rejected(&h, p2)), (vec![ann.clone(), ann.clone()], vec![ann]));
    assert_eq!(h.app.session.catalog.photo(p1).unwrap().meta.regions.len(), 2, "no box was removed");
}

#[test]
fn a_name_typed_on_the_strip_of_a_face_of_an_open_stack_names_that_face_alone() {
    let (mut h, [_, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    open_the_stack(&mut h, (p1, 0));
    ok(&mut h, "ui.clickWidget", json!({"id": part("name", p2, 0, false)}));
    ok(&mut h, "ui.text", json!({"text": "Zed Q"}));
    ok(&mut h, "ui.key", json!({"key": "enter"}));
    let some = |s: &str| Some(s.to_string());
    assert_eq!((names(&h, p1), names(&h, p2)), (vec![None, None], vec![some("Zed Q")]));
}

#[test]
fn the_tooltip_of_the_subtitle_says_the_bar_the_stacks_were_made_at() {
    let (mut h, [_, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    // no run has finished: no bar to say, so no tooltip on the subtitle
    assert!(widgets(&mut h, "unnamed:subtitle").is_empty());
    let mut list = (*crate::panels::unnamed::faces(&mut h.app, 0.0)).clone();
    list.stack_bar = Some(0.83);
    let (rev, indexed) = (h.app.session.catalog.revision, h.app.caches.faces_indexed);
    h.app.caches.unnamed = Some((rev, indexed, f64::MAX, Arc::new(list)));
    h.step();
    h.step();
    assert_eq!(widgets(&mut h, "unnamed:subtitle").len(), 1);
    assert_eq!(h.request("ui.hoverWidget", json!({"id": "unnamed:subtitle"}), T)["ok"], true);
    assert!(
        h.step_until(T, |h| h.painted_text().iter().any(|t| t.contains("0.83") && t.contains("alike"))),
        "the tooltip names the bar: {:?}",
        h.painted_text()
    );
}
