//! The Unnamed faces are asked for without waiting for the order (`faces.unnamed` with `wait: false`): the window asks again
//! when the engine took in a result of the thread that puts them in order, not on every frame, and with no list on screen
//! and an order that is not final it shows a status instead of tiles. The engine is the real one, with a tiny recognition
//! model and a run that a test can hold.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lightcraft_catalog::PhotoId;
use serde_json::json;

use super::unnamed_tiles::{people, region, set_regions, step};
use crate::headless::Headless;
use crate::panels::unnamed::{Unnamed, faces};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn widgets(h: &mut Headless, filter: &str) -> usize {
    let r = h.request("ui.widgets", json!({"filter": filter}), T);
    r["result"].as_array().map_or(0, Vec::len)
}

#[test]
fn the_list_is_asked_again_when_a_result_was_taken_in_and_not_on_every_frame() {
    let (mut h, _) = people();
    let first = faces(&mut h.app, 0.0);
    for _ in 0..10 {
        step(&mut h);
    }
    assert!(Arc::ptr_eq(&first, &faces(&mut h.app, 0.1)), "nothing changed: the same list, not asked again");
    h.app.caches.faces_arranged += 1;
    let second = faces(&mut h.app, 0.2);
    assert!(!Arc::ptr_eq(&first, &second), "a result was taken in: asked again");
    assert!(Arc::ptr_eq(&second, &faces(&mut h.app, 0.3)), "and only once for it");
}

/// A unit vector of 64 numbers with the given parts (the dimension of the tiny test model).
fn unit(parts: &[(usize, f32)]) -> Vec<f32> {
    let mut v = vec![0.0f32; 64];
    for (i, x) in parts {
        v[*i] = *x;
    }
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / n).collect()
}

