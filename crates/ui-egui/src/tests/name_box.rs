//! The name box in the loupe, driven through the control channel: what a person types, which keys they press, and what
//! ends up in the catalog.

use std::sync::Arc;
use std::time::Duration;

use lightcraft_catalog::{Op, PhotoId};
use serde_json::json;

use crate::headless::Headless;
use crate::panels::faces::{Hint, Hints};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn region(x: f64, name: Option<&str>) -> lightcraft_meta::Region {
    lightcraft_meta::Region {
        rect: lightcraft_geom::Rect { x0: x, y0: 0.2, x1: x + 0.2, y1: 0.6 },
        kind: lightcraft_meta::RegionKind::Face,
        name: name.map(str::to_string),
        description: None,
        rejected: Vec::new(),
    }
}

fn set_regions(h: &mut Headless, id: PhotoId, regions: Vec<lightcraft_meta::Region>) {
    let mut meta = h.app.session.catalog.photo(id).unwrap().meta.clone();
    meta.regions = regions;
    h.app.session.commit("setup", Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
}

/// A demo library in the loupe on its first photo, which has three face boxes (the given names, `None` = unnamed),
/// while a second photo holds the people Ann Lee, Bob Ray and Joanna Ann.
fn loupe(names: [Option<&str>; 3]) -> (Headless, PhotoId) {
    let services = Services { png: None, write_shared: Some(Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    h.settle(T);
    let id = h.app.session.active().unwrap();
    let other = h.app.session.catalog.photos().map(|p| p.id).find(|p| *p != id).unwrap();
    set_regions(&mut h, other, vec![region(0.1, Some("Ann Lee")), region(0.4, Some("Bob Ray")), region(0.7, Some("Joanna Ann"))]);
    set_regions(&mut h, id, names.iter().enumerate().map(|(i, n)| region(0.1 + 0.3 * i as f64, *n)).collect());
    h.request("ui.set", json!({"view": "detail", "right": "none"}), T);
    h.settle(T);
    (h, id)
}

fn step(h: &mut Headless) {
    h.step();
    h.step();
}

fn ok(h: &mut Headless, method: &str, params: serde_json::Value) {
    let r = h.request(method, params.clone(), T);
    assert_eq!(r["ok"], true, "{method} {params}: {r}");
    step(h);
}

fn key(h: &mut Headless, key: &str) {
    ok(h, "ui.key", json!({"key": key}));
}

fn shift_key(h: &mut Headless, key: &str) {
    ok(h, "ui.key", json!({"key": key, "shift": true}));
}

fn type_text(h: &mut Headless, text: &str) {
    ok(h, "ui.text", json!({"text": text}));
}

/// Point at face `i` (its label shows) and click the label: the name box opens on it.
fn open(h: &mut Headless, i: usize) {
    let x = 0.2 + 0.3 * i as f64;
    ok(h, "ui.pointer", json!({"events": [{"kind": "move", "x": x, "y": 0.4}]}));
    ok(h, "ui.clickWidget", json!({"id": format!("regionLabel:{i}")}));
    assert_eq!(h.app.ui.name_edit.as_ref().map(|e| e.index), Some(i), "{:?}", h.app.ui.name_edit);
}

/// A click on the window's top edge: outside the photo (a click on the photo zooms it), every face box and the name box.
fn click_outside(h: &mut Headless) {
    ok(h, "ui.click", json!({"x": 700.0, "y": 2.0}));
}

fn shown(h: &Headless) -> String {
    h.app.ui.name_edit.as_ref().map(|e| e.input.text.clone()).unwrap_or_default()
}

fn names(h: &Headless, id: PhotoId) -> Vec<Option<String>> {
    h.app.session.catalog.photo(id).unwrap().meta.regions.iter().map(|r| r.name.clone()).collect()
}

fn n(s: &str) -> Option<String> {
    Some(s.to_string())
}

/// Suggestions for the unnamed faces of `id` as if recognition had scanned them: `by_index` → (suggestion, closest).
fn suggest(h: &mut Headless, id: PhotoId, by_index: &[(usize, Option<&str>, &[(&str, f32)])]) {
    let mut hints = Hints::default();
    for (i, suggestion, candidates) in by_index {
        hints.by_index.insert(
            *i,
            Hint { suggestion: suggestion.map(|s| (s.to_string(), 0.8)), candidates: candidates.iter().map(|(n, s)| (n.to_string(), *s)).collect() },
        );
    }
    h.app.caches.faces_active = true;
    h.app.caches.faces_next_pump = f64::MAX;
    h.app.caches.face_hints = Some((id.0, h.app.session.catalog.revision, h.app.caches.faces_indexed, Arc::new(hints)));
    step(h);
}

#[test]
fn the_box_opens_with_the_name_selected_and_typing_replaces_it() {
    let (mut h, id) = loupe([Some("Zed"), None, None]);
    open(&mut h, 0);
    assert_eq!(shown(&h), "Zed");
    type_text(&mut h, "Qux");
    assert_eq!(shown(&h), "Qux", "the first letters replace the name instead of joining it");
    let undo = h.app.session.undo.len();
    key(&mut h, "enter");
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(names(&h, id), [n("Qux"), None, None]);
    assert_eq!(h.app.session.undo.len(), undo + 1);
    assert!(h.app.ui.dialog.is_none(), "a new name makes a new person without a question");
}

#[test]
fn typing_the_start_of_a_name_completes_it_and_backspace_removes_the_completed_part() {
    let (mut h, id) = loupe([None, None, None]);
    open(&mut h, 0);
    type_text(&mut h, "bo");
    assert_eq!(shown(&h), "Bob Ray", "completed in the box");
    key(&mut h, "backspace");
    assert_eq!(shown(&h), "Bo", "only the completed part went");
    type_text(&mut h, "b");
    assert_eq!(shown(&h), "Bob Ray", "typing on completes again");
    key(&mut h, "backspace");
    type_text(&mut h, "by");
    assert_eq!(shown(&h), "Bobby", "a new name that begins like an existing one can be entered");
    key(&mut h, "enter");
    assert_eq!(names(&h, id), [n("Bobby"), None, None]);
}

#[test]
fn typing_over_a_selected_name_completes_too() {
    let (mut h, id) = loupe([Some("Zed"), None, None]);
    suggest(&mut h, id, &[(1, Some("Bob Ray"), &[])]);
    open(&mut h, 0);
    type_text(&mut h, "an");
    assert_eq!(shown(&h), "Ann Lee", "the typed letters replaced the selected name, and completed");
    key(&mut h, "escape");
    open(&mut h, 1);
    type_text(&mut h, "j");
    assert_eq!(shown(&h), "Joanna Ann", "a later word is not completed, a name that begins with j is");
}

#[test]
fn the_list_is_walked_with_the_arrows_and_a_name_in_it_is_clicked() {
    let (mut h, id) = loupe([None, None, None]);
    open(&mut h, 0);
    type_text(&mut h, "a");
    for want in ["Ann Lee", "Joanna Ann"] {
        let r = h.request("ui.widgets", json!({"filter": "faceName:pick:"}), T);
        assert!(r.to_string().contains(&format!("faceName:pick:{want}")), "{want} is listed: {r}");
    }
    assert!(!h.request("ui.widgets", json!({"filter": "faceName:pick:"}), T).to_string().contains("Bob Ray"));
    key(&mut h, "down");
    key(&mut h, "down");
    key(&mut h, "down");
    key(&mut h, "up");
    key(&mut h, "enter");
    assert_eq!(names(&h, id), [n("Ann Lee"), None, None], "Up went back to the first listed name");
    open(&mut h, 1);
    type_text(&mut h, "a");
    key(&mut h, "down");
    key(&mut h, "down");
    key(&mut h, "enter");
    assert_eq!(names(&h, id), [n("Ann Lee"), n("Joanna Ann"), None], "Enter saves the listed name, not the completed text");
    open(&mut h, 2);
    type_text(&mut h, "j");
    ok(&mut h, "ui.clickWidget", json!({"id": "faceName:pick:Joanna Ann"}));
    assert_eq!(names(&h, id)[2], n("Joanna Ann"));
    assert!(h.app.ui.name_edit.is_none());
}

#[test]
fn escape_is_the_only_cancel_and_leaves_what_was_there() {
    let (mut h, id) = loupe([Some("Zed"), None, None]);
    let undo = h.app.session.undo.len();
    open(&mut h, 0);
    type_text(&mut h, "Qux");
    key(&mut h, "escape");
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(names(&h, id), [n("Zed"), None, None]);
    assert_eq!(h.app.session.undo.len(), undo);
}

#[test]
fn enter_on_an_untouched_suggestion_confirms_it() {
    let (mut h, id) = loupe([None, None, None]);
    suggest(&mut h, id, &[(1, Some("Bob Ray"), &[("Bob Ray", 0.8), ("Ann Lee", 0.5)])]);
    open(&mut h, 1);
    assert_eq!(shown(&h), "Bob Ray", "the suggestion is in the box, selected");
    key(&mut h, "enter");
    assert_eq!(names(&h, id), [None, n("Bob Ray"), None]);
}

#[test]
fn a_press_outside_saves_changed_text_and_closes_an_untouched_box_without_naming() {
    let (mut h, id) = loupe([None, None, None]);
    suggest(&mut h, id, &[(0, Some("Bob Ray"), &[])]);
    open(&mut h, 0);
    click_outside(&mut h);
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(names(&h, id), [None, None, None], "the untouched suggestion was not confirmed");
    open(&mut h, 0);
    type_text(&mut h, "Cy");
    click_outside(&mut h);
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(names(&h, id), [n("Cy"), None, None]);
}

#[test]
fn tab_and_shift_tab_move_between_faces_saving_typed_text_but_never_a_suggestion() {
    let (mut h, id) = loupe([None, None, None]);
    suggest(&mut h, id, &[(0, Some("Bob Ray"), &[]), (1, Some("Ann Lee"), &[])]);
    open(&mut h, 0);
    key(&mut h, "tab");
    assert_eq!(h.app.ui.name_edit.as_ref().map(|e| e.index), Some(1), "Tab goes to the next face");
    assert_eq!(names(&h, id), [None, None, None], "the untouched suggestion was not confirmed");
    assert_eq!(shown(&h), "Ann Lee", "and the next box opens selected on its own suggestion");
    type_text(&mut h, "Cy");
    let undo = h.app.session.undo.len();
    key(&mut h, "tab");
    assert_eq!(names(&h, id), [None, n("Cy"), None], "typed text is saved");
    assert_eq!(h.app.session.undo.len(), undo + 1);
    assert_eq!(h.app.ui.name_edit.as_ref().map(|e| e.index), Some(2));
    shift_key(&mut h, "tab");
    assert_eq!(h.app.ui.name_edit.as_ref().map(|e| e.index), Some(1), "Shift-Tab goes back");
    assert_eq!(shown(&h), "Cy");
    shift_key(&mut h, "tab");
    shift_key(&mut h, "tab");
    assert_eq!(h.app.ui.name_edit.as_ref().map(|e| e.index), Some(2), "going round");
    assert_eq!(names(&h, id), [None, n("Cy"), None]);
}

#[test]
fn a_capitalization_only_difference_takes_the_existing_persons_spelling() {
    let (mut h, id) = loupe([None, Some("zed"), None]);
    open(&mut h, 0);
    type_text(&mut h, "ANN LEE");
    key(&mut h, "enter");
    assert_eq!(names(&h, id)[0], n("Ann Lee"));
    // a face that already has the name keeps a change of capitalization
    open(&mut h, 1);
    type_text(&mut h, "ZED");
    key(&mut h, "enter");
    assert_eq!(names(&h, id)[1], n("ZED"));
}

#[test]
fn emptying_a_named_faces_box_and_pressing_enter_removes_the_name() {
    let (mut h, id) = loupe([Some("Zed"), None, None]);
    open(&mut h, 0);
    key(&mut h, "backspace");
    assert_eq!(shown(&h), "", "the selected name went");
    key(&mut h, "enter");
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(names(&h, id), [None, None, None]);
}

#[test]
fn the_box_closes_without_effect_when_its_face_is_no_longer_shown() {
    let (mut h, id) = loupe([None, None, None]);
    open(&mut h, 0);
    type_text(&mut h, "Cy");
    ok(&mut h, "ui.set", json!({"view": "photoGrid"}));
    step(&mut h);
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(names(&h, id), [None, None, None]);
}

/// Names listed by the open box, top to bottom.
fn listed(h: &mut Headless) -> Vec<String> {
    let r = h.request("ui.widgets", json!({"filter": "faceName:pick:"}), T);
    let mut items: Vec<(f64, String)> = r["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| Some((w["rect"][1].as_f64()?, w["id"].as_str()?.strip_prefix("faceName:pick:")?.to_string())))
        .collect();
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    items.into_iter().map(|(_, n)| n).collect()
}

fn more_people(h: &mut Headless, id: PhotoId) {
    let other = h.app.session.catalog.photos().map(|p| p.id).find(|p| *p != id).unwrap();
    let mut regions = h.app.session.catalog.photo(other).unwrap().meta.regions.clone();
    regions.push(region(0.0, Some("Anders Bo")));
    regions.push(region(0.05, Some("Anna Fox")));
    set_regions(h, other, regions);
}

#[test]
fn the_name_that_looks_most_like_the_face_is_completed_and_listed_first() {
    let (mut h, id) = loupe([None, None, None]);
    more_people(&mut h, id);
    // the whole ranking (five people), not the closest three: the best match among those that begin with "an" is
    // Anna Fox, though two other people look more like the face and Anders comes first alphabetically
    suggest(&mut h, id, &[(0, None, &[("Bob Ray", 0.9), ("Anna Fox", 0.8), ("Joanna Ann", 0.7), ("Ann Lee", 0.5), ("Anders Bo", 0.31)])]);
    open(&mut h, 0);
    type_text(&mut h, "an");
    assert_eq!(shown(&h), "Anna Fox");
    assert_eq!(listed(&mut h), ["Anna Fox", "Ann Lee", "Anders Bo", "Joanna Ann"]);
    key(&mut h, "enter");
    assert_eq!(names(&h, id)[0], n("Anna Fox"));
}

#[test]
fn a_face_with_no_ranking_gets_matching_names_alphabetically() {
    let (mut h, id) = loupe([None, None, None]);
    more_people(&mut h, id);
    suggest(&mut h, id, &[]);
    open(&mut h, 0);
    type_text(&mut h, "an");
    assert_eq!(shown(&h), "Anders Bo");
    assert_eq!(listed(&mut h), ["Anders Bo", "Ann Lee", "Anna Fox", "Joanna Ann"]);
}

#[test]
fn opening_the_box_starts_no_embedding() {
    let (mut h, id) = loupe([None, None, None]);
    h.app.caches.faces_active = true;
    h.app.caches.faces_next_pump = f64::MAX;
    open(&mut h, 0);
    type_text(&mut h, "a");
    assert!(h.app.ui.name_edit.is_some());
    assert!(h.app.session.activity.list().is_empty(), "no task for the box: {:?}", h.app.session.activity.list());
    let _ = id;
}

// ---------------------------------------------------------------- accept and reject icons on a suggested label

/// Whether the widget `id` is on screen.
fn has_widget(h: &mut Headless, id: &str) -> bool {
    let r = h.request("ui.widgets", json!({"filter": id}), T);
    r["result"].as_array().is_some_and(|l| l.iter().any(|w| w["id"] == id))
}

fn point_at(h: &mut Headless, i: usize) {
    ok(h, "ui.pointer", json!({"events": [{"kind": "move", "x": 0.2 + 0.3 * i as f64, "y": 0.4}]}));
}

fn rejected_names(h: &Headless, id: PhotoId, i: usize) -> Vec<String> {
    h.app.session.catalog.photo(id).unwrap().meta.regions[i].rejected.clone()
}

#[test]
fn a_suggested_label_shows_accept_and_reject_while_pointed_at_and_only_then() {
    let (mut h, id) = loupe([None, Some("Zed"), None]);
    suggest(&mut h, id, &[(0, Some("Ann Lee"), &[("Ann Lee", 0.8)])]);
    ok(&mut h, "ui.pointer", json!({"events": [{"kind": "move", "x": 0.95, "y": 0.95}]}));
    assert!(!has_widget(&mut h, "regionAccept:0"), "not pointed at");
    point_at(&mut h, 0);
    assert!(has_widget(&mut h, "regionAccept:0"));
    assert!(has_widget(&mut h, "regionReject:0"));
    point_at(&mut h, 1);
    assert!(!has_widget(&mut h, "regionAccept:1"), "a named face shows none");
    assert!(!has_widget(&mut h, "regionReject:1"));
    point_at(&mut h, 2);
    assert!(!has_widget(&mut h, "regionAccept:2"), "a face with no suggestion shows none");
    assert!(!has_widget(&mut h, "regionReject:2"));
}

#[test]
fn accepting_names_the_face_with_the_suggestion_in_one_undo_step() {
    let (mut h, id) = loupe([None, None, None]);
    suggest(&mut h, id, &[(0, Some("Ann Lee"), &[("Ann Lee", 0.8)])]);
    point_at(&mut h, 0);
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": "regionAccept:0"}));
    assert_eq!(names(&h, id), [n("Ann Lee"), None, None]);
    assert_eq!(h.app.session.undo.len(), undo + 1);
    assert!(h.app.ui.name_edit.is_none(), "no name box opened");
}

#[test]
fn rejecting_records_the_rejection_in_one_undo_step_and_the_label_goes_quiet() {
    let (mut h, id) = loupe([None, None, None]);
    suggest(&mut h, id, &[(0, Some("Ann Lee"), &[("Ann Lee", 0.8)])]);
    point_at(&mut h, 0);
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": "regionReject:0"}));
    assert_eq!(rejected_names(&h, id, 0), ["Ann Lee"]);
    assert_eq!(names(&h, id), [None, None, None]);
    assert_eq!(h.app.session.undo.len(), undo + 1);
    // even when the ranking would now offer a next-best name, this visit of the photo shows none
    suggest(&mut h, id, &[(0, Some("Bob Ray"), &[("Bob Ray", 0.7)])]);
    point_at(&mut h, 0);
    assert!(!has_widget(&mut h, "regionAccept:0"));
    // undo takes the rejection back
    h.request("engine.execute", json!({"command": "edit.undo", "params": {}}), T);
    step(&mut h);
    assert!(rejected_names(&h, id, 0).is_empty());
}

#[test]
fn a_click_on_the_suggested_name_still_opens_the_box() {
    let (mut h, id) = loupe([None, None, None]);
    suggest(&mut h, id, &[(0, Some("Ann Lee"), &[("Ann Lee", 0.8)])]);
    open(&mut h, 0);
    assert_eq!(shown(&h), "Ann Lee");
    key(&mut h, "escape");
    assert!(h.app.ui.name_edit.is_none());
}
