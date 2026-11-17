//! Reject on every face tile, in two steps, on many faces at once, and the Delete key: the first reject on a suggested face
//! (or on a named confirmed face) says "not them" and leaves the tile in place as a question mark, the second (and the
//! first on a face with no suggestion) removes the face box. Over a selection the same, two-level. Driven through the
//! control channel on the People view and a person's page.

use lightcraft_catalog::PhotoId;
use serde_json::json;

use super::person_tiles::{self as pt, done, ok};
use super::unnamed_tiles::{people, picture, suggest};
use crate::headless::Headless;

fn regions(h: &Headless, id: PhotoId) -> usize {
    h.app.session.catalog.photo(id).unwrap().meta.regions.len()
}

fn button(prefix: &str, kind: &str, id: PhotoId, i: usize) -> String {
    format!("{prefix}-face-{kind}:{}:{i}", id.0)
}

fn click(h: &mut Headless, id: String) {
    ok(h, "ui.clickWidget", json!({"id": id}));
}

fn undo(h: &mut Headless) {
    ok(h, "engine.execute", json!({"command": "edit.undo"}));
}

fn rejects(h: &Headless, id: PhotoId, i: usize) -> Vec<String> {
    h.app.session.catalog.photo(id).unwrap().meta.regions[i].rejected.clone()
}

fn press(h: &mut Headless, key: &str) {
    ok(h, "ui.key", json!({"key": key}));
}

/// Unnamed faces re-suggested the way the engine would after a catalog change (its next-best name).
fn resuggest(h: &mut Headless, who: &[(PhotoId, usize, &str)]) {
    suggest(h, who);
}

// ---- a person's page

#[test]
fn a_second_reject_on_a_more_tile_removes_its_face_box() {
    let (mut h, [_, _, _, p3, _]) = pt::page();
    let undo_len = h.app.session.undo.len();
    click(&mut h, button("more", "hide", p3, 0));
    assert_eq!((regions(&h, p3), rejects(&h, p3, 0)), (1, vec!["Ann Lee".to_string()]));
    assert!(pt::widgets(&mut h, "more-face:").contains(&pt::more(p3, 0)), "the tile stays as a question mark");
    click(&mut h, button("more", "hide", p3, 0));
    assert_eq!(regions(&h, p3), 0, "the second reject removes the face box");
    assert_eq!(h.app.session.undo.len(), undo_len + 2);
    undo(&mut h);
    assert_eq!((regions(&h, p3), rejects(&h, p3, 0)), (1, vec!["Ann Lee".to_string()]), "undo brings the box back with its rejection");
}

#[test]
fn reject_is_the_only_hover_button_on_a_confirmed_face_and_takes_two_steps() {
    let (mut h, [p0, _, _, _, _]) = pt::page();
    assert!(!pt::widgets(&mut h, "person-face-hide:").is_empty());
    for kind in ["confirm", "above"] {
        assert!(pt::widgets(&mut h, &format!("person-face-{kind}:")).is_empty(), "{kind}");
    }
    let undo_len = h.app.session.undo.len();
    click(&mut h, button("person", "hide", p0, 0));
    assert_eq!((pt::names(&h, p0), rejects(&h, p0, 0)), (vec![None], vec!["Ann Lee".to_string()]), "name removed, not that person");
    assert!(pt::widgets(&mut h, "person-face:").contains(&done(p0, 0)), "the tile stays as a question mark");
    assert!(pt::widgets(&mut h, "person-face-hide:").contains(&button("person", "hide", p0, 0)));
    click(&mut h, button("person", "hide", p0, 0));
    assert_eq!(regions(&h, p0), 0, "the second reject removes the face box");
    assert_eq!(h.app.session.undo.len(), undo_len + 2);
    undo(&mut h);
    undo(&mut h);
    assert_eq!((pt::names(&h, p0), rejects(&h, p0, 0)), (vec![pt::n("Ann Lee")], Vec::<String>::new()), "both steps undone: name back");
}

