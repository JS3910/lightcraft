//! The Unnamed faces are put in order on a thread of their own: `faces.unnamed` with `wait: false` answers at once with the
//! best order it has and says the rest is being worked out; `faces.pump` takes the result in; a naming costs what it
//! changed, never a rescore of everything; and the result is always what scoring every face against every person gives.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lightcraft_catalog::PhotoId;
use lightcraft_faces::matching;
use serde_json::{Value, json};

use super::face_recognize::{region, set_regions, setup, temp, two_photos, unit64};
use crate::Session;
use crate::faces_arrange::Fail;

/// Photo `a` holds Ann and Bob (named) and a face like Ann; photo `b` a face like Ann and a face like nobody; photo `c` two
/// faces that look like each other (but not enough to be one stack) and like nobody named. Every face is embedded.
fn library(name: &str) -> (Session, std::path::PathBuf, [PhotoId; 3]) {
    let d = temp(name);
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let (a, b) = two_photos(&s);
    let c = s.catalog.photos().map(|p| p.id).filter(|id| *id != a && *id != b).min().unwrap();
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.4, Some("Bob")), region(0.65, None)]);
    set_regions(&mut s, b, vec![region(0.1, None), region(0.5, None)]);
    set_regions(&mut s, c, vec![region(0.1, None), region(0.5, None)]);
    let mut put = |id: PhotoId, x: f64, v: Vec<f32>| s.faces.index.insert(id.0, &region(x, None).rect, v);
    put(a, 0.1, unit64(&[(0, 1.0)])); // Ann
    put(a, 0.4, unit64(&[(1, 1.0)])); // Bob
    put(a, 0.65, unit64(&[(0, 0.97), (3, 0.2)])); // like Ann
    put(b, 0.1, unit64(&[(0, 0.7), (4, 0.71)])); // less like Ann, and not like the face before: no stack
    put(b, 0.5, unit64(&[(5, 1.0)])); // like nobody
    put(c, 0.1, unit64(&[(6, 1.0)])); // like nobody
    put(c, 0.5, unit64(&[(6, 0.6), (7, 0.8)])); // like the one before, but not nearly enough to stack
    (s, d, [a, b, c])
}

fn ask(s: &mut Session, wait: bool) -> Value {
    s.execute("faces.unnamed", &json!({"wait": wait})).unwrap()
}

fn keys(r: &Value) -> Vec<(u64, u64)> {
    r["faces"].as_array().unwrap().iter().map(|f| (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap())).collect()
}

fn suggestion(r: &Value, photo: PhotoId, index: u64) -> Option<String> {
    let f = super::face_recognize::entry_listing(r, photo.0, index)?;
    f["suggestion"]["name"].as_str().map(str::to_string)
}

/// How many results were taken in so far: read it before the call that starts the run, so that a result taken in by that very
/// call (or by a `faces.pump` before the next one) is not waited for.
fn taken(s: &Session) -> u64 {
    s.faces.arrange.taken
}

