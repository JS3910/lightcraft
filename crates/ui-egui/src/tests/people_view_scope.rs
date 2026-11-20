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
    assert!(said(&h, "2") && !said(&h, "4"), "the header counts the faces within scope: {:?}", h.painted_text());
    let _ = (p0, p2);
    let other = album_of(&mut h, "Other", &[p2]);
    show_album(&mut h, other);
    assert_eq!(ids(&mut h, "unnamed-face:"), [picture(p2, 0)]);
    assert!(said(&h, "1") && !said(&h, "2"), "{:?}", h.painted_text());
    run(&mut h, "library.source", json!({"kind": "all"}));
    assert_eq!(ids(&mut h, "unnamed-face:").len(), 4);
    assert!(said(&h, "4"), "{:?}", h.painted_text());
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
    assert!(
        said(&h, "Face names written to XMP by Lightroom and other apps show up here, and faces found in your photos can be named."),
        "the existing explanation: {:?}",
        h.painted_text()
    );
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
    assert!(!unnamed_photos(&h).contains(&p1.0) && unnamed_photos(&h).contains(&p2.0), "with the view not held, its faces leave the list at once");
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

/// The boxes of the faces named `(photo, index)`, as the catalog has them now.
fn boxes(h: &Headless, list: &[(u64, usize)]) -> Vec<[f64; 4]> {
    list.iter()
        .map(|(p, i)| {
            let r = h.app.session.catalog.photo(PhotoId(*p)).and_then(|ph| ph.meta.regions.get(*i).map(|r| r.rect)).unwrap();
            [r.x0, r.y0, r.x1, r.y1]
        })
        .collect()
}

#[test]
fn removing_a_box_above_the_screen_and_undoing_it_leave_the_tiles_on_screen_showing_the_same_faces() {
    let (mut h, [_, p1, _]) = scrolled();
    let before = tiles(&mut h);
    let seen = boxes(&h, &before);
    let shown = h.app.caches.unnamed_shown.unwrap();
    assert!(shown.above > 0);
    // the face just above the screen goes: the later regions of its photo are renumbered, but no tile on screen changes its face
    let list = h.app.caches.unnamed.as_ref().unwrap().3.clone();
    let gone = &list.faces[shown.above - 1];
    assert_eq!(gone.photo, p1.0, "a face of the photo the screen is in");
    run(&mut h, "faces.removeBoxes", json!({"faces": [{"photo": gone.photo, "index": gone.index}]}));
    let after = tiles(&mut h);
    assert_eq!(boxes(&h, &after)[..seen.len()], seen[..], "the tiles on screen show the faces they showed: {before:?} then {after:?}");
    run(&mut h, "edit.undo", json!({}));
    let back = tiles(&mut h);
    assert_eq!(boxes(&h, &back)[..seen.len()], seen[..], "so does undo");
}

#[test]
fn removing_a_box_on_screen_makes_its_tile_leave_and_the_tiles_after_it_move_up() {
    let (mut h, [_, p1, _]) = scrolled();
    let before = tiles(&mut h);
    let seen = boxes(&h, &before);
    let at = before.len() / 2;
    assert_eq!(before[at].0, p1.0);
    run(&mut h, "faces.removeBoxes", json!({"faces": [{"photo": before[at].0, "index": before[at].1}]}));
    let after = tiles(&mut h);
    let now = boxes(&h, &after);
    assert_eq!(now[..at], seen[..at], "nothing before it moved");
    assert_eq!(now[at..seen.len() - 1], seen[at + 1..], "each later tile is one place up");
}

#[test]
fn a_refresh_with_a_changed_order_while_scrolled_keeps_the_tiles_on_screen() {
    let (mut h, [_, p1, _]) = scrolled();
    let before = tiles(&mut h);
    let seen = boxes(&h, &before);
    assert!(h.app.caches.unnamed_shown.unwrap().above > 0, "scrolled part-way down");
    // a face the scan found sorts into the middle of the tiles on screen (and renumbers the later regions of its photo)
    let at = before[before.len() / 2];
    assert_eq!(at.0, p1.0);
    let mut regions = h.app.session.catalog.photo(p1).unwrap().meta.regions.clone();
    regions.insert(at.1, super::person_tiles::region(0.9, None));
    super::person_tiles::set_regions(&mut h, p1, regions);
    h.step();
    h.step();
    let after = tiles(&mut h);
    assert_eq!(boxes(&h, &after)[..seen.len()], seen[..], "the tiles on screen still show the faces they showed");
    let shown: Vec<(u64, usize)> = h.app.caches.unnamed.as_ref().unwrap().3.faces.iter().map(|f| (f.photo, f.index)).collect();
    let engine: Vec<(u64, usize)> = engine_order(&mut h).iter().map(|(p, i)| (*p, *i as usize)).collect();
    assert_ne!(shown, engine, "the list shown is not in the engine's order: the new face waits below the tiles in view");
    assert_eq!(shown.len(), engine.len());
}

/// The photos whose faces the Unnamed faces list on screen now.
fn unnamed_photos(h: &Headless) -> Vec<u64> {
    let mut v: Vec<u64> = h.app.caches.unnamed.as_ref().map(|c| c.3.faces.iter().map(|f| f.photo).collect()).unwrap_or_default();
    v.dedup();
    v.sort_unstable();
    v
}

