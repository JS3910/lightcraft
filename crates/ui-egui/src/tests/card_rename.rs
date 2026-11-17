//! Renaming a person in place: the name under a Named People card opens the shared name box; typing another
//! person's name asks before combining.

use std::time::Duration;

use lightcraft_catalog::{Op, PhotoId};
use serde_json::json;

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn face(x: f64, name: &str) -> lightcraft_meta::Region {
    lightcraft_meta::Region {
        rect: lightcraft_geom::Rect { x0: x, y0: 0.2, x1: x + 0.2, y1: 0.6 },
        kind: lightcraft_meta::RegionKind::Face,
        name: Some(name.to_string()),
        description: None,
        rejected: Vec::new(),
    }
}

fn set(h: &mut Headless, id: PhotoId, regions: Vec<lightcraft_meta::Region>) {
    let mut meta = h.app.session.catalog.photo(id).unwrap().meta.clone();
    meta.regions = regions;
    h.app.session.commit("setup", Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
}

/// The People view with Jane Doe (3 faces on 2 photos) and John Roe (1 face).
fn people() -> (Headless, Vec<PhotoId>) {
    let services = Services { png: None, write_shared: Some(std::sync::Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    h.settle(T);
    let ids: Vec<_> = h.app.session.catalog.photos().map(|p| p.id).take(2).collect();
    set(&mut h, ids[0], vec![face(0.1, "Jane Doe"), face(0.5, "jane doe")]);
    set(&mut h, ids[1], vec![face(0.3, "Jane Doe"), face(0.6, "John Roe")]);
    ok(&mut h, "engine.execute", json!({"command": "view.people"}));
    h.settle(T);
    (h, ids)
}

fn ok(h: &mut Headless, method: &str, params: serde_json::Value) {
    let r = h.request(method, params.clone(), T);
    assert_eq!(r["ok"], true, "{method} {params}: {r}");
    h.step();
    h.step();
}

fn names(h: &Headless, id: PhotoId) -> Vec<Option<String>> {
    h.app.session.catalog.photo(id).unwrap().meta.regions.iter().map(|r| r.name.clone()).collect()
}

fn n(s: &str) -> Option<String> {
    Some(s.to_string())
}

fn typed(h: &mut Headless, text: &str) {
    ok(h, "ui.text", json!({"text": text}));
    ok(h, "ui.key", json!({"key": "enter"}));
}

#[test]
fn a_click_on_the_name_opens_the_box_with_the_name_selected_and_a_click_on_the_picture_opens_the_page() {
    let (mut h, _) = people();
    ok(&mut h, "ui.clickWidget", json!({"id": "personName:Jane Doe"}));
    let edit = h.app.ui.card_name.as_ref().expect("the name box is open");
    assert_eq!((edit.from.as_str(), edit.input.text.as_str()), ("Jane Doe", "Jane Doe"));
    assert_eq!(h.app.ui.person_page, None, "the page did not open");
    // typing replaces the selected name
    ok(&mut h, "ui.text", json!({"text": "Ada"}));
    assert_eq!(h.app.ui.card_name.as_ref().map(|e| e.input.text.starts_with("Ada")), Some(true));
    ok(&mut h, "ui.key", json!({"key": "escape"}));
    assert!(h.app.ui.card_name.is_none());
    ok(&mut h, "ui.clickWidget", json!({"id": "person:Jane Doe"}));
    assert_eq!(h.app.ui.person_page.as_deref(), Some("Jane Doe"));
}

#[test]
fn a_new_name_renames_every_face_in_one_undo_step_and_the_page_and_filter_follow() {
    let (mut h, ids) = people();
    h.app.ui.last_person = Some("Jane Doe".into());
    h.app.session.execute("library.filter", &json!({"person": "Jane Doe"})).unwrap();
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": "personName:Jane Doe"}));
    typed(&mut h, "Ada Lovelace");
    assert!(h.app.ui.card_name.is_none());
    assert_eq!(names(&h, ids[0]), vec![n("Ada Lovelace"), n("Ada Lovelace")]);
    assert_eq!(names(&h, ids[1])[0], n("Ada Lovelace"));
    assert_eq!(h.app.session.undo.len(), undo + 1);
    assert_eq!(h.app.ui.last_person.as_deref(), Some("Ada Lovelace"));
    assert_eq!(h.app.session.filter.person.as_deref(), Some("Ada Lovelace"));
    h.app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(names(&h, ids[0]), vec![n("Jane Doe"), n("jane doe")]);
}

#[test]
fn a_change_of_capitalization_alone_renames_the_person() {
    let (mut h, ids) = people();
    ok(&mut h, "ui.clickWidget", json!({"id": "personName:Jane Doe"}));
    typed(&mut h, "JANE DOE");
    assert_eq!(names(&h, ids[0]), vec![n("JANE DOE"), n("JANE DOE")]);
    assert!(h.app.ui.dialog.is_none());
}

#[test]
fn another_persons_name_asks_first_and_combine_merges_in_one_undo_step() {
    let (mut h, ids) = people();
    let undo = h.app.session.undo.len();
    ok(&mut h, "ui.clickWidget", json!({"id": "personName:Jane Doe"}));
    typed(&mut h, "john roe");
    let Some(Dialog::CombinePeople { from, to }) = &h.app.ui.dialog else { panic!("{:?}", h.app.ui.dialog) };
    assert_eq!((from.as_str(), to.as_str()), ("Jane Doe", "John Roe"));
    assert_eq!(names(&h, ids[0]), vec![n("Jane Doe"), n("jane doe")], "nothing changes before Combine");
    assert_eq!(h.app.session.undo.len(), undo);
    assert_eq!(
        (crate::dialogs::person_faces(&h.app.session.catalog, "Jane Doe"), crate::dialogs::person_faces(&h.app.session.catalog, "john roe")),
        (3, 1)
    );
    ok(&mut h, "ui.clickWidget", json!({"id": "button:dialogOk"}));
    assert!(h.app.ui.dialog.is_none());
    assert_eq!(names(&h, ids[0]), vec![n("John Roe"), n("John Roe")]);
    assert_eq!(h.app.session.undo.len(), undo + 1);
    h.app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(names(&h, ids[0]), vec![n("Jane Doe"), n("jane doe")]);
    assert_eq!(names(&h, ids[1]), vec![n("Jane Doe"), n("John Roe")]);
}

#[test]
fn cancel_and_escape_leave_everything_as_it_was() {
    let (mut h, ids) = people();
    for cancel in ["button:dialogCancel", "escape"] {
        ok(&mut h, "ui.clickWidget", json!({"id": "personName:Jane Doe"}));
        ok(&mut h, "ui.text", json!({"text": "John Roe"}));
        ok(&mut h, "ui.key", json!({"key": "enter"}));
        assert!(matches!(h.app.ui.dialog, Some(Dialog::CombinePeople { .. })));
        if cancel.starts_with("button") {
            ok(&mut h, "ui.clickWidget", json!({"id": cancel}));
        } else {
            ok(&mut h, "ui.key", json!({"key": cancel}));
        }
        assert!(h.app.ui.dialog.is_none(), "{cancel}");
        assert_eq!(names(&h, ids[0]), vec![n("Jane Doe"), n("jane doe")]);
        assert_eq!(names(&h, ids[1]), vec![n("Jane Doe"), n("John Roe")]);
    }
}

#[test]
fn the_rename_person_dialog_from_the_cards_menu_still_works() {
    let (mut h, _) = people();
    let c = h.app.widgets.iter().find(|(w, _)| w == "person:Jane Doe").map(|(_, r)| r.center()).unwrap();
    ok(&mut h, "ui.click", json!({"x": c.x, "y": c.y, "button": "right"}));
    ok(&mut h, "ui.clickWidget", json!({"id": "personMenu:rename"}));
    assert!(matches!(&h.app.ui.dialog, Some(Dialog::RenamePerson { from, .. }) if from == "Jane Doe"));
}

/// The name box opens ready to type every time, also right after it closed.
#[test]
fn a_box_closed_with_escape_opens_again_ready_to_type() {
    let (mut h, _) = people();
    for _ in 0..2 {
        ok(&mut h, "ui.clickWidget", json!({"id": "personName:Jane Doe"}));
        ok(&mut h, "ui.text", json!({"text": "Zed"}));
        assert_eq!(h.app.ui.card_name.as_ref().map(|e| e.input.text.starts_with("Zed")), Some(true), "typing reaches the box");
        ok(&mut h, "ui.key", json!({"key": "escape"}));
    }
}
