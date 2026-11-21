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
    head.stack = members.iter().map(|(p, i)| StackFace { photo: p.0, index: *i, rect: at(*p, *i).rect }).collect();
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
    h.app.ui.face_gallery = Some(crate::panels::face_tile::Gallery::Unnamed);
    h.app.ui.unnamed_selected = [(p1.0, 0)].into_iter().collect();
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