/// Call `faces.pump` until the count of results taken in is not `was` any more, and say what it is now.
fn take_in(s: &mut Session, was: u64) -> u64 {
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        let now = s.execute("faces.pump", &json!({})).unwrap()["arranged"].as_u64().unwrap();
        if now != was {
            return now;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("no result was taken in within a minute");
}

/// Ask without waiting until the answer says the order is complete.
fn settled(s: &mut Session) -> Value {
    for _ in 0..20 {
        let was = taken(s);
        let r = ask(s, false);
        if r["complete"] == true {
            assert_eq!((r["ordering"].clone(), r["stopped"].clone()), (json!(false), json!(false)), "{r}");
            return r;
        }
        assert_eq!(r["ordering"], true, "an order that is not complete is being worked out (or the run stopped): {r}");
        take_in(s, was);
    }
    panic!("the order never settled");
}

fn rows(s: &mut Session) -> usize {
    s.execute("activity.list", &json!({})).unwrap()["tasks"].as_array().unwrap().iter().filter(|t| t["kind"] == "faceOrder").count()
}

/// Wait until no `faceOrder` row is left (the run's thread has ended).
fn row_gone(s: &mut Session) -> bool {
    for _ in 0..3000 {
        if rows(s) == 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn without_waiting_the_faces_come_at_once_and_the_order_is_taken_in_when_it_is_worked_out() {
    let (mut s, d, [a, b, c]) = library("bg-first");
    let r = ask(&mut s, false);
    assert_eq!(r["ordering"], true, "{r}");
    assert_eq!(r["total"], 5);
    assert_eq!((r["complete"].clone(), r["stopped"].clone()), (json!(false), json!(false)), "{r}");
    assert_eq!(
        (r["lookAlike"].clone(), r["ordered"].clone()),
        (json!(false), json!(false)),
        "faces not scored yet are in no order but photo order: {r}"
    );
    assert_eq!(keys(&r), vec![(a.0, 2), (b.0, 0), (b.0, 1), (c.0, 0), (c.0, 1)], "nothing worked out yet: the faces in photo order");
    assert!(r["faces"].as_array().unwrap().iter().all(|f| f["suggestion"].is_null()), "{r}");
    let r = settled(&mut s);
    assert_eq!((r["ordering"].clone(), r["complete"].clone()), (json!(false), json!(true)));
    let order = keys(&r);
    assert_eq!(&order[..2], &[(a.0, 2), (b.0, 0)], "the faces Ann is suggested for come first: {order:?}");
    assert_eq!(suggestion(&r, a, 2).as_deref(), Some("Ann"));
    assert_eq!(order[2], (b.0, 1), "then the faces nobody is suggested for, look-alikes together: {order:?}");
    assert_eq!(order[3..].iter().copied().collect::<std::collections::BTreeSet<_>>(), [(c.0, 0), (c.0, 1)].into_iter().collect());
    assert_eq!(r["lookAlike"], true);
    // what a caller that waits is given is the same
    let waited = s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!(keys(&waited), order);
    assert_eq!(waited["ordering"], false);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn wait_must_be_true_or_false() {
    let (mut s, d, _) = library("bg-wait-param");
    for bad in [json!("no"), json!("false"), json!(0), json!([])] {
        let e = s.execute("faces.unnamed", &json!({"wait": bad})).unwrap_err();
        assert!(e.to_string().contains("`wait` must be true or false"), "{bad}: {e}");
    }
    assert_eq!(rows(&mut s), 0, "a refused call starts nothing");
    assert_eq!(ask(&mut s, true)["ordering"], false);
    assert_eq!(s.execute("faces.unnamed", &json!({"wait": null})).unwrap()["ordering"], false, "null is the default");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_naming_takes_the_face_out_at_once_and_the_rest_keep_what_was_worked_out() {
    let (mut s, d, [a, b, _]) = library("bg-naming");
    let before = settled(&mut s);
    assert_eq!(suggestion(&before, b, 0).as_deref(), Some("Ann"));
    let order = keys(&before);
    s.execute("faces.setName", &json!({"id": a.0, "index": 2, "name": "Ann"})).unwrap();
    let now = ask(&mut s, false);
    assert_eq!(now["ordering"], true, "the new named face has not been worked into the scores yet");
    let expect: Vec<(u64, u64)> = order.iter().copied().filter(|k| *k != (a.0, 2)).collect();
    assert_eq!(keys(&now), expect, "the named face is gone and the tiles after it moved up one place; nothing else moved");
    assert_eq!(suggestion(&now, b, 0).as_deref(), Some("Ann"), "what was worked out is kept");
    let done = settled(&mut s);
    assert_eq!(keys(&done), keys(&s.execute("faces.unnamed", &json!({})).unwrap()));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_face_whose_rejections_changed_never_shows_its_old_suggestion() {
    let (mut s, d, [a, b, _]) = library("bg-reject");
    let before = settled(&mut s);
    assert_eq!((suggestion(&before, a, 2).as_deref(), suggestion(&before, b, 0).as_deref()), (Some("Ann"), Some("Ann")));
    s.execute("faces.reject", &json!({"faces": [{"photo": b.0, "index": 0}], "name": "Ann"})).unwrap();
    let now = ask(&mut s, false);
    assert_eq!(now["ordering"], true);
    assert_eq!(suggestion(&now, b, 0), None, "no old suggestion for it");
    assert_eq!(suggestion(&now, a, 2).as_deref(), Some("Ann"), "the others keep theirs");
    assert!(!keys(&now)[..1].contains(&(b.0, 0)), "and it is not among the suggested faces");
    let done = settled(&mut s);
    assert_eq!(suggestion(&done, b, 0), None);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn removing_a_box_renumbers_the_faces_after_it_and_they_keep_what_was_worked_out() {
    let (mut s, d, [a, b, c]) = library("bg-remove");
    let before = settled(&mut s);
    assert_eq!(keys(&before)[..3], [(a.0, 2), (b.0, 0), (b.0, 1)]);
    let hold = Arc::new(AtomicBool::new(true));
    s.faces.arrange.hold = Some(hold.clone());
    // the first face of photo b goes: the second becomes region 0 of that photo
    s.execute("faces.removeBoxes", &json!({"faces": [{"photo": b.0, "index": 0}]})).unwrap();
    let now = ask(&mut s, false);
    assert_eq!(now["ordering"], true);
    let mut expect = keys(&before);
    expect.retain(|k| *k != (b.0, 0));
    for k in expect.iter_mut().filter(|k| k.0 == b.0) {
        k.1 = 0;
    }
    assert_eq!(keys(&now), expect, "the faces after it keep their places under their new numbers: {now}");
    assert_eq!(suggestion(&now, a, 2).as_deref(), Some("Ann"), "a face of another photo keeps its suggestion");
    assert_eq!(suggestion(&now, b, 0), None, "the face that took the number does not take the suggestion of the face that had it");
    assert_eq!(keys(&now).iter().filter(|k| k.0 == c.0).count(), 2);
    hold.store(false, Ordering::Relaxed);
    let was = taken(&s);
    take_in(&mut s, was);
    let done = settled(&mut s);
    assert_eq!(keys(&done), expect);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_job_holds_an_activity_row_and_a_result_for_other_inputs_is_not_the_complete_order() {
    let (mut s, d, [a, b, _]) = library("bg-row");
    let hold = Arc::new(AtomicBool::new(true));
    s.faces.arrange.hold = Some(hold.clone());
    let r = ask(&mut s, false);
    assert_eq!(r["ordering"], true);
    let tasks = s.execute("activity.list", &json!({})).unwrap();
    let row = tasks["tasks"].as_array().unwrap().iter().find(|t| t["kind"] == "faceOrder").cloned();
    let row = row.unwrap_or_else(|| panic!("a row for the job: {tasks}"));
    assert_eq!(row["cancellable"], true);
    // the named faces change while it runs
    s.execute("faces.setName", &json!({"id": a.0, "index": 2, "name": "Bob"})).unwrap();
    assert_eq!(ask(&mut s, false)["ordering"], true);
    let was = taken(&s);
    hold.store(false, Ordering::Relaxed);
    take_in(&mut s, was);
    let r = ask(&mut s, false);
    assert_eq!(r["ordering"], true, "the result was for the old named faces: a new run starts for these");
    let done = settled(&mut s);
    let waited = s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!(keys(&done), keys(&waited));
    assert_eq!(suggestion(&done, b, 0), suggestion(&waited, b, 0));
    assert!(row_gone(&mut s), "the row goes with the job");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn cancelling_the_row_stops_the_job_and_nothing_of_it_is_applied() {
    let (mut s, d, [a, _, _]) = library("bg-cancel");
    let hold = Arc::new(AtomicBool::new(true));
    s.faces.arrange.hold = Some(hold);
    assert_eq!(ask(&mut s, false)["ordering"], true);
    let tasks = s.execute("activity.list", &json!({})).unwrap();
    let id = tasks["tasks"].as_array().unwrap().iter().find(|t| t["kind"] == "faceOrder").unwrap()["id"].as_u64().unwrap();
    s.execute("activity.cancel", &json!({"id": id})).unwrap();
    // the job ends (the row goes before its result is announced, so the pump may find nothing for a moment); its result is dropped
    let mut ended = false;
    for _ in 0..3000 {
        if s.execute("faces.pump", &json!({})).unwrap()["ordering"] == false {
            ended = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(ended, "the cancelled run never ended");
    let r = ask(&mut s, false);
    assert_eq!(r["ordering"], false, "a cancelled run is not started again by itself: {r}");
    assert_eq!((r["complete"].clone(), r["stopped"].clone()), (json!(false), json!(true)), "the reply says the order was stopped, not final: {r}");
    assert_eq!(rows(&mut s), 0);
    assert_eq!(suggestion(&r, a, 2), None);
    // asking to restart starts it again
    s.faces.arrange.hold = None;
    let r = s.execute("faces.unnamed", &json!({"wait": false, "restart": true})).unwrap();
    assert_eq!((r["ordering"].clone(), r["stopped"].clone()), (json!(true), json!(false)), "{r}");
    let done = settled(&mut s);
    assert_eq!(suggestion(&done, a, 2).as_deref(), Some("Ann"));
    // a caller that waits still gets a complete order
    let waited = s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!(suggestion(&waited, a, 2).as_deref(), Some("Ann"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_call_that_waits_is_given_an_error_when_its_run_is_cancelled_dies_or_cannot_start() {
    let (mut s, d, [a, _, _]) = library("bg-fail");
    // the row is cancelled during the call: it must not start another run
    s.faces.arrange.fail = Some(Fail::Cancel);
    let e = s.execute("faces.unnamed", &json!({})).unwrap_err().to_string();
    assert!(e.contains("cancelled"), "{e}");
    assert!(row_gone(&mut s));
    // the thread panics
    s.faces.arrange.fail = Some(Fail::Panic);
    let e = s.execute("faces.unnamed", &json!({})).unwrap_err().to_string();
    assert!(e.contains("stopped"), "{e}");
    assert!(row_gone(&mut s), "a thread that died leaves no row");
    // no thread can be started
    s.faces.arrange.fail = Some(Fail::Spawn);
    let e = s.execute("faces.unnamed", &json!({})).unwrap_err().to_string();
    assert!(e.contains("could not start"), "{e}");
    assert_eq!(rows(&mut s), 0);
    // none of it is remembered against the next call
    s.faces.arrange.fail = None;
    let r = s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!((r["ordering"].clone(), suggestion(&r, a, 2).as_deref().map(str::to_string)), (json!(false), Some("Ann".to_string())));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_thread_that_cannot_be_started_is_a_stop_that_keeps_what_was_kept_and_is_not_tried_on_every_ask() {
    let (mut s, d, [a, _, _]) = library("bg-spawn");
    settled(&mut s);
    s.execute("faces.setName", &json!({"id": a.0, "index": 2, "name": "Ann"})).unwrap();
    s.faces.arrange.fail = Some(Fail::Spawn);
    let r = ask(&mut s, false);
    assert_eq!((r["ordering"].clone(), r["complete"].clone(), r["stopped"].clone()), (json!(false), json!(false), json!(true)), "{r}");
    assert_eq!(rows(&mut s), 0, "no row is left");
    // the failure is gone, but nothing starts by itself for the same inputs
    s.faces.arrange.fail = None;
    let r = ask(&mut s, false);
    assert_eq!((r["ordering"].clone(), r["stopped"].clone()), (json!(false), json!(true)), "not tried again on every ask: {r}");
    assert_eq!(rows(&mut s), 0);
    // restarting works, from what the last run kept (only Ann's change is worked out, for the 4 faces left)
    let r = s.execute("faces.unnamed", &json!({"wait": false, "restart": true})).unwrap();
    assert_eq!(r["ordering"], true, "{r}");
    settled(&mut s);
    assert_eq!(s.faces.arrange.last_scored, 4);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn removing_a_box_in_front_of_a_named_face_scores_nobody_again() {
    let (mut s, d, [_, _, c]) = library("bg-remove-named");
    // photo c: an unnamed face, then Cy (its second face, named)
    set_regions(&mut s, c, vec![region(0.1, None), region(0.5, Some("Cy"))]);
    s.execute("faces.unnamed", &json!({})).unwrap();
    s.execute("faces.removeBoxes", &json!({"faces": [{"photo": c.0, "index": 0}]})).unwrap();
    assert_eq!(s.catalog.photo(c).unwrap().meta.regions.len(), 1, "Cy is region 0 now");
    s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!(s.faces.arrange.last_scored, 0, "Cy's faces did not change, only their region number");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn opening_another_library_leaves_nothing_of_the_old_one() {
    let (mut s, d, [a, _, _]) = library("bg-library");
    let hold = Arc::new(AtomicBool::new(false));
    s.faces.arrange.hold = Some(hold.clone());
    settled(&mut s);
    // a result is kept, then a run is held
    hold.store(true, Ordering::Relaxed);
    s.execute("faces.setName", &json!({"id": a.0, "index": 2, "name": "Bob"})).unwrap();
    assert_eq!(ask(&mut s, false)["ordering"], true);
    let was = taken(&s);
    s.faces.library_changed();
    assert!(s.faces.arrange.is_empty(), "no run, result, state or stop is kept");
    assert_ne!(taken(&s), was, "a window asks again");
    assert!(row_gone(&mut s), "the held run saw its flag and ended");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn closing_the_session_stops_a_running_job() {
    let (mut s, d, _) = library("bg-drop");
    let hold = Arc::new(AtomicBool::new(true));
    s.faces.arrange.hold = Some(hold);
    assert_eq!(ask(&mut s, false)["ordering"], true);
    let activity = s.activity.clone();
    drop(s);
    let mut gone = false;
    for _ in 0..3000 {
        if activity.list().iter().all(|t| t.kind != "faceOrder") {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(gone, "the held run saw its flag when the session went away");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn opening_another_library_stops_a_running_job() {
    let (mut s, d, _) = library("bg-reset");
    let hold = Arc::new(AtomicBool::new(true));
    s.faces.arrange.hold = Some(hold);
    assert_eq!(ask(&mut s, false)["ordering"], true);
    assert_eq!(rows(&mut s), 1);
    s.faces.arrange.reset();
    assert!(!s.faces.arrange.is_running());
    assert!(row_gone(&mut s), "the held run saw its flag and ended");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_change_costs_what_it_changed_and_the_runs_hand_what_they_kept_to_the_next() {
    let (mut s, d, [a, b, _]) = library("bg-cost");
    s.execute("faces.unnamed", &json!({})).unwrap();
    // 5 unnamed faces against Ann and Bob
    assert_eq!((s.faces.arrange.last_scored, s.faces.arrange.chain_reused), (10, false));
    // a naming: only Ann's faces changed, the 4 faces left are scored against her alone, and the faces nobody is suggested for
    // are the same, so their look-alike order is the one before
    s.execute("faces.setName", &json!({"id": a.0, "index": 2, "name": "Ann"})).unwrap();
    s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!((s.faces.arrange.last_scored, s.faces.arrange.chain_reused), (4, true));
    // asking again without a change runs nothing
    let runs = taken(&s);
    s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!((taken(&s), s.faces.arrange.last_scored), (runs, 4));
    // a face whose box moved is scored against everybody (Ann and Bob), the others against nobody
    set_regions(&mut s, b, vec![region(0.1, None), region(0.52, None)]);
    s.faces.index.insert(b.0, &region(0.52, None).rect, unit64(&[(5, 1.0)]));
    s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!((s.faces.arrange.last_scored, s.faces.arrange.chain_reused), (2, false));
    let _ = std::fs::remove_dir_all(&d);
}

/// What scoring every face against every named person from scratch suggests for each unnamed face: the order the
/// suggestion code used before there was a background job, and the figures to compare with.
fn from_scratch(s: &Session, threshold: f32) -> Vec<((u64, usize), Option<(String, f32)>)> {
    let mut gallery: Vec<(String, Arc<[f32]>)> = Vec::new();
    let mut loose = Vec::new();
    for ph in s.catalog.photos().filter(|p| p.in_library()) {
        for (i, r) in ph.meta.regions.iter().enumerate() {
            let Some(e) = s.faces.index.get(ph.id.0, &r.rect) else { continue };
            match &r.name {
                Some(n) => gallery.push((n.clone(), e.clone())),
                None => loose.push((ph.id.0, i, e.clone(), r.rejected.clone())),
            }
        }
    }
    let refs: Vec<(&str, &[f32])> = gallery.iter().map(|(n, e)| (n.as_str(), &e[..])).collect();
    let mut out: Vec<_> = loose
        .into_iter()
        .map(|(p, i, e, rejected)| {
            let keys: Vec<String> = rejected.iter().map(|r| r.trim().to_lowercase()).collect();
            let kept: Vec<(&str, &[f32])> = refs.iter().filter(|(n, _)| !keys.contains(&n.trim().to_lowercase())).copied().collect();
            let x = matching::suggest(&matching::rank(&e, &kept), threshold, 0.06);
            ((p, i), x.map(|x| (x.name, x.score)))
        })
        .collect();
    out.sort_by_key(|(k, _)| *k);
    out
}

/// A small deterministic generator (xorshift64*), one for each seed, so that a failing seed can be run again.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % n.max(1)
    }
}

/// A box moved a little: the region at `index` of `id` is put at another place, with the embedding it had (as the scan would
/// leave it, once it has looked at the new box).
fn move_box(s: &mut Session, id: PhotoId, index: usize) {
    let mut regions = s.catalog.photo(id).unwrap().meta.regions.clone();
    let Some(r) = regions.get_mut(index) else { return };
    let old = r.rect;
    r.rect = lightcraft_geom::Rect { x0: old.x0 + 0.013, y0: old.y0, x1: old.x1 + 0.013, y1: old.y1 };
    let new = r.rect;
    let embedding = s.faces.index.get(id.0, &old).map(|e| e.to_vec());
    set_regions(s, id, regions);
    if let Some(e) = embedding {
        s.faces.index.insert(id.0, &new, e);
    }
}

#[test]
fn after_every_random_change_the_order_is_what_scoring_everyone_again_gives() {
    let d = temp("bg-random");
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let mut ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort();
    let threshold = {
        // the model's bar, as the command reads it
        let r = s.execute("faces.suggest", &json!({"budgetMs": 0})).unwrap();
        r["threshold"].as_f64().unwrap() as f32
    };
    let names = ["Ann", "Bob", "Cy", "Di", "Eve", "Flo"];
    for seed in 0..12u64 {
        let mut rng = Rng::new(seed);
        // a fresh set of faces: each photo gets six faces, a third of them named, embeddings of a few clusters
        for (n, id) in ids.iter().enumerate().take(6) {
            let regions = (0..6).map(|i| region(0.1 * i as f64, (rng.below(3) == 0).then(|| names[rng.below(names.len())]))).collect();
            set_regions(&mut s, *id, regions);
            for i in 0..6 {
                let cluster = rng.below(6);
                let v = unit64(&[(cluster, 1.0), (6 + (seed as usize + n + i) % 50, 0.3 + 0.1 * rng.below(5) as f32)]);
                s.faces.index.insert(id.0, &region(0.1 * i as f64, None).rect, v);
            }
        }
        let mut suggested = 0usize;
        for step in 0..16 {
            let id = ids[rng.below(6)];
            let regions = s.catalog.photo(id).unwrap().meta.regions.clone();
            let index = rng.below(regions.len());
            let named = regions.get(index).is_some_and(|r| r.name.is_some());
            match rng.below(9) {
                0 | 1 if !regions.is_empty() => {
                    let name = names[rng.below(names.len())];
                    let name = if rng.below(2) == 0 { name.to_uppercase() } else { name.to_string() };
                    s.execute("faces.setName", &json!({"id": id.0, "index": index, "name": name})).unwrap();
                }
                2 if !regions.is_empty() => {
                    s.execute("faces.setName", &json!({"id": id.0, "index": index, "name": Value::Null})).unwrap();
                }
                3 if !regions.is_empty() && !named => {
                    let name = names[rng.below(names.len())];
                    s.execute("faces.reject", &json!({"faces": [{"photo": id.0, "index": index}], "name": name})).unwrap();
                }
                4 if !regions.is_empty() => move_box(&mut s, id, index),
                5 if !regions.is_empty() => {
                    s.execute("faces.removeBoxes", &json!({"faces": [{"photo": id.0, "index": index}]})).unwrap();
                }
                6 => {
                    // recognition switched off and on again
                    s.execute("faces.enable", &json!({"enabled": false})).unwrap();
                    assert_eq!(ask(&mut s, false)["ready"], false, "seed {seed} step {step}");
                    s.execute("faces.enable", &json!({"enabled": true})).unwrap();
                }
                _ => {
                    let (from, to) = (names[rng.below(names.len())], names[rng.below(names.len())]);
                    let _ = s.execute("faces.renamePerson", &json!({"from": from, "to": to, "merge": true}));
                }
            }
            // every other step is asked without waiting first: the job works from what the last one left
            if step % 2 == 0 {
                settled(&mut s);
            }
            // what the runs worked through the changes give, against what a run that has seen nothing before gives (the state
            // they kept is put back after): the same faces in the same order with the same suggestions and exactly the same scores
            let inc = s.execute("faces.unnamed", &json!({})).unwrap();
            let kept = std::mem::take(&mut s.faces.arrange);
            let fresh = s.execute("faces.unnamed", &json!({})).unwrap();
            s.faces.arrange = kept;
            assert_eq!(inc, fresh, "seed {seed} step {step}");
            // (the faces in stacks are listed only inside their stack)
            let got: std::collections::HashMap<(u64, usize), Option<(String, f32)>> = inc["faces"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|f| {
                    let k = (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap() as usize);
                    let own = (k, f["suggestion"]["name"].as_str().map(|n| (n.to_string(), f["suggestion"]["score"].as_f64().unwrap() as f32)));
                    match f["stack"]["faces"].as_array() {
                        // (a stack with a suggestion says it for every face; the score is its representative's)
                        Some(m) => m
                            .iter()
                            .map(|x| ((x["photo"].as_u64().unwrap(), x["index"].as_u64().unwrap() as usize), own.1.clone().map(|(n, _)| (n, f32::NAN))))
                            .collect(),
                        None => vec![own],
                    }
                })
                .collect();
            let want = from_scratch(&s, threshold);
            assert_eq!(got.len(), want.len(), "seed {seed} step {step}");
            for (k, w) in want {
                let g = got.get(&k).unwrap_or_else(|| panic!("seed {seed} step {step}: {k:?} is not listed"));
                match (g, &w) {
                    (None, None) => {}
                    (Some((n, x)), Some((m, y))) => assert!(n == m && (x.is_nan() || (x - y).abs() < 1e-6), "seed {seed} step {step} {k:?}: {g:?} vs {w:?}"),
                    _ => panic!("seed {seed} step {step} {k:?}: {g:?} vs {w:?}"),
                }
            }
            // the order: suggested faces by score, most confident first
            let scores: Vec<f64> = inc["faces"].as_array().unwrap().iter().filter_map(|f| f["suggestion"]["score"].as_f64()).collect();
            assert!(scores.windows(2).all(|w| w[0] >= w[1]), "seed {seed} step {step}: {scores:?}");
            suggested += scores.len();
        }
        assert!(suggested > 0, "seed {seed}: no face was ever suggested a name, so nothing was compared");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn with_no_face_embedded_there_is_nothing_to_order_and_no_run_is_started() {
    let d = temp("bg-none");
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let (a, _) = two_photos(&s);
    set_regions(&mut s, a, vec![region(0.1, None), region(0.4, None)]);
    let r = ask(&mut s, false);
    assert_eq!((r["ready"].clone(), r["ordering"].clone(), r["total"].clone()), (json!(true), json!(false), json!(2)), "{r}");
    assert_eq!(rows(&mut s), 0);
    let _ = std::fs::remove_dir_all(&d);
}
