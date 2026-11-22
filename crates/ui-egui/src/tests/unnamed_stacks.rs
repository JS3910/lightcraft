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
        [picture(p1, 0), member(p1, 0), member(p1, 1), member(p2, 0), picture(p0, 1)],
        "the faces follow the stack tile, then the rest of the grid"
    );
    assert_eq!(widgets(&mut h, "unnamed-tint").len(), 4, "the stack's tile and each of its faces sit on the tint");
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

#[test]
fn each_face_of_an_opened_stack_has_a_cross_and_the_stack_tile_still_has_none() {
    let (mut h, [_, p1, p2]) = people();
    stack_up(&mut h, &[(p1, 0), (p1, 1), (p2, 0)]);
    assert!(widgets(&mut h, "unnamed-member-face-hide:").is_empty(), "nothing to take out of a closed stack");
    ok(&mut h, "ui.clickWidget", json!({"id": count_id(p1, 0)}));
    let mut crosses = widgets(&mut h, "unnamed-member-face-hide:");
    crosses.sort();
    assert_eq!(crosses.len(), 3, "{crosses:?}");
    assert!(widgets(&mut h, &format!("unnamed-face-hide:{}:0", p1.0)).is_empty(), "the stack has no cross");
    assert!(widgets(&mut h, "unnamed-member-face-confirm:").is_empty() && widgets(&mut h, "unnamed-member-face-above:").is_empty());
}

/// [`recognising`](super::unnamed_ordering::recognising) with the three faces of photos 1 and 2 as alike as one person's: a
/// stack of three once the run is done.
fn a_stack_of_three() -> (Headless, [PhotoId; 3], std::path::PathBuf) {
    use super::unnamed_ordering::{recognising, unit};
    use super::unnamed_tiles::region;
    let (mut h, ids, _hold, dir) = recognising("take-out");
    let put = |h: &mut Headless, id: PhotoId, x: f64, v: Vec<f32>| h.app.session.test_face_embedding(id, &region(x, None).rect, v);
    put(&mut h, ids[1], 0.3, unit(&[(10, 1.0), (11, 0.1)]));
    put(&mut h, ids[1], 0.6, unit(&[(10, 1.0), (11, -0.1)]));
    put(&mut h, ids[2], 0.3, unit(&[(10, 1.0)]));
    ok(&mut h, "ui.set", json!({"view": "people"}));
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < until && !stack_in_list(&h) {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(stack_in_list(&h), "the run found the stack");
    (h, ids, dir)
}

fn stack_in_list(h: &Headless) -> bool {
    h.app.caches.unnamed.as_ref().is_some_and(|(_, _, _, l)| l.faces.iter().any(|f| f.stack.len() == 3))
}

#[test]
fn the_cross_on_a_face_of_an_opened_stack_takes_it_out_to_a_tile_of_its_own_and_the_others_stay_a_stack() {
    let (mut h, ids, dir) = a_stack_of_three();
    let list = h.app.caches.unnamed.as_ref().unwrap().3.clone();
    let stack = list.faces.iter().find(|f| f.stack.len() == 3).unwrap().clone();
    let tiles_before: Vec<String> = reading(&mut h);
    let before = tiles_before.iter().position(|t| *t == picture(PhotoId(stack.photo), stack.index)).unwrap();
    let still_before: Vec<(String, [f64; 4])> = rects(&mut h, "unnamed-face:").into_iter().take(before).collect();
    ok(&mut h, "ui.clickWidget", json!({"id": count_id(PhotoId(stack.photo), stack.index)}));
    let out = stack.stack.last().unwrap().clone();
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": format!("unnamed-member-face-hide:{}:{}", out.photo, out.index)}));
    assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step");
    done(&mut h);
    let region = &h.app.session.catalog.photo(PhotoId(out.photo)).unwrap().meta.regions[out.index];
    assert!(!region.apart.is_empty(), "the face remembers it was taken out");
    // the face is a tile of its own and the stack still stands with the two others, open
    let list = h.app.caches.unnamed.as_ref().unwrap().3.clone();
    assert!(list.faces.iter().any(|f| !f.is_stack() && (f.photo, f.index) == (out.photo, out.index)), "{list:?}");
    let two = list.faces.iter().find(|f| f.is_stack()).unwrap();
    assert_eq!(two.stack.len(), 2);
    assert!(two.stack.iter().all(|f| (f.photo, f.index) != (out.photo, out.index)));
    assert_eq!(widgets(&mut h, "unnamed-member-face:").len(), 2, "the stack is still open, with its two faces");
    assert!(widgets(&mut h, &member(PhotoId(out.photo), out.index)).is_empty());
    assert!(widgets(&mut h, &picture(PhotoId(out.photo), out.index)).len() == 1, "its own tile: {:?}", reading(&mut h));
    // the tiles before the stack are where they were
    for (id, r) in still_before {
        assert_eq!(rect_of(&mut h, &id), r, "{id} moved");
    }
    let _ = ids;
    let _ = std::fs::remove_dir_all(&dir);
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
