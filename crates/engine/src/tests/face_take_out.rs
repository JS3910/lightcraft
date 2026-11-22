//! Taking a face out of a stack (`faces.takeOut`): the face becomes a single tile that never stacks with the faces it was
//! stacked with again, as one undo step, kept by the catalogue (and nowhere else) across saving and reopening.

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use super::face_recognize::{keep_on_cpu, region, set_regions, temp, unit64, write_photo};
use super::face_stacks::{Key, entries, library, settle, stacks};
use crate::Session;

fn take_out(s: &mut Session, faces: &[Key]) -> Value {
    let faces: Vec<Value> = faces.iter().map(|(p, i)| json!({"photo": p, "index": i})).collect();
    s.execute("faces.takeOut", &json!({"faces": faces})).unwrap()
}

#[test]
fn a_face_taken_out_is_a_single_tile_and_never_stacks_with_those_faces_again() {
    let (mut s, d, [a, b, _]) = library("out-single");
    let r = settle(&mut s);
    assert!(stacks(&r).iter().any(|st| st.len() == 3), "the trio is a stack: {:?}", stacks(&r));
    let undo = s.undo.len();
    let reply = take_out(&mut s, &[(a.0, 3)]);
    assert_eq!(reply["taken"], 1, "{reply}");
    assert_eq!(s.undo.len(), undo + 1, "one undo step");
    let r = settle(&mut s);
    assert!(entries(&r).contains(&vec![(a.0, 3)]), "a single tile: {:?}", entries(&r));
    let mut rest = stacks(&r).into_iter().find(|st| st.contains(&(a.0, 2))).unwrap();
    rest.sort_unstable();
    assert_eq!(rest, vec![(a.0, 2), (b.0, 0)], "the others are still a stack: {:?}", stacks(&r));
    // a face that was taken out never goes back, however many runs there are: the faces still look alike
    s.execute("faces.unnamed", &json!({"restart": true})).unwrap();
    let r = settle(&mut s);
    assert!(entries(&r).contains(&vec![(a.0, 3)]), "{:?}", entries(&r));
    // taking out another leaves a stack of one: no stack
    take_out(&mut s, &[(b.0, 0)]);
    let r = settle(&mut s);
    assert!(stacks(&r).iter().all(|st| !st.contains(&(a.0, 2))), "a stack left with one face is no stack: {:?}", stacks(&r));
    assert!(entries(&r).contains(&vec![(a.0, 2)]) && entries(&r).contains(&vec![(b.0, 0)]), "{:?}", entries(&r));
    // undo brings the face back to its stack
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let r = settle(&mut s);
    assert!(stacks(&r).iter().any(|st| st.len() == 3), "two undos: {:?}", stacks(&r));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn taking_out_refuses_what_is_not_in_a_stack_and_hostile_arguments_and_changes_nothing() {
    let (mut s, d, [a, b, _]) = library("out-bad");
    settle(&mut s);
    let undo = s.undo.len();
    for bad in [
        json!({}),
        json!({"faces": []}),
        json!({"faces": "x"}),
        json!({"faces": [{"photo": a.0}]}),
        json!({"faces": [{"photo": a.0, "index": -1}]}),
        json!({"faces": [{"photo": a.0, "index": 4}]}),  // a face with a suggestion: no stack
        json!({"faces": [{"photo": b.0, "index": 4}]}),  // a lone face
        json!({"faces": [{"photo": a.0, "index": 99}]}), // no such face
        json!({"faces": [{"photo": 9_999_999u64, "index": 0}]}),
        json!({"faces": (0..5001).map(|_| json!({"photo": a.0, "index": 2})).collect::<Vec<_>>()}),
    ] {
        assert!(s.execute("faces.takeOut", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.undo.len(), undo, "nothing changed");
    let _ = std::fs::remove_dir_all(&d);
}

/// A library on disk with two photos and a stack of two faces (photo 1, x 0.1 and 0.2) and a lone face (photo 2).
fn on_disk(name: &str) -> (Session, std::path::PathBuf, std::path::PathBuf, [PhotoId; 2]) {
    let d = temp(name);
    let lib = d.join("lib");
    let mut s = Session::new().with_fs();
    s.face_models_dir = Some(d.join("models"));
    keep_on_cpu(&mut s);
    s.open_library(&lib, false).unwrap();
    let model = d.join("tiny64.onnx");
    std::fs::write(&model, lightcraft_faces::synthetic::tiny_embedder_model(64)).unwrap();
    let r = s.execute("faces.models.install", &json!({"path": model.to_string_lossy(), "acknowledged": true})).unwrap();
    s.execute("faces.models.select", &json!({"id": r["installed"]["id"]})).unwrap();
    let mut ids = Vec::new();
    for seed in [5u8, 6] {
        write_photo(&d.join(format!("p{seed}.png")), seed);
        let r = s.execute("library.import", &json!({"paths": [d.join(format!("p{seed}.png")).to_string_lossy()]})).unwrap();
        ids.push(PhotoId(r["imported"][0].as_u64().unwrap()));
    }
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let [p, q] = [ids[0], ids[1]];
    set_regions(&mut s, p, vec![region(0.1, None), region(0.5, None)]);
    set_regions(&mut s, q, vec![region(0.1, None)]);
    s.faces.index.insert(p.0, &region(0.1, None).rect, unit64(&[(10, 1.0), (11, 0.1)]));
    s.faces.index.insert(p.0, &region(0.5, None).rect, unit64(&[(10, 1.0), (11, -0.1)]));
    s.faces.index.insert(q.0, &region(0.1, None).rect, unit64(&[(30, 1.0)]));
    (s, d, lib, [p, q])
}

fn all_text(lib: &std::path::Path) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(lib).unwrap().flatten() {
        if entry.path().is_file() {
            text.push_str(&String::from_utf8_lossy(&std::fs::read(entry.path()).unwrap_or_default()));
        }
    }
    text
}

#[test]
fn a_catalogue_with_no_face_taken_out_says_nothing_of_it() {
    let (mut s, d, lib, [p, _]) = on_disk("out-none");
    let r = settle(&mut s);
    assert_eq!(stacks(&r).len(), 1, "{:?}", stacks(&r));
    s.close_library().unwrap();
    assert!(!all_text(&lib).contains("apart"), "no face taken out, no field written (an older catalogue reads the same)");
    s.open_library(&lib, false).unwrap();
    assert!(s.catalog.photo(p).unwrap().meta.regions.iter().all(|r| r.apart.is_empty()));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_face_taken_out_survives_saving_and_reopening_and_is_written_nowhere_else() {
    let (mut s, d, lib, [p, _]) = on_disk("out-reopen");
    let r = settle(&mut s);
    assert_eq!(stacks(&r).len(), 1, "{:?}", stacks(&r));
    take_out(&mut s, &[(p.0, 1)]);
    assert!(!s.catalog.photo(p).unwrap().meta.regions[1].apart.is_empty());
    assert!(s.catalog.photo(p).unwrap().meta.regions[0].apart.is_empty(), "only the face taken out remembers");
    s.close_library().unwrap();
    drop(s);
    assert!(all_text(&lib).contains("apart"), "the catalogue holds it");
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let regions = &s.catalog.photo(p).unwrap().meta.regions;
    assert_eq!(regions[1].apart.len(), 1, "kept by the catalogue across a reopen");
    assert!(regions[0].apart.is_empty());
    // never in a sidecar or a file: the library's other files say nothing of it
    for entry in std::fs::read_dir(&d).unwrap().flatten() {
        if entry.path().extension().is_some_and(|e| e.eq_ignore_ascii_case("xmp")) {
            assert!(!std::fs::read_to_string(entry.path()).unwrap_or_default().contains("apart"));
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}