/// [`people`] with face recognition on (a tiny model) and every face embedded, as a scan would leave them: photo 0 has Ann Lee
/// and a face that looks like her, photo 1 two faces and photo 2 one face that look like nobody named. The People view is not
/// open yet. The runs that put the unnamed faces in order are held while the returned flag is set.
fn recognising(name: &str) -> (Headless, [PhotoId; 3], Arc<AtomicBool>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("lc-unnamed-ordering-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("models")).unwrap();
    std::fs::write(dir.join("models").join("settings.json"), br#"{"cpuOnly":true}"#).unwrap();
    let mut session = lightcraft_engine::Session::with_demo();
    session.face_models_dir = Some(dir.join("models"));
    let model = dir.join("tiny64.onnx");
    std::fs::write(&model, lightcraft_faces::synthetic::tiny_embedder_model(64)).unwrap();
    let installed = session.execute("faces.models.install", &json!({"path": model.to_string_lossy(), "acknowledged": true})).unwrap();
    session.execute("faces.models.select", &json!({"id": installed["installed"]["id"]})).unwrap();
    session.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let hold = Arc::new(AtomicBool::new(false));
    session.test_hold_face_order(Some(hold.clone()));
    let services = Services { png: None, write_shared: Some(Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let mut h = Headless::new(LightcraftApp::new(session, services), [1400.0, 900.0], 1.0);
    h.settle(T);
    let ids: Vec<PhotoId> = h.app.session.catalog.photos().map(|p| p.id).take(3).collect();
    set_regions(&mut h, ids[0], vec![region(0.1, Some("Ann Lee")), region(0.5, None)]);
    set_regions(&mut h, ids[1], vec![region(0.3, None), region(0.6, None)]);
    set_regions(&mut h, ids[2], vec![region(0.3, None)]);
    let put = |h: &mut Headless, id: PhotoId, x: f64, v: Vec<f32>| h.app.session.test_face_embedding(id, &region(x, None).rect, v);
    put(&mut h, ids[0], 0.1, unit(&[(0, 1.0)]));
    put(&mut h, ids[0], 0.5, unit(&[(0, 0.97), (3, 0.2)]));
    put(&mut h, ids[1], 0.3, unit(&[(5, 1.0)]));
    put(&mut h, ids[1], 0.6, unit(&[(5, 0.99), (7, 0.1)]));
    put(&mut h, ids[2], 0.3, unit(&[(6, 1.0)]));
    (h, [ids[0], ids[1], ids[2]], hold, dir)
}

/// The faces of the list on screen, in order, as (photo, index).
fn shown(h: &Headless) -> Vec<(u64, usize)> {
    h.app.caches.unnamed.as_ref().map_or_else(Vec::new, |(_, _, _, l)| l.faces.iter().map(|f| (f.photo, f.index)).collect())
}

/// Wait (without drawing frames) until the run that was held has ended: its row is gone and its result is sent.
fn run_ended(h: &mut Headless) {
    let until = Instant::now() + Duration::from_secs(30);
    while Instant::now() < until {
        let rows = h.app.session.execute("activity.list", &json!({})).unwrap();
        if rows["tasks"].as_array().unwrap().iter().all(|t| t["kind"] != "faceOrder") {
            std::thread::sleep(Duration::from_millis(50));
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the run never ended");
}

/// Draw frames, with no input, until `done`: how many it took (at most `most`).
fn frames_until(h: &mut Headless, most: usize, done: impl Fn(&Headless) -> bool) -> Option<usize> {
    (0..most).find(|_| {
        h.step();
        done(h)
    })
}

#[test]
fn opening_the_view_shows_a_status_until_the_order_is_complete_then_the_tiles_without_any_input() {
    let (mut h, [p0, _, _], hold, dir) = recognising("open");
    hold.store(true, Ordering::Relaxed);
    h.request("ui.set", json!({"view": "people"}), T);
    for _ in 0..6 {
        h.step();
    }
    assert_eq!(widgets(&mut h, "unnamed-face:"), 0, "no tiles that would jump when the order comes in");
    assert_eq!(widgets(&mut h, "unnamed:status"), 1, "a status instead");
    assert_eq!(widgets(&mut h, "unnamed:selectAll"), 0);
    let list = h.app.caches.unnamed.as_ref().map(|(_, _, _, l)| l.clone()).expect("the list was asked for");
    assert!(list.ordering && list.faces.is_empty() && list.total == 4, "{list:?}");
    // the run ends. Nothing is touched; the window takes the result in by itself, within a few frames (it looks every 50 ms
    // while an order is being worked out, not every second or every five)
    hold.store(false, Ordering::Relaxed);
    run_ended(&mut h);
    let frames = frames_until(&mut h, 20, |h| !shown(h).is_empty());
    assert!(frames.is_some(), "the tiles never replaced the status ({} frames)", 20);
    h.settle(T);
    assert_eq!(h.app.caches.unnamed_arranged, h.app.caches.faces_arranged, "the list is the one for the result that came in");
    let order = shown(&h);
    assert_eq!(order.len(), 4);
    assert_eq!(order[0], (p0.0, 1), "the face that looks like Ann Lee comes first: {order:?}");
    assert!(widgets(&mut h, "unnamed-face:") > 0 && widgets(&mut h, "unnamed:status") == 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_naming_while_the_order_is_worked_out_removes_the_tile_and_moves_the_rest_up_and_the_result_is_merged() {
    let (mut h, [p0, p1, _], hold, dir) = recognising("naming");
    h.request("ui.set", json!({"view": "people"}), T);
    assert!(frames_until(&mut h, 400, |h| h.app.caches.unnamed.as_ref().is_some_and(|(_, _, _, l)| !l.faces.is_empty() && !l.ordering)).is_some());
    h.settle(T);
    let before = shown(&h);
    assert_eq!(before.len(), 4);
    // a face is named while the next run is held: its tile goes at once and the tiles after it each move up one place
    hold.store(true, Ordering::Relaxed);
    h.app.session.execute("faces.setName", &json!({"id": p1.0, "index": 0, "name": "Ann Lee"})).unwrap();
    for _ in 0..6 {
        h.step();
    }
    let mut expect = before.clone();
    expect.retain(|k| *k != (p1.0, 0));
    assert_eq!(shown(&h), expect, "the tile left and nothing else moved");
    assert!(
        h.app.caches.unnamed.as_ref().is_some_and(|(_, _, _, l)| l.ordering && !l.faces.is_empty()),
        "the best order there is, still being worked out"
    );
    assert_eq!(widgets(&mut h, "unnamed:status"), 0, "tiles are on screen: no status");
    hold.store(false, Ordering::Relaxed);
    run_ended(&mut h);
    assert!(
        frames_until(&mut h, 20, |h| h.app.caches.unnamed.as_ref().is_some_and(|(_, _, _, l)| !l.ordering)).is_some(),
        "the result was not taken in"
    );
    h.settle(T);
    assert_eq!(h.app.caches.unnamed_arranged, h.app.caches.faces_arranged);
    let mut now = shown(&h);
    assert_eq!(now.len(), 3, "{now:?}");
    now.sort();
    expect.sort();
    assert_eq!(now, expect, "the same faces");
    assert!(shown(&h).contains(&(p0.0, 1)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_no_list_on_screen_and_an_order_that_is_not_final_the_section_shows_no_tiles() {
    let (mut h, _) = people();
    assert!(widgets(&mut h, "unnamed-face:") > 0, "tiles when the order is final");
    // a list that is only a status (what the first answer is while the order is worked out and nothing is on screen)
    let waiting = Unnamed { total: 4, ordering: true, ready: true, ..Default::default() };
    let (rev, indexed) = (h.app.session.catalog.revision, h.app.caches.faces_indexed);
    h.app.caches.unnamed = Some((rev, indexed, f64::MAX, Arc::new(waiting)));
    step(&mut h);
    assert_eq!(widgets(&mut h, "unnamed-face:"), 0);
    assert_eq!(widgets(&mut h, "unnamed:selectAll"), 0);
    assert_eq!(widgets(&mut h, "unnamed:status"), 1, "the status is drawn");
}
