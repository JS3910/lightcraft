//! What putting the Unnamed faces in confidence order costs: every embedded unnamed face is scored against every
//! named face. A timing harness, not a check: run it in a release build, with the numbers printed,
//! `cargo test --release -p lightcraft-engine --lib face_unnamed_cost -- --ignored --nocapture`.

use std::time::Instant;

use lightcraft_geom::Rect;
use lightcraft_meta::{Region, RegionKind};
use serde_json::json;

use super::face_recognize::{set_regions, setup, temp};

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
    Region { rect: Rect { x0, y0, x1: x0 + 0.0005, y1: y0 + 0.0003 }, kind: RegionKind::Face, name, description: None, rejected: Vec::new() }
}

/// `unnamed` unnamed faces (half of them noisy copies of a named person's face, so they get suggestions) against
/// `named` named faces of `named / 10` people, on the demo's photos.
fn time(unnamed: usize, named: usize) {
    let d = temp(&format!("unnamed-cost-{unnamed}"));
    let (mut s, _) = setup(&d, DIM as u64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let mut ids: Vec<_> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let people: Vec<Vec<f32>> = (0..named / 10).map(|_| unit((0..DIM).map(|_| rng.next()).collect())).collect();
    let mut per_photo: Vec<Vec<Region>> = vec![Vec::new(); ids.len()];
    let mut vectors = Vec::new();
    for i in 0..named + unnamed {
        let photo = i % ids.len();
        let regions = &mut per_photo[photo];
        let (name, base, noise) = if i < named {
            (Some(format!("Person {}", i % people.len())), i % people.len(), 0.4)
        } else if i % 2 == 0 {
            (None, i % people.len(), 0.5)
        } else {
            (None, usize::MAX, 0.0)
        };
        let v = match people.get(base) {
            Some(p) => unit(p.iter().map(|x| x + noise * rng.next() * 0.2).collect()),
            None => unit((0..DIM).map(|_| rng.next()).collect()),
        };
        vectors.push((photo, region_at(regions.len(), name.clone()).rect, v));
        regions.push(region_at(regions.len(), name));
    }
    for (id, regions) in ids.iter().zip(per_photo) {
        set_regions(&mut s, *id, regions);
    }
    for (photo, rect, v) in vectors {
        let Some(id) = ids.get(photo) else { continue };
        s.faces.index.insert(id.0, &rect, v);
    }
    let started = Instant::now();
    let r = s.execute("faces.unnamed", &json!({})).unwrap();
    let took = started.elapsed();
    let suggested = r["faces"].as_array().unwrap().iter().filter(|f| !f["suggestion"].is_null()).count();
    println!(
        "faces.unnamed: {unnamed} unnamed vs {named} named ({} people): {took:.2?}, {suggested} of the {} listed suggested",
        people.len(),
        r["faces"].as_array().unwrap().len()
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
#[ignore = "timing harness: run it in a release build and read the printed times"]
fn face_unnamed_cost() {
    for (unnamed, named) in [(10_000, 3_000), (30_000, 3_000), (60_000, 3_000), (30_000, 6_000)] {
        time(unnamed, named);
    }
}
