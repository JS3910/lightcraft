//! What putting the Unnamed faces in confidence order costs: every embedded unnamed face is scored against every
//! named face. A timing harness, not a check: run it in a release build, with the numbers printed,
//! `cargo test --release -p lightcraft-engine --lib face_unnamed_cost -- --ignored --nocapture`
//! (`RAYON_NUM_THREADS=4` or `8` for a smaller machine).
//!
//! For each size it prints the first arrangement, what a naming costs and what the scan finding more faces costs, each as
//! the time `faces.unnamed` takes on the calling thread when asked not to wait (what the window waits for), and the time
//! until the order is complete (what runs on the thread of its own). It only uses commands, so it runs on older code too,
//! where the two are the same.

use std::time::{Duration, Instant};

use lightcraft_geom::Rect;
use lightcraft_meta::{Region, RegionKind};
use serde_json::{Value, json};

use super::face_recognize::{set_regions, setup, temp};
use crate::Session;

const DIM: usize = 512;

/// A small deterministic generator (xorshift64*), so the runs are comparable.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32 - 0.5
    }
}

fn unit(v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.into_iter().map(|x| x / n).collect()
}

fn region_at(i: usize, name: Option<String>) -> Region {
    let (x0, y0) = ((i % 1000) as f64 * 0.0009, (i / 1000) as f64 * 0.0004);
    Region { rect: Rect { x0, y0, x1: x0 + 0.0005, y1: y0 + 0.0003 }, kind: RegionKind::Face, name, description: None, ..Default::default() }
}

/// What `faces.unnamed` took, and what it listed.
fn ask(s: &mut Session, wait: bool) -> (Duration, Value) {
    let started = Instant::now();
    let r = s.execute("faces.unnamed", &json!({"wait": wait})).unwrap();
    (started.elapsed(), r)
}

/// Ask without waiting, then wait for the order to be complete: the time on the calling thread, and until complete.
fn settle(s: &mut Session) -> (Duration, Duration) {
    let started = Instant::now();
    let (call, _) = ask(s, false);
    ask(s, true);
    (call, started.elapsed())
}

