//! The Unnamed faces (`faces.unnamed`) and a person's page (`faces.person`) with `scope: "visible"`: they list the faces of
//! the People view's photos only (the source chosen in the sidebar, every filter, the person filter), while the
//! suggestions and More's scores compare against every named face in the library. Without a scope they list the whole
//! library, as before.

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use super::face_recognize::{region, set_regions, setup, temp, unit64};
use crate::Session;

/// Three photos `[a, b, c]`: `a` has Ann (named) and two unnamed faces, one like Ann and one a stranger; `b` has Bob
/// (named) and an unnamed face like Ann; `c` has an unnamed face like Bob. Album "Set" holds `a` and `c`.
fn library(tag: &str) -> (Session, [PhotoId; 3], u64, std::path::PathBuf) {
    let d = temp(tag);
    let (mut s, _) = setup(&d, 64);
    s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
    let mut ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort();
    let [a, b, c] = [ids[0], ids[1], ids[2]];
    set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.3, None), region(0.5, None)]);
    set_regions(&mut s, b, vec![region(0.1, Some("Bob")), region(0.3, None)]);
    set_regions(&mut s, c, vec![region(0.1, None)]);
    let put = |s: &mut Session, id: PhotoId, x: f64, v: Vec<f32>| s.faces.index.insert(id.0, &region(x, None).rect, v);
    put(&mut s, a, 0.1, unit64(&[(0, 1.0)])); // Ann
    put(&mut s, a, 0.3, unit64(&[(0, 0.99), (2, 0.1)])); // like Ann
    put(&mut s, a, 0.5, unit64(&[(5, 1.0)])); // a stranger
    put(&mut s, b, 0.1, unit64(&[(1, 1.0)])); // Bob
    put(&mut s, b, 0.3, unit64(&[(0, 0.97), (3, 0.2)])); // like Ann, in a photo outside the album
    put(&mut s, c, 0.1, unit64(&[(1, 0.98), (4, 0.1)])); // like Bob
    let album = s.execute("album.create", &json!({"name": "Set"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": album, "ids": [a.0, c.0]})).unwrap();
    (s, [a, b, c], album, d)
}

fn faces(r: &Value, key: &str) -> Vec<(u64, u64)> {
    let mut v: Vec<(u64, u64)> = r[key].as_array().unwrap().iter().map(|f| (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap())).collect();
    v.sort();
    v
}

#[test]
fn an_album_narrows_the_unnamed_faces_and_their_total_but_not_without_a_scope() {
    let (mut s, [a, b, c], album, d) = library("scope-unnamed");
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    let r = s.execute("faces.unnamed", &json!({"scope": "visible"})).unwrap();
    assert_eq!(faces(&r, "faces"), vec![(a.0, 1), (a.0, 2), (c.0, 0)], "{r}");
    assert_eq!(r["total"], 3, "the total is within the scope: {r}");
    let r = s.execute("faces.unnamed", &json!({})).unwrap();
    assert_eq!(faces(&r, "faces"), vec![(a.0, 1), (a.0, 2), (b.0, 1), (c.0, 0)], "no scope: the whole library: {r}");
    assert_eq!(r["total"], 4);
    // any other scope value is the whole library too (it is only a hint, never an error)
    let r = s.execute("faces.unnamed", &json!({"scope": "everything"})).unwrap();
    assert_eq!(r["total"], 4, "{r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_filter_narrows_the_unnamed_faces_too() {
    let (mut s, [a, b, _], _, d) = library("scope-unnamed-filter");
    s.execute("library.filter", &json!({"person": "Bob"})).unwrap();
    let r = s.execute("faces.unnamed", &json!({"scope": "visible"})).unwrap();
    assert_eq!(faces(&r, "faces"), vec![(b.0, 1)], "only the photos with Bob in them: {r}");
    s.execute("library.filter", &json!({"person": "Ann"})).unwrap();
    let r = s.execute("faces.unnamed", &json!({"scope": "visible"})).unwrap();
    assert_eq!(faces(&r, "faces"), vec![(a.0, 1), (a.0, 2)], "{r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn suggestions_in_a_folder_come_from_people_named_only_outside_it() {
    let (mut s, [a, _, c], album, d) = library("scope-unnamed-suggest");
    // the album holds neither Bob nor a face of his: his name is known only from the photo outside it
    s.execute("faces.setName", &json!({"id": a.0, "index": 0, "name": null})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    let r = s.execute("faces.unnamed", &json!({"scope": "visible"})).unwrap();
    let like_bob = r["faces"].as_array().unwrap().iter().find(|f| f["photo"] == c.0).unwrap();
    assert_eq!(like_bob["suggestion"]["name"], "Bob", "Bob is named only in a photo outside the album: {r}");
    assert_eq!(r["faces"][0]["photo"], c.0, "and a suggested face comes first: {r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_persons_confirmed_faces_and_total_follow_the_scope() {
    let (mut s, [a, b, _], album, d) = library("scope-person");
    s.execute("faces.setName", &json!({"id": b.0, "index": 1, "name": "Ann"})).unwrap();
    let r = s.execute("faces.person", &json!({"name": "Ann"})).unwrap();
    assert_eq!((r["total"].clone(), faces(&r, "confirmed")), (json!(2), vec![(a.0, 0), (b.0, 1)]), "{r}");
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    let r = s.execute("faces.person", &json!({"name": "Ann", "scope": "visible"})).unwrap();
    assert_eq!((r["total"].clone(), faces(&r, "confirmed")), (json!(1), vec![(a.0, 0)]), "only her faces in the album: {r}");
    let r = s.execute("faces.person", &json!({"name": "Ann"})).unwrap();
    assert_eq!(r["total"], 2, "no scope: the whole library: {r}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn more_looks_only_at_the_scoped_photos_but_scores_against_all_of_the_persons_faces() {
    let (mut s, [a, b, c], album, d) = library("scope-more");
    // Ann's face in `a` is in the album, the one like her in `b` is not
    let r = s.execute("faces.person", &json!({"name": "Ann"})).unwrap();
    assert_eq!(faces(&r, "more"), vec![(a.0, 1), (b.0, 1)], "{r}");
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    let r = s.execute("faces.person", &json!({"name": "Ann", "scope": "visible"})).unwrap();
    assert_eq!(faces(&r, "more"), vec![(a.0, 1)], "More is drawn from the album's photos only: {r}");
    // a person named only outside the album: no named faces in scope, yet More still searches the album for them
    let r = s.execute("faces.person", &json!({"name": "Bob", "scope": "visible"})).unwrap();
    assert_eq!((r["total"].clone(), faces(&r, "confirmed")), (json!(0), vec![]), "Bob has no named face in the album: {r}");
    assert_eq!(faces(&r, "more"), vec![(c.0, 0)], "but his possible faces in the album are offered, scored against his face outside it: {r}");
    assert_eq!(r["ready"], true);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn more_ignores_a_person_filter_for_the_open_person_only() {
    let (mut s, [a, b, c], _, d) = library("scope-more-filter");
    s.execute("library.filter", &json!({"person": "Bob"})).unwrap();
    // photos with Bob: only `b`; Ann's page asked from there still finds the faces in the photos Bob is not in?
    // no: the filter names Bob, not Ann, so it applies
    let r = s.execute("faces.person", &json!({"name": "Ann", "scope": "visible"})).unwrap();
    assert_eq!((r["total"].clone(), faces(&r, "more")), (json!(0), vec![(b.0, 1)]), "another person's filter still applies: {r}");
    // Bob's own page: the filter naming him is ignored for More (trimmed, any capitalization), his confirmed faces keep it
    s.execute("library.filter", &json!({"person": " bOB "})).unwrap();
    let r = s.execute("faces.person", &json!({"name": "Bob", "scope": "visible"})).unwrap();
    assert_eq!((r["total"].clone(), faces(&r, "confirmed")), (json!(1), vec![(b.0, 0)]), "{r}");
    assert_eq!(faces(&r, "more"), vec![(c.0, 0)], "More from photos Bob is not named in: {r}");
    // a filter bar filter (here a search nothing matches) still applies to More
    s.execute("library.filter", &json!({"person": "Bob", "text": "nothing-is-called-this"})).unwrap();
    let r = s.execute("faces.person", &json!({"name": "Bob", "scope": "visible"})).unwrap();
    assert_eq!(faces(&r, "more"), vec![], "the search still narrows More: {r}");
    let _ = a;
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn hostile_scopes_are_not_errors_and_never_panic() {
    let (mut s, _, _, d) = library("scope-hostile");
    for scope in [json!(7), json!(null), json!(""), json!("VISIBLE"), json!(["visible"]), json!({"a": 1})] {
        assert!(s.execute("faces.unnamed", &json!({"scope": scope})).is_ok());
        assert!(s.execute("faces.person", &json!({"name": "Ann", "scope": scope})).is_ok());
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_changed_set_of_named_faces_or_a_rejection_changes_the_suggestions_after_they_were_worked_out() {
    let (mut s, [a, b, _], _, d) = library("scope-cache");
    let suggestion = |s: &mut Session, photo: PhotoId, index: u64| -> Value {
        let r = s.execute("faces.unnamed", &json!({})).unwrap();
        r["faces"].as_array().unwrap().iter().find(|f| f["photo"] == photo.0 && f["index"] == index).unwrap()["suggestion"]["name"].clone()
    };
    assert_eq!(suggestion(&mut s, a, 1), "Ann");
    assert_eq!(suggestion(&mut s, a, 1), "Ann", "asked again, the same answer");
    // a rejection of that name takes it away
    s.execute("faces.reject", &json!({"faces": [{"photo": a.0, "index": 1}], "name": "Ann"})).unwrap();
    assert_eq!(suggestion(&mut s, a, 1), json!(null));
    // the face in `b` that looks like Ann is suggested her, until the face that makes her known is no longer named
    assert_eq!(suggestion(&mut s, b, 1), "Ann");
    s.execute("faces.setName", &json!({"id": a.0, "index": 0, "name": null})).unwrap();
    assert_eq!(suggestion(&mut s, b, 1), json!(null), "with nobody named Ann, nothing suggests her any more");
    // naming it again brings the suggestion back
    s.execute("faces.setName", &json!({"id": a.0, "index": 0, "name": "Ann"})).unwrap();
    assert_eq!(suggestion(&mut s, b, 1), "Ann");
    let _ = std::fs::remove_dir_all(&d);
}
