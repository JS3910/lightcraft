//! What picking the varied faces of a person's page costs: `faces.person` with `varied` against without it, for a person of
//! thousands of confirmed faces. A timing harness, not a check: run it in a release build, with the numbers printed,
//! `cargo test --release -p lightcraft-engine --lib face_person_cost -- --ignored --nocapture`
//! (`RAYON_NUM_THREADS=4` or `8` for a smaller machine).

use std::time::{Duration, Instant};

use lightcraft_geom::Rect;
use lightcraft_meta::{Region, RegionKind};
use serde_json::json;

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

fn region_at(i: usize, name: Option<&str>) -> Region {
    let (x0, y0) = ((i % 1000) as f64 * 0.0009, (i / 1000) as f64 * 0.0004);
    Region {
        rect: Rect { x0, y0, x1: x0 + 0.0005, y1: y0 + 0.0003 },
        kind: RegionKind::Face,
        name: name.map(str::to_string),
        description: None,
        rejected: Vec::new(),
    }
}

/// The best of a few asks of `faces.person`, and the number of faces it listed.
fn ask(s: &mut Session, varied: usize) -> (Duration, usize) {
    let mut best = Duration::MAX;
    let mut listed = 0;
    for _ in 0..5 {
        let started = Instant::now();
        let r = s.execute("faces.person", &json!({"name": "Ann", "scope": "visible", "varied": varied})).unwrap();
        best = best.min(started.elapsed());
        listed = r["confirmed"].as_array().map_or(0, Vec::len);
    }
    (best, listed)
}

#[test]
#[ignore = "timing harness: run it in a release build and read the printed times"]
fn face_person_cost() {
    let confirmed = 5_000;
    let d = temp("person-cost");
    let (mut s, _) = setup(&d, DIM as u64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let mut ids: Vec<_> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    // Ann's faces: noisy copies of a dozen looks. And 2,000 unnamed faces for More to look through.
    let looks: Vec<Vec<f32>> = (0..12).map(|_| unit((0..DIM).map(|_| rng.next()).collect())).collect();
    let mut per_photo: Vec<Vec<Region>> = vec![Vec::new(); ids.len()];
    let mut vectors = Vec::new();
    for i in 0..confirmed + 2_000 {
        let photo = i % ids.len();
        let Some(regions) = per_photo.get_mut(photo) else { continue };
        let region = region_at(regions.len(), (i < confirmed).then_some("Ann"));
        let base = &looks[i % looks.len()];
        vectors.push((photo, region.rect, unit(base.iter().map(|x| x + rng.next() * 0.2).collect())));
        regions.push(region);
    }
    for (id, regions) in ids.iter().zip(per_photo) {
        set_regions(&mut s, *id, regions);
    }
    for (photo, rect, v) in vectors {
        if let Some(id) = ids.get(photo) {
            s.faces.index.insert(id.0, &rect, v);
        }
    }
    println!("a person of {confirmed} confirmed faces (512 numbers each), {} threads, best of 5:", rayon::current_num_threads());
    let (base, listed) = ask(&mut s, 0);
    println!("    faces.person without varied:   {base:>9.2?} ({listed} confirmed listed)");
    for varied in [21, 63, 174] {
        let (t, listed) = ask(&mut s, varied);
        let extra = t.saturating_sub(base);
        println!("    varied {varied:>3}: {t:>9.2?} in all, {extra:>9.2?} more than without ({listed} confirmed listed)");
    }
    let _ = std::fs::remove_dir_all(&d);
}
