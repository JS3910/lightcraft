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
        ..Default::default()
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
    assert_eq!(r["hidden"], 0, "a collapsed stack keeps all its photos in scope, so none is left out: {r}");
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
        assert_eq!(r["hidden"], 0, "neither Dan's face nor the deleted photo's face is counted as left out: {r}");
    }
    // Recently Deleted shows the deleted photo, which is still not a library photo
    s.execute("library.source", &json!({"kind": "recentlyDeleted"})).unwrap();
    assert_eq!(s.visible().len(), 1, "the grid shows the deleted photo");
    let r = cards(&mut s, Some("visible"));
    assert_eq!(summary(&r), owned(&[]), "{r}");
    assert_eq!(r["hidden"], 6, "the library's six faces, not Dan's nor the deleted photo's: {r}");
}

#[test]
fn a_folder_chosen_in_the_sidebar_narrows_the_unnamed_faces_and_includes_its_subfolders() {
    let (mut s, ids) = library();
    let e = add(&mut s, "/pics/2024/e.jpg", 2, &[("", 0.2)]);
    let f = add(&mut s, "/pics/2025/f.jpg", 2, &[("", 0.2)]);
    let listed = |s: &mut Session, scope: Option<&str>| -> (Vec<(u64, u64)>, u64) {
        let p = scope.map_or(json!({}), |sc| json!({"scope": sc}));
        let r = s.execute("faces.unnamed", &p).unwrap();
        let mut v: Vec<(u64, u64)> =
            r["faces"].as_array().unwrap().iter().map(|x| (x["photo"].as_u64().unwrap(), x["index"].as_u64().unwrap())).collect();
        v.sort();
        (v, r["total"].as_u64().unwrap())
    };
    s.execute("library.source", &json!({"kind": "libraryFolder", "path": "/pics/2024"})).unwrap();
    let mut want = vec![(ids[1].0, 1), (e.0, 0)];
    want.sort();
    assert_eq!(listed(&mut s, Some("visible")), (want, 2), "the folder's photos, its subfolder's too");
    s.execute("library.source", &json!({"kind": "libraryFolder", "path": "/pics/2025"})).unwrap();
    assert_eq!(listed(&mut s, Some("visible")), (vec![(f.0, 0)], 1));
    let (all, total) = listed(&mut s, None);
    assert_eq!((all.len(), total), (3, 3), "no scope: the whole library");
}

/// A folder of two small PNGs on disk.
fn png_folder(tag: &str, with_subfolder: bool) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lc-people-view-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("in")).unwrap();
    for (name, seed) in [("a.png", 1u8), ("b.png", 2)] {
        let img = lightcraft_raster::Rgba8::from_fn(16, 12, |x, y| [(x * 9) as u8, (y * 11) as u8, seed, 255]);
        let opts = crate::export::ExportOptions { format: crate::export::ExportFormat::Png, ..Default::default() };
        let bytes = crate::export::encode_image(&img, &opts).unwrap();
        let target = if with_subfolder && name == "b.png" { dir.join("in").join(name) } else { dir.join(name) };
        std::fs::write(target, bytes).unwrap();
    }
    dir
}