#[test]
fn reject_on_a_selected_more_tile_acts_on_every_selected_face_and_on_an_unselected_one_alone() {
    let (mut h, [_, _, p2, p3, p4]) = pt::page();
    click(&mut h, pt::more(p3, 0));
    click(&mut h, pt::more(p4, 0));
    let undo_len = h.app.session.undo.len();
    click(&mut h, button("more", "hide", p3, 0));
    assert_eq!((rejects(&h, p3, 0).len(), rejects(&h, p4, 0).len(), rejects(&h, p2, 0).len()), (1, 1, 0));
    assert_eq!(h.app.session.undo.len(), undo_len + 1, "one step");
    // the second reject removes the boxes of the whole selection, still one step
    click(&mut h, button("more", "hide", p3, 0));
    assert_eq!((regions(&h, p3), regions(&h, p4), regions(&h, p2)), (0, 0, 2));
    assert_eq!(h.app.session.undo.len(), undo_len + 2);
    assert!(h.app.ui.unnamed_selected.is_empty(), "removed faces leave the selection");
}

#[test]
fn a_mixed_selection_only_clears_suggestions_on_the_first_reject() {
    let (mut h, [_, _, _, p3, p4]) = pt::page();
    click(&mut h, button("more", "hide", p3, 0)); // p3 is a question mark now
    click(&mut h, pt::more(p3, 0));
    click(&mut h, pt::more(p4, 0));
    let undo_len = h.app.session.undo.len();
    click(&mut h, button("more", "hide", p4, 0));
    assert_eq!((regions(&h, p3), regions(&h, p4), rejects(&h, p4, 0)), (1, 1, vec!["Ann Lee".to_string()]), "nothing removed");
    assert_eq!(h.app.session.undo.len(), undo_len + 1);
    click(&mut h, button("more", "hide", p4, 0));
    assert_eq!((regions(&h, p3), regions(&h, p4)), (0, 0), "now none has a suggestion: the boxes go");
    assert_eq!(h.app.session.undo.len(), undo_len + 2);
}

#[test]
fn the_delete_key_rejects_the_selection_in_two_steps_and_waits_for_text_fields() {
    let (mut h, [_, _, _, p3, p4]) = pt::page();
    press(&mut h, "delete");
    assert_eq!((regions(&h, p3), rejects(&h, p3, 0).len()), (1, 0), "nothing selected: nothing happens");
    assert!(h.app.ui.dialog.is_none(), "and no photo delete is offered behind the view");
    click(&mut h, pt::more(p3, 0));
    click(&mut h, pt::more(p4, 0));
    // a name box has the keyboard: Delete (Backspace) only edits its text
    click(&mut h, pt::more_name(p3, 0));
    assert!(h.app.ui.name_edit.is_some(), "the box is open");
    press(&mut h, "delete");
    assert_eq!((rejects(&h, p3, 0).len(), rejects(&h, p4, 0).len(), regions(&h, p3)), (0, 0, 1));
    press(&mut h, "escape");
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(h.app.ui.unnamed_selected.len(), 2, "Esc only closed the box");
    let undo_len = h.app.session.undo.len();
    press(&mut h, "delete");
    assert_eq!((rejects(&h, p3, 0).len(), rejects(&h, p4, 0).len()), (1, 1));
    press(&mut h, "delete");
    assert_eq!((regions(&h, p3), regions(&h, p4)), (0, 0));
    assert_eq!(h.app.session.undo.len(), undo_len + 2);
}

// ---- the unnamed faces

fn unnamed_ids(h: &mut Headless) -> Vec<String> {
    pt::widgets(h, "unnamed-face:")
}

#[test]
fn a_suggested_unnamed_face_rejects_in_place_then_loses_its_box_and_one_without_a_suggestion_loses_it_at_once() {
    let (mut h, [_p0, p1, p2]) = people();
    suggest(&mut h, &[(p1, 0, "Ann Lee")]);
    let before = unnamed_ids(&mut h);
    assert_eq!(pt::widgets(&mut h, "unnamed-face-hide:").len(), before.len(), "reject is on every tile");
    click(&mut h, button("unnamed", "hide", p1, 0));
    assert_eq!((regions(&h, p1), rejects(&h, p1, 0)), (2, vec!["Ann Lee".to_string()]));
    // the engine would offer the next-best name straight away; the tile must not show it
    resuggest(&mut h, &[(p1, 0, "Bob Ray")]);
    assert_eq!(unnamed_ids(&mut h), before, "the tile stays");
    assert!(!pt::widgets(&mut h, "unnamed-face-confirm:").contains(&button("unnamed", "confirm", p1, 0)));
    click(&mut h, button("unnamed", "hide", p1, 0));
    assert_eq!(regions(&h, p1), 1, "the second reject removes the face box");
    // a face with no suggestion: removed at once
    let undo_len = h.app.session.undo.len();
    click(&mut h, button("unnamed", "hide", p2, 0));
    assert_eq!(regions(&h, p2), 0);
    assert_eq!(h.app.session.undo.len(), undo_len + 1);
}

