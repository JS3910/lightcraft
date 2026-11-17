//! Rejections: saying "this is not that person" for a face is kept on the face in the catalog (`faces.reject`), undoable,
//! never suggested again, and followed by renames and merges.

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use super::face_recognize::{region, set_regions, setup, temp, two_photos, unit64};
use crate::Session;

fn rejected(s: &Session, id: PhotoId, i: usize) -> Vec<String> {
    s.catalog.photo(id).unwrap().meta.regions[i].rejected.clone()
}

fn name_of(s: &Session, id: PhotoId, i: usize) -> Option<String> {
    s.catalog.photo(id).unwrap().meta.regions[i].name.clone()
}

/// Photo `a` holds Ann (named, emb 0) and Bob (named, a different look) and two unnamed faces that look like Ann; photo
/// `b` holds one more such face. Every face is embedded.
fn library(name: &str) -> (Session, std::path::PathBuf, PhotoId, PhotoId) {
    let d = temp(name);
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let (a, b) = two_photos(&s);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.4, Some("Bob")), region(0.65, None)]);
    set_regions(&mut s, b, vec![region(0.1, None), region(0.5, None)]);
    let mut put = |id: PhotoId, x: f64, v: Vec<f32>| s.faces.index.insert(id.0, &region(x, None).rect, v);
    put(a, 0.1, unit64(&[(0, 1.0)])); // Ann
    put(a, 0.4, unit64(&[(0, 0.6), (2, 0.8)])); // Bob: a second, further person
    put(a, 0.65, unit64(&[(0, 0.97), (3, 0.2)])); // like Ann
    put(b, 0.1, unit64(&[(0, 0.96), (4, 0.2)])); // like Ann
    put(b, 0.5, unit64(&[(5, 1.0)])); // like nobody
    (s, d, a, b)
}

fn suggestion(s: &mut Session, id: PhotoId, i: usize) -> Option<String> {
    let r = s.execute("faces.unnamed", &json!({})).unwrap();
    let f = r["faces"].as_array().unwrap().iter().find(|f| f["photo"] == id.0 && f["index"] == i).cloned().unwrap_or(Value::Null);
    f["suggestion"]["name"].as_str().map(str::to_string)
}

