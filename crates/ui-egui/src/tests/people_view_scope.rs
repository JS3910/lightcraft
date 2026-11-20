//! The People view works from the photos the grid shows, on screen: the Unnamed faces and a person's page follow the source
//! and filters, an empty view says why and names the source, a source or filter change starts the faces afresh, an edit that
//! takes photos out of the view drops only their faces, and the tiles on screen keep their places when the list is asked
//! for again. Driven through the control channel; recognition is off here, so the list is in photo order.

use std::time::Duration;

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use super::unnamed_tiles::{people, picture};
use crate::headless::Headless;
use crate::panels::face_tile::Gallery;
use crate::panels::person::Face;
use crate::state::ViewMode;

const T: Duration = Duration::from_secs(20);

fn run(h: &mut Headless, command: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    h.step();
    h.step();
    h.settle(T);
    r["result"].clone()
}

fn album_of(h: &mut Headless, name: &str, photos: &[PhotoId]) -> u64 {
    let id = run(h, "album.create", json!({"name": name}))["id"].as_u64().unwrap();
    if !photos.is_empty() {
        let ids: Vec<u64> = photos.iter().map(|p| p.0).collect();
        run(h, "album.addPhotos", json!({"id": id, "ids": ids}));
    }
    id
}

fn show_album(h: &mut Headless, album: u64) {
    run(h, "library.source", json!({"kind": "album", "id": album}));
}

fn widgets(h: &mut Headless, filter: &str) -> Vec<(String, [f64; 4])> {
    let r = h.request("ui.widgets", json!({"filter": filter}), T);
    r["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let n = |i: usize| w["rect"][i].as_f64();
            Some((w["id"].as_str()?.to_string(), [n(0)?, n(1)?, n(2)?, n(3)?]))
        })
        .filter(|(id, _)| id.starts_with(filter))
        .collect()
}

fn ids(h: &mut Headless, filter: &str) -> Vec<String> {
    let mut v: Vec<String> = widgets(h, filter).into_iter().map(|(id, _)| id).collect();
    v.sort();
    v
}

fn said(h: &Headless, text: &str) -> bool {
    h.painted_text().iter().any(|t| t == text)
}

#[test]
fn choosing_an_album_narrows_the_unnamed_faces_to_its_photos() {
    let (mut h, [p0, p1, p2]) = people();
    let mut all = ids(&mut h, "unnamed-face:");
    all.sort();
    assert_eq!(all.len(), 4, "every unnamed face of the library to begin with: {all:?}");
    let album = album_of(&mut h, "Pair", &[p1]);
    show_album(&mut h, album);
    assert_eq!(h.app.ui.view, ViewMode::People);
    assert_eq!(ids(&mut h, "unnamed-face:"), [picture(p1, 0), picture(p1, 1)], "only the unnamed faces of the album's photos");
    let _ = (p0, p2);
    let other = album_of(&mut h, "Other", &[p2]);
    show_album(&mut h, other);
    assert_eq!(ids(&mut h, "unnamed-face:"), [picture(p2, 0)]);
    run(&mut h, "library.source", json!({"kind": "all"}));
    assert_eq!(ids(&mut h, "unnamed-face:").len(), 4);
}

#[test]
fn the_header_names_the_source_as_the_grid_does() {
    let (mut h, [_, p1, _]) = people();
    assert!(said(&h, "All Photos"), "{:?}", h.painted_text());
    let album = album_of(&mut h, "Beach day", &[p1]);
    show_album(&mut h, album);
    assert!(said(&h, "Beach day") && !said(&h, "All Photos"), "{:?}", h.painted_text());
    assert_eq!(widgets(&mut h, "people:source").len(), 1);
}

