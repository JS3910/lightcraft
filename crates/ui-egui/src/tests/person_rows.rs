//! A person's page shows their confirmed faces in at most three rows, the most varied faces first, and the rest behind
//! "Show all N". Driven through the control channel on the real engine (recognition is off here, so the faces are in
//! the order the catalog lists them; the picking itself is tested in the engine).

use std::sync::Arc;
use std::time::Duration;

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use super::person_tiles::region;
use super::unnamed_tiles::set_regions;
use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn run(h: &mut Headless, command: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    h.step();
    h.step();
    h.settle(T);
    r["result"].clone()
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.step();
    h.step();
}

/// The confirmed tiles on screen as `(photo, region, top, left)`, top to bottom, left to right.
fn tiles(h: &mut Headless) -> Vec<(u64, usize, i64, i64)> {
    let r = h.request("ui.widgets", json!({"filter": "person-face:"}), T);
    let mut out: Vec<(u64, usize, i64, i64)> = r["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let id = w["id"].as_str()?.strip_prefix("person-face:")?;
            let (photo, index) = id.split_once(':')?;
            Some((photo.parse().ok()?, index.parse().ok()?, w["rect"][1].as_f64()? as i64, w["rect"][0].as_f64()? as i64))
        })
        .collect();
    out.sort_by_key(|(_, _, y, x)| (*y, *x));
    out
}

fn rows_on_screen(h: &mut Headless) -> usize {
    let mut tops: Vec<i64> = tiles(h).iter().map(|t| t.2).collect();
    tops.dedup();
    tops.len()
}

fn said(h: &Headless, text: &str) -> bool {
    h.painted_text().iter().any(|t| t == text)
}