#[test]
fn rejecting_a_name_for_many_faces_is_one_undo_step_that_undo_takes_back() {
    let (mut s, d, a, b) = library("rej-many");
    assert_eq!((suggestion(&mut s, a, 2).as_deref(), suggestion(&mut s, b, 0).as_deref()), (Some("Ann"), Some("Ann")));
    let undo = s.undo.len();
    let r = s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 2}, {"photo": b.0, "index": 0}], "name": " Ann "})).unwrap();
    assert_eq!((r["rejected"].clone(), r["photos"].clone(), r["cleared"].clone()), (json!(2), json!(2), json!(0)), "{r}");
    assert_eq!(s.undo.len(), undo + 1, "one step for the whole list");
    assert_eq!((rejected(&s, a, 2), rejected(&s, b, 0)), (vec!["Ann".to_string()], vec!["Ann".to_string()]), "stored trimmed");
    // the next-best person is suggested when they pass the bar; the faces are still unnamed
    assert_eq!((suggestion(&mut s, a, 2).as_deref(), suggestion(&mut s, b, 0).as_deref()), (Some("Bob"), Some("Bob")));
    assert_eq!(name_of(&s, a, 2), None);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((rejected(&s, a, 2), rejected(&s, b, 0)), (Vec::<String>::new(), Vec::<String>::new()));
    assert_eq!(suggestion(&mut s, a, 2).as_deref(), Some("Ann"), "undo restores the suggestion");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_rejected_name_is_kept_once_whatever_its_spelling() {
    let (mut s, d, a, _) = library("rej-once");
    let face = json!([{"photo": a.0, "index": 2}]);
    s.execute("faces.reject", &json!({"faces": face, "name": "Ann"})).unwrap();
    let undo = s.undo.len();
    let r = s.execute("faces.reject", &json!({"faces": face, "name": "  aNN"})).unwrap();
    assert_eq!(r["rejected"], 0, "{r}");
    assert_eq!(s.undo.len(), undo, "nothing changed, nothing to undo");
    assert_eq!(rejected(&s, a, 2), ["Ann"]);
    // every face can name its own (as confirming a selection does), and an entry's name wins over the shared one
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 2, "name": "Bob"}], "name": "Cy"})).unwrap();
    assert_eq!(rejected(&s, a, 2), ["Ann", "Bob"]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_suggestion_query_skips_what_the_face_rejected() {
    let (mut s, d, a, _) = library("rej-suggest");
    let top = |s: &mut Session| {
        let r = s.execute("faces.suggest", &json!({"ids": [a.0], "budgetMs": 0})).unwrap();
        let f = r["photos"][0]["faces"].as_array().unwrap().iter().find(|f| f["index"] == 2).cloned().unwrap();
        (f["suggestion"]["name"].as_str().map(str::to_string), f["candidates"].as_array().unwrap().len())
    };
    assert_eq!(top(&mut s), (Some("Ann".to_string()), 2));
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 2}], "name": "ann"})).unwrap();
    assert_eq!(top(&mut s), (Some("Bob".to_string()), 1), "the next-best person, and the rejected one is no candidate");
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 2}], "name": "Bob"})).unwrap();
    assert_eq!(top(&mut s), (None, 0), "nobody left to suggest");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_persons_more_leaves_out_faces_that_rejected_that_person() {
    let (mut s, d, a, b) = library("rej-more");
    let more = |s: &mut Session| -> Vec<(u64, u64)> {
        let r = s.execute("faces.person", &json!({"name": "Ann"})).unwrap();
        let mut v: Vec<_> = r["more"].as_array().unwrap().iter().map(|f| (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap())).collect();
        v.sort();
        v
    };
    assert_eq!(more(&mut s), [(a.0, 2), (b.0, 0)]);
    s.execute("faces.reject", &json!({"faces": [{"photo": b.0, "index": 0}], "name": "ANN"})).unwrap();
    assert_eq!(more(&mut s), [(a.0, 2)]);
    // rejecting someone else changes nothing for Ann
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 2}], "name": "Bob"})).unwrap();
    assert_eq!(more(&mut s), [(a.0, 2)]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn naming_a_face_with_a_rejected_name_drops_that_rejection_in_the_same_edit() {
    let (mut s, d, a, b) = library("rej-name");
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 2}, {"photo": b.0, "index": 0}], "name": "Ann"})).unwrap();
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 2}], "name": "Bob"})).unwrap();
    let undo = s.undo.len();
    s.execute("faces.setName", &json!({"id": a.0, "index": 2, "name": "ann"})).unwrap();
    assert_eq!((name_of(&s, a, 2).as_deref(), rejected(&s, a, 2)), (Some("ann"), vec!["Bob".to_string()]));
    s.execute("faces.nameFaces", &json!({"faces": [{"photo": b.0, "index": 0}], "name": "Ann"})).unwrap();
    assert!(rejected(&s, b, 0).is_empty());
    assert_eq!(s.undo.len(), undo + 2);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((name_of(&s, b, 0), rejected(&s, b, 0)), (None, vec!["Ann".to_string()]), "one undo brings name and rejection back");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn rejecting_the_current_name_can_clear_it_in_the_same_step() {
    let (mut s, d, a, _) = library("rej-clear");
    let undo = s.undo.len();
    // not asked for: a face called that is left alone (rejecting its own name would contradict it), and counted
    let r = s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 1}], "name": "bob"})).unwrap();
    assert_eq!((r["rejected"].clone(), r["skipped"].clone(), r["photos"].clone()), (json!(0), json!(1), json!(0)), "{r}");
    assert_eq!((name_of(&s, a, 1).as_deref(), rejected(&s, a, 1)), (Some("Bob"), Vec::<String>::new()));
    assert_eq!(s.undo.len(), undo, "nothing changed");
    // asked for: only a face whose name is the rejected one loses it
    let r = s
        .execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 1}, {"photo": a.0, "index": 0}], "name": " BOB", "clearName": true}))
        .unwrap();
    assert_eq!((r["rejected"].clone(), r["cleared"].clone()), (json!(2), json!(1)), "{r}");
    assert_eq!((name_of(&s, a, 1), name_of(&s, a, 0).as_deref()), (None, Some("Ann")));
    assert_eq!(s.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((name_of(&s, a, 1).as_deref(), rejected(&s, a, 1)), (Some("Bob"), Vec::<String>::new()));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn rename_and_merge_carry_rejections_to_the_new_name_in_the_same_undo_step() {
    let (mut s, d, a, b) = library("rej-rename");
    // an unnamed face rejected Ann and Bob; a face named Bob once rejected Ann
    s.execute("faces.reject", &json!({"faces": [{"photo": b.0, "index": 0}], "name": "Ann"})).unwrap();
    s.execute("faces.reject", &json!({"faces": [{"photo": b.0, "index": 0}], "name": "Bob"})).unwrap();
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 1}], "name": "ann"})).unwrap();
    let undo = s.undo.len();
    s.execute("faces.renamePerson", &json!({"from": "Ann", "to": "Anna"})).unwrap();
    assert_eq!(s.undo.len(), undo + 1);
    assert_eq!((rejected(&s, b, 0), rejected(&s, a, 1)), (vec!["Anna".to_string(), "Bob".to_string()], vec!["Anna".to_string()]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((rejected(&s, b, 0), rejected(&s, a, 1)), (vec!["Ann".to_string(), "Bob".to_string()], vec!["ann".to_string()]));
    // a merge: both people become Bob; the face rejecting both says it once, and Bob's own face no longer rejects Bob
    s.execute("faces.renamePerson", &json!({"from": "Ann", "to": "Bob", "merge": true})).unwrap();
    assert_eq!(s.undo.len(), undo + 1, "the undone rename is gone, the merge is one step");
    assert_eq!((rejected(&s, b, 0), rejected(&s, a, 1)), (vec!["Bob".to_string()], Vec::<String>::new()));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(rejected(&s, b, 0), ["Ann", "Bob"]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_ranking_marks_the_names_a_face_rejected() {
    let (mut s, d, a, _) = library("rej-rank");
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 2}], "name": "ann"})).unwrap();
    let r = s.execute("faces.rank", &json!({"id": a.0, "index": 2})).unwrap();
    let flags: Vec<(String, bool)> =
        r["ranking"].as_array().unwrap().iter().map(|x| (x["name"].as_str().unwrap().to_string(), x["rejected"].as_bool().unwrap())).collect();
    assert_eq!(flags, [("Ann".to_string(), true), ("Bob".to_string(), false)], "still listed, so the name box can offer it last: {r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn rejections_survive_saving_and_reopening_the_library() {
    let d = temp("rej-reopen");
    let lib = d.join("lib");
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    super::face_recognize::write_photo(&d.join("p.png"), 3);
    let r = s.execute("library.import", &json!({"paths": [d.join("p.png").to_string_lossy()]})).unwrap();
    let id = PhotoId(r["imported"][0].as_u64().unwrap());
    set_regions(&mut s, id, vec![region(0.1, None), region(0.5, Some("Bob"))]);
    s.execute("faces.reject", &json!({"faces": [{"photo": id.0, "index": 0}], "name": "Ann"})).unwrap();
    s.close_library().unwrap();
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    assert_eq!(rejected(&s, id, 0), ["Ann"], "kept by the catalog across a reopen");
    assert!(rejected(&s, id, 1).is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_catalog_written_before_rejections_opens_unchanged() {
    let d = temp("rej-old");
    let lib = d.join("lib");
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    super::face_recognize::write_photo(&d.join("p.png"), 4);
    let r = s.execute("library.import", &json!({"paths": [d.join("p.png").to_string_lossy()]})).unwrap();
    let id = PhotoId(r["imported"][0].as_u64().unwrap());
    set_regions(&mut s, id, vec![region(0.1, Some("Ann"))]);
    s.close_library().unwrap();
    drop(s);
    // nothing of a region without rejections says a word of them: an older catalog is byte-compatible with this reader
    let mut text = String::new();
    for entry in std::fs::read_dir(&lib).unwrap().flatten() {
        if entry.path().is_file() {
            text.push_str(&String::from_utf8_lossy(&std::fs::read(entry.path()).unwrap_or_default()));
        }
    }
    assert!(!text.contains("rejected"), "no rejections, no field written");
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    assert_eq!(name_of(&s, id, 0).as_deref(), Some("Ann"));
    assert!(rejected(&s, id, 0).is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn rejecting_refuses_hostile_arguments_and_changes_nothing() {
    let (mut s, d, a, _) = library("rej-bad");
    let undo = s.undo.len();
    let mut pet = s.catalog.photo(a).unwrap().meta.clone();
    pet.regions[2].kind = lightcraft_meta::RegionKind::Pet;
    s.commit("pet", lightcraft_catalog::Op::SetMeta { id: a, meta: Box::new(pet) }).unwrap();
    let undo = undo + 1;
    for bad in [
        json!({}),
        json!({"name": "Ann"}),
        json!({"faces": [], "name": "Ann"}),
        json!({"faces": "x", "name": "Ann"}),
        json!({"faces": [{"photo": a.0, "index": 0}]}),
        json!({"faces": [{"photo": a.0, "index": 0}], "name": ""}),
        json!({"faces": [{"photo": a.0, "index": 0}], "name": "   "}),
        json!({"faces": [{"photo": a.0, "index": 0}], "name": null}),
        json!({"faces": [{"photo": a.0, "index": 0}], "name": 7}),
        json!({"faces": [{"photo": a.0, "index": 0}], "name": "bad\u{7}"}),
        json!({"faces": [{"photo": a.0, "index": 0}], "name": "x".repeat(201)}),
        json!({"faces": [{"photo": a.0, "index": 0, "name": ["x"]}], "name": "Ann"}),
        json!({"faces": [{"photo": a.0}], "name": "Ann"}),
        json!({"faces": [{"index": 0}], "name": "Ann"}),
        json!({"faces": [{"photo": a.0, "index": -1}], "name": "Ann"}),
        json!({"faces": [{"photo": a.0, "index": u64::MAX}], "name": "Ann"}),
        json!({"faces": [{"photo": 987_654, "index": 0}], "name": "Ann"}),
        json!({"faces": [{"photo": a.0, "index": 99}], "name": "Ann"}),
        json!({"faces": [{"photo": a.0, "index": 2}], "name": "Ann"}),
        json!({"faces": [{"photo": a.0, "index": 0}], "name": "Ann", "clearName": "yes"}),
        json!({"faces": vec![json!({"photo": a.0, "index": 0}); 5001], "name": "Ann"}),
    ] {
        assert!(s.execute("faces.reject", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.undo.len(), undo, "a refusal is no undo step");
    assert!(rejected(&s, a, 0).is_empty());
    // a list with one good face and some that do not exist still rejects the good one
    let r = s.execute("faces.reject", &json!({"faces": [{"photo": 987_654, "index": 0}, {"photo": a.0, "index": 0}], "name": "Zed"})).unwrap();
    assert_eq!(r["rejected"], 1, "{r}");
    // a name that is too many for one face is refused, not silently dropped
    for i in 0..199 {
        let _ = s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 0}], "name": format!("n{i}")}));
    }
    assert!(s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 0}], "name": "one too many"})).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

