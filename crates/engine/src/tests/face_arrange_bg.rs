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

/// Photo `a` holds Ann and Bob (named) and a face like Ann; photo `b` a face like Ann and a face like nobody; photo `c` two
/// faces that look like each other and like nobody named. Every face is embedded.
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
    put(b, 0.1, unit64(&[(0, 0.9), (4, 0.4)])); // a little less like Ann
    put(b, 0.5, unit64(&[(5, 1.0)])); // like nobody
    put(c, 0.1, unit64(&[(6, 1.0)])); // like nobody
    put(c, 0.5, unit64(&[(6, 0.99), (7, 0.1)])); // like the one before
    (s, d, [a, b, c])
}

fn ask(s: &mut Session, wait: bool) -> Value {
    s.execute("faces.unnamed", &json!({"wait": wait})).unwrap()
}

fn keys(r: &Value) -> Vec<(u64, u64)> {
    r["faces"].as_array().unwrap().iter().map(|f| (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap())).collect()
}

fn suggestion(r: &Value, photo: PhotoId, index: u64) -> Option<String> {
    let f = r["faces"].as_array().unwrap().iter().find(|f| f["photo"] == photo.0 && f["index"] == index)?;
    f["suggestion"]["name"].as_str().map(str::to_string)
}

/// Call `faces.pump` until a result has been taken in (its `arranged` count moved), and say what it is now.
fn take_in(s: &mut Session) -> u64 {
    let was = s.execute("faces.pump", &json!({})).unwrap()["arranged"].as_u64().unwrap();
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
        let r = ask(s, false);
        if r["ordering"] == false {
            return r;
        }
        take_in(s);
    }
    panic!("the order never settled");
}

#[test]
fn without_waiting_the_faces_come_at_once_and_the_order_is_taken_in_when_it_is_worked_out() {
    let (mut s, d, [a, b, c]) = library("bg-first");
    let r = ask(&mut s, false);
    assert_eq!(r["ordering"], true, "{r}");
    assert_eq!(r["total"], 5);
    assert_eq!(keys(&r), vec![(a.0, 2), (b.0, 0), (b.0, 1), (c.0, 0), (c.0, 1)], "nothing worked out yet: the faces in photo order");
    assert!(r["faces"].as_array().unwrap().iter().all(|f| f["suggestion"].is_null()), "{r}");
    let r = settled(&mut s);
    assert_eq!(r["ordering"], false);
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
    hold.store(false, Ordering::Relaxed);
    take_in(&mut s);
    let r = ask(&mut s, false);
    assert_eq!(r["ordering"], true, "the result was for the old named faces: a new run starts for these");
    let done = settled(&mut s);
    let waited = s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!(keys(&done), keys(&waited));
    assert_eq!(suggestion(&done, b, 0), suggestion(&waited, b, 0));
    let after = s.execute("activity.list", &json!({})).unwrap();
    assert!(after["tasks"].as_array().unwrap().iter().all(|t| t["kind"] != "faceOrder"), "the row goes with the job: {after}");
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
    // the job ends; its result is dropped, so no list of suggestions came of it
    for _ in 0..2000 {
        if s.execute("activity.list", &json!({})).unwrap()["tasks"].as_array().unwrap().iter().all(|t| t["kind"] != "faceOrder") {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    s.execute("faces.pump", &json!({})).unwrap();
    let r = ask(&mut s, false);
    assert_eq!(r["ordering"], false, "a cancelled run is not started again by itself: {r}");
    assert_eq!(suggestion(&r, a, 2), None);
    // a caller that waits still gets a complete order
    s.faces.arrange.hold = None;
    let waited = s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!(suggestion(&waited, a, 2).as_deref(), Some("Ann"));
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

#[test]
fn after_every_random_change_the_suggestions_are_what_scoring_everyone_again_gives() {
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
    let mut rng = 0x1234_5678_9ABC_DEF1u64;
    let mut next = move |n: usize| {
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        (rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % n.max(1)
    };
    let names = ["Ann", "Bob", "Cy", "Di", "Eve", "Flo"];
    for seed in 0..6 {
        // a fresh set of faces: each photo gets six faces, a third of them named, embeddings of a few clusters
        for (n, id) in ids.iter().enumerate().take(6) {
            let regions = (0..6).map(|i| region(0.1 * i as f64, (next(3) == 0).then(|| names[next(names.len())]))).collect();
            set_regions(&mut s, *id, regions);
            for i in 0..6 {
                let cluster = next(6);
                let v = unit64(&[(cluster, 1.0), (6 + (seed + n + i) % 50, 0.3 + 0.1 * next(5) as f32)]);
                s.faces.index.insert(id.0, &region(0.1 * i as f64, None).rect, v);
            }
        }
        for step in 0..14 {
            let id = ids[next(6)];
            let index = next(6);
            let named = s.catalog.photo(id).unwrap().meta.regions[index].name.clone();
            match next(5) {
                0 | 1 => {
                    let name = names[next(names.len())];
                    let name = if next(2) == 0 { name.to_uppercase() } else { name.to_string() };
                    s.execute("faces.setName", &json!({"id": id.0, "index": index, "name": name})).unwrap();
                }
                2 => {
                    s.execute("faces.setName", &json!({"id": id.0, "index": index, "name": Value::Null})).unwrap();
                }
                3 if named.is_none() => {
                    let name = names[next(names.len())];
                    s.execute("faces.reject", &json!({"faces": [{"photo": id.0, "index": index}], "name": name})).unwrap();
                }
                _ => {
                    let (from, to) = (names[next(names.len())], names[next(names.len())]);
                    let _ = s.execute("faces.renamePerson", &json!({"from": from, "to": to, "merge": true}));
                }
            }
            // every other step is asked without waiting first: the job works from what the last one left
            if step % 2 == 0 {
                settled(&mut s);
            }
            let r = s.execute("faces.unnamed", &json!({})).unwrap();
            let got: std::collections::HashMap<(u64, usize), Option<(String, f32)>> = r["faces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    let k = (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap() as usize);
                    (k, f["suggestion"]["name"].as_str().map(|n| (n.to_string(), f["suggestion"]["score"].as_f64().unwrap() as f32)))
                })
                .collect();
            let want = from_scratch(&s, threshold);
            assert_eq!(got.len(), want.len(), "seed {seed} step {step}");
            for (k, w) in want {
                let g = got.get(&k).unwrap_or_else(|| panic!("seed {seed} step {step}: {k:?} is not listed"));
                match (g, &w) {
                    (None, None) => {}
                    (Some((n, x)), Some((m, y))) => assert!(n == m && (x - y).abs() < 1e-5, "seed {seed} step {step} {k:?}: {g:?} vs {w:?}"),
                    _ => panic!("seed {seed} step {step} {k:?}: {g:?} vs {w:?}"),
                }
            }
            // the order: suggested faces by score, most confident first
            let scores: Vec<f64> = r["faces"].as_array().unwrap().iter().filter_map(|f| f["suggestion"]["score"].as_f64()).collect();
            assert!(scores.windows(2).all(|w| w[0] >= w[1]), "seed {seed} step {step}: {scores:?}");
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}
