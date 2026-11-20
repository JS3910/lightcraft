//! Any edit that takes the active photo out of the view goes on from its place: the photo after it, or the one before it
//! at the end. The rule is the engine's for every command; Compare and Survey opt out with `keepActive`.
use lightcraft_catalog::{Op, Photo, PhotoId, Source};
use lightcraft_geom::Rect;
use lightcraft_meta::{Region, RegionKind};
use serde_json::json;

use crate::{Selection, Session};

/// 10 photos (`01.jpg`..`10.jpg`, ids 1..=10) that all have the keyword `cat`, the caption `sunset` and a face named Ann.
fn tagged() -> Session {
    let mut s = Session::new();
    for id in 1..=10 {
        let mut photo = Photo::new(
            PhotoId(id),
            Source::File { path: format!("/active-moves-on/{id:02}.jpg") },
            &format!("{id:02}.jpg"),
            "JPEG",
            40,
            30,
            "2026-11-20",
        );
        photo.meta.keywords = vec!["cat".into()];
        photo.meta.caption = "sunset".into();
        photo.meta.regions = vec![Region {
            rect: Rect { x0: 0.1, y0: 0.2, x1: 0.4, y1: 0.6 },
            kind: RegionKind::Face,
            name: Some("Ann".into()),
            description: None,
            rejected: Vec::new(),
        }];
        s.catalog.apply(Op::AddPhoto { photo: Box::new(photo) }).unwrap();
    }
    s.execute("library.sort", &json!({"key": "fileName", "ascending": true})).unwrap();
    s
}

fn open(s: &mut Session, id: u64) {
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
}

#[test]
fn un_naming_a_face_under_a_person_filter_moves_on() {
    let mut s = tagged();
    s.execute("library.filter", &json!({"person": "Ann"})).unwrap();
    open(&mut s, 4);
    s.execute("faces.setName", &json!({"id": 4, "index": 0, "name": null})).unwrap();
    assert!(!s.visible_cloned().contains(&PhotoId(4)));
    assert_eq!(s.selection, Selection::single(PhotoId(5)));
}

#[test]
fn removing_a_keyword_under_a_keyword_filter_moves_on() {
    let mut s = tagged();
    s.execute("library.filter", &json!({"keyword": "cat"})).unwrap();
    open(&mut s, 4);
    s.execute("photo.setMeta", &json!({"removeKeywords": ["cat"]})).unwrap();
    assert!(!s.visible_cloned().contains(&PhotoId(4)));
    assert_eq!(s.selection, Selection::single(PhotoId(5)));
}

#[test]
fn editing_a_caption_under_a_text_search_moves_on() {
    let mut s = tagged();
    s.execute("library.filter", &json!({"text": "sunset"})).unwrap();
    open(&mut s, 4);
    s.execute("photo.setMeta", &json!({"caption": "dawn"})).unwrap();
    assert!(!s.visible_cloned().contains(&PhotoId(4)));
    assert_eq!(s.selection, Selection::single(PhotoId(5)));
}

#[test]
fn the_last_photo_of_the_view_hands_on_to_the_one_before_and_an_empty_view_has_none() {
    let mut s = tagged();
    s.execute("library.filter", &json!({"keyword": "cat"})).unwrap();
    open(&mut s, 10);
    s.execute("photo.setMeta", &json!({"removeKeywords": ["cat"]})).unwrap();
    assert_eq!(s.selection, Selection::single(PhotoId(9)));
    // photos named outright: the active photo still moves on
    s.execute("photo.setMeta", &json!({"ids": [9], "removeKeywords": ["cat"]})).unwrap();
    assert_eq!(s.selection, Selection::single(PhotoId(8)));
    s.execute("photo.setMeta", &json!({"ids": [1, 2, 3, 4, 5, 6, 7, 8], "removeKeywords": ["cat"]})).unwrap();
    assert!(s.visible_cloned().is_empty());
    assert_eq!(s.selection, Selection::default());
}

#[test]
fn a_held_view_keeps_the_photo_and_the_active_photo() {
    let mut s = tagged();
    s.execute("library.filter", &json!({"keyword": "cat"})).unwrap();
    s.execute("library.holdView", &json!({"on": true})).unwrap();
    assert_eq!(s.visible_cloned().len(), 10);
    open(&mut s, 4);
    s.execute("photo.setMeta", &json!({"removeKeywords": ["cat"]})).unwrap();
    assert!(s.visible_cloned().contains(&PhotoId(4)));
    assert_eq!(s.selection, Selection::single(PhotoId(4)));
}

#[test]
fn keep_active_is_how_compare_and_survey_keep_the_photo_they_edit() {
    let mut s = tagged();
    s.execute("library.filter", &json!({"keyword": "cat"})).unwrap();
    s.execute("library.select", &json!({"ids": [4, 5], "active": 4})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [4], "keepActive": true, "removeKeywords": ["cat"]})).unwrap();
    assert!(!s.visible_cloned().contains(&PhotoId(4)));
    assert_eq!(s.selection, Selection { ids: vec![PhotoId(4), PhotoId(5)], active: Some(PhotoId(4)) });
}

#[test]
fn edits_that_leave_the_active_photo_in_the_view_change_nothing() {
    let mut s = tagged();
    s.execute("library.filter", &json!({"keyword": "cat"})).unwrap();
    s.execute("library.select", &json!({"ids": [4, 5], "active": 4})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [6], "removeKeywords": ["cat"]})).unwrap();
    s.execute("photo.setMeta", &json!({"caption": "still here"})).unwrap();
    assert_eq!(s.selection, Selection { ids: vec![PhotoId(4), PhotoId(5)], active: Some(PhotoId(4)) });
}