/// `unnamed` unnamed faces (half of them noisy copies of a named person's face, so they get suggestions) against
/// `named` named faces of `named / 10` people, on the demo's photos.
fn time(unnamed: usize, named: usize, clusters: usize) {
    let d = temp(&format!("unnamed-cost-{unnamed}-{clusters}"));
    let (mut s, _) = setup(&d, DIM as u64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let mut ids: Vec<_> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort();
    // how much the faces that are suggested a named person differ from her: 0.17 makes them one person's faces, which stack with her name
    // (`LC_COST_SUGGESTED_NOISE=0.5` makes them only alike enough to be suggested her, which stack with nobody: the cost without them)
    let suggested_noise: f32 = std::env::var("LC_COST_SUGGESTED_NOISE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.17);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let people: Vec<Vec<f32>> = (0..named / 10).map(|_| unit((0..DIM).map(|_| rng.next()).collect())).collect();
    // (the people nobody has named: the faces of a quarter of the unnamed ones, in `clusters` groups)
    let strangers: Vec<Vec<f32>> = (0..clusters).map(|_| unit((0..DIM).map(|_| rng.next()).collect())).collect();
    let mut per_photo: Vec<Vec<Region>> = vec![Vec::new(); ids.len()];
    let mut next = 0usize;
    // `count` more faces, from the `next`th on: the first `named` are named
    let mut add = |s: &mut Session, per_photo: &mut Vec<Vec<Region>>, count: usize, rng: &mut Rng| {
        let mut vectors = Vec::new();
        for i in next..next + count {
            let photo = i % ids.len();
            let Some(regions) = per_photo.get_mut(photo) else { continue };
            let stranger = clusters > 0 && i >= named && i % 4 == 1;
            let (name, base, noise) = if i < named {
                (Some(format!("Person {}", i % people.len())), i % people.len(), 0.4)
            } else if i % 2 == 0 {
                // (with stacks asked for, the faces of one named person that suggest her are as alike as one person's faces are, so that they form
                // stacks that carry her name too; without, they are only alike enough to be suggested her)
                (None, i % people.len(), if clusters > 0 { suggested_noise } else { 0.5 })
            } else {
                (None, usize::MAX, 0.0)
            };
            let noise = if stranger { 0.5 } else { noise };
            let v = match if stranger { strangers.get(i % clusters) } else { people.get(base) } {
                Some(p) => unit(p.iter().map(|x| x + noise * rng.next() * 0.2).collect()),
                None => unit((0..DIM).map(|_| rng.next()).collect()),
            };
            let region = region_at(regions.len(), name);
            vectors.push((photo, region.rect, v));
            regions.push(region);
        }
        next += count;
        for (id, regions) in ids.iter().zip(per_photo.iter()) {
            set_regions(s, *id, regions.clone());
        }
        for (photo, rect, v) in vectors {
            let Some(id) = ids.get(photo) else { continue };
            s.faces.index.insert(id.0, &rect, v);
        }
    };
    add(&mut s, &mut per_photo, named + unnamed, &mut rng);
    // the first arrangement: nothing is known yet
    let (first_call, r) = ask(&mut s, false);
    let first_started = Instant::now();
    ask(&mut s, true);
    let first = first_call + first_started.elapsed();
    let (idle, _) = ask(&mut s, false);
    let suggested = r["faces"].as_array().map_or(0, |f| f.iter().filter(|f| !f["suggestion"].is_null()).count());
    let listed = ask(&mut s, true).1;
    let suggested = suggested.max(listed["faces"].as_array().map_or(0, |f| f.iter().filter(|f| !f["suggestion"].is_null()).count()));
    // a naming (of the first face listed)
    // (confirmed as the person it is suggested: naming it as someone else would be a mistake in the named faces, which raises the
    // stacking bar and so starts the stacks afresh, as the docs say)
    let first_face = listed["faces"].as_array().and_then(|f| f.first());
    let first_face = first_face.expect("some face is listed");
    let (photo, index) = (first_face["photo"].as_u64().unwrap_or(0), first_face["index"].as_u64().unwrap_or(0));
    let who = first_face["suggestion"]["name"].as_str().unwrap_or("Person 0").to_string();
    s.execute("faces.setName", &json!({"id": photo, "index": index, "name": who})).unwrap();
    let after_first = (s.faces.arrange.stack_new, s.faces.arrange.stack_compared);
    let (naming_call, naming) = settle(&mut s);
    let after_naming = (s.faces.arrange.stack_new, s.faces.arrange.stack_compared);
    // More on the page of the first person (a person's page asks for the unnamed faces that look like them: out of scope here)
    let started = Instant::now();
    let page = s.execute("faces.person", &json!({"name": "Person 0"})).unwrap();
    let more = started.elapsed();
    let (their_faces, found) = (page["confirmed"].as_array().map_or(0, Vec::len), page["more"].as_array().map_or(0, Vec::len));
    // the scan finds more faces (2 % more)
    add(&mut s, &mut per_photo, unnamed / 50, &mut rng);
    let (refresh_call, refresh) = settle(&mut s);
    let after_refresh = (s.faces.arrange.stack_new, s.faces.arrange.stack_compared);
    println!(
        "{unnamed} unnamed vs {named} named ({} people), {} threads, {suggested} of the {} listed suggested:",
        people.len(),
        rayon::current_num_threads(),
        listed["faces"].as_array().map_or(0, Vec::len)
    );
    println!("    first arrangement:       {first:>9.2?} until complete, {first_call:>9.2?} on the calling thread without waiting");
    // the stacks of the list that carry a suggested name, and how many faces they hold (a stack of 50 faces is one entry)
    let named_stacks: Vec<&Value> =
        listed["faces"].as_array().into_iter().flatten().filter(|f| f["stack"].is_object() && !f["suggestion"].is_null()).collect();
    let in_named_stacks: u64 = named_stacks.iter().filter_map(|f| f["stack"]["count"].as_u64()).sum();
    println!(
        "    stacks: {} in the list, {} of the {} listed carry a suggested name ({in_named_stacks} faces); faces looked at / comparisons made by the stacking at the bar {:.3}: first {:?}, after the naming {:?}, after the scan {:?}",
        listed["stacks"],
        named_stacks.len(),
        listed["faces"].as_array().map_or(0, Vec::len),
        s.faces.arrange.bar,
        after_first,
        after_naming,
        after_refresh
    );
    println!("    asked again unchanged:   {idle:>9.2?} on the calling thread");
    println!("    after one naming:        {naming:>9.2?} until complete, {naming_call:>9.2?} on the calling thread");
    println!("    More for a person of {their_faces} faces: {more:>9.2?} on the calling thread ({found} found)");
    println!("    scan finds {:>4} faces:   {refresh:>9.2?} until complete, {refresh_call:>9.2?} on the calling thread", unnamed / 50);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
#[ignore = "timing harness: run it in a release build and read the printed times"]
fn face_unnamed_cost() {
    for (unnamed, named) in [(10_000, 3_000), (30_000, 3_000), (60_000, 3_000), (30_000, 6_000)] {
        time(unnamed, named, 0);
    }
}

/// The same with stacks: a quarter of the unnamed faces are faces of people nobody has named, 50 faces of each (so they stack),
/// and the rest as above. The first arrangement includes the stacking; "after one naming" and "scan finds" show what it costs to
/// keep them.
#[test]
#[ignore = "timing harness: run it in a release build and read the printed times"]
fn face_unnamed_stacks_cost() {
    for (unnamed, named) in [(10_000, 3_000), (30_000, 6_000), (60_000, 3_000)] {
        time(unnamed, named, unnamed / 200);
    }
}
