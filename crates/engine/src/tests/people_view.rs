//! The People view's photos (`Session::people_view_photos`) and the Named People cards made from them
//! (`faces.people`): a folder or album chosen in the sidebar, or any filter, narrows the cards to the people
//! named in the photos the grid shows, counts them there, and shows their largest face there.

use lightcraft_catalog::{Op, Photo, PhotoId, Source};
use lightcraft_geom::Rect;
use lightcraft_meta::{Region, RegionKind};
use serde_json::{Value, json};

use crate::Session;

fn face(size: f64, name: Option<&str>) -> Region {
    Region {
        rect: Rect { x0: 0.1, y0: 0.1, x1: 0.1 + size, y1: 0.1 + size },
        kind: RegionKind::Face,
        name: name.map(str::to_string),
        description: None,
        rejected: Vec::new(),
    }
}

/// A photo at `path` with the faces `(name, size)`; every photo is 100 x 100 pixels.
fn add(s: &mut Session, path: &str, rating: u8, faces: &[(&str, f64)]) -> PhotoId {
    let id = s.catalog.alloc_photo_id();
    let mut p = Photo::new(id, Source::File { path: path.into() }, "x.jpg", "JPEG", 100, 100, "2026-01-01T10:00:00");
    p.rating = rating;
    p.meta.regions = faces.iter().map(|(n, size)| face(*size, (!n.is_empty()).then_some(*n))).collect();
    s.catalog.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    id
}

/// Ann in two 2024 photos (one in a subfolder), Bob in one 2024 photo and one 2025 photo, Cat only in 2025.
fn library() -> (Session, Vec<PhotoId>) {
    let mut s = Session::new();
    let ids = vec![
        add(&mut s, "/pics/2024/a.jpg", 5, &[("Ann", 0.2), ("Bob", 0.1)]),
        add(&mut s, "/pics/2024/trip/b.jpg", 3, &[("Ann", 0.3), ("", 0.1)]),
        add(&mut s, "/pics/2025/c.jpg", 5, &[("Bob", 0.4), ("Cat", 0.2)]),
        add(&mut s, "/pics/2025/d.jpg", 1, &[("Cat", 0.3)]),
    ];
    (s, ids)
}

fn cards(s: &mut Session, scope: Option<&str>) -> Value {
    let p = scope.map_or(json!({}), |sc| json!({"scope": sc}));
    s.execute("faces.people", &p).unwrap()
}

fn summary(r: &Value) -> Vec<(String, u64)> {
    r["people"].as_array().unwrap().iter().map(|c| (c["name"].as_str().unwrap().to_string(), c["photos"].as_u64().unwrap())).collect()
}

fn owned(v: &[(&str, u64)]) -> Vec<(String, u64)> {
    v.iter().map(|(n, c)| (n.to_string(), *c)).collect()
}

#[test]
fn without_a_scope_the_cards_cover_the_whole_library_whatever_the_grid_shows() {
    let (mut s, _) = library();
    s.execute("library.source", &json!({"kind": "libraryFolder", "path": "/pics/2024"})).unwrap();
    let r = cards(&mut s, None);
    assert_eq!(summary(&r), owned(&[("Ann", 2), ("Bob", 2), ("Cat", 2)]), "{r}");
    assert_eq!(r["hidden"], 0);
}

#[test]
fn a_folder_chosen_in_the_sidebar_includes_its_subfolders_and_counts_the_faces_it_leaves_out() {
    let (mut s, ids) = library();
    s.execute("library.source", &json!({"kind": "libraryFolder", "path": "/pics/2024"})).unwrap();
    let r = cards(&mut s, Some("visible"));
    assert_eq!(summary(&r), owned(&[("Ann", 2), ("Bob", 1)]), "{r}");
    // Ann's largest face in the folder is the 0.3 one, in the subfolder
    assert_eq!(r["people"][0]["photo"], ids[1].0);
    assert!((r["people"][0]["face"]["x1"].as_f64().unwrap() - 0.4).abs() < 1e-9);
    // 2025: Bob, Cat, Cat
    assert_eq!(r["hidden"], 3);
}

