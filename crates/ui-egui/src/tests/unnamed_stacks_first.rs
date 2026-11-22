//! The Unnamed faces' Stacks first switch: it starts off, puts every stack before any single face when on (the engine's real
//! order, with a tiny recognition model), says so in the subtitle, and is remembered with the preferences.

use std::time::Duration;

use lightcraft_catalog::PhotoId;
use serde_json::json;

use super::unnamed_ordering::{recognising, unit};
use super::unnamed_tiles::region;
use crate::headless::Headless;

const T: Duration = Duration::from_secs(20);

/// [`recognising`] with the faces of photo 1 and photo 2 at 0.3 alike enough to be a stack (without a suggestion), and the
/// face of photo 0 suggested Ann Lee: the stack comes after it unless stacks come first.
fn with_a_stack(name: &str) -> (Headless, [PhotoId; 3], std::path::PathBuf) {
    let (mut h, ids, _, dir) = recognising(name);
    h.app.session.test_face_embedding(ids[2], &region(0.3, None).rect, unit(&[(5, 1.0), (9, 0.05)]));
    (h, ids, dir)
}

/// The tiles of the list on screen, in order, each as the faces it holds.
fn tiles(h: &Headless) -> Vec<Vec<(u64, usize)>> {
    h.app.caches.unnamed.as_ref().map_or_else(Vec::new, |(_, _, _, l)| l.faces.iter().map(|f| f.keys()).collect())
}

/// Draw frames until the list on screen is in its final order.
fn settled(h: &mut Headless) {
    for _ in 0..400 {
        h.step();
        if h.app.caches.unnamed.as_ref().is_some_and(|(_, _, _, l)| l.complete && !l.faces.is_empty()) {
            h.settle(T);
            return;
        }
    }
    panic!("the list never settled");
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    settled(h);
}

fn subtitle(h: &Headless) -> String {
    crate::panels::unnamed::subtitle(&h.app.caches.unnamed.as_ref().expect("a list").3, 0)
}

#[test]
fn the_switch_starts_off_and_puts_every_stack_before_the_single_faces_and_back() {
    let (mut h, [p0, p1, p2], dir) = with_a_stack("stacks-first-switch");
    h.request("ui.set", json!({"view": "people"}), T);
    settled(&mut h);
    let suggested = vec![(p0.0, 1)];
    let mut stack = vec![(p1.0, 0), (p2.0, 0)];
    stack.sort_unstable();
    let sorted = |t: Vec<Vec<(u64, usize)>>| -> Vec<Vec<(u64, usize)>> {
        t.into_iter()
            .map(|mut e| {
                e.sort_unstable();
                e
            })
            .collect()
    };
    assert!(!h.app.ui.unnamed_stacks_first, "it starts off");
    let off = sorted(tiles(&h));
    assert_eq!(&off[..2], &[suggested.clone(), stack.clone()], "off: the suggested face first, then the stack");
    assert!(!subtitle(&h).contains("Stacks come first"), "{}", subtitle(&h));
    click(&mut h, "unnamed:stacksFirst");
    assert!(h.app.ui.unnamed_stacks_first);
    let on = sorted(tiles(&h));
    assert_eq!(&on[..2], &[stack.clone(), suggested.clone()], "on: the stack ahead of the single faces");
    assert_eq!(on.len(), off.len(), "the same tiles");
    assert!(subtitle(&h).starts_with("Stacks come first"), "the subtitle says the order in use: {}", subtitle(&h));
    click(&mut h, "unnamed:stacksFirst");
    assert!(!h.app.ui.unnamed_stacks_first);
    assert_eq!(sorted(tiles(&h)), off, "off again: the order without the switch");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_switch_is_remembered_with_the_preferences() {
    let (mut h, _, dir) = with_a_stack("stacks-first-saved");
    h.request("ui.set", json!({"view": "people"}), T);
    settled(&mut h);
    click(&mut h, "unnamed:stacksFirst");
    let saved = serde_json::to_string(&h.app.ui).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    // the next session reads the preferences before its first frame
    let (mut next, [_, p1, p2], dir) = with_a_stack("stacks-first-saved-next");
    next.app.ui = serde_json::from_str(&saved).unwrap();
    assert!(next.app.ui.unnamed_stacks_first, "{saved}");
    next.request("ui.set", json!({"view": "people"}), T);
    settled(&mut next);
    let mut first = tiles(&next).into_iter().next().unwrap_or_default();
    first.sort_unstable();
    assert_eq!(first, [(p1.0, 0), (p2.0, 0)], "the stack comes first in the next session too");
    let _ = std::fs::remove_dir_all(&dir);
}