/// Ann Lee's page, with `n` confirmed faces in the first photo of the demo library (and nothing else of hers).
fn ann_with(n: usize) -> (Headless, Vec<PhotoId>) {
    let services = Services { png: None, write_shared: Some(Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let mut h = Headless::new(LightcraftApp::new(lightcraft_engine::Session::with_demo(), services), [1400.0, 1100.0], 1.0);
    h.settle(T);
    let ids: Vec<PhotoId> = h.app.session.catalog.photos().map(|p| p.id).take(4).collect();
    set_regions(&mut h, ids[0], (0..n).map(|i| region(i as f64 * 0.01, Some("Ann Lee"))).collect());
    run(&mut h, "view.person", json!({"name": "Ann Lee"}));
    (h, ids)
}

#[test]
fn many_confirmed_faces_show_three_rows_and_a_button_that_opens_the_rest_in_place() {
    let (mut h, _) = ann_with(60);
    let per_row = tiles(&mut h).iter().take_while(|t| t.2 == tiles(&mut h)[0].2).count();
    assert!(per_row >= 3, "a row has several tiles: {per_row}");
    assert_eq!(rows_on_screen(&mut h), 3, "three rows");
    assert_eq!(tiles(&mut h).len(), 3 * per_row, "and no more tiles than three rows hold");
    assert!(said(&h, "Show all 60"), "{:?}", h.painted_text());
    click(&mut h, "person:showAll");
    assert!(said(&h, "Show fewer") && !said(&h, "Show all 60"), "{:?}", h.painted_text());
    assert!(rows_on_screen(&mut h) > 3, "the rest opened in place");
    let first: Vec<(u64, usize)> = tiles(&mut h).iter().take(3 * per_row).map(|t| (t.0, t.1)).collect();
    assert_eq!(first, (0..3 * per_row).map(|i| (tiles(&mut h)[0].0, i)).collect::<Vec<_>>(), "the faces at the top stay where they were");
    click(&mut h, "person:showAll");
    assert_eq!(rows_on_screen(&mut h), 3, "and back to three rows");
    assert!(said(&h, "Show all 60"));
}

#[test]
fn faces_that_fit_in_three_rows_have_no_button() {
    let (mut h, _) = ann_with(5);
    assert_eq!(tiles(&mut h).len(), 5);
    assert!(h.painted_text().iter().all(|t| !t.starts_with("Show all") && t != "Show fewer"), "{:?}", h.painted_text());
    assert!(h.request("ui.widgets", json!({"filter": "person:showAll"}), T)["result"].as_array().is_none_or(Vec::is_empty));
}

/// How many tiles a row has, from the first tile's row.
fn per_row(h: &mut Headless) -> usize {
    let t = tiles(h);
    t.iter().take_while(|x| x.2 == t[0].2).count()
}

fn named_ann(h: &Headless) -> usize {
    h.app.session.catalog.photos().flat_map(|p| p.meta.regions.iter()).filter(|r| r.name.as_deref() == Some("Ann Lee")).count()
}

#[test]
fn opening_the_page_again_starts_with_three_rows() {
    let (mut h, _) = ann_with(60);
    click(&mut h, "person:showAll");
    assert!(said(&h, "Show fewer"));
    run(&mut h, "view.people", json!({}));
    for _ in 0..3 {
        h.step();
    }
    run(&mut h, "view.person", json!({"name": "Ann Lee"}));
    assert_eq!(rows_on_screen(&mut h), 3, "closed again");
    assert!(said(&h, "Show all 60"), "{:?}", h.painted_text());
}

#[test]
fn a_face_confirmed_during_the_visit_takes_a_place_in_the_three_rows_before_the_others() {
    let (mut h, ids) = ann_with(60);
    set_regions(&mut h, ids[1], vec![region(0.3, None)]);
    set_regions(&mut h, ids[2], vec![region(0.3, None)]);
    h.settle(T);
    let per = per_row(&mut h);
    let before = tiles(&mut h);
    let name = |h: &mut Headless, photo: PhotoId| run(h, "faces.nameFaces", json!({"faces": [{"photo": photo.0, "index": 0}], "name": "Ann Lee"}));
    name(&mut h, ids[1]);
    let after = tiles(&mut h);
    assert_eq!(rows_on_screen(&mut h), 3, "the section does not grow");
    assert_eq!(after.len(), before.len());
    assert_eq!((after[0].0, after[0].1), (ids[1].0, 0), "the new face is first: {after:?}");
    assert_eq!((after[1].0, after[1].1), (ids[0].0, 0), "and the others follow in the order they had");
    assert!(said(&h, "Show all 61"), "{:?}", h.painted_text());
    name(&mut h, ids[2]);
    let after = tiles(&mut h);
    assert_eq!(
        (after[0].0, after[1].0, after[2].0, after.len()),
        (ids[2].0, ids[1].0, ids[0].0, 3 * per),
        "the latest first, then the one before: {after:?}"
    );
    // opened, the same faces stay at the top and the rest follow
    click(&mut h, "person:showAll");
    let open = tiles(&mut h);
    assert_eq!(open.iter().take(3 * per).map(|t| (t.0, t.1)).collect::<Vec<_>>(), after.iter().map(|t| (t.0, t.1)).collect::<Vec<_>>());
    assert_eq!((open[3 * per].0, open[3 * per].1), (ids[0].0, 3 * per - 2), "the rest as before: {:?}", &open[3 * per..]);
}

#[test]
fn select_all_and_the_delete_key_act_on_the_tiles_on_screen_only() {
    let (mut h, _) = ann_with(60);
    let per = per_row(&mut h);
    click(&mut h, "person:selectAll");
    assert_eq!(h.app.ui.unnamed_selected.len(), 3 * per, "only the visible tiles are selected");
    let on_screen: Vec<(u64, usize)> = tiles(&mut h).iter().map(|t| (t.0, t.1)).collect();
    assert!(h.app.ui.unnamed_selected.iter().all(|k| on_screen.contains(k)));
    let r = h.request("ui.key", json!({"key": "Delete"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    assert_eq!(named_ann(&h), 60 - 3 * per, "the faces left out of the rows are still hers");
    // opened, select all takes every face; closing the section leaves only those on screen selected
    let (mut h, _) = ann_with(60);
    click(&mut h, "person:showAll");
    click(&mut h, "person:selectAll");
    assert_eq!(h.app.ui.unnamed_selected.len(), 60);
    click(&mut h, "person:showAll");
    assert_eq!(h.app.ui.unnamed_selected.len(), 3 * per_row(&mut h), "a hidden face is never selected");
}

#[test]
fn a_source_change_that_leaves_fewer_confirmed_faces_updates_the_rows_and_the_count() {
    let (mut h, ids) = ann_with(30);
    set_regions(&mut h, ids[1], (0..4).map(|i| region(i as f64 * 0.1, Some("Ann Lee"))).collect());
    h.settle(T);
    assert!(said(&h, "Show all 34"), "{:?}", h.painted_text());
    let album = run(&mut h, "album.create", json!({"name": "Few"}))["id"].as_u64().unwrap();
    run(&mut h, "album.addPhotos", json!({"id": album, "ids": [ids[1].0]}));
    run(&mut h, "library.source", json!({"kind": "album", "id": album}));
    assert_eq!(tiles(&mut h).len(), 4, "the four faces of the album's photo");
    assert!(h.painted_text().iter().all(|t| !t.starts_with("Show all")), "{:?}", h.painted_text());
    run(&mut h, "library.source", json!({"kind": "all"}));
    assert!(said(&h, "Show all 34"), "{:?}", h.painted_text());
}

#[test]
fn the_page_asks_for_the_most_varied_faces_of_the_widest_window() {
    let r = crate::panels::person::request("Ann Lee");
    assert_eq!((r["name"].as_str(), r["scope"].as_str()), (Some("Ann Lee"), Some("visible")));
    let picks = r["varied"].as_u64().unwrap();
    assert!((3 * 50..=5000).contains(&picks), "three rows of a 4K window, bounded: {picks}");
}
