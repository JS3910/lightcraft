//! Recognition: indexing faces, suggesting names, naming, with a tiny working network standing in for a real
//! model (its embedding depends on the picture's colours, which is enough to test the machinery; how well real
//! models recognise people is measured separately, on real photos).

use std::path::PathBuf;

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_geom::Rect;
use lightcraft_meta::{Region, RegionKind};
use serde_json::{Value, json};

use crate::Session;

pub(crate) fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-facerec-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Seed CPU-only settings before installing models in a fresh test folder. These tests cover catalog and
/// model logic; GPU kernels have separate tests. Calling the settings mutation command here would also
/// detach the process-wide detector accelerator, interfering with other tests in the same process.
pub(crate) fn keep_on_cpu(s: &mut Session) {
    let dir = s.face_models_dir.as_ref().unwrap();
    let settings = dir.join("settings.json");
    assert!(!settings.exists(), "only seed a fresh fixture's settings");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(settings, br#"{"cpuOnly":true}"#).unwrap();
    let r = s.execute("faces.gpu", &json!({})).unwrap();
    assert_eq!(r["cpuOnly"], true, "the test session must run face models on the CPU: {r}");
}

/// A demo session with a tiny model installed and chosen.
pub(crate) fn setup(d: &std::path::Path, dim: u64) -> (Session, String) {
    let mut s = Session::with_demo();
    s.face_models_dir = Some(d.join("models"));
    keep_on_cpu(&mut s);
    let model = d.join(format!("tiny{dim}.onnx"));
    std::fs::write(&model, lightcraft_faces::synthetic::tiny_embedder_model(dim)).unwrap();
    let r = s.execute("faces.models.install", &json!({"path": model.to_string_lossy(), "acknowledged": true})).unwrap();
    let id = r["installed"]["id"].as_str().unwrap().to_string();
    s.execute("faces.models.select", &json!({"id": id})).unwrap();
    (s, id)
}

/// A fresh empty library with the tiny model installed and chosen. The tests below only need a decodable
/// picture (and enough pixels for an optional real detector), not the demo's 12 procedural 24 MP scenes:
/// each demo photo costs a full develop render before the 112 px model input.
fn fresh_library(d: &std::path::Path, dim: u64) -> (Session, PathBuf) {
    let lib = d.join("lib");
    let mut s = Session::new().with_fs();
    s.face_models_dir = Some(d.join("models"));
    keep_on_cpu(&mut s);
    s.open_library(&lib, false).unwrap();
    let model = d.join(format!("tiny{dim}.onnx"));
    std::fs::write(&model, lightcraft_faces::synthetic::tiny_embedder_model(dim)).unwrap();
    s.execute("faces.models.install", &json!({"path": model.to_string_lossy(), "acknowledged": true})).unwrap();
    (s, lib)
}

/// A real, distinct 1024×768 PNG (seed picks its colours).
pub(crate) fn write_photo(path: &std::path::Path, seed: u8) {
    let (w, h) = (1024usize, 768usize);
    let data: Vec<[u8; 4]> = (0..w * h)
        .map(|i| [((i % w * 3 + seed as usize * 17) % 256) as u8, ((i / w * 5 + seed as usize * 29) % 256) as u8, seed.wrapping_mul(41), 255])
        .collect();
    let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// Import three distinct real file-backed photos: two for faces and one with none for the optional detector.
fn import_three_photos(s: &mut Session, d: &std::path::Path) -> (PhotoId, PhotoId, PhotoId) {
    let mut paths = Vec::new();
    for (i, name) in ["ann.png", "bea.png", "empty.png"].iter().enumerate() {
        let path = d.join("photos").join(name);
        write_photo(&path, i as u8 + 1);
        paths.push(path.to_string_lossy().into_owned());
    }
    let r = s.execute("library.import", &json!({"paths": paths})).unwrap();
    let ids: Vec<PhotoId> = r["imported"].as_array().unwrap().iter().map(|v| PhotoId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 3, "three real photos imported: {r}");
    (ids[0], ids[1], ids[2])
}

/// Installs the real YuNet from `LC_YUNET_MODEL=<the .onnx file>` (what Settings > Faces downloads; nothing is
/// committed) and says whether it did: the parts of a test that need to find faces run only then.
pub(crate) fn install_yunet(s: &mut Session) -> bool {
    let Some(file) = std::env::var_os("LC_YUNET_MODEL") else {
        lightcraft_testkit::skip!(Model, "LC_YUNET_MODEL is not set, so the face-finding part is not checked");
        return false;
    };
    let r = s.execute("faces.models.install", &json!({"path": file.to_string_lossy(), "acknowledged": true}));
    assert!(r.is_ok(), "{r:?}");
    true
}

pub(crate) fn region(x: f64, name: Option<&str>) -> Region {
    Region {
        rect: Rect { x0: x, y0: 0.2, x1: x + 0.25, y1: 0.6 },
        kind: RegionKind::Face,
        name: name.map(str::to_string),
        description: None,
        ..Default::default()
    }
}

pub(crate) fn set_regions(s: &mut Session, id: PhotoId, regions: Vec<Region>) {
    let mut meta = s.catalog.photo(id).unwrap().meta.clone();
    meta.regions = regions;
    s.commit("setup", Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
}

pub(crate) fn two_photos(s: &Session) -> (PhotoId, PhotoId) {
    let mut ids = s.catalog.photos().map(|p| p.id).collect::<Vec<_>>();
    ids.sort();
    (ids[0], ids[1])
}

#[test]
fn indexing_embeds_each_face_once_and_reports_progress() {
    let d = temp("index");
    let (mut s, _) = fresh_library(&d, 64);
    let (a, b, c) = import_three_photos(&mut s, &d);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.5, None)]);
    set_regions(&mut s, b, vec![region(0.3, None)]);
    // only a report
    let r = s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    assert_eq!((r["embedded"].clone(), r["photosDone"].clone(), r["pendingPhotos"].clone()), (json!(0), json!(0), json!(2)), "{r}");
    // `faces.index` stops between whole photos once the budget is spent, so follow its documented contract:
    // call again until `pendingPhotos` is 0. Two photos need at most two productive calls; the named photo is
    // first in the queue, and every face is embedded exactly once.
    let mut embedded = 0u64;
    for call in 0..2 {
        let r = s.execute("faces.index", &json!({"budgetMs": 60_000})).unwrap();
        embedded += r["embedded"].as_u64().unwrap();
        if call == 0 {
            assert!(s.faces.index.contains(a.0, &region(0.1, Some("Ann")).rect), "the named photo is done first: {r}");
            assert!(s.faces.index.contains(a.0, &region(0.5, None).rect), "both of its faces are embedded: {r}");
        }
        if r["pendingPhotos"] == 0 {
            break;
        }
    }
    let r = s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    assert_eq!((r["pendingPhotos"].clone(), r["indexedFaces"].clone()), (json!(0), json!(3)), "{r}");
    assert_eq!(embedded, 3, "each of the three faces was embedded exactly once across calls");
    // nothing is embedded twice
    let r = s.execute("faces.index", &json!({"budgetMs": 60_000})).unwrap();
    assert_eq!((r["embedded"].clone(), r["photosDone"].clone()), (json!(0), json!(0)));
    // a small budget still makes progress (at least one photo per call), and `ids` go first
    set_regions(&mut s, c, vec![region(0.2, None)]);
    set_regions(&mut s, b, vec![region(0.3, None), region(0.6, None)]);
    let r = s.execute("faces.index", &json!({"budgetMs": 1, "ids": [c.0]})).unwrap();
    assert_eq!(r["photosDone"], 1, "{r}");
    assert_eq!(r["pendingPhotos"], 1, "{r}");
    assert!(s.faces.index.contains(c.0, &region(0.2, None).rect), "the photo asked for first was done first");
    // keep the imported files until the session that references them is gone
    drop(s);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn suggestions_come_from_the_named_faces_and_are_never_applied() {
    let d = temp("suggest");
    // Gallery and candidate semantics need real embeddings, not full 24 MP demo renders.
    let (mut s, _) = fresh_library(&d, 64);
    let (a, b, _) = import_three_photos(&mut s, &d);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann"))]);
    set_regions(&mut s, b, vec![region(0.1, None), region(0.5, Some("Pet?")), region(0.7, None)]);
    // thresholds that accept anything: Ann is the only person with a named face except "Pet?"
    let r = s.execute("faces.suggest", &json!({"ids": [b.0], "threshold": -1.0, "margin": -2.0, "budgetMs": 60_000})).unwrap();
    assert_eq!(r["galleryFaces"], 2, "{r}");
    let faces = r["photos"][0]["faces"].as_array().unwrap();
    assert_eq!(faces.len(), 2, "only the unnamed faces get suggestions: {r}");
    assert!(faces.iter().all(|f| f["suggestion"]["name"].is_string()), "{r}");
    assert_eq!(faces[0]["index"], 0);
    assert_eq!(faces[1]["index"], 2);
    assert!(faces[0]["candidates"].as_array().unwrap().len() == 2);
    // an impossible threshold suggests nobody but still lists the candidates
    let r = s.execute("faces.suggest", &json!({"ids": [b.0], "threshold": 2.0, "budgetMs": 60_000})).unwrap();
    assert!(r["photos"][0]["faces"][0]["suggestion"].is_null());
    assert!(!r["photos"][0]["faces"][0]["candidates"].as_array().unwrap().is_empty());
    // nothing was named by suggesting
    assert!(s.catalog.photo(b).unwrap().meta.regions.iter().filter(|r| r.name.is_none()).count() == 2);
    // pets and other kinds are not people
    let mut meta = s.catalog.photo(b).unwrap().meta.clone();
    meta.regions[1].kind = RegionKind::Pet;
    s.commit("pet", Op::SetMeta { id: b, meta: Box::new(meta) }).unwrap();
    let r = s.execute("faces.suggest", &json!({"ids": [b.0], "threshold": -1.0, "margin": -2.0, "budgetMs": 60_000})).unwrap();
    assert_eq!(r["galleryFaces"], 1, "{r}");
    // odd parameters are tolerated
    for p in [json!({"threshold": "x"}), json!({"margin": null}), json!({"budgetMs": 0}), json!({"ids": []}), json!({"ids": [999999]})] {
        assert!(s.execute("faces.suggest", &p).is_ok(), "{p}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn naming_a_face_is_undoable_and_makes_it_yours() {
    let d = temp("name");
    let (mut s, _) = setup(&d, 64);
    let (a, _) = two_photos(&s);
    let mut detected = region(0.1, None);
    detected.description = Some("Detected by YuNet 2023mar".into());
    set_regions(&mut s, a, vec![detected, region(0.5, None)]);
    let undo = s.undo.len();
    let r = s.execute("faces.setName", &json!({"id": a.0, "index": 0, "name": "  Ann  "})).unwrap();
    assert_eq!(r["name"], "Ann");
    let regions = s.catalog.photo(a).unwrap().meta.regions.clone();
    assert_eq!((regions[0].name.as_deref(), regions[0].description.as_deref()), (Some("Ann"), None), "a named detection loses its detected marker");
    assert_eq!(s.undo.len(), undo + 1);
    // a new detection run now leaves it alone
    if install_yunet(&mut s) {
        s.execute("faces.detect", &json!({"id": a.0})).unwrap();
        assert_eq!(s.catalog.photo(a).unwrap().meta.regions.iter().filter(|r| r.name.as_deref() == Some("Ann")).count(), 1);
    }
    // clearing, undo
    s.execute("faces.setName", &json!({"id": a.0, "index": 0, "name": null})).unwrap();
    assert_eq!(s.catalog.photo(a).unwrap().meta.regions[0].name, None);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.catalog.photo(a).unwrap().meta.regions[0].name.as_deref(), Some("Ann"));
    // bad input
    for p in [
        json!({"id": a.0}),
        json!({"id": a.0, "index": 9, "name": "x"}),
        json!({"id": a.0, "index": 0, "name": 5}),
        json!({"id": a.0, "index": 0, "name": "a\u{7}b"}),
        json!({"id": a.0, "index": 0, "name": "x".repeat(201)}),
        json!({"id": 999999, "index": 0, "name": "x"}),
    ] {
        assert!(s.execute("faces.setName", &p).is_err(), "{p}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn embeddings_are_cached_beside_the_library_and_reset_for_another_model() {
    let d = temp("persist");
    let lib = d.join("lib");
    let models = d.join("models");
    let install = |s: &mut Session, dim: u64| -> String {
        let model = d.join(format!("tiny{dim}.onnx"));
        std::fs::write(&model, lightcraft_faces::synthetic::tiny_embedder_model(dim)).unwrap();
        let r = s.execute("faces.models.install", &json!({"path": model.to_string_lossy(), "acknowledged": true})).unwrap();
        let id = r["installed"]["id"].as_str().unwrap().to_string();
        s.execute("faces.models.select", &json!({"id": id})).unwrap();
        id
    };
    let mut s = Session::new().with_fs();
    s.face_models_dir = Some(models.clone());
    keep_on_cpu(&mut s);
    s.open_library(&lib, true).unwrap();
    let (a, _) = two_photos(&s);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.5, None)]);
    install(&mut s, 64);
    s.execute("faces.index", &json!({"budgetMs": 60_000})).unwrap();
    assert!(lib.join(format!("face-embeddings-{}.bin", s.faces.index.tag.split('@').next().unwrap())).is_file());
    s.close_library().unwrap();
    drop(s);

    // a new session finds the embeddings on disk and has nothing left to do
    let mut s2 = Session::new().with_fs();
    s2.face_models_dir = Some(models);
    s2.open_library(&lib, true).unwrap();
    let r = s2.execute("faces.index", &json!({"budgetMs": 60_000})).unwrap();
    assert_eq!((r["embedded"].clone(), r["indexedFaces"].clone(), r["pendingPhotos"].clone()), (json!(0), json!(2), json!(0)), "{r}");
    // choosing another model starts over: its faces are different numbers, the old ones are not used
    install(&mut s2, 32);
    let r = s2.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    assert_eq!((r["indexedFaces"].clone(), r["pendingPhotos"].clone()), (json!(0), json!(1)), "{r}");
    let r = s2.execute("faces.index", &json!({"budgetMs": 60_000})).unwrap();
    assert_eq!(r["embedded"], 2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn without_a_chosen_model_recognition_says_so() {
    let d = temp("nomodel");
    let mut s = Session::with_demo();
    assert!(s.execute("faces.index", &json!({})).is_err(), "no models folder");
    s.face_models_dir = Some(d.join("models"));
    for cmd in ["faces.index", "faces.suggest"] {
        let e = s.execute(cmd, &json!({})).unwrap_err().to_string();
        assert!(e.contains("no recognition model is chosen"), "{cmd}: {e}");
    }
    let _: Value = json!(null);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_background_pump_indexes_by_itself_once_recognition_is_on() {
    let d = temp("pump");
    // Exercise real background embedding without unrelated full 24 MP demo renders.
    let (mut s, _) = fresh_library(&d, 64);
    let (a, b, _) = import_three_photos(&mut s, &d);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.5, None)]);
    set_regions(&mut s, b, vec![region(0.3, None)]);
    // installing the model switched recognition on; switched off, the pump does nothing, and says so
    assert_eq!(s.execute("faces.enable", &json!({"enabled": false})).unwrap()["enabled"], false);
    assert_eq!(s.execute("faces.pump", &json!({})).unwrap()["active"], false);
    assert_eq!(s.faces.index.len(), 0);
    s.execute("faces.enable", &json!({"enabled": true})).unwrap();
    // `enabled` is looked up at most once a second, so the first pump may still see "off"
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut last = json!(null);
    while std::time::Instant::now() < deadline {
        last = s.execute("faces.pump", &json!({})).unwrap();
        if last["active"] == true && last["pendingPhotos"] == 0 && last["indexedFaces"] == 3 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!((last["active"].clone(), last["pendingPhotos"].clone(), last["indexedFaces"].clone()), (json!(true), json!(0), json!(3)), "{last}");
    // a new face is picked up (the catalog changed), and suggestions use what the pump embedded
    set_regions(&mut s, b, vec![region(0.3, None), region(0.6, None)]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline && s.faces.index.len() < 4 {
        s.execute("faces.pump", &json!({})).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(s.faces.index.len(), 4);
    let r = s.execute("faces.suggest", &json!({"ids": [b.0], "threshold": -1.0, "margin": -2.0, "budgetMs": 0})).unwrap();
    assert_eq!(r["photos"][0]["faces"].as_array().unwrap().len(), 2);
    assert!(r["photos"][0]["faces"][0]["suggestion"]["name"].is_string(), "{r}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Three photos with one unnamed face each, the album "Set" holding only the last: the grid shows the album.
fn three_faces_and_a_view_of_the_last(tag: &str) -> (Session, [PhotoId; 3], std::path::PathBuf) {
    let d = temp(tag);
    let (mut s, _) = fresh_library(&d, 64);
    let (a, b, c) = import_three_photos(&mut s, &d);
    let mut ids = [a, b, c];
    ids.sort();
    for id in ids {
        set_regions(&mut s, id, vec![region(0.3, None)]);
    }
    let album = s.execute("album.create", &json!({"name": "Set"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": album, "ids": [ids[2].0]})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    (s, ids, d)
}

#[test]
fn with_a_small_time_budget_the_photos_of_the_view_are_embedded_before_the_others() {
    let (mut s, [a, b, c], d) = three_faces_and_a_view_of_the_last("budget-view");
    let rect = region(0.3, None).rect;
    // one photo fits in the budget: it is the album's, though it is the last in the library
    let r = s.execute("faces.index", &json!({"budgetMs": 1})).unwrap();
    assert_eq!(r["photosDone"], 1, "{r}");
    assert!(s.faces.index.contains(c.0, &rect) && !s.faces.index.contains(a.0, &rect) && !s.faces.index.contains(b.0, &rect), "{r}");
    // the next calls carry on with the rest, and the whole library is done in the end
    let r = s.execute("faces.index", &json!({"budgetMs": 600_000})).unwrap();
    assert_eq!((r["pendingPhotos"].clone(), r["indexedFaces"].clone()), (json!(0), json!(3)), "{r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_background_pump_embeds_the_photos_of_the_view_first() {
    let (mut s, [a, b, c], d) = three_faces_and_a_view_of_the_last("pump-view");
    let rect = region(0.3, None).rect;
    s.execute("faces.enable", &json!({"enabled": true})).unwrap();
    // one photo at a time, so the first face to land is the first photo the pump took
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline && s.faces.index.len() == 0 {
        s.execute("faces.pump", &json!({"pace": "light"})).unwrap();
    }
    assert_eq!(s.faces.index.len(), 1, "the first photo is embedded");
    assert!(s.faces.index.contains(c.0, &rect), "and it is the one the grid shows");
    // changing what the grid shows changes what the pump takes next: the rest is done, whatever the source
    s.execute("library.source", &json!({"kind": "all"})).unwrap();
    while std::time::Instant::now() < deadline && s.faces.index.len() < 3 {
        s.execute("faces.pump", &json!({"pace": "light"})).unwrap();
    }
    assert!(s.faces.index.contains(a.0, &rect) && s.faces.index.contains(b.0, &rect));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_background_pump_saves_what_it_embeds_for_the_next_launch() {
    let d = temp("pumpsave");
    // Installing the tiny model switches recognition on: nothing else is asked for.
    let (mut s, lib) = fresh_library(&d, 64);
    // without the detector (not bundled) photos without faces are left unsearched
    let searching = install_yunet(&mut s);
    let (a, b, c) = import_three_photos(&mut s, &d);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.5, None)]);
    set_regions(&mut s, b, vec![region(0.3, None)]);
    assert!(s.catalog.photo(c).unwrap().meta.regions.is_empty(), "the third photo has no face to find");
    let photos = s.catalog.photos().count();
    assert_eq!(photos, 3, "a fresh library holds only the three imported photos");
    // only the one photo without a face region is searched when the optional detector is installed
    let searched_photos = usize::from(searching);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let (mut last, mut embedded) = (json!(null), 0u64);
    while std::time::Instant::now() < deadline {
        last = s.execute("faces.pump", &json!({})).unwrap();
        embedded += last["embedded"].as_u64().unwrap_or(0);
        if last["pendingPhotos"] == 0 && s.faces.index.len() == 3 && s.faces.scanned.len() == searched_photos {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!((s.faces.index.len(), s.faces.scanned.len()), (3, searched_photos), "{last}");
    assert_eq!(embedded, 3, "the background worker embedded each face exactly once: {last}");
    // once everything is done another pump starts no work and reports no new embedding
    let idle = s.execute("faces.pump", &json!({})).unwrap();
    assert_eq!((idle["embedded"].clone(), idle["inFlight"].clone(), idle["pendingPhotos"].clone()), (json!(0), json!(0), json!(0)), "{idle}");
    let (tag, path) = (s.faces.index.tag.clone(), s.faces.index.path.clone().unwrap());
    assert!(path.file_name().unwrap().to_string_lossy().starts_with("face-embeddings-custom-"), "one cache file per model: {}", path.display());
    // the first batch is written at once; whatever is left is written when the session ends, so a launch never redoes
    // work: neither the embeddings nor the list of photos already searched for faces
    s.close_library().unwrap();
    drop(s);
    let mut again = crate::faces_index::Index::default();
    again.reset(&tag, 64, Some(path));
    assert_eq!(again.len(), 3);
    let mut searched = crate::faces_index::Scanned::default();
    searched.reset(Some(lib.join("face-scanned.bin")));
    assert_eq!(searched.len(), searched_photos, "every photo without faces is remembered as searched");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn faces_person_lists_a_persons_faces_and_looks_for_more_only_while_recognition_runs() {
    let d = temp("person");
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let (a, b) = two_photos(&s);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.5, Some("Bob"))]);
    set_regions(&mut s, b, vec![region(0.1, None), region(0.5, Some("ann"))]);
    let unit = |parts: &[(usize, f32)]| {
        let mut v = vec![0.0f32; 64];
        for (i, x) in parts {
            v[*i] = *x;
        }
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect::<Vec<f32>>()
    };
    for (id, x, v) in [(a, 0.1, unit(&[(0, 1.0)])), (a, 0.5, unit(&[(1, 1.0)])), (b, 0.1, unit(&[(0, 0.9), (1, 0.1)])), (b, 0.5, unit(&[(0, 1.0)]))] {
        s.faces.index.insert(id.0, &region(x, None).rect, v);
    }
    // the name is matched without regard to case or surrounding spaces; every face is listed, wherever it is
    let r = s.execute("faces.person", &json!({"name": "  ANN "})).unwrap();
    assert_eq!((r["name"].clone(), r["total"].clone()), (json!("Ann"), json!(2)));
    let mut at: Vec<(u64, u64)> =
        r["confirmed"].as_array().unwrap().iter().map(|f| (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap())).collect();
    at.sort();
    let mut want = vec![(a.0, 0), (b.0, 1)];
    want.sort();
    assert_eq!(at, want);
    assert!(r["confirmed"].as_array().unwrap().iter().any(|f| f["rect"]["x0"] == 0.1 && f["rect"]["y1"] == 0.6));
    // recognition is on (installing the model switched it on): the unnamed face that looks like Ann is offered
    assert_eq!(r["ready"], true);
    let more = r["more"].as_array().unwrap();
    assert_eq!((more.len(), more[0]["photo"].clone(), more[0]["index"].clone()), (1, json!(b.0), json!(0)), "{r}");
    assert!(more[0]["score"].as_f64().unwrap() > 0.9);
    // confirming it moves it from one list to the other (and is one undo step)
    s.execute("faces.setName", &json!({"id": b.0, "index": 0, "name": "Ann"})).unwrap();
    let r = s.execute("faces.person", &json!({"name": "Ann"})).unwrap();
    assert_eq!((r["total"].clone(), r["more"].as_array().unwrap().len()), (json!(3), 0));
    // switched off, the faces are still listed but nothing is searched for
    s.execute("faces.enable", &json!({"enabled": false})).unwrap();
    let r = s.execute("faces.person", &json!({"name": "Ann"})).unwrap();
    assert_eq!((r["total"].clone(), r["ready"].clone(), r["more"].as_array().unwrap().len()), (json!(3), json!(false), 0));
    // nobody by that name, and bad input, are plain answers and errors
    let r = s.execute("faces.person", &json!({"name": "Nobody"})).unwrap();
    assert_eq!((r["total"].clone(), r["confirmed"].as_array().unwrap().len(), r["ready"].clone()), (json!(0), 0, json!(false)));
    for bad in [json!({}), json!({"name": ""}), json!({"name": "   "}), json!({"name": 7})] {
        assert!(s.execute("faces.person", &bad).is_err(), "{bad}");
    }
    assert!(s.execute("faces.person", &json!({"name": "Ann", "more": u64::MAX})).is_ok());
    let _ = std::fs::remove_dir_all(&d);
}

pub(crate) fn unit64(parts: &[(usize, f32)]) -> Vec<f32> {
    let mut v = vec![0.0f32; 64];
    for (i, x) in parts {
        v[*i] = *x;
    }
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / n).collect()
}

#[test]
fn many_faces_are_named_in_one_undo_step() {
    let d = temp("namemany");
    let (mut s, _) = setup(&d, 64);
    let (a, b) = two_photos(&s);
    let detected = |x: f64| Region { description: Some("Detected by YuNet 2023mar".into()), ..region(x, None) };
    set_regions(&mut s, a, vec![detected(0.1), region(0.5, None)]);
    set_regions(&mut s, b, vec![detected(0.3)]);
    let undo = s.undo.len();
    let r = s
        .execute(
            "faces.nameFaces",
            &json!({"faces": [{"photo": a.0, "index": 0}, {"photo": a.0, "index": 1}, {"photo": b.0, "index": 0}], "name": "  Jane Doe "}),
        )
        .unwrap();
    assert_eq!((r["named"].clone(), r["photos"].clone()), (json!(3), json!(2)));
    assert_eq!(s.undo.len(), undo + 1, "one step for the whole group");
    for (id, i) in [(a, 0), (a, 1), (b, 0)] {
        let reg = &s.catalog.photo(id).unwrap().meta.regions[i];
        assert_eq!(reg.name.as_deref(), Some("Jane Doe"));
        assert!(reg.description.is_none() || !reg.description.as_deref().unwrap().starts_with("Detected by"), "a named face is the user's now");
    }
    // undo restores every one of them together
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.catalog.photo(a).unwrap().meta.regions.iter().chain(s.catalog.photo(b).unwrap().meta.regions.iter()).all(|r| r.name.is_none()));
    // an empty name clears; faces that do not exist are skipped, and with none left it is an error
    s.execute("faces.nameFaces", &json!({"faces": [{"photo": a.0, "index": 0}], "name": "Ann"})).unwrap();
    let r = s
        .execute(
            "faces.nameFaces",
            &json!({"faces": [{"photo": a.0, "index": 0}, {"photo": a.0, "index": 99}, {"photo": 987_654, "index": 0}], "name": null}),
        )
        .unwrap();
    assert_eq!(r["named"], 1);
    assert!(s.catalog.photo(a).unwrap().meta.regions[0].name.is_none());
    for bad in [
        json!({"name": "x"}),
        json!({"faces": "x", "name": "x"}),
        json!({"faces": [{"photo": a.0}], "name": "x"}),
        json!({"faces": [{"photo": 987_654, "index": 0}], "name": "x"}),
        json!({"faces": [{"photo": a.0, "index": 0}], "name": 7}),
        json!({"faces": [{"photo": a.0, "index": 0}], "name": "bad\u{7}name"}),
        json!({"faces": vec![json!({"photo": a.0, "index": 0}); 5001], "name": "x"}),
    ] {
        assert!(s.execute("faces.nameFaces", &bad).is_err(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn unnamed_faces_come_with_look_alikes_together_and_the_name_the_named_ones_suggest() {
    let d = temp("unnamed");
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let (a, b) = two_photos(&s);
    let c = s.catalog.photos().map(|p| p.id).find(|id| *id != a && *id != b).unwrap();
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.5, None), region(0.7, None)]);
    set_regions(&mut s, b, vec![region(0.1, None), region(0.5, None)]);
    set_regions(&mut s, c, vec![region(0.3, None)]);
    // three faces like Ann, two like Bob (unnamed), one unembedded
    let put = |s: &mut Session, id: lightcraft_catalog::PhotoId, x: f64, v: Vec<f32>| s.faces.index.insert(id.0, &region(x, None).rect, v);
    put(&mut s, a, 0.1, unit64(&[(0, 1.0)])); // Ann, named
    put(&mut s, a, 0.5, unit64(&[(1, 1.0)])); // like Bob
    put(&mut s, b, 0.1, unit64(&[(0, 0.97), (2, 0.2)])); // like Ann
    put(&mut s, b, 0.5, unit64(&[(1, 0.6), (2, 0.8)])); // like Bob, but not enough to stack
    put(&mut s, c, 0.3, unit64(&[(0, 0.95), (3, 0.3)])); // like Ann
    // (a's last face, at 0.7, is not embedded yet)
    let r = s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!((r["total"].clone(), r["ordered"].clone(), r["ready"].clone()), (json!(5), json!(true), json!(true)), "{r}");
    let faces = r["faces"].as_array().unwrap();
    let at = super::face_stacks::face_at;
    let order: Vec<(u64, u64)> = faces.iter().map(at).collect();
    // look-alikes are neighbours: the two like Bob are adjacent, and the three like Ann are adjacent; the unembedded one is last
    let pos = |x: (u64, u64)| order.iter().position(|o| *o == x).unwrap();
    assert_eq!(pos((a.0, 1)).abs_diff(pos((b.0, 1))), 1, "{order:?}");
    assert_eq!(order.last(), Some(&(a.0, 2)), "{order:?}");
    // a face the named ones recognise carries the name they suggest
    let like_ann = faces.iter().find(|f| at(f) == (b.0, 0)).unwrap();
    assert_eq!(like_ann["suggestion"]["name"], "Ann", "{like_ann}");
    assert!(faces.iter().find(|f| at(f) == (b.0, 1)).unwrap()["suggestion"].is_null(), "Bob is nobody the named faces know");
    // each face says which box to show it by; with no scan view it is its own box
    for k in ["x0", "y0", "x1", "y1"] {
        assert!((like_ann["view"][k].as_f64().unwrap() - like_ann["rect"][k].as_f64().unwrap()).abs() < 1e-3, "{like_ann}");
    }
    // a limit cuts the list, not the count
    let r = s.execute("faces.unnamed", &json!({"limit": 2})).unwrap();
    assert_eq!((r["total"].clone(), r["faces"].as_array().unwrap().len()), (json!(5), 2));
    // switched off, the faces are still listed, in photo order, without suggestions
    s.execute("faces.enable", &json!({"enabled": false})).unwrap();
    let r = s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!((r["total"].clone(), r["ordered"].clone(), r["ready"].clone()), (json!(5), json!(false), json!(false)));
    assert!(r["faces"].as_array().unwrap().iter().all(|f| f["suggestion"].is_null()));
    for odd in [json!({"limit": 0}), json!({"limit": "x"}), json!({"limit": u64::MAX})] {
        assert!(s.execute("faces.unnamed", &odd).is_ok(), "{odd}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn unnamed_faces_put_the_suggested_ones_first_by_score_then_look_alikes_then_the_unembedded() {
    let d = temp("unnamed-order");
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let (a, b) = two_photos(&s);
    let c = s.catalog.photos().map(|p| p.id).find(|id| *id != a && *id != b).unwrap();
    // a: Ann, Bob (named), a face like Bob, an unembedded face, two strangers
    set_regions(
        &mut s,
        a,
        vec![region(0.1, Some("Ann")), region(0.2, Some("Bob")), region(0.3, None), region(0.4, None), region(0.5, None), region(0.6, None)],
    );
    // b: a face very like Ann, a stranger, an unembedded face, another stranger
    set_regions(&mut s, b, vec![region(0.1, None), region(0.2, None), region(0.3, None), region(0.4, None)]);
    // c: a face fairly like Ann, a face like Bob scoring the same as the one in a (a tie)
    set_regions(&mut s, c, vec![region(0.1, None), region(0.2, None)]);
    let put = |s: &mut Session, id: lightcraft_catalog::PhotoId, x: f64, v: Vec<f32>| s.faces.index.insert(id.0, &region(x, None).rect, v);
    put(&mut s, a, 0.1, unit64(&[(0, 1.0)])); // Ann
    put(&mut s, a, 0.2, unit64(&[(1, 1.0)])); // Bob
    put(&mut s, a, 0.3, unit64(&[(1, 0.97), (3, 0.2)])); // like Bob
    put(&mut s, a, 0.5, unit64(&[(5, 1.0)])); // a stranger
    put(&mut s, a, 0.6, unit64(&[(6, 1.0)])); // another kind of stranger
    put(&mut s, b, 0.1, unit64(&[(0, 0.99), (2, 0.1)])); // very like Ann
    put(&mut s, b, 0.2, unit64(&[(5, 0.6), (8, 0.8)])); // looks like the first stranger, not enough to stack
    put(&mut s, b, 0.4, unit64(&[(6, 0.6), (9, 0.8)])); // looks like the other stranger, not enough to stack
    put(&mut s, c, 0.1, unit64(&[(0, 0.7), (4, 0.71)])); // fairly like Ann, not like the face of b (no stack)
    put(&mut s, c, 0.2, unit64(&[(1, 0.97), (3, 0.2)])); // like Bob, as much as a's: the two are one stack, shown by the earlier photo's
    let r = s.execute("faces.unnamed", &json!({})).unwrap();
    let faces = r["faces"].as_array().unwrap();
    let at = super::face_stacks::face_at;
    let order: Vec<(u64, u64)> = faces.iter().map(at).collect();
    // suggested faces first, the most confident first, with Ann's and Bob's mixed; the two faces like Bob, as much as each other
    // and alike, are one stack, shown by the earlier photo's (the tie)
    let (first, second) = if a.0 < c.0 { ((a.0, 2), (c.0, 1)) } else { ((c.0, 1), (a.0, 2)) };
    assert_eq!(&order[..3], &[(b.0, 0), first, (c.0, 0)], "{order:?}");
    assert_eq!(faces[1]["stack"]["faces"].as_array().map(Vec::len), Some(2), "{}", faces[1]);
    assert!(faces[1]["stack"]["faces"].as_array().unwrap().iter().any(|f| at(f) == second), "{}", faces[1]);
    let names: Vec<&str> = faces[..3].iter().map(|f| f["suggestion"]["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Ann", "Bob", "Ann"]);
    let scores: Vec<f64> = faces[..3].iter().map(|f| f["suggestion"]["score"].as_f64().unwrap()).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "{scores:?}");
    // then the faces nobody is suggested for, look-alikes together; the unembedded ones last, in photo order
    let rest = &order[3..];
    assert!(faces[3..].iter().all(|f| f["suggestion"].is_null()));
    let pos = |x: (u64, u64)| rest.iter().position(|o| *o == x).unwrap();
    assert_eq!(pos((a.0, 4)).abs_diff(pos((b.0, 1))), 1, "{rest:?}");
    assert_eq!(pos((a.0, 5)).abs_diff(pos((b.0, 3))), 1, "{rest:?}");
    let mut unembedded = [(a.0, 3), (b.0, 2)];
    unembedded.sort();
    assert_eq!(&rest[rest.len() - 2..], &unembedded[..], "{rest:?}");
    // a limit still cuts from the front, so the likeliest are what is kept
    let r = s.execute("faces.unnamed", &json!({"limit": 2})).unwrap();
    let kept: Vec<(u64, u64)> = r["faces"].as_array().unwrap().iter().map(at).collect();
    assert_eq!(kept, vec![(b.0, 0), first]);
    let _ = std::fs::remove_dir_all(&d);
}
#[test]
fn without_a_limit_the_unnamed_faces_are_listed_up_to_the_most_the_command_gives() {
    let d = temp("unnamed-many");
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.enable", &json!({"enabled": false})).unwrap();
    let (a, _) = two_photos(&s);
    set_regions(&mut s, a, (0..700).map(|i| region(0.001 * f64::from(i), None)).collect());
    let r = s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!((r["total"].clone(), r["faces"].as_array().unwrap().len()), (json!(700), 700), "more than the 600 it once stopped at");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn every_scanned_face_is_shown_by_the_same_kind_of_box() {
    let d = temp("view");
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let (a, _) = two_photos(&s);
    // a loosely drawn box (from some other tool) and the detector's tighter box of the same face
    let loose = lightcraft_geom::Rect { x0: 0.2, y0: 0.1, x1: 0.8, y1: 0.9 };
    let tight = lightcraft_geom::Rect { x0: 0.35, y0: 0.25, x1: 0.65, y1: 0.65 };
    let mut meta = s.catalog.photo(a).unwrap().meta.clone();
    meta.regions = vec![Region { rect: loose, kind: RegionKind::Face, name: Some("Ann".into()), description: None, ..Default::default() }];
    s.commit("setup", Op::SetMeta { id: a, meta: Box::new(meta) }).unwrap();
    // before the scan has looked at it, the face is shown by its own box
    assert_eq!(s.face_view(a, loose), loose);
    s.faces.index.insert_with_view(a.0, &loose, unit64(&[(0, 1.0)]), &tight);
    let shown = s.face_view(a, loose);
    assert!((shown.x0 - tight.x0).abs() < 1e-3 && (shown.y1 - tight.y1).abs() < 1e-3, "shown by the detector's box once known: {shown:?}");
    // a face nobody has looked at, or in another photo, is shown by its own box
    let other = lightcraft_geom::Rect { x0: 0.1, y0: 0.1, x1: 0.2, y1: 0.2 };
    assert_eq!(s.face_view(a, other), other);
    // the page of the person uses it too
    let r = s.execute("faces.person", &json!({"name": "Ann"})).unwrap();
    let view = &r["confirmed"][0]["view"];
    assert!((view["x0"].as_f64().unwrap() - tight.x0).abs() < 1e-3, "{r}");
    // and it survives a restart: the index file keeps it beside the embedding
    let (tag, path) = (s.faces.index.tag.clone(), d.join("view-cache.bin"));
    let mut written = crate::faces_index::Index::default();
    written.reset(&tag, 64, Some(path.clone()));
    written.insert_with_view(a.0, &loose, unit64(&[(0, 1.0)]), &tight);
    written.save().unwrap();
    drop(written);
    let mut again = crate::faces_index::Index::default();
    again.reset(&tag, 64, Some(path));
    let v = again.view(a.0, &loose).expect("the face is remembered");
    assert!((v.x0 - tight.x0).abs() < 1e-3 && (v.y0 - tight.y0).abs() < 1e-3, "{v:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn face_scan_row_follows_pending_photos() {
    let d = temp("activity");
    // The activity row needs real background embedding, not full 24 MP demo renders.
    let (mut s, _) = fresh_library(&d, 64);
    let (a, b, _) = import_three_photos(&mut s, &d);
    set_regions(&mut s, a, vec![region(0.1, None), region(0.5, None)]);
    set_regions(&mut s, b, vec![region(0.3, None)]);
    let faces_rows = |s: &Session| s.activity.list().into_iter().filter(|t| t.kind == "faces").collect::<Vec<_>>();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let (mut last, mut shown) = (json!(null), false);
    while std::time::Instant::now() < deadline {
        last = s.execute("faces.pump", &json!({})).unwrap();
        let rows = faces_rows(&s);
        if last["pendingPhotos"].as_u64().unwrap_or(0) > 0 {
            // while photos are left: one row, not cancellable, its whole the most photos left at once
            assert_eq!(rows.len(), 1, "{last}");
            assert_eq!((rows[0].label.as_str(), rows[0].cancellable), ("Finding faces", false));
            assert_eq!(rows[0].total, last["peak"].as_u64().unwrap(), "{last}");
            assert_eq!(rows[0].done, rows[0].total - last["pendingPhotos"].as_u64().unwrap());
            shown = true;
        }
        if last["active"] == true && last["pendingPhotos"] == 0 && last["indexedFaces"] == 3 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(shown, "the scan never showed a row: {last}");
    assert_eq!((last["pendingPhotos"].clone(), last["peak"].clone()), (json!(0), json!(0)), "{last}");
    assert!(faces_rows(&s).is_empty(), "the row goes when the scan is done");
    let _ = std::fs::remove_dir_all(&d);
}

fn face_names(s: &Session, id: PhotoId) -> Vec<Option<String>> {
    s.catalog.photo(id).unwrap().meta.regions.iter().map(|r| r.name.clone()).collect()
}

/// Every in-library spelling of a person becomes the exact new name, including a change of case only,
/// and a person filter follows it. One undo step puts the old names back.
#[test]
fn rename_person_rewrites_every_spelling_and_undoes() {
    let mut s = Session::with_demo();
    let (a, b) = two_photos(&s);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.5, Some("  ANN  ")), region(0.7, Some("Rex"))]);
    let mut pet = region(0.2, Some("Ann"));
    pet.kind = RegionKind::Pet;
    set_regions(&mut s, b, vec![region(0.3, Some("ann")), pet]);
    s.execute("library.filter", &json!({"person": "ANN"})).unwrap();
    let undo = s.undo.len();
    let r = s.execute("faces.renamePerson", &json!({"from": "  ann ", "to": "  Ada  "})).unwrap();
    assert_eq!((r["faces"].clone(), r["photos"].clone(), r["to"].clone(), r["merged"].clone()), (json!(3), json!(2), json!("Ada"), json!(false)));
    assert_eq!(face_names(&s, a), vec![Some("Ada".into()), Some("Ada".into()), Some("Rex".into())]);
    assert_eq!(face_names(&s, b)[0], Some("Ada".into()));
    assert_eq!(face_names(&s, b)[1], Some("Ann".into()), "a pet is not a person");
    assert_eq!(s.catalog.people().iter().find(|p| p.name.eq_ignore_ascii_case("ada")).map(|p| p.name.as_str()), Some("Ada"));
    assert_eq!(s.filter.person.as_deref(), Some("Ada"));
    assert_eq!(s.undo.len(), undo + 1);
    assert_eq!(s.undo.last().map(|u| u.label.as_str()), Some("Rename Person"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(face_names(&s, a), vec![Some("Ann".into()), Some("  ANN  ".into()), Some("Rex".into())]);
    assert_eq!(face_names(&s, b)[0], Some("ann".into()));
}

/// Renaming onto someone else is refused until merge is asked for; then one undo step restores both people.
#[test]
fn rename_person_refuses_a_merge_until_it_is_asked_for() {
    let mut s = Session::with_demo();
    let (a, b) = two_photos(&s);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann"))]);
    set_regions(&mut s, b, vec![region(0.3, Some("Bea"))]);
    let before = (face_names(&s, a), face_names(&s, b), s.undo.len());
    let err = s.execute("faces.renamePerson", &json!({"from": "Ann", "to": "Bea"})).unwrap_err();
    assert!(err.to_string().contains("already another person"), "{err}");
    assert_eq!((face_names(&s, a), face_names(&s, b), s.undo.len()), before, "a refusal changes nothing");
    let r = s.execute("faces.renamePerson", &json!({"from": "Ann", "to": "Bea", "merge": true})).unwrap();
    assert_eq!(r["merged"], true);
    assert_eq!(s.undo.last().map(|u| u.label.as_str()), Some("Merge People"));
    let names: Vec<String> = s.catalog.people().into_iter().map(|p| p.name).collect();
    assert_eq!(names, vec!["Bea".to_string()]);
    s.execute("edit.undo", &json!({})).unwrap();
    let mut names: Vec<String> = s.catalog.people().into_iter().map(|p| p.name).collect();
    names.sort();
    assert_eq!(names, vec!["Ann".to_string(), "Bea".to_string()]);
}

/// More faces than `faces.person` lists are still renamed. A bad call, a missing person, or a name that
/// is already exact changes nothing, and deleted or only-browsed photos are not part of the person.
#[test]
fn rename_person_covers_more_than_the_person_query_and_rejects_bad_input() {
    let mut s = Session::with_demo();
    let (a, b) = two_photos(&s);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann"))]);
    let undo = s.undo.len();
    for p in [
        json!({}),
        json!({"from": "Ann"}),
        json!({"from": "  ", "to": "Ada"}),
        json!({"from": 1, "to": "Ada"}),
        json!({"from": "Ann", "to": ""}),
        json!({"from": "Ann", "to": 5}),
        json!({"from": "Ann", "to": "a\u{7}b"}),
        json!({"from": "Ann", "to": "x".repeat(201)}),
        json!({"from": "Nobody", "to": "Ada"}),
        json!({"from": "Ann", "to": "Ann"}),
    ] {
        let err = s.execute("faces.renamePerson", &p).unwrap_err();
        assert!(s.undo.len() == undo, "{p} changed the catalog: {err}");
    }
    assert_eq!(face_names(&s, a), vec![Some("Ann".into())]);
    let regions: Vec<_> = (0..5001).map(|i| region(0.1, Some(if i == 0 { "ANN" } else { "Ann" }))).collect();
    set_regions(&mut s, a, regions);
    set_regions(&mut s, b, vec![region(0.2, Some("Ann"))]);
    s.commit("delete", Op::SetDeleted { id: b, deleted: true }).unwrap();
    let listed = s.execute("faces.person", &json!({"name": "Ann"})).unwrap();
    assert_eq!((listed["total"].clone(), listed["confirmed"].as_array().unwrap().len()), (json!(5001), 5000));
    let r = s.execute("faces.renamePerson", &json!({"from": "ann", "to": "Ada"})).unwrap();
    assert_eq!((r["faces"].clone(), r["photos"].clone()), (json!(5001), json!(1)), "the deleted photo is left alone");
    assert!(s.catalog.photo(a).unwrap().meta.regions.iter().all(|r| r.name.as_deref() == Some("Ada")));
    assert_eq!(s.catalog.photo(b).unwrap().meta.regions[0].name.as_deref(), Some("Ann"));
    let local = s.catalog.photos().map(|p| p.id).find(|id| *id != a && *id != b).unwrap();
    set_regions(&mut s, local, vec![region(0.1, Some("Ada"))]);
    s.commit("browse", Op::SetLocal { id: local, local: true }).unwrap();
    let r = s.execute("faces.renamePerson", &json!({"from": "Ada", "to": "Bea"})).unwrap();
    assert_eq!(r["photos"], 1, "the browsed photo is not another person and is not renamed");
    assert_eq!(s.catalog.photo(a).unwrap().meta.regions[0].name.as_deref(), Some("Bea"));
    assert_eq!(s.catalog.photo(local).unwrap().meta.regions[0].name.as_deref(), Some("Ada"));
}

#[test]
fn many_faces_are_named_each_with_its_own_name_in_one_undo_step() {
    let d = temp("nameeach");
    let (mut s, _) = setup(&d, 64);
    let (a, b) = two_photos(&s);
    set_regions(&mut s, a, vec![region(0.1, None), region(0.5, None)]);
    set_regions(&mut s, b, vec![region(0.3, Some("Old"))]);
    let undo = s.undo.len();
    let r = s
        .execute(
            "faces.nameFaces",
            &json!({"faces": [
                {"photo": a.0, "index": 0, "name": " Ann "},
                {"photo": a.0, "index": 1},
                {"photo": b.0, "index": 0, "name": "Bob"},
            ], "name": "Cy"}),
        )
        .unwrap();
    assert_eq!(r["named"], 3);
    assert_eq!(s.undo.len(), undo + 1, "one step for the whole list");
    let name = |s: &Session, id, i: usize| s.catalog.photo(id).unwrap().meta.regions[i].name.clone();
    assert_eq!(name(&s, a, 0).as_deref(), Some("Ann"));
    assert_eq!(name(&s, a, 1).as_deref(), Some("Cy"), "an entry without a name takes the default");
    assert_eq!(name(&s, b, 0).as_deref(), Some("Bob"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((name(&s, a, 0), name(&s, a, 1), name(&s, b, 0).as_deref().map(str::to_string)), (None, None, Some("Old".to_string())));
    // an entry with an empty or null name and no default clears it; a name each, no default needed
    s.execute("faces.nameFaces", &json!({"faces": [{"photo": a.0, "index": 0, "name": "Ann"}, {"photo": b.0, "index": 0, "name": ""}]})).unwrap();
    assert_eq!((name(&s, a, 0).as_deref().map(str::to_string), name(&s, b, 0)), (Some("Ann".to_string()), None));
    // hostile input is an error and changes nothing
    let undo = s.undo.len();
    for bad in [
        json!({"faces": [{"photo": a.0, "index": 0, "name": 7}]}),
        json!({"faces": [{"photo": a.0, "index": 0, "name": "bad\u{7}"}]}),
        json!({"faces": [{"photo": a.0, "index": 0, "name": "x".repeat(201)}]}),
        json!({"faces": [{"photo": a.0, "index": 1, "name": "Ok"}, {"photo": a.0, "index": 0, "name": ["x"]}]}),
        json!({"faces": [{"photo": 987_654, "index": 0, "name": "x"}, {"photo": a.0, "index": 99, "name": "x"}]}),
        json!({"faces": [{"photo": a.0, "index": -1, "name": "x"}]}),
        json!({"faces": vec![json!({"photo": a.0, "index": 0, "name": "x"}); 5001]}),
    ] {
        assert!(s.execute("faces.nameFaces", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.undo.len(), undo);
    assert_eq!(name(&s, a, 1), None);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_ranking_of_one_face_lists_every_named_person_best_first_and_never_embeds() {
    let d = temp("rank");
    let (mut s, _) = fresh_library(&d, 64);
    let (a, b, c) = import_three_photos(&mut s, &d);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.5, Some("Bob"))]);
    set_regions(&mut s, c, vec![region(0.1, Some("Cy")), region(0.5, Some("Dee"))]);
    set_regions(&mut s, b, vec![region(0.1, None), region(0.5, Some("Ann"))]);
    // nothing is embedded yet: the ranking is empty, and asking does not start any embedding
    let r = s.execute("faces.rank", &json!({"id": b.0, "index": 0})).unwrap();
    assert_eq!(r["ranking"], json!([]), "{r}");
    assert_eq!(s.faces.index.len(), 0, "a ranking never embeds");
    s.execute("faces.index", &json!({"budgetMs": 60_000})).unwrap();
    let r = s.execute("faces.rank", &json!({"id": b.0, "index": 0})).unwrap();
    let ranking = r["ranking"].as_array().unwrap();
    let names: Vec<&str> = ranking.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 4, "every named person, once: {r}");
    for n in ["Ann", "Bob", "Cy", "Dee"] {
        assert!(names.contains(&n), "{n} missing: {r}");
    }
    let scores: Vec<f64> = ranking.iter().map(|x| x["score"].as_f64().unwrap()).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "best first: {r}");
    // the closest few are exactly what suggestions report
    let sg = s.execute("faces.suggest", &json!({"ids": [b.0], "budgetMs": 0})).unwrap();
    let top3: Vec<(String, f64)> = sg["photos"][0]["faces"][0]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x["name"].as_str().unwrap().to_string(), x["score"].as_f64().unwrap()))
        .collect();
    let mine: Vec<(String, f64)> = ranking.iter().take(3).map(|x| (x["name"].as_str().unwrap().to_string(), x["score"].as_f64().unwrap())).collect();
    assert_eq!(mine, top3);
    // a named face is ranked without itself in the gallery
    let r = s.execute("faces.rank", &json!({"id": b.0, "index": 1})).unwrap();
    assert!(r["ranking"].as_array().unwrap().iter().all(|x| x["score"].as_f64().unwrap() < 0.9999), "a face is not compared with itself: {r}");
    // a face the scan has not embedded has no ranking
    set_regions(&mut s, b, vec![region(0.1, None), region(0.5, Some("Ann")), region(0.3, None)]);
    let r = s.execute("faces.rank", &json!({"id": b.0, "index": 2})).unwrap();
    assert_eq!(r["ranking"], json!([]), "{r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_ranking_query_refuses_hostile_arguments_without_panicking() {
    let d = temp("rankbad");
    let (mut s, _) = fresh_library(&d, 64);
    let (a, _, _) = import_three_photos(&mut s, &d);
    set_regions(&mut s, a, vec![region(0.1, None)]);
    for p in [
        json!({"id": 999_999, "index": 0}),
        json!({"id": a.0, "index": 1}),
        json!({"id": a.0, "index": u64::MAX}),
        json!({"id": a.0, "index": -1}),
        json!({"id": a.0}),
        json!({"id": "x", "index": 0}),
        json!({"index": "0"}),
    ] {
        assert!(s.execute("faces.rank", &p).is_err(), "{p}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// `faces.evaluate {scope: "visible"}` draws from the People view's photos like every other `scope: "visible"` face
/// command: the hidden photos of a collapsed stack are in it, though the grid shows only the stack's top photo.
#[test]
fn evaluating_the_visible_photos_covers_the_hidden_photos_of_a_collapsed_stack() {
    let d = temp("evaluate-stack");
    let (mut s, _) = fresh_library(&d, 64);
    let (a, b, c) = import_three_photos(&mut s, &d);
    set_regions(&mut s, a, vec![region(0.1, Some("Ann"))]);
    set_regions(&mut s, b, vec![region(0.3, Some("Bea"))]);
    s.selection = crate::Selection::single(a);
    s.execute("stack.group", &json!({"ids": [a.0, b.0], "top": a.0, "collapsed": true})).unwrap();
    assert!(!s.visible().contains(&b), "the grid shows one photo for the stack");
    let r = s.execute("faces.evaluate", &json!({"budgetMs": 0, "scope": "visible"})).unwrap();
    assert_eq!(r["pendingPhotos"], 2, "the stack's hidden photo is evaluated too: {r}");
    let _ = c;
}

/// The entry of a `faces.unnamed` answer that lists the face `(photo, index)`: the face itself, or the stack that holds it (faces
/// that share a suggestion, or are alike without one, are listed inside one entry).
pub(crate) fn entry_listing(r: &Value, photo: u64, index: u64) -> Option<&Value> {
    r["faces"].as_array()?.iter().find(|f| {
        (f["photo"] == photo && f["index"] == index)
            || f["stack"]["faces"].as_array().is_some_and(|m| m.iter().any(|x| super::face_stacks::face_at(x) == (photo, index)))
    })
}