/// Photos 1 and 2 (or the ones given) are picks and shown with Hold View Until Refreshed on.
fn hold_picks(h: &mut Headless, photos: &[PhotoId]) {
    let ids: Vec<u64> = photos.iter().map(|p| p.0).collect();
    run(h, "library.select", json!({"ids": ids, "active": ids[0]}));
    run(h, "photo.flag", json!({"ids": ids, "flag": "pick", "keepActive": true}));
    run(h, "library.filter", json!({"flag": "pick"}));
    run(h, "library.holdView", json!({"on": true}));
}

#[test]
fn a_refresh_drops_a_held_photos_faces_from_the_unnamed_faces_as_it_does_from_the_cards() {
    let (mut h, [_, p1, p2]) = people();
    hold_picks(&mut h, &[p1, p2]);
    assert!(unnamed_photos(&h).contains(&p1.0) && unnamed_photos(&h).contains(&p2.0));
    // photo 2 stops being a pick: the held view keeps it, and so do the faces
    run(&mut h, "photo.flag", json!({"ids": [p2.0], "flag": "none", "keepActive": true}));
    assert!(unnamed_photos(&h).contains(&p2.0), "a held photo keeps its faces");
    // Refresh View lets it go, with no catalog revision to say so
    run(&mut h, "library.refresh", json!({}));
    assert!(!unnamed_photos(&h).contains(&p2.0), "refreshed, the faces of the photo that left are gone");
    assert!(unnamed_photos(&h).contains(&p1.0));
    // turning the hold off is the same
    run(&mut h, "library.holdView", json!({"on": true}));
    run(&mut h, "photo.flag", json!({"ids": [p1.0], "flag": "none", "keepActive": true}));
    assert!(unnamed_photos(&h).contains(&p1.0));
    run(&mut h, "library.holdView", json!({"on": false}));
    assert!(!unnamed_photos(&h).contains(&p1.0), "no longer held");
}

#[test]
fn a_refresh_drops_a_held_photos_faces_from_a_persons_page_too() {
    let (mut h, [p0, p1, _]) = people();
    hold_picks(&mut h, &[p0, p1]);
    run(&mut h, "view.person", json!({"name": "Ann Lee"}));
    assert!(!said(&h, "Ann Lee has no named faces in these photos."), "{:?}", h.painted_text());
    run(&mut h, "photo.flag", json!({"ids": [p0.0], "flag": "none", "keepActive": true}));
    assert!(!said(&h, "Ann Lee has no named faces in these photos."), "held: her face is still on the page");
    run(&mut h, "library.refresh", json!({}));
    assert!(said(&h, "Ann Lee has no named faces in these photos."), "{:?}", h.painted_text());
}

#[test]
fn an_edit_on_a_persons_page_keeps_the_selection_and_name_box_of_their_more_faces_under_their_own_person_filter() {
    let (mut h, [_, p1, p2]) = people();
    // photos with Ann: only photo 0; the faces in photos 1 and 2 are More faces, outside that filter
    run(&mut h, "library.filter", json!({"person": "Ann Lee"}));
    run(&mut h, "view.person", json!({"name": "Ann Lee"}));
    let more = h.app.session.people_view_photos_for_more("Ann Lee");
    assert!(more.contains(&p1) && more.contains(&p2), "{more:?}");
    h.app.ui.unnamed_selected.insert((p1.0, 0));
    h.app.ui.unnamed_selected.insert((p2.0, 0));
    h.app.ui.face_gallery = Some(Gallery::More("Ann Lee".into()));
    // one of them is confirmed as Ann (the catalog changed): what the page held of the other stays
    h.app.session.execute("faces.nameFaces", &json!({"faces": [{"photo": p1.0, "index": 0}], "name": "Ann Lee"})).unwrap();
    crate::panels::people::drop_left_photos(&mut h.app);
    let selected: Vec<(u64, usize)> = h.app.ui.unnamed_selected.iter().copied().collect();
    assert!(selected.contains(&(p2.0, 0)), "the other More face stays selected: {selected:?}");
}

#[test]
fn in_a_narrow_window_the_header_keeps_the_source_the_chip_and_the_count_apart() {
    let (mut h, [p0, _, _]) = super::unnamed_tiles::people_in([420.0, 700.0]);
    h.app.ui.last_person = Some("Ann Lee".into());
    let album = album_of(&mut h, "An album with a rather long name for a narrow window", &[p0]);
    show_album(&mut h, album);
    let by = |h: &mut Headless, id: &str| widgets(h, id).into_iter().find(|(i, _)| i == id).map(|(_, r)| r);
    let source = by(&mut h, "people:source").expect("the source name is drawn");
    let chip = by(&mut h, "people:last").expect("the back chip is drawn");
    assert!(source[2] <= chip[0] + 0.5, "the source {source:?} ends before the chip {chip:?}");
    // the count is painted at the right edge: the chip leaves room for it
    assert!(chip[2] < 420.0 - 8.0, "the chip {chip:?} stays inside the panel");
}