#[test]
fn a_filter_bar_filter_narrows_the_cards() {
    let (mut s, ids) = library();
    s.execute("library.filter", &json!({"rating": 5, "ratingOp": "exactly"})).unwrap();
    let r = cards(&mut s, Some("visible"));
    assert_eq!(summary(&r), owned(&[("Bob", 2), ("Ann", 1), ("Cat", 1)]), "{r}");
    // Cat's only face among the photos shown is the 0.2 one
    let cat = r["people"].as_array().unwrap().iter().find(|c| c["name"] == "Cat").unwrap();
    assert_eq!(cat["photo"], ids[2].0);
    assert_eq!(r["hidden"], 3, "the faces of the two photos the filter leaves out");
}

#[test]
fn the_search_narrows_the_cards_too() {
    let (mut s, _) = library();
    s.execute("library.filter", &json!({"text": "nothing-is-called-this"})).unwrap();
    let r = cards(&mut s, Some("visible"));
    assert_eq!(summary(&r), owned(&[]));
    assert_eq!(r["hidden"], 7);
}

#[test]
fn a_person_filter_is_a_photo_filter_and_shows_everyone_named_in_those_photos() {
    let (mut s, _) = library();
    s.execute("library.filter", &json!({"person": "Ann"})).unwrap();
    let r = cards(&mut s, Some("visible"));
    assert_eq!(summary(&r), owned(&[("Ann", 2), ("Bob", 1)]), "Bob is named in a photo with Ann, Cat is not: {r}");
}

#[test]
fn faces_in_the_hidden_photos_of_a_collapsed_stack_count() {
    let (mut s, ids) = library();
    s.selection = crate::Selection::single(ids[2]);
    s.execute("stack.group", &json!({"ids": [ids[2].0, ids[3].0], "top": ids[2].0, "collapsed": true})).unwrap();
    assert_eq!(s.visible().len(), 3, "the grid shows one photo for the stack");
    let r = cards(&mut s, Some("visible"));
    assert_eq!(summary(&r), owned(&[("Ann", 2), ("Bob", 2), ("Cat", 2)]), "{r}");
    assert_eq!(s.people_view_photos().len(), 4);
}

#[test]
fn deleted_photos_and_photos_only_browsed_in_a_local_folder_do_not_count() {
    let (mut s, ids) = library();
    let local = s.catalog.alloc_photo_id();
    let mut p = Photo::new(local, Source::File { path: "/elsewhere/e.jpg".into() }, "e.jpg", "JPEG", 100, 100, "2026-01-01T10:00:00");
    p.local = true;
    p.meta.regions = vec![face(0.2, Some("Dan"))];
    s.catalog.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    s.catalog.apply(Op::SetDeleted { id: ids[3], deleted: true }).unwrap();
    for scope in [None, Some("visible")] {
        let r = cards(&mut s, scope);
        assert_eq!(summary(&r), owned(&[("Ann", 2), ("Bob", 2), ("Cat", 1)]), "{scope:?}: {r}");
    }
    s.source = crate::LibrarySource::RecentlyDeleted;
    assert!(s.people_view_photos().is_empty(), "a deleted photo is not in the People view's photos");
}

#[test]
fn a_held_view_keeps_the_photos_the_grid_holds_and_a_refreshed_one_drops_them() {
    for hold in [true, false] {
        let (mut s, ids) = library();
        s.execute("library.filter", &json!({"rating": 5, "ratingOp": "exactly"})).unwrap();
        if hold {
            s.execute("library.holdView", &json!({"on": true})).unwrap();
        }
        assert_eq!(cards(&mut s, Some("visible"))["hidden"], 3);
        s.selection = crate::Selection::single(ids[2]);
        s.execute("photo.rate", &json!({"ids": [ids[2].0], "rating": 2})).unwrap();
        let r = cards(&mut s, Some("visible"));
        if hold {
            assert_eq!(summary(&r), owned(&[("Bob", 2), ("Ann", 1), ("Cat", 1)]), "held: {r}");
        } else {
            assert_eq!(summary(&r), owned(&[("Ann", 1), ("Bob", 1)]), "not held: {r}");
        }
    }
}
