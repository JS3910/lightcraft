//! Stacks in the Unnamed faces: `faces.unnamed` lists faces without a suggestion that are virtually certain to be the same
//! person as one entry with a count. Seen from the command: the bar calibrated from the named faces, no chaining, the
//! order, the scope, naming a stack in one undo step, and that the work is the background run's (never the caller's
//! frame) and is kept from one run to the next.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use super::face_recognize::{region, set_regions, setup, temp, unit64};
use crate::Session;

/// Three photos: `a` has Ann (e0) and Bob (e1) named, so the bar is the fixed minimum; the unnamed faces are
/// - a stack of three: `a` x 0.5 and 0.7, `b` x 0.1 (the middle one is the most typical),
/// - a pair in `c`,
/// - a face like Ann and another like Ann (suggested her; not alike each other, so they do not stack),
/// - three faces in a chain (A like B, B like C, A not like C), in `b`,
/// - a lone face, and one not embedded.
pub(super) fn library(name: &str) -> (Session, std::path::PathBuf, [PhotoId; 3]) {
    let d = temp(name);
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let mut ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort();
    let [a, b, c] = [ids[0], ids[1], ids[2]];
    set_regions(
        &mut s,
        a,
        vec![region(0.05, Some("Ann")), region(0.1, Some("Bob")), region(0.5, None), region(0.7, None), region(0.8, None), region(0.85, None)],
    );
    set_regions(&mut s, b, (1..=6).map(|i| region(0.1 * i as f64, None)).collect());
    set_regions(&mut s, c, vec![region(0.1, None), region(0.2, None), region(0.3, None)]);
    let put = |s: &mut Session, id: PhotoId, x: f64, v: Vec<f32>| s.faces.index.insert(id.0, &region(x, None).rect, v);
    put(&mut s, a, 0.05, unit64(&[(0, 1.0)])); // Ann
    put(&mut s, a, 0.1, unit64(&[(1, 1.0)])); // Bob
    put(&mut s, a, 0.5, unit64(&[(10, 1.0), (11, 0.15)])); // the stack of three
    put(&mut s, a, 0.7, unit64(&[(10, 1.0), (11, -0.15)]));
    put(&mut s, b, 0.1, unit64(&[(10, 1.0)])); // the most typical
    put(&mut s, c, 0.1, unit64(&[(12, 1.0), (13, 0.05)])); // the pair
    put(&mut s, c, 0.2, unit64(&[(12, 1.0), (13, 0.06)]));
    put(&mut s, a, 0.8, unit64(&[(0, 0.97), (40, 0.2)])); // like Ann
    put(&mut s, a, 0.85, unit64(&[(0, 0.7), (41, 0.7)])); // like Ann too, but not like the one before
    // a chain: the middle one first, then two that are 0.9 alike it and 0.62 alike each other
    put(&mut s, b, 0.2, unit64(&[(20, 1.0)]));
    put(&mut s, b, 0.3, unit64(&[(20, 0.9), (21, 0.436)]));
    put(&mut s, b, 0.4, unit64(&[(20, 0.9), (21, -0.436)]));
    put(&mut s, b, 0.5, unit64(&[(30, 1.0)])); // alone
    // (b's last face, at 0.6, and c's last, at 0.3, are not embedded)
    (s, d, [a, b, c])
}

pub(super) fn ask(s: &mut Session) -> Value {
    s.execute("faces.unnamed", &json!({})).unwrap()
}

pub(super) type Key = (u64, u64);

/// The entries of the list: each as the faces it holds (the face alone, or the stack's faces, the most typical first).
pub(super) fn entries(r: &Value) -> Vec<Vec<Key>> {
    let at = |f: &Value| (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap());
    r["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| match f["stack"]["faces"].as_array() {
            Some(m) => m.iter().map(at).collect(),
            None => vec![at(f)],
        })
        .collect()
}

pub(super) fn stacks(r: &Value) -> Vec<Vec<Key>> {
    entries(r).into_iter().filter(|e| e.len() > 1).collect()
}

pub(super) fn settle(s: &mut Session) -> Value {
    let r = ask(s);
    assert_eq!(r["complete"], true, "{r}");
    r
}