#[test]
fn rejecting_a_mixed_selection_of_unnamed_faces_clears_suggestions_first_then_removes_all_boxes_in_one_undo_step() {
    let (mut h, [p0, p1, p2]) = people();
    // a rejection a box already carries comes back with it
    ok(&mut h, "engine.execute", json!({"command": "faces.reject", "params": {"faces": [{"photo": p1.0, "index": 1}], "name": "Zed"}}));
    suggest(&mut h, &[(p1, 0, "Ann Lee"), (p2, 0, "Ann Lee")]);
    click(&mut h, "unnamed:selectAll".into());
    assert_eq!(h.app.ui.unnamed_selected.len(), 4);
    let before: Vec<_> = [p0, p1, p2].iter().map(|id| h.app.session.catalog.photo(*id).unwrap().meta.regions.clone()).collect();
    let undo_len = h.app.session.undo.len();
    click(&mut h, button("unnamed", "hide", p2, 0));
    assert_eq!((regions(&h, p0), regions(&h, p1), regions(&h, p2)), (2, 2, 1), "the first reject removes no box");
    assert_eq!((rejects(&h, p1, 0).len(), rejects(&h, p2, 0).len(), rejects(&h, p0, 1).len()), (1, 1, 0));
    assert_eq!(h.app.session.undo.len(), undo_len + 1);
    resuggest(&mut h, &[(p1, 0, "Bob Ray"), (p2, 0, "Bob Ray")]);
    assert!(pt::widgets(&mut h, "unnamed-face-confirm:").is_empty(), "no new suggestion straight after a reject");
    click(&mut h, button("unnamed", "hide", p2, 0));
    // the four unnamed faces are gone, Ann Lee stays; two of them were on one photo
    assert_eq!((regions(&h, p0), regions(&h, p1), regions(&h, p2)), (1, 0, 0));
    assert_eq!(pt::names(&h, p0), [pt::n("Ann Lee")]);
    assert_eq!(h.app.session.undo.len(), undo_len + 2, "one step for all the boxes");
    assert!(h.app.ui.unnamed_selected.is_empty());
    undo(&mut h);
    let after_first: Vec<_> = [p0, p1, p2].iter().map(|id| h.app.session.catalog.photo(*id).unwrap().meta.regions.clone()).collect();
    assert_eq!(after_first[0], before[0]);
    assert_eq!(after_first[1].len(), 2, "every box back");
    assert_eq!((after_first[1][1].rejected.clone(), after_first[1][0].rejected.clone()), (vec!["Zed".to_string()], vec!["Ann Lee".to_string()]));
    undo(&mut h);
    let restored: Vec<_> = [p0, p1, p2].iter().map(|id| h.app.session.catalog.photo(*id).unwrap().meta.regions.clone()).collect();
    assert_eq!(restored, before, "one more undo: as before the first reject");
}

#[test]
fn reject_on_an_unselected_unnamed_tile_acts_on_that_face_alone() {
    let (mut h, [_, p1, p2]) = people();
    suggest(&mut h, &[(p1, 0, "Ann Lee"), (p2, 0, "Ann Lee")]);
    click(&mut h, picture(p1, 0));
    click(&mut h, button("unnamed", "hide", p2, 0));
    assert_eq!((rejects(&h, p2, 0).len(), rejects(&h, p1, 0).len()), (1, 0));
    assert!(h.app.ui.unnamed_selected.contains(&(p1.0, 0)), "the selection is left as it was");
}

