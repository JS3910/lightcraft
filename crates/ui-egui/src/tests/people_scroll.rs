//! Scrolling the People view's face galleries: the tiles under the window are the ones drawn, however far down.

use std::time::Duration;

use serde_json::json;

use super::person_tiles::{inject, page, region, set_regions};
use crate::headless::Headless;

const T: Duration = Duration::from_secs(20);

/// The registered tiles whose id starts with `prefix` and that lie, at least in part, inside the 1400 x 900 window.
fn on_screen(h: &mut Headless, prefix: &str) -> usize {
    let r = h.request("ui.widgets", json!({"filter": prefix}), T);
    let inside = |w: &serde_json::Value| {
        let n = |i: usize| w["rect"][i].as_f64().unwrap_or(f64::NAN);
        w["id"].as_str().is_some_and(|id| id.starts_with(prefix)) && n(1) < 900.0 && n(1) + n(3) > 0.0
    };
    r["result"].as_array().into_iter().flatten().filter(|w| inside(w)).count()
}

/// Wheel down by `dy` points with the pointer over the gallery.
fn scroll(h: &mut Headless, dy: f64, after: fn(&mut Headless)) {
    for request in [("ui.move", json!({"x": 700.0, "y": 500.0})), ("ui.scroll", json!({"dx": 0.0, "dy": -dy}))] {
        let r = h.request(request.0, request.1, T);
        assert_eq!(r["ok"], true, "{}: {r}", request.0);
        for _ in 0..30 {
            after(h);
        }
    }
}

fn many(from: usize, n: usize, name: Option<&str>) -> Vec<lightcraft_meta::Region> {
    (from..from + n).map(|i| region(0.004 * i as f64, name)).collect()
}

#[test]
fn a_persons_faces_stay_on_screen_while_the_page_is_scrolled() {
    let (mut h, ids) = page();
    set_regions(&mut h, ids[0], many(0, 150, Some("Ann Lee")));
    inject(&mut h);
    let before = on_screen(&mut h, "person-face:");
    assert!(before > 10, "a screenful of her faces: {before}");
    for step in 1..=6 {
        scroll(&mut h, 300.0, inject);
        let now = on_screen(&mut h, "person-face:");
        assert!(now + 20 > before, "after {} points {now} of her faces are on screen, {before} before", step * 300);
    }
}

#[test]
fn the_more_faces_stay_on_screen_while_the_page_is_scrolled() {
    let (mut h, ids) = page();
    set_regions(&mut h, ids[3], many(0, 150, None));
    inject(&mut h);
    let before = on_screen(&mut h, "more-face:");
    assert!(before > 10, "a screenful of More faces: {before}");
    for step in 1..=6 {
        scroll(&mut h, 300.0, inject);
        let now = on_screen(&mut h, "more-face:");
        assert!(now + 20 > before, "after {} points {now} More faces are on screen, {before} before", step * 300);
    }
}

#[test]
fn the_unnamed_faces_stay_on_screen_while_the_people_view_is_scrolled() {
    let (mut h, ids) = super::unnamed_tiles::people();
    set_regions(&mut h, ids[1], many(0, 150, None));
    // several rows of named people above them
    let named = (0..40).map(|i| region(0.004 * i as f64, Some(&format!("Person {i:02}")))).collect();
    set_regions(&mut h, ids[2], named);
    super::unnamed_tiles::step(&mut h);
    // (the unnamed faces begin below the named people, so the first screens hold few of them)
    let mut counts = Vec::new();
    for _ in 0..10 {
        scroll(&mut h, 300.0, super::unnamed_tiles::step);
        counts.push(on_screen(&mut h, "unnamed-face:"));
    }
    let full = counts.iter().copied().max().unwrap_or(0);
    assert!(full > 10, "a screenful of unnamed faces is reached: {counts:?}");
    let first_full = counts.iter().position(|c| *c == full).unwrap_or(0);
    assert!(counts[first_full..].iter().all(|c| c + 20 > full), "once the window is full of unnamed faces it stays full: {counts:?}");
}