fn region_names(s: &Session, id: PhotoId) -> Vec<Option<String>> {
    s.catalog.photo(id).unwrap().meta.regions.iter().map(|r| r.name.clone()).collect()
}

#[test]
fn removing_many_face_boxes_across_photos_and_on_one_photo_is_one_undo_step() {
    let (mut s, d, a, b) = library("rm-many");
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 1}], "name": "Zed"})).unwrap();
    let (before_a, before_b) = (s.catalog.photo(a).unwrap().meta.regions.clone(), s.catalog.photo(b).unwrap().meta.regions.clone());
    let undo = s.undo.len();
    // two boxes of photo a (the first and the last: later indexes shift as the first goes) and one of photo b
    let r = s
        .execute("faces.removeBoxes", &json!({"faces": [{"photo": a.0, "index": 2}, {"photo": b.0, "index": 0}, {"photo": a.0, "index": 0}]}))
        .unwrap();
    assert_eq!((r["removed"].clone(), r["photos"].clone()), (json!(3), json!(2)), "{r}");
    assert_eq!(s.undo.len(), undo + 1, "one step for the whole list");
    assert_eq!(region_names(&s, a), [Some("Bob".to_string())], "only the middle box is left");
    assert_eq!(rejected(&s, a, 0), ["Zed"], "the box that stayed keeps its rejections");
    assert_eq!(s.catalog.photo(b).unwrap().meta.regions.len(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.catalog.photo(a).unwrap().meta.regions, before_a, "every box back, with name and rejections");
    assert_eq!(s.catalog.photo(b).unwrap().meta.regions, before_b);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn removing_face_boxes_refuses_hostile_lists_and_skips_what_is_not_there() {
    let (mut s, d, a, _) = library("rm-hostile");
    let undo = s.undo.len();
    for bad in [
        json!({}),
        json!({"faces": []}),
        json!({"faces": "x"}),
        json!({"faces": [{"photo": a.0}]}),
        json!({"faces": [{"index": 0}]}),
        json!({"faces": [{"photo": a.0, "index": -1}]}),
        json!({"faces": [{"photo": a.0, "index": u64::MAX}]}),
        json!({"faces": [{"photo": a.0, "index": 99}]}),
        json!({"faces": [{"photo": 987_654, "index": 0}]}),
        json!({"faces": vec![json!({"photo": a.0, "index": 0}); 5001]}),
    ] {
        assert!(s.execute("faces.removeBoxes", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.undo.len(), undo, "a refusal is no undo step");
    // the same face twice, and one that does not exist, count once
    let r = s
        .execute("faces.removeBoxes", &json!({"faces": [{"photo": a.0, "index": 0}, {"photo": a.0, "index": 0}, {"photo": a.0, "index": 99}]}))
        .unwrap();
    assert_eq!(r["removed"], 1, "{r}");
    assert_eq!(s.catalog.photo(a).unwrap().meta.regions.len(), 2);
    // a pet box is not a face box
    let mut pet = s.catalog.photo(a).unwrap().meta.clone();
    pet.regions[0].kind = lightcraft_meta::RegionKind::Pet;
    s.commit("pet", lightcraft_catalog::Op::SetMeta { id: a, meta: Box::new(pet) }).unwrap();
    assert!(s.execute("faces.removeBoxes", &json!({"faces": [{"photo": a.0, "index": 0}]})).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_photo_whose_last_face_box_was_removed_is_not_searched_again() {
    let (mut s, d, _, b) = library("rm-scan");
    assert!(!s.faces.scanned.contains(b.0), "boxes that came from a file: never searched by the scan");
    s.execute("faces.removeBoxes", &json!({"faces": [{"photo": b.0, "index": 0}]})).unwrap();
    assert!(!s.faces.scanned.contains(b.0), "one box is left: nothing to search for yet");
    s.execute("faces.removeBoxes", &json!({"faces": [{"photo": b.0, "index": 0}]})).unwrap();
    assert!(s.catalog.photo(b).unwrap().meta.regions.is_empty());
    assert!(s.faces.scanned.contains(b.0), "the scan must not detect the removed boxes again");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_merge_leaves_no_face_rejecting_the_name_it_carries() {
    let (mut s, d, a, _) = library("rej-merge-own");
    // the face named Ann once rejected Bob (a suggestion it did not want); merging Ann into Bob names it Bob
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 0}], "name": "Bob"})).unwrap();
    let undo = s.undo.len();
    s.execute("faces.renamePerson", &json!({"from": "Ann", "to": "Bob", "merge": true})).unwrap();
    assert_eq!((name_of(&s, a, 0).as_deref(), rejected(&s, a, 0)), (Some("Bob"), Vec::<String>::new()));
    assert_eq!(s.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((name_of(&s, a, 0).as_deref(), rejected(&s, a, 0)), (Some("Ann"), vec!["Bob".to_string()]), "one undo brings both back");
    let _ = std::fs::remove_dir_all(&d);
}