#[test]
fn removing_a_box_keeps_what_the_view_holds_about_the_later_faces_of_the_photo_right() {
    let (mut h, [_, p1, _]) = people();
    // photo p1 holds two unnamed faces (0 and 1); (p1,1) is suggested, rejected in place, and selected with (p1,0) not
    suggest(&mut h, &[(p1, 1, "Ann Lee")]);
    click(&mut h, button("unnamed", "hide", p1, 1));
    click(&mut h, picture(p1, 1));
    assert!(h.app.ui.unnamed_selected.contains(&(p1.0, 1)));
    // remove the first box of the photo: the second becomes the first
    click(&mut h, button("unnamed", "hide", p1, 0));
    assert_eq!(regions(&h, p1), 1);
    assert!(h.app.ui.unnamed_selected.contains(&(p1.0, 0)) && h.app.ui.unnamed_selected.len() == 1, "{:?}", h.app.ui.unnamed_selected);
    // the surviving face was rejected in place: the engine's next-best name must not show on it
    resuggest(&mut h, &[(p1, 0, "Bob Ray")]);
    assert!(!pt::widgets(&mut h, "unnamed-face-confirm:").contains(&button("unnamed", "confirm", p1, 0)));
    // and no name box is left open on a face that is not the one it was opened for
    assert!(h.app.ui.name_edit.is_none());
}

#[test]
fn the_delete_key_rejects_the_selected_unnamed_faces() {
    let (mut h, [_, p1, p2]) = people();
    suggest(&mut h, &[(p1, 0, "Ann Lee")]);
    click(&mut h, picture(p1, 0));
    click(&mut h, picture(p2, 0));
    let undo_len = h.app.session.undo.len();
    press(&mut h, "delete");
    assert_eq!((rejects(&h, p1, 0).len(), regions(&h, p2), regions(&h, p1)), (1, 1, 2), "first: the suggestion only");
    press(&mut h, "delete");
    assert_eq!((regions(&h, p1), regions(&h, p2)), (1, 0));
    assert_eq!(h.app.session.undo.len(), undo_len + 2);
}

#[test]
fn escape_on_a_persons_page_closes_an_open_box_first_and_only_a_second_escape_clears_the_selection() {
    let (mut h, [_, _, _, p3, p4]) = pt::page();
    click(&mut h, pt::more(p3, 0));
    click(&mut h, pt::more(p4, 0));
    click(&mut h, pt::more_name(p3, 0));
    assert!(h.app.ui.name_edit.is_some());
    press(&mut h, "delete");
    press(&mut h, "escape");
    assert!(h.app.ui.name_edit.is_none());
    assert_eq!(h.app.ui.unnamed_selected.len(), 2, "the first Esc only closed the box");
    assert_eq!(h.app.ui.person_page.as_deref(), Some("Ann Lee"));
    press(&mut h, "escape");
    assert!(h.app.ui.unnamed_selected.is_empty(), "the second clears the selection");
    assert_eq!(h.app.ui.person_page.as_deref(), Some("Ann Lee"), "and still does not leave the page");
    press(&mut h, "escape");
    assert_eq!(h.app.ui.person_page, None, "the third leaves it");
}

#[test]
fn undo_and_redo_of_a_box_removal_leave_no_view_state_pointing_at_another_face() {
    let (mut h, [_, p1, _]) = people();
    // (p1,1) is suggested, rejected in place and selected; removing (p1,0) moves it to index 0
    suggest(&mut h, &[(p1, 1, "Ann Lee")]);
    click(&mut h, button("unnamed", "hide", p1, 1));
    click(&mut h, picture(p1, 1));
    click(&mut h, button("unnamed", "hide", p1, 0));
    assert!(h.app.ui.unnamed_selected.contains(&(p1.0, 0)), "shifted up while the box is gone");
    // Undo brings the box back at index 0: what was held for (p1,0) would now be about the other face
    undo(&mut h);
    assert_eq!(regions(&h, p1), 2);
    assert!(h.app.ui.unnamed_selected.is_empty(), "{:?}", h.app.ui.unnamed_selected);
    assert!(h.app.ui.rejected_unnamed.is_empty(), "no tile is shown as rejected in place for the wrong face");
    assert!(h.app.ui.name_edit.is_none() && h.app.ui.unnamed_anchor.is_none());
    // and the same for Redo
    click(&mut h, picture(p1, 1));
    ok(&mut h, "engine.execute", json!({"command": "edit.redo"}));
    assert_eq!(regions(&h, p1), 1);
    assert!(h.app.ui.unnamed_selected.is_empty(), "{:?}", h.app.ui.unnamed_selected);
}
