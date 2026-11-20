//! The Unnamed faces are asked for without waiting for the order (`faces.unnamed` with `wait: false`): the window asks again
//! when the engine took in a result of the thread that puts them in order, not on every frame, and with no list on screen
//! and an order that is not final it shows a status instead of tiles.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::unnamed_tiles::{people, step};
use crate::headless::Headless;
use crate::panels::unnamed::{Unnamed, faces};

fn widgets(h: &mut Headless, filter: &str) -> usize {
    let r = h.request("ui.widgets", json!({"filter": filter}), Duration::from_secs(20));
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

#[test]
fn with_no_list_on_screen_and_an_order_that_is_not_final_the_section_shows_no_tiles() {
    let (mut h, _) = people();
    assert!(widgets(&mut h, "unnamed-face:") > 0, "tiles when the order is final");
    let waiting = Unnamed { total: 4, ordering: true, ready: true, ..Default::default() };
    let (rev, indexed) = (h.app.session.catalog.revision, h.app.caches.faces_indexed);
    h.app.caches.unnamed = Some((rev, indexed, f64::MAX, Arc::new(waiting)));
    step(&mut h);
    assert_eq!(widgets(&mut h, "unnamed-face:"), 0);
    assert_eq!(widgets(&mut h, "unnamed:selectAll"), 0);
}