#[test]
fn a_browsed_local_folder_shows_only_its_library_photos_in_the_cards() {
    let (s, _) = library();
    let mut s = s.with_fs();
    let dir = png_folder("local", false);
    s.execute("library.browse", &json!({"path": dir.to_string_lossy()})).unwrap();
    let mut browsed: Vec<PhotoId> = s.catalog.photos().filter(|p| p.local).map(|p| p.id).collect();
    browsed.sort();
    assert_eq!(browsed.len(), 2);
    let (local, mine) = (browsed[0], browsed[1]);
    s.execute("photo.addToLibrary", &json!({"ids": [mine.0]})).unwrap();
    for (id, name) in [(local, "Dan"), (mine, "Eve")] {
        let mut meta = s.catalog.photo(id).unwrap().meta.clone();
        meta.regions = vec![face(0.2, Some(name))];
        s.catalog.apply(Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
    }
    s.execute("library.browse", &json!({"path": dir.to_string_lossy()})).unwrap();
    assert!(s.visible().contains(&local), "the grid shows the Local photo");
    assert_eq!(s.people_view_photos(), vec![mine], "the browsed photo is not a library photo");
    let r = cards(&mut s, Some("visible"));
    assert_eq!(summary(&r), owned(&[("Eve", 1)]), "{r}");
    assert_eq!(r["hidden"], 7, "the library's seven faces in /pics, not Dan's: {r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_local_folder_source_includes_its_subfolders_only_when_asked() {
    for subfolders in [false, true] {
        let (s, _) = library();
        let mut s = s.with_fs();
        let dir = png_folder(&format!("sub{subfolders}"), true);
        s.execute("library.browse", &json!({"path": dir.to_string_lossy(), "subfolders": subfolders})).unwrap();
        assert_eq!(s.visible().len(), if subfolders { 2 } else { 1 }, "subfolders {subfolders}: the grid's photos");
        let mut browsed: Vec<PhotoId> = s.catalog.photos().filter(|p| p.local).map(|p| p.id).collect();
        browsed.sort();
        // add every browsed photo to My Photos, with a named face each
        let all: Vec<u64> = browsed.iter().map(|i| i.0).collect();
        s.execute("photo.addToLibrary", &json!({"ids": all})).unwrap();
        for (n, id) in browsed.iter().enumerate() {
            let mut meta = s.catalog.photo(*id).unwrap().meta.clone();
            meta.regions = vec![face(0.2, Some(if n == 0 { "Top" } else { "Deep" }))];
            s.catalog.apply(Op::SetMeta { id: *id, meta: Box::new(meta) }).unwrap();
        }
        s.execute("library.browse", &json!({"path": dir.to_string_lossy(), "subfolders": subfolders})).unwrap();
        let r = cards(&mut s, Some("visible"));
        let names: Vec<String> = summary(&r).into_iter().map(|(n, _)| n).filter(|n| n == "Top" || n == "Deep").collect();
        assert_eq!(names.len(), if subfolders { 2 } else { 1 }, "subfolders {subfolders}: {r}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Photo 0 (Ann, Bob) has every attribute a filter-bar field looks for; photo 1 (Cat) has none of them.
#[test]
fn each_filter_bar_field_narrows_the_cards() {
    let cases = [
        json!({"flag": "pick"}),
        json!({"flags": ["pick"]}),
        json!({"label": "red"}),
        json!({"labels": ["red"]}),
        json!({"kind": "image"}),
        json!({"edited": true}),
        json!({"date": "2024"}),
        json!({"dateFrom": "2024-01-01", "dateTo": "2024-12"}),
        json!({"keyword": "travel"}),
        json!({"camera": "Acme X1"}),
        json!({"lens": "50mm"}),
        json!({"rating": 4, "ratingOp": "atLeast"}),
    ];
    for filter in cases {
        let mut s = Session::new();
        let id = s.catalog.alloc_photo_id();
        let mut a = Photo::new(id, Source::File { path: "/pics/a.jpg".into() }, "a.jpg", "JPEG", 100, 100, "2024-05-01T10:00:00");
        a.flag = lightcraft_catalog::Flag::Pick;
        a.label = Some(lightcraft_catalog::ColorLabel::Red);
        a.edited = Some("2024-05-02T10:00:00".into());
        std::sync::Arc::make_mut(&mut a.develop).light.exposure = 0.7;
        a.rating = 5;
        a.captured = Some("2024-05-01T10:00:00".into());
        a.meta.keywords = vec!["travel|italy".into()];
        a.meta.camera = "Acme X1".into();
        a.meta.lens = "50mm".into();
        a.meta.regions = vec![face(0.2, Some("Ann")), face(0.1, Some("Bob"))];
        s.catalog.apply(Op::AddPhoto { photo: Box::new(a) }).unwrap();
        let bid = s.catalog.alloc_photo_id();
        let mut b = Photo::new(bid, Source::File { path: "/pics/b.mov".into() }, "b.mov", "MOV", 100, 100, "2025-05-01T10:00:00");
        b.kind = lightcraft_catalog::MediaKind::Video;
        b.rating = 1;
        b.captured = Some("2025-05-01T10:00:00".into());
        b.meta.regions = vec![face(0.2, Some("Cat"))];
        s.catalog.apply(Op::AddPhoto { photo: Box::new(b) }).unwrap();
        let r = s.execute("library.filter", &filter).unwrap();
        assert_eq!(r["count"], 1, "{filter}: the filter leaves one photo: {r}");
        let r = cards(&mut s, Some("visible"));
        assert_eq!(summary(&r), owned(&[("Ann", 1), ("Bob", 1)]), "{filter}: {r}");
        assert_eq!(r["hidden"], 1, "{filter}: Cat's face is left out: {r}");
    }
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
        assert_eq!(r["hidden"], if hold { 3 } else { 5 }, "hold {hold}: {r}");
    }
}