#[test]
fn faces_that_are_virtually_certain_to_be_one_person_are_one_entry_with_their_count() {
    let (mut s, d, [a, b, c]) = library("stacks-list");
    let r = settle(&mut s);
    let mut stacked = stacks(&r);
    stacked.sort_by_key(|st| std::cmp::Reverse(st.len()));
    assert_eq!(stacked.len(), 3, "the trio, the pair and one of the chain's pairs: {stacked:?}");
    assert_eq!(stacked[0][0], (b.0, 0), "the most typical face of the trio first: {stacked:?}");
    let mut trio = stacked[0].clone();
    trio.sort_unstable();
    assert_eq!(trio, vec![(a.0, 2), (a.0, 3), (b.0, 0)]);
    let mut pair = stacked.iter().find(|st| st.iter().all(|k| k.0 == c.0)).cloned().unwrap_or_default();
    pair.sort_unstable();
    assert_eq!(pair, vec![(c.0, 0), (c.0, 1)]);
    // the entry says its count, and is shown by its representative: the first face
    let first = r["faces"].as_array().unwrap().iter().find(|f| f["stack"]["count"] == 3).unwrap();
    assert_eq!((first["photo"].clone(), first["index"].clone()), (json!(b.0), json!(0)), "{first}");
    assert_eq!(first["stack"]["faces"].as_array().unwrap().len(), 3);
    assert!(first["rect"]["x0"].is_number() && first["view"]["x0"].is_number(), "the representative's box: {first}");
    assert_eq!(r["stacks"], 3);
    assert_eq!(r["total"], 13, "the total counts faces, not entries: {r}");
    // every face is in the list once: stacked faces appear nowhere else
    let mut all: Vec<Key> = entries(&r).into_iter().flatten().collect();
    let n = all.len();
    all.sort_unstable();
    all.dedup();
    assert_eq!((all.len(), n), (13, 13), "{all:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn among_the_faces_without_a_suggestion_stacks_come_first_biggest_first_then_single_faces_then_the_unembedded() {
    let (mut s, d, [a, b, c]) = library("stacks-order");
    let r = settle(&mut s);
    let e = entries(&r);
    // the two faces like Ann are suggested her and come first, by score, and are never stacked together
    assert_eq!(&e[..2], &[vec![(a.0, 4)], vec![(a.0, 5)]], "{e:?}");
    assert!(r["faces"][0]["suggestion"]["name"] == "Ann" && r["faces"][1]["suggestion"]["name"] == "Ann", "{r}");
    let sizes: Vec<usize> = e[2..].iter().map(Vec::len).collect();
    assert_eq!(&sizes[..3], &[3, 2, 2], "the stacks, biggest first: {e:?}");
    assert!(sizes[3..].iter().all(|n| *n == 1), "then single faces: {e:?}");
    let lone: Vec<Key> = e[5..].iter().map(|x| x[0]).collect();
    assert!(lone.contains(&(b.0, 4)), "{lone:?}");
    assert_eq!(
        &lone[lone.len() - 2..],
        &{
            let mut u = [(b.0, 5), (c.0, 2)];
            u.sort();
            u
        }[..],
        "the faces not embedded last, in photo order: {lone:?}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_chain_of_look_alikes_never_puts_its_ends_in_one_stack() {
    let (mut s, d, [_, b, _]) = library("stacks-chain");
    let r = settle(&mut s);
    // b's faces at index 2 and 3 are the ends of the chain (0.62 alike)
    assert!(stacks(&r).iter().all(|st| !(st.contains(&(b.0, 2)) && st.contains(&(b.0, 3)))), "{:?}", stacks(&r));
    let with_middle = stacks(&r).into_iter().filter(|st| st.contains(&(b.0, 1))).count();
    assert_eq!(with_middle, 1, "the middle one stacks with one of the ends");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_bar_comes_from_the_faces_named_as_different_people_and_never_falls_below_the_minimum() {
    let (mut s, d, [a, _, c]) = library("stacks-bar");
    // two more people, named by one face each, 0.93 alike: faces less alike than that (plus the margin) must not stack
    set_regions(
        &mut s,
        c,
        vec![region(0.1, None), region(0.2, None), region(0.3, Some("Cy")), region(0.4, Some("Di")), region(0.5, None), region(0.6, None)],
    );
    let put = |s: &mut Session, id: PhotoId, x: f64, v: Vec<f32>| s.faces.index.insert(id.0, &region(x, None).rect, v);
    put(&mut s, c, 0.3, unit64(&[(50, 1.0)]));
    put(&mut s, c, 0.4, unit64(&[(50, 0.93), (51, 0.368)])); // 0.93 alike Cy
    put(&mut s, c, 0.5, unit64(&[(52, 1.0)])); // a pair of unnamed faces 0.96 alike: above the minimum, under what Cy and Di reach
    put(&mut s, c, 0.6, unit64(&[(52, 0.96), (53, 0.28)]));
    let r = settle(&mut s);
    assert!((s.faces.arrange.bar - 0.98).abs() < 1e-3, "the highest similarity between two people (0.93) and the margin: {}", s.faces.arrange.bar);
    assert!(stacks(&r).iter().all(|st| !st.contains(&(c.0, 4))), "{:?}", stacks(&r));
    // the pair in the photo before, alike at 0.9995, is a stack still
    assert!(stacks(&r).iter().any(|st| st.contains(&(c.0, 0)) && st.contains(&(c.0, 1))), "{:?}", stacks(&r));
    // naming Cy as Di takes the two people away: the bar is the minimum again, and the 0.96 pair stacks
    s.execute("faces.setName", &json!({"id": c.0, "index": 2, "name": "Di"})).unwrap();
    let r = settle(&mut s);
    assert!((s.faces.arrange.bar - 0.75).abs() < 1e-6, "{}", s.faces.arrange.bar);
    assert!(stacks(&r).iter().any(|st| st.contains(&(c.0, 4)) && st.contains(&(c.0, 5))), "{:?}", stacks(&r));
    let _ = a;
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_stacks_follow_the_scope_and_a_stack_left_with_one_face_is_a_single_face() {
    let (mut s, d, [a, b, _]) = library("stacks-scope");
    // the trio is in photos a (two faces) and b (one): an album of `a` holds two of its faces
    let album = s.execute("album.create", &json!({"name": "Only a"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": album, "ids": [a.0]})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    let r = s.execute("faces.unnamed", &json!({"scope": "visible"})).unwrap();
    let with_a: Vec<Vec<Key>> = stacks(&r);
    assert_eq!(with_a.len(), 1, "{with_a:?}");
    let mut two = with_a[0].clone();
    two.sort_unstable();
    assert_eq!(two, vec![(a.0, 2), (a.0, 3)], "the count shows only the faces in scope");
    assert_eq!(r["faces"].as_array().unwrap().iter().find(|f| f["stack"].is_object()).unwrap()["stack"]["count"], 2);
    // an album of `b` leaves one face of the trio: a single face
    let album_b = s.execute("album.create", &json!({"name": "Only b"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": album_b, "ids": [b.0]})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": album_b})).unwrap();
    let r = s.execute("faces.unnamed", &json!({"scope": "visible"})).unwrap();
    assert!(entries(&r).iter().any(|e| e == &vec![(b.0, 0)]), "a stack of one is a single face: {:?}", entries(&r));
    assert!(r["faces"].as_array().unwrap().iter().filter(|f| f["stack"].is_object()).all(|f| f["stack"]["count"].as_u64() >= Some(2)));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn naming_every_face_of_a_stack_is_one_undo_step_and_the_stack_leaves_the_list() {
    let (mut s, d, _) = library("stacks-name");
    let r = settle(&mut s);
    let trio = stacks(&r).into_iter().max_by_key(Vec::len).unwrap();
    let faces: Vec<Value> = trio.iter().map(|(p, i)| json!({"photo": p, "index": i})).collect();
    let undo = s.undo.len();
    s.execute("faces.nameFaces", &json!({"faces": faces, "name": "Zed Q"})).unwrap();
    assert_eq!(s.undo.len(), undo + 1);
    let r = settle(&mut s);
    assert!(entries(&r).iter().flatten().all(|k| !trio.contains(k)), "the stack left the list: {:?}", entries(&r));
    assert_eq!(r["total"], 10, "{r}");
    s.execute("edit.undo", &json!({})).unwrap();
    let r = settle(&mut s);
    assert!(stacks(&r).iter().any(|st| st.len() == 3), "one undo puts every face back: {:?}", stacks(&r));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn stacks_are_worked_out_in_the_background_run_and_never_in_the_call_that_does_not_wait() {
    let (mut s, d, _) = library("stacks-bg");
    let hold = Arc::new(AtomicBool::new(true));
    s.faces.arrange.hold = Some(hold.clone());
    let r = s.execute("faces.unnamed", &json!({"wait": false})).unwrap();
    assert_eq!((r["ordering"].clone(), r["stacks"].clone()), (json!(true), json!(0)), "no stacks until the run is done: {r}");
    assert_eq!(entries(&r).len(), 13, "every face is a tile of its own meanwhile: {:?}", entries(&r));
    let tasks = s.execute("activity.list", &json!({})).unwrap();
    let row = tasks["tasks"].as_array().unwrap().iter().find(|t| t["kind"] == "faceOrder").cloned().unwrap_or_else(|| panic!("{tasks}"));
    assert_eq!(row["cancellable"], true, "the stacking is the run with the faceOrder row: {row}");
    hold.store(false, Ordering::Relaxed);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while s.execute("faces.pump", &json!({})).unwrap()["ordering"] == true && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let r = s.execute("faces.unnamed", &json!({"wait": false})).unwrap();
    assert_eq!((r["complete"].clone(), r["stacks"].clone()), (json!(true), json!(3)), "{r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_cancelled_run_leaves_no_stacks_and_nothing_of_it_is_applied() {
    let (mut s, d, _) = library("stacks-cancel");
    s.faces.arrange.hold = Some(Arc::new(AtomicBool::new(true)));
    s.execute("faces.unnamed", &json!({"wait": false})).unwrap();
    let tasks = s.execute("activity.list", &json!({})).unwrap();
    let id = tasks["tasks"].as_array().unwrap().iter().find(|t| t["kind"] == "faceOrder").unwrap()["id"].as_u64().unwrap();
    s.execute("activity.cancel", &json!({"id": id})).unwrap();
    for _ in 0..3000 {
        if s.execute("faces.pump", &json!({})).unwrap()["ordering"] == false {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let r = s.execute("faces.unnamed", &json!({"wait": false})).unwrap();
    assert_eq!((r["stopped"].clone(), r["stacks"].clone()), (json!(true), json!(0)), "{r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_new_face_costs_what_it_changed_and_the_stacks_are_the_ones_from_scratch_would_keep_together() {
    let (mut s, d, [_, _, c]) = library("stacks-kept");
    settle(&mut s);
    let cold = s.faces.arrange.stack_compared;
    assert!(cold > 0);
    // the scan finds one more face like the pair in `c`: it joins that stack, and little is compared
    let mut regions = s.catalog.photo(c).unwrap().meta.regions.clone();
    regions.push(region(0.9, None));
    set_regions(&mut s, c, regions);
    s.faces.index.insert(c.0, &region(0.9, None).rect, unit64(&[(12, 1.0), (13, 0.055)]));
    let r = settle(&mut s);
    assert!(stacks(&r).iter().any(|st| st.len() == 3 && st.contains(&(c.0, 3))), "{:?}", stacks(&r));
    assert_eq!(s.faces.arrange.stack_new, 1, "only the new face was looked at");
    assert!(s.faces.arrange.stack_compared * 5 < cold, "{} against {cold}", s.faces.arrange.stack_compared);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn with_recognition_off_there_are_no_stacks_and_faces_without_embeddings_never_stack() {
    let (mut s, d, _) = library("stacks-off");
    s.execute("faces.enable", &json!({"enabled": false})).unwrap();
    let r = ask(&mut s);
    assert_eq!((r["ready"].clone(), stacks(&r).len(), r["stacks"].clone()), (json!(false), 0, json!(0)), "{r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_face_whose_stamp_changed_since_the_run_is_out_of_its_stack_until_the_next_run() {
    let (mut s, d, [_, b, _]) = library("stacks-stamp");
    settle(&mut s);
    // a face of the trio is rejected a name (its own stamp changes): until the run is done for it, it is out of the stack
    s.execute("faces.reject", &json!({"faces": [{"photo": b.0, "index": 0}], "name": "Zed"})).unwrap();
    let r = s.execute("faces.unnamed", &json!({"wait": false})).unwrap();
    assert_eq!(r["ordering"], true, "{r}");
    assert!(entries(&r).contains(&vec![(b.0, 0)]), "the face is a tile of its own: {:?}", entries(&r));
    assert!(stacks(&r).iter().any(|st| st.len() == 2 && !st.contains(&(b.0, 0))), "the trio is two faces now: {:?}", stacks(&r));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn after_another_scope_a_stack_the_old_run_found_shows_only_the_faces_in_scope_and_one_left_is_a_single_face() {
    let (mut s, d, [_, b, _]) = library("stacks-stale-scope");
    settle(&mut s);
    let album = s.execute("album.create", &json!({"name": "Only b"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": album, "ids": [b.0]})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    // asked at once, the answer is what the run for the whole library found, as far as it holds for the faces in scope
    let r = s.execute("faces.unnamed", &json!({"scope": "visible", "wait": false})).unwrap();
    assert_eq!(r["ordering"], true, "{r}");
    assert!(entries(&r).contains(&vec![(b.0, 0)]), "the one face of the trio in scope is a face: {:?}", entries(&r));
    assert!(r["faces"].as_array().unwrap().iter().all(|f| f["stack"]["count"].as_u64().is_none_or(|n| n >= 2)), "no stack of one: {r}");
    let _ = std::fs::remove_dir_all(&d);
}

// ---- stacks of faces that share a suggested name ----

/// Ann is `e0` and Bob 50 degrees away, both named in photo `a`; photo `b` holds the unnamed faces `faces` in order (the `i`th
/// at `x = 0.1 * (i + 1)`), each given by its non-zero components.
fn suggested(name: &str, faces: &[&[(usize, f32)]], reject_ann: &[usize]) -> (Session, std::path::PathBuf, [PhotoId; 2]) {
    let d = temp(name);
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let mut ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort();
    let [a, b] = [ids[0], ids[1]];
    set_regions(&mut s, a, vec![region(0.05, Some("Ann")), region(0.1, Some("Bob"))]);
    let regions: Vec<_> = (0..faces.len())
        .map(|i| {
            let mut r = region(0.1 * (i + 1) as f64, None);
            if reject_ann.contains(&i) {
                r.rejected.push("Ann".into());
            }
            r
        })
        .collect();
    set_regions(&mut s, b, regions);
    s.faces.index.insert(a.0, &region(0.05, None).rect, unit64(&[(0, 1.0)]));
    s.faces.index.insert(a.0, &region(0.1, None).rect, unit64(&[(0, 0.5), (1, 0.866)]));
    for (i, f) in faces.iter().enumerate() {
        s.faces.index.insert(b.0, &region(0.1 * (i + 1) as f64, None).rect, unit64(f));
    }
    (s, d, [a, b])
}

/// The entry of the list that holds `key`, with the suggestion it carries.
fn entry_of(r: &Value, key: Key) -> (Vec<Key>, Option<String>) {
    let at = |f: &Value| (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap());
    let f = r["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| at(f) == key || f["stack"]["faces"].as_array().is_some_and(|m| m.iter().any(|x| at(x) == key)))
        .unwrap_or_else(|| panic!("{key:?} is not listed: {r}"));
    let faces = f["stack"]["faces"].as_array().map_or_else(|| vec![at(f)], |m| m.iter().map(at).collect());
    (faces, f["suggestion"]["name"].as_str().map(str::to_string))
}

/// Two faces 10 degrees from Ann towards Bob, and two 10 degrees from Bob towards Ann: each close to the other pair (0.766, above
/// the minimum bar), each pair suggested a different person.
const NEAR_ANN: [(usize, f32); 2] = [(0, 0.985), (1, 0.174)];
const NEAR_BOB: [(usize, f32); 2] = [(0, 0.643), (1, 0.766)];

#[test]
fn faces_with_the_same_suggestion_are_a_stack_that_carries_it_and_faces_with_different_suggestions_never_share_one() {
    let (mut s, d, [_, b]) = suggested(
        "stacks-sg-names",
        &[
            &[NEAR_ANN[0], NEAR_ANN[1], (20, 0.02)],
            &[NEAR_BOB[0], NEAR_BOB[1], (21, 0.02)],
            &[NEAR_ANN[0], NEAR_ANN[1], (20, 0.03)],
            &[NEAR_BOB[0], NEAR_BOB[1], (21, 0.03)],
        ],
        &[],
    );
    let r = settle(&mut s);
    let (ann, ann_name) = entry_of(&r, (b.0, 0));
    let (bob, bob_name) = entry_of(&r, (b.0, 1));
    let sorted = |mut v: Vec<Key>| {
        v.sort_unstable();
        v
    };
    assert_eq!((sorted(ann), ann_name.as_deref()), (vec![(b.0, 0), (b.0, 2)], Some("Ann")), "{r}");
    assert_eq!((sorted(bob), bob_name.as_deref()), (vec![(b.0, 1), (b.0, 3)], Some("Bob")), "{r}");
    assert_eq!(r["stacks"], 2, "{r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_face_that_rejected_a_name_is_never_in_a_stack_suggesting_it() {
    let same = |extra: f32| -> Vec<(usize, f32)> { vec![NEAR_ANN[0], NEAR_ANN[1], (20, extra)] };
    let (a, b, c) = (same(0.02), same(0.03), same(0.04));
    let (mut s, d, [_, p]) = suggested("stacks-sg-rejected", &[&a, &b, &c], &[2]);
    let r = settle(&mut s);
    let (two, name) = entry_of(&r, (p.0, 0));
    assert_eq!((two.len(), name.as_deref()), (2, Some("Ann")), "{r}");
    assert!(!two.contains(&(p.0, 2)), "{two:?}");
    let (alone, name) = entry_of(&r, (p.0, 2));
    assert_eq!(alone, vec![(p.0, 2)], "it is in no stack of the name it rejected");
    assert_ne!(name.as_deref(), Some("Ann"), "and is not suggested it");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_stack_with_a_suggestion_sorts_among_the_suggested_faces_by_the_score_of_its_representative() {
    // three faces near Ann, 0.95, 0.75 and 0.62 alike her, none alike another (their other parts are apart): the 0.75 ones are two
    let (mut s, d, [_, p]) = suggested(
        "stacks-sg-order",
        &[&[(0, 0.62), (13, 0.785)], &[(0, 0.75), (11, 0.661), (12, 0.01)], &[(0, 0.95), (10, 0.312)], &[(0, 0.75), (11, 0.661), (12, 0.02)]],
        &[],
    );
    let r = settle(&mut s);
    let order: Vec<Vec<Key>> = entries(&r)
        .into_iter()
        .take(3)
        .map(|mut e| {
            e.sort_unstable();
            e
        })
        .collect();
    assert_eq!(order, vec![vec![(p.0, 2)], vec![(p.0, 1), (p.0, 3)], vec![(p.0, 0)]], "the better face, the stack, the weaker face: {r}");
    let score = |at: usize| r["faces"][at]["suggestion"]["score"].as_f64().unwrap();
    assert!(score(0) > score(1) && score(1) > score(2), "{r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn naming_a_stack_with_a_suggestion_is_one_undo_step_and_rejecting_its_suggestion_is_one_step_that_holds_for_every_face() {
    let same = |extra: f32| -> Vec<(usize, f32)> { vec![NEAR_ANN[0], NEAR_ANN[1], (20, extra)] };
    let (a, b, c) = (same(0.02), same(0.03), same(0.04));
    let (mut s, d, [_, p]) = suggested("stacks-sg-act", &[&a, &b, &c], &[]);
    let r = settle(&mut s);
    let (faces, name) = entry_of(&r, (p.0, 0));
    assert_eq!((faces.len(), name.as_deref()), (3, Some("Ann")), "{r}");
    let list: Vec<Value> = faces.iter().map(|(p, i)| json!({"photo": p, "index": i, "name": "Ann"})).collect();
    // the tick: every face is named with the suggestion, in one undo step
    let undo = s.undo.len();
    s.execute("faces.nameFaces", &json!({"faces": list})).unwrap();
    assert_eq!(s.undo.len(), undo + 1);
    let named = |s: &Session| s.catalog.photo(p).unwrap().meta.regions.iter().filter(|r| r.name.as_deref() == Some("Ann")).count();
    assert_eq!(named(&s), 3);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(named(&s), 0, "one undo puts every face back");
    // the cross: the suggestion is rejected for every face in one undo step, and no stack suggests it again
    let list: Vec<Value> = faces.iter().map(|(p, i)| json!({"photo": p, "index": i, "name": "Ann"})).collect();
    let undo = s.undo.len();
    s.execute("faces.reject", &json!({"faces": list})).unwrap();
    assert_eq!(s.undo.len(), undo + 1);
    let rejected = |s: &Session| s.catalog.photo(p).unwrap().meta.regions.iter().filter(|r| r.rejected.iter().any(|n| n == "Ann")).count();
    assert_eq!(rejected(&s), 3, "stored on each face, as a single rejection is");
    let r = settle(&mut s);
    let (still, name) = entry_of(&r, (p.0, 0));
    assert_ne!(name.as_deref(), Some("Ann"), "nobody suggests the rejected name again: {r}");
    assert_eq!(still.len(), 3, "the three are still alike, and stack for the name they are suggested now: {r}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(rejected(&s), 0);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn every_face_of_a_stack_carries_the_box_to_show_it_by_as_a_single_face_does() {
    let (mut s, d, [a, _, _]) = library("stacks-view");
    // the scan embedded a's face at 0.5 by a bigger box than its own
    let own = region(0.5, None).rect;
    let view = lightcraft_geom::Rect { x0: own.x0 - 0.05, y0: own.y0 - 0.05, x1: own.x1 + 0.05, y1: own.y1 + 0.05 };
    s.faces.index.insert_with_view(a.0, &own, unit64(&[(10, 1.0), (11, 0.15)]), &view);
    let r = settle(&mut s);
    let trio = r["faces"].as_array().unwrap().iter().find(|f| f["stack"]["count"] == 3).unwrap();
    let faces = trio["stack"]["faces"].as_array().unwrap();
    let one = faces.iter().find(|f| f["photo"] == json!(a.0) && f["index"] == json!(2)).expect("the face in the stack");
    assert!(one["view"]["x0"].as_f64().unwrap() < one["rect"]["x0"].as_f64().unwrap(), "the view is the bigger box: {one}");
    assert!(faces.iter().all(|f| f["view"]["x0"].is_number()), "every face has one: {faces:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_face_of_a_stack_with_a_suggestion_can_be_taken_out_and_the_help_says_so_and_that_the_web_build_has_no_stacks() {
    let same = |extra: f32| -> Vec<(usize, f32)> { vec![NEAR_ANN[0], NEAR_ANN[1], (20, extra)] };
    let (mut s, d, [_, b]) = suggested("stacks-sg-out", &[&same(0.02), &same(0.03)], &[]);
    let r = settle(&mut s);
    assert_eq!(entry_of(&r, (b.0, 0)).1.as_deref(), Some("Ann"), "a stack with a suggestion: {r}");
    let reply = s.execute("faces.takeOut", &json!({"faces": [{"photo": b.0, "index": 1}]})).unwrap();
    assert_eq!(reply["taken"], 1, "{reply}");
    let help = crate::cmd::command_specs().iter().find(|c| c.id == "faces.takeOut").map(|c| c.params).unwrap_or_default();
    assert!(!help.contains("a face with a suggestion"), "that is not skipped: {help}");
    assert!(help.contains("suggested name") && help.contains("web"), "{help}");
    let _ = std::fs::remove_dir_all(&d);
}
