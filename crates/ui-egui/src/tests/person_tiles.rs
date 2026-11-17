//! A person's page on the shared face tile, driven through the control channel: the always-visible name strip on the
//! confirmed and the "More" faces, naming in place (another person's name on a More face, a correction on a confirmed
//! one), selecting instead of confirming, one selection across the People view's galleries, and the hover buttons the
//! "More" tiles switch on.

use std::sync::Arc;
use std::time::Duration;

use lightcraft_catalog::{Op, PhotoId};
use serde_json::json;

use crate::headless::Headless;
use crate::panels::person::{Face, PersonPage};
use crate::state::ViewMode;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn region(x: f64, name: Option<&str>) -> lightcraft_meta::Region {
    lightcraft_meta::Region {
        rect: lightcraft_geom::Rect { x0: x, y0: 0.2, x1: x + 0.2, y1: 0.6 },
        kind: lightcraft_meta::RegionKind::Face,
        name: name.map(str::to_string),
        description: None,
    }
}

fn set_regions(h: &mut Headless, id: PhotoId, regions: Vec<lightcraft_meta::Region>) {
    let mut meta = h.app.session.catalog.photo(id).unwrap().meta.clone();
    meta.regions = regions;
    h.app.session.commit("setup", Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
}

/// What the engine would say about Ann Lee's page, rebuilt from the catalog (the headless build has no recognition):
/// her named faces, and every unnamed face as a More face. Scores: photos 0 to 4 0.9, 0.85, 0.8, 0.8 and 0.7 (a tie
/// between the third and the fourth), the rest 0.5.
fn page_now(h: &Headless) -> PersonPage {
    let mut page = PersonPage { name: "Ann Lee".into(), ready: true, ..Default::default() };
    for (n, p) in h.app.session.catalog.photos().enumerate() {
        for (i, r) in p.meta.regions.iter().enumerate() {
            let score = [0.9, 0.85, 0.8, 0.8, 0.7].get(n).copied().unwrap_or(0.5);
            let face = Face { photo: p.id.0, index: i, rect: r.rect, score };
            match r.name.as_deref() {
                Some("Ann Lee") => page.confirmed.push(face),
                None => page.more.push(face),
                Some(_) => {}
            }
        }
    }
    page.total = page.confirmed.len();
    page
}

fn inject(h: &mut Headless) {
    let page = page_now(h);
    h.app.caches.person_page = Some(("Ann Lee".into(), h.app.session.catalog.revision, h.app.caches.faces_indexed, f64::MAX, Arc::new(page)));
    h.step();
    h.step();
}

fn ok(h: &mut Headless, method: &str, params: serde_json::Value) {
    let r = h.request(method, params.clone(), T);
    assert_eq!(r["ok"], true, "{method} {params}: {r}");
    inject(h);
}

fn key(h: &mut Headless, key: &str) {
    ok(h, "ui.key", json!({"key": key}));
}

/// Ann Lee's page. Photos 0 and 1 hold her confirmed faces (photo 1 also a face named Wrong Person); photo 2 an unnamed
/// face and a Bob Ray face; photos 3 and 4 an unnamed face each. The More faces are (p2,0) 0.80, (p3,0) 0.80, (p4,0) 0.70.
fn page() -> (Headless, [PhotoId; 5]) {
    let services = Services { png: None, write_shared: Some(Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    h.settle(T);
    let ids: Vec<PhotoId> = h.app.session.catalog.photos().map(|p| p.id).take(5).collect();
    set_regions(&mut h, ids[0], vec![region(0.1, Some("Ann Lee"))]);
    set_regions(&mut h, ids[1], vec![region(0.1, Some("Ann Lee")), region(0.5, Some("Wrong Person"))]);
    set_regions(&mut h, ids[2], vec![region(0.3, None), region(0.6, Some("Bob Ray"))]);
    set_regions(&mut h, ids[3], vec![region(0.3, None)]);
    set_regions(&mut h, ids[4], vec![region(0.3, None)]);
    ok(&mut h, "engine.execute", json!({"command": "view.person", "params": {"name": "Ann Lee"}}));
    h.settle(T);
    inject(&mut h);
    (h, [ids[0], ids[1], ids[2], ids[3], ids[4]])
}

fn names(h: &Headless, id: PhotoId) -> Vec<Option<String>> {
    h.app.session.catalog.photo(id).unwrap().meta.regions.iter().map(|r| r.name.clone()).collect()
}

fn n(s: &str) -> Option<String> {
    Some(s.to_string())
}

fn widgets(h: &mut Headless, filter: &str) -> Vec<String> {
    let r = h.request("ui.widgets", json!({"filter": filter}), T);
    r["result"].as_array().into_iter().flatten().filter_map(|w| w["id"].as_str().map(str::to_string)).collect()
}

fn more(id: PhotoId, i: usize) -> String {
    format!("more-face:{}:{i}", id.0)
}

fn more_name(id: PhotoId, i: usize) -> String {
    format!("more-name:{}:{i}", id.0)
}

fn done(id: PhotoId, i: usize) -> String {
    format!("person-face:{}:{i}", id.0)
}

fn done_name(id: PhotoId, i: usize) -> String {
    format!("person-name:{}:{i}", id.0)
}

fn selected(h: &Headless) -> usize {
    h.app.ui.unnamed_selected.len()
}

fn shown(h: &Headless) -> String {
    h.app.ui.name_edit.as_ref().map(|e| e.input.text.clone()).unwrap_or_default()
}

#[test]
fn every_confirmed_and_more_tile_has_a_name_strip() {
    let (mut h, [p0, p1, p2, p3, p4]) = page();
    let strips = widgets(&mut h, "-name:");
    for id in [done_name(p0, 0), done_name(p1, 0), more_name(p2, 0), more_name(p3, 0), more_name(p4, 0)] {
        assert!(strips.contains(&id), "{id} in {strips:?}");
    }
    // a face named as somebody else is on no page of Ann Lee's
    assert!(!strips.contains(&done_name(p1, 1)) && !strips.contains(&more_name(p2, 1)));
}

#[test]
fn a_click_on_a_more_picture_selects_and_never_names() {
    let (mut h, [_, _, p2, p3, p4]) = page();
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": more(p3, 0)}));
    assert_eq!(selected(&h), 1);
    assert_eq!(names(&h, p3), [None], "a click does not confirm");
    ok(&mut h, "ui.clickWidget", json!({"id": more(p4, 0), "shift": true}));
    assert_eq!(selected(&h), 2, "Shift-click selects the range from the last click");
    ok(&mut h, "ui.clickWidget", json!({"id": more(p2, 0), "shift": true}));
    assert_eq!(selected(&h), 3);
    assert_eq!(h.app.session.undo.len(), undo, "nothing was named");
}

#[test]
fn a_click_on_a_confirmed_picture_selects_it_and_a_double_click_opens_the_photo() {
    let (mut h, [p0, p1, ..]) = page();
    ok(&mut h, "ui.clickWidget", json!({"id": done(p0, 0)}));
    assert_eq!((h.app.ui.view, selected(&h)), (ViewMode::People, 1));
    key(&mut h, "escape");
    assert_eq!(selected(&h), 0, "Esc clears the selection before it leaves the page");
    assert_eq!(h.app.ui.person_page.as_deref(), Some("Ann Lee"));
    for _ in 0..30 {
        h.step();
    }
    ok(&mut h, "ui.clickWidget", json!({"id": done(p1, 0), "count": 2}));
    assert_eq!((h.app.ui.view, h.app.session.active()), (ViewMode::Detail, Some(p1)));
    // the way back: Escape returns to the page
    key(&mut h, "escape");
    assert_eq!((h.app.ui.view, h.app.ui.person_page.as_deref()), (ViewMode::People, Some("Ann Lee")));
}

#[test]
fn a_double_click_on_a_more_picture_opens_its_photo_and_confirms_nothing() {
    let (mut h, [_, _, _, p3, _]) = page();
    ok(&mut h, "ui.clickWidget", json!({"id": more(p3, 0), "count": 2}));
    assert_eq!((h.app.ui.view, h.app.session.active()), (ViewMode::Detail, Some(p3)));
    assert_eq!(names(&h, p3), [None]);
}

#[test]
fn another_persons_name_in_a_more_faces_box_names_it_as_that_person() {
    let (mut h, [_, _, p2, p3, _]) = page();
    ok(&mut h, "ui.clickWidget", json!({"id": more_name(p3, 0)}));
    assert_eq!(shown(&h), "Ann Lee", "the box opens with this person's name, selected");
    ok(&mut h, "ui.text", json!({"text": "Bob Ray"}));
    key(&mut h, "enter");
    assert_eq!(names(&h, p3), [n("Bob Ray")]);
    assert!(!widgets(&mut h, "more-face:").contains(&more(p3, 0)), "it left this person's More");
    assert_eq!(names(&h, p2), [None, n("Bob Ray")]);
}

#[test]
fn enter_on_an_untouched_more_box_confirms_the_face_as_this_person() {
    let (mut h, [_, _, _, p3, _]) = page();
    ok(&mut h, "ui.clickWidget", json!({"id": more_name(p3, 0)}));
    key(&mut h, "enter");
    assert_eq!(names(&h, p3), [n("Ann Lee")]);
    assert!(h.app.ui.name_edit.is_none());
}

#[test]
fn the_box_on_a_confirmed_face_renames_that_one_face_and_emptying_it_removes_the_name() {
    let (mut h, [p0, p1, ..]) = page();
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": done_name(p1, 0)}));
    assert_eq!(shown(&h), "Ann Lee");
    ok(&mut h, "ui.text", json!({"text": "Cy Dee"}));
    key(&mut h, "enter");
    assert_eq!((names(&h, p1), names(&h, p0)), (vec![n("Cy Dee"), n("Wrong Person")], vec![n("Ann Lee")]));
    assert_eq!(h.app.session.undo.len(), undo + 1);
    // emptied: the name goes
    ok(&mut h, "ui.clickWidget", json!({"id": done_name(p0, 0)}));
    key(&mut h, "backspace");
    key(&mut h, "enter");
    assert_eq!(names(&h, p0), [None]);
}

#[test]
fn an_untouched_confirmed_box_is_not_an_edit() {
    let (mut h, [_, p1, ..]) = page();
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": done_name(p1, 0)}));
    key(&mut h, "enter");
    assert_eq!(h.app.session.undo.len(), undo, "Enter on the name as it is changes nothing");
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(names(&h, p1)[0], n("Ann Lee"));
}

#[test]
fn a_selection_lives_in_one_gallery_at_a_time() {
    let (mut h, [p0, p1, _, p3, p4]) = page();
    ok(&mut h, "ui.clickWidget", json!({"id": done(p0, 0)}));
    ok(&mut h, "ui.clickWidget", json!({"id": done(p1, 0)}));
    assert_eq!(selected(&h), 2);
    ok(&mut h, "ui.clickWidget", json!({"id": more(p3, 0)}));
    assert_eq!(selected(&h), 1, "selecting in More replaced the confirmed selection");
    assert!(h.app.ui.unnamed_selected.contains(&(p3.0, 0)));
    ok(&mut h, "ui.clickWidget", json!({"id": more(p4, 0)}));
    assert_eq!(selected(&h), 2);
    ok(&mut h, "ui.clickWidget", json!({"id": done(p0, 0)}));
    assert_eq!(selected(&h), 1, "and back");
    // the same face is in the Unnamed faces and in More: they are two galleries
    ok(&mut h, "ui.clickWidget", json!({"id": more(p3, 0)}));
    ok(&mut h, "engine.execute", json!({"command": "view.people"}));
    ok(&mut h, "ui.clickWidget", json!({"id": format!("unnamed-face:{}:0", p4.0)}));
    assert_eq!(selected(&h), 1, "the People view's click replaced the page's selection");
    assert!(h.app.ui.unnamed_selected.contains(&(p4.0, 0)));
}

#[test]
fn a_box_on_a_selected_tile_names_the_whole_selection_in_one_undo_step() {
    let (mut h, [_, _, _, p3, p4]) = page();
    ok(&mut h, "ui.clickWidget", json!({"id": more(p3, 0)}));
    ok(&mut h, "ui.clickWidget", json!({"id": more(p4, 0)}));
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": more_name(p4, 0)}));
    ok(&mut h, "ui.text", json!({"text": "Bob Ray"}));
    key(&mut h, "enter");
    assert_eq!((names(&h, p3), names(&h, p4)), (vec![n("Bob Ray")], vec![n("Bob Ray")]));
    assert_eq!(h.app.session.undo.len(), undo + 1);
    ok(&mut h, "engine.execute", json!({"command": "edit.undo"}));
    assert_eq!((names(&h, p3), names(&h, p4)), (vec![None], vec![None]));
}

#[test]
fn a_box_on_a_more_tile_ignores_a_selection_in_the_other_gallery() {
    let (mut h, [p0, _, _, p3, _]) = page();
    ok(&mut h, "ui.clickWidget", json!({"id": done(p0, 0)}));
    ok(&mut h, "ui.clickWidget", json!({"id": more_name(p3, 0)}));
    ok(&mut h, "ui.text", json!({"text": "Bob Ray"}));
    key(&mut h, "enter");
    assert_eq!((names(&h, p0), names(&h, p3)), (vec![n("Ann Lee")], vec![n("Bob Ray")]));
}

#[test]
fn the_hover_buttons_on_more_faces_still_confirm_all_above_and_hide() {
    let (mut h, [_, _, p2, p3, p4]) = page();
    // the buttons exist on More tiles, and not on the confirmed ones
    for kind in ["face-confirm", "face-above", "face-hide"] {
        assert!(widgets(&mut h, &format!("more-{kind}:")).contains(&format!("more-{kind}:{}:0", p3.0)), "{kind}");
        assert!(widgets(&mut h, &format!("person-{kind}:")).is_empty(), "{kind}");
    }
    // confirm: this face alone
    ok(&mut h, "ui.clickWidget", json!({"id": format!("more-face-confirm:{}:0", p4.0)}));
    assert_eq!((names(&h, p4), names(&h, p3)), (vec![n("Ann Lee")], vec![None]));
    // all above: the face and every shown face at least as alike, ties included (p2 0.80, p3 0.80)
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": format!("more-face-above:{}:0", p3.0)}));
    assert_eq!((names(&h, p2), names(&h, p3)), (vec![n("Ann Lee"), n("Bob Ray")], vec![n("Ann Lee")]));
    assert_eq!(h.app.session.undo.len(), undo + 1);
}

#[test]
fn hide_removes_a_more_face_for_the_session_and_names_nothing() {
    let (mut h, [_, _, _, p3, _]) = page();
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": format!("more-face-hide:{}:0", p3.0)}));
    assert!(h.app.ui.dismissed_faces.contains(&(p3.0, 0)));
    assert!(!widgets(&mut h, "more-face:").contains(&more(p3, 0)));
    assert_eq!((names(&h, p3), h.app.session.undo.len()), (vec![None], undo));
}

#[test]
fn unnamed_faces_without_a_suggestion_have_no_hover_buttons() {
    let (mut h, _) = page();
    ok(&mut h, "engine.execute", json!({"command": "view.people"}));
    assert!(widgets(&mut h, "unnamed-face:").len() >= 3);
    for kind in ["confirm", "above", "hide"] {
        assert!(widgets(&mut h, &format!("unnamed-face-{kind}:")).is_empty(), "{kind}");
    }
}

#[test]
fn escape_closes_a_name_box_on_the_page_without_leaving_it() {
    let (mut h, [_, _, _, p3, _]) = page();
    ok(&mut h, "ui.clickWidget", json!({"id": more_name(p3, 0)}));
    assert!(h.app.ui.name_edit.is_some());
    key(&mut h, "escape");
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(h.app.ui.person_page.as_deref(), Some("Ann Lee"), "only the box closed");
    assert_eq!(names(&h, p3), [None]);
}
