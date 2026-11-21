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