#[test]
fn a_view_without_photos_says_why_and_how_many_faces_the_source_and_filters_hide() {
    let (mut h, _) = people();
    let album = album_of(&mut h, "Nothing here", &[]);
    show_album(&mut h, album);
    assert!(said(&h, "No people in these photos"), "{:?}", h.painted_text());
    assert!(said(&h, "5 faces are hidden by the current filters"), "the library's five faces: {:?}", h.painted_text());
    assert!(!said(&h, "No people yet"));
    // a filter the chips show, removable, after the message
    run(&mut h, "library.filter", json!({"text": "nothing-is-called-this"}));
    assert!(said(&h, "No people in these photos"));
    assert_eq!(widgets(&mut h, "filterchips").len(), 1, "the chips stay");
    // one face hidden is said in the singular
    let (mut h, [_, _, p2]) = people();
    for photo in h.app.session.catalog.photos().map(|p| p.id).collect::<Vec<_>>() {
        if photo != p2 {
            super::person_tiles::set_regions(&mut h, photo, vec![]);
        }
    }
    let album = album_of(&mut h, "Empty", &[]);
    show_album(&mut h, album);
    assert!(said(&h, "1 face is hidden by the current filters"), "{:?}", h.painted_text());
}

#[test]
fn a_library_without_faces_says_no_people_yet_and_never_a_hidden_line() {
    let services = crate::Services { png: None, write_shared: Some(std::sync::Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let mut h = Headless::new(crate::LightcraftApp::new(lightcraft_engine::Session::with_demo(), services), [1400.0, 900.0], 1.0);
    h.settle(T);
    run(&mut h, "view.people", json!({}));
    assert!(said(&h, "No people yet"), "{:?}", h.painted_text());
    assert!(!h.painted_text().iter().any(|t| t.contains("hidden by the current filters")), "{:?}", h.painted_text());
    // an album of nothing in a library without faces is still "no people yet": nothing is hidden
    let album = album_of(&mut h, "Empty", &[]);
    show_album(&mut h, album);
    assert!(said(&h, "No people yet"), "{:?}", h.painted_text());
}

#[test]
fn a_person_with_no_named_faces_in_these_photos_keeps_their_page_and_it_says_so() {
    let (mut h, [_, p1, _]) = people();
    run(&mut h, "view.person", json!({"name": "Ann Lee"}));
    assert!(!h.painted_text().iter().any(|t| t.contains("no named faces in these photos")), "{:?}", h.painted_text());
    // Ann Lee is named only in photo 0: the page stays open when the source has not got that photo
    let album = album_of(&mut h, "Elsewhere", &[p1]);
    show_album(&mut h, album);
    assert_eq!(h.app.ui.person_page.as_deref(), Some("Ann Lee"), "the page stays open across a source change");
    assert!(said(&h, "Ann Lee has no named faces in these photos."), "{:?}", h.painted_text());
    assert!(!said(&h, "No faces by that name"), "she is named in the library");
    // a person nobody has named has no page to keep
    run(&mut h, "view.person", json!({"name": "Nobody At All"}));
    assert!(said(&h, "No faces by that name"), "{:?}", h.painted_text());
}

/// Open the name box on the strip of the first unnamed face of `photo`, as a click does.
fn open_box(h: &mut Headless, photo: PhotoId) {
    let r = h.request("ui.clickWidget", json!({"id": super::unnamed_tiles::strip(photo, 0)}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
}

fn hold_a_selection(h: &mut Headless, photos: &[PhotoId]) {
    for p in photos {
        h.app.ui.unnamed_selected.insert((p.0, 0));
        let face = Face { photo: p.0, index: 0, rect: lightcraft_geom::Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }, score: 0.0 };
        h.app.ui.rejected_unnamed.remember(&face, 0);
    }
    h.app.ui.face_gallery = Some(Gallery::Unnamed);
    h.step();
    h.step();
    open_box(h, photos[0]);
}

#[test]
fn changing_the_source_or_a_filter_clears_the_selection_the_rejections_and_the_name_box() {
    let (mut h, [p0, p1, p2]) = people();
    hold_a_selection(&mut h, &[p1, p2]);
    assert!(!h.app.ui.unnamed_selected.is_empty() && h.app.ui.name_edit.is_some());
    let album = album_of(&mut h, "Some", &[p1, p2]);
    show_album(&mut h, album);
    assert!(h.app.ui.unnamed_selected.is_empty(), "the selection is cleared");
    assert!(h.app.ui.rejected_unnamed.is_empty(), "so are the faces rejected in place");
    assert!(h.app.ui.name_edit.is_none(), "and an open name box");
    // a filter does the same
    hold_a_selection(&mut h, &[p1, p2]);
    run(&mut h, "library.filter", json!({"text": "a"}));
    assert!(h.app.ui.unnamed_selected.is_empty() && h.app.ui.rejected_unnamed.is_empty() && h.app.ui.name_edit.is_none());
    let _ = p0;
}

#[test]
fn an_edit_that_drops_a_photo_removes_only_its_faces_from_what_the_view_holds() {
    let (mut h, [p0, p1, p2]) = people();
    let album = album_of(&mut h, "Three", &[p0, p1, p2]);
    show_album(&mut h, album);
    hold_a_selection(&mut h, &[p2, p1]);
    // photo 1 leaves the album: its face leaves the selection and the rejections; the box is on photo 2's face and stays
    run(&mut h, "album.removePhotos", json!({"id": album, "ids": [p1.0]}));
    let selected: Vec<(u64, usize)> = h.app.ui.unnamed_selected.iter().copied().collect();
    assert_eq!(selected, vec![(p2.0, 0)], "only the face of the photo that left is dropped");
    assert!(h.app.ui.rejected_unnamed.contains((p2.0, 0)) && !h.app.ui.rejected_unnamed.contains((p1.0, 0)));
    assert!(h.app.ui.name_edit.as_ref().is_some_and(|e| e.photo == p2.0));
    // and the box, still on photo 2's face, closes when that photo leaves
    run(&mut h, "album.removePhotos", json!({"id": album, "ids": [p2.0]}));
    assert!(h.app.ui.name_edit.is_none());
}

/// Scroll the wheel `dy` points down with the pointer over the faces.
fn scroll(h: &mut Headless, dy: f64) {
    for request in [("ui.move", json!({"x": 700.0, "y": 500.0})), ("ui.scroll", json!({"dx": 0.0, "dy": -dy}))] {
        let r = h.request(request.0, request.1, T);
        assert_eq!(r["ok"], true, "{}: {r}", request.0);
        for _ in 0..30 {
            h.step();
        }
    }
}

/// The unnamed face tiles drawn, in reading order, as `(photo, index)`.
fn tiles(h: &mut Headless) -> Vec<(u64, usize)> {
    let mut v = widgets(h, "unnamed-face:");
    v.sort_by(|a, b| (a.1[1], a.1[0]).partial_cmp(&(b.1[1], b.1[0])).unwrap_or(std::cmp::Ordering::Equal));
    v.into_iter()
        .filter_map(|(id, _)| {
            let mut it = id.strip_prefix("unnamed-face:")?.split(':');
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect()
}

/// Many unnamed faces in photo 1, the People view scrolled part-way down them.
fn scrolled() -> (Headless, [PhotoId; 3]) {
    let (mut h, ids) = people();
    let many: Vec<_> = (0..150).map(|i| super::person_tiles::region(0.004 * i as f64, None)).collect();
    super::person_tiles::set_regions(&mut h, ids[1], many);
    h.step();
    h.step();
    scroll(&mut h, 500.0);
    (h, ids)
}

fn engine_order(h: &mut Headless) -> Vec<(u64, u64)> {
    let r = run(h, "faces.unnamed", json!({"scope": "visible"}));
    r["faces"].as_array().unwrap().iter().map(|f| (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap())).collect()
}

#[test]
fn a_face_the_scan_finds_never_moves_the_tiles_on_screen() {
    let (mut h, [p0, _, _]) = scrolled();
    let before = tiles(&mut h);
    assert!(before.len() > 10 && before.first() != Some(&(p0.0, 1)), "scrolled into the faces: {before:?}");
    // a face found in the first photo sorts before every tile shown; the tiles on screen stay as they were
    let mut regions = h.app.session.catalog.photo(p0).unwrap().meta.regions.clone();
    regions.push(super::person_tiles::region(0.7, None));
    super::person_tiles::set_regions(&mut h, p0, regions);
    h.step();
    h.step();
    assert_eq!(tiles(&mut h), before, "the tiles on screen kept their places");
    // scrolling back finds the new face where it sorts: at the front
    scroll(&mut h, -4000.0);
    let top = tiles(&mut h);
    assert!(top.contains(&(p0.0, 2)), "the new face is in the first rows: {top:?}");
    assert_eq!(top.first(), Some(&(p0.0, 1)));
}

#[test]
fn naming_a_face_on_screen_moves_the_tiles_after_it_up_one_place() {
    let (mut h, [_, p1, _]) = scrolled();
    let before = tiles(&mut h);
    let at = before.len() / 2;
    let (photo, index) = before[at];
    assert_eq!(photo, p1.0);
    run(&mut h, "faces.setName", json!({"id": photo, "index": index, "name": "Cy"}));
    let after = tiles(&mut h);
    assert!(!after.contains(&(photo, index)), "the named face is gone");
    // (indexes of the later regions are unchanged: only the name was set)
    assert_eq!(&after[..at], &before[..at], "nothing before it moved");
    assert_eq!(&after[at..after.len() - 1], &before[at + 1..after.len()], "each later tile is one place up");
}

#[test]
fn opening_the_people_view_again_sorts_the_faces_afresh() {
    let (mut h, [p0, _, _]) = scrolled();
    // start from a list that is sorted (the view is opened again), scrolled into the faces
    run(&mut h, "view.photoGrid", json!({}));
    run(&mut h, "view.people", json!({}));
    scroll(&mut h, 500.0);
    assert!(h.app.caches.unnamed_shown.is_some_and(|s| s.above > 0 && s.on_screen > 0), "{:?}", h.app.caches.unnamed_shown);
    let mut regions = h.app.session.catalog.photo(p0).unwrap().meta.regions.clone();
    regions.push(super::person_tiles::region(0.7, None));
    super::person_tiles::set_regions(&mut h, p0, regions);
    h.step();
    h.step();
    let shown: Vec<(u64, usize)> = h.app.caches.unnamed.as_ref().unwrap().3.faces.iter().map(|f| (f.photo, f.index)).collect();
    let as_engine: Vec<(u64, usize)> = engine_order(&mut h).iter().map(|(p, i)| (*p, *i as usize)).collect();
    assert_ne!(shown, as_engine, "the tiles in view were kept, so the list is not in the engine's order yet");
    run(&mut h, "view.photoGrid", json!({}));
    h.step();
    h.step();
    run(&mut h, "view.people", json!({}));
    let shown: Vec<(u64, usize)> = h.app.caches.unnamed.as_ref().unwrap().3.faces.iter().map(|f| (f.photo, f.index)).collect();
    assert_eq!(shown, as_engine, "opened again, the list is sorted afresh");
}

#[test]
fn changing_the_source_sorts_the_faces_afresh() {
    let (mut h, [p0, p1, _]) = scrolled();
    let mut regions = h.app.session.catalog.photo(p0).unwrap().meta.regions.clone();
    regions.push(super::person_tiles::region(0.7, None));
    super::person_tiles::set_regions(&mut h, p0, regions);
    h.step();
    h.step();
    let album = album_of(&mut h, "Both", &[p0, p1]);
    show_album(&mut h, album);
    let shown: Vec<(u64, usize)> = h.app.caches.unnamed.as_ref().unwrap().3.faces.iter().map(|f| (f.photo, f.index)).collect();
    let engine: Vec<(u64, usize)> = engine_order(&mut h).iter().map(|(p, i)| (*p, *i as usize)).collect();
    assert_eq!(shown, engine);
}
