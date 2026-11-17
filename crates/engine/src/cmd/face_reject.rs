//! `faces.reject`: say "this is not that person" for faces. A face region keeps the names rejected for it
//! (`Region::rejected`), in the catalog only: suggestions skip them, a person's More leaves the face out, and naming the
//! face with one of them drops it. One undo step for however many faces.

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_meta::{Region, RegionKind};
use serde_json::{Value, json};

use super::face_recognize::name_value;
use super::{CommandSpec, bad, cmd};
use crate::{Result, Session};

/// Most faces one call takes.
const MAX_FACES: usize = 5000;

/// Most names one face keeps rejected.
const MAX_REJECTED: usize = 200;

/// What rejecting a name did to one face.
#[derive(Default)]
struct Effect {
    /// The name was new to the face's list.
    listed: bool,
    /// The face's own name was cleared.
    cleared: bool,
    /// The face is called the name and `clear` was not asked for, so it was left alone.
    skipped: bool,
}

/// Reject `name` for `region`; with `clear`, a face called `name` loses its name too; without it such a face is left alone.
fn reject(region: &mut Region, name: &str, clear: bool) -> Result<Effect> {
    const C: &str = "faces.reject";
    let mut effect = Effect::default();
    let called = region.name.as_deref().is_some_and(|n| n.trim().to_lowercase() == name.trim().to_lowercase());
    if called && !clear {
        // rejecting a face's own name would leave it carrying a name it rejects
        effect.skipped = true;
        return Ok(effect);
    }
    if !region.rejects(name) {
        if region.rejected.len() >= MAX_REJECTED {
            return Err(bad(C, format!("a face keeps at most {MAX_REJECTED} rejected names")));
        }
        region.rejected.push(name.to_string());
        effect.listed = true;
    }
    if called {
        region.name = None;
        effect.cleared = true;
    }
    Ok(effect)
}

/// `clearName`: a boolean, false when absent.
fn clear_flag(p: &Value) -> Result<bool> {
    match p.get("clearName") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(bad("faces.reject", "`clearName` must be true or false")),
    }
}

/// One face asked about: where it is, and the name to reject for it.
struct Target {
    photo: u64,
    index: usize,
    name: String,
}

/// The faces and names of a call, all checked before anything changes.
fn targets(p: &Value) -> Result<Vec<Target>> {
    const C: &str = "faces.reject";
    let default = name_value(p.get("name"), C)?;
    let list = p.get("faces").and_then(Value::as_array).ok_or_else(|| bad(C, "missing `faces`"))?;
    if list.is_empty() || list.len() > MAX_FACES {
        return Err(bad(C, format!("between 1 and {MAX_FACES} faces at a time")));
    }
    let mut out = Vec::with_capacity(list.len());
    for f in list {
        let (Some(photo), Some(index)) =
            (f.get("photo").and_then(Value::as_u64), f.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()))
        else {
            return Err(bad(C, "each face needs a `photo` and an `index`"));
        };
        let name = match f.get("name") {
            Some(own) => name_value(Some(own), C)?,
            None => default.clone(),
        };
        let Some(name) = name else { return Err(bad(C, "each face needs a name to reject (its own `name`, or `name` for all)")) };
        out.push(Target { photo, index, name });
    }
    Ok(out)
}

/// `faces.reject {faces: [{photo, index, name?}], name?, clearName?}`: record that the name is wrong for each face,
/// as one undo step. An entry's own `name` wins over the shared one (confirming a selection rejects each face's own
/// suggestion). With `clearName`, a face whose name is the rejected one loses it. Faces that do not exist or are not
/// faces are skipped, and the call is refused when none exist; a name a face already rejected changes nothing.
fn reject_faces(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "faces.reject";
    let clear = clear_flag(p)?;
    let targets = targets(p)?;
    let mut by_photo: std::collections::BTreeMap<u64, Vec<Target>> = std::collections::BTreeMap::new();
    for t in targets {
        by_photo.entry(t.photo).or_default().push(t);
    }
    let (mut ops, mut found, mut listed, mut cleared, mut skipped) = (Vec::new(), 0usize, 0usize, 0usize, 0usize);
    for (photo, faces) in by_photo {
        let id = PhotoId(photo);
        let Some(ph) = s.catalog.photo(id) else { continue };
        let mut meta = ph.meta.clone();
        let mut changed = false;
        for t in faces {
            let Some(region) = meta.regions.get_mut(t.index).filter(|r| r.kind == RegionKind::Face) else { continue };
            found += 1;
            let e = reject(region, &t.name, clear)?;
            listed += usize::from(e.listed);
            cleared += usize::from(e.cleared);
            skipped += usize::from(e.skipped);
            changed |= e.listed || e.cleared;
        }
        if changed {
            ops.push(Op::SetMeta { id, meta: Box::new(meta) });
        }
    }
    if found == 0 {
        return Err(bad(C, "none of those faces exist"));
    }
    let photos = ops.len();
    if !ops.is_empty() {
        s.commit(if listed == 1 { "Reject Name" } else { "Reject Names" }, Op::Batch { ops })?;
        s.skip_auto_write = true;
    }
    Ok(json!({"rejected": listed, "cleared": cleared, "skipped": skipped, "photos": photos}))
}

/// The `(photo, index)` pairs of a `faces.removeBoxes` call, checked before anything changes.
fn box_targets(p: &Value) -> Result<Vec<(u64, usize)>> {
    const C: &str = "faces.removeBoxes";
    let list = p.get("faces").and_then(Value::as_array).ok_or_else(|| bad(C, "missing `faces`"))?;
    if list.is_empty() || list.len() > MAX_FACES {
        return Err(bad(C, format!("between 1 and {MAX_FACES} faces at a time")));
    }
    let mut out = Vec::with_capacity(list.len());
    for f in list {
        let (Some(photo), Some(index)) =
            (f.get("photo").and_then(Value::as_u64), f.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()))
        else {
            return Err(bad(C, "each face needs a `photo` and an `index`"));
        };
        out.push((photo, index));
    }
    Ok(out)
}

/// A photo with no face box left counts as searched by the scan, so the scan does not find the removed boxes again.
#[cfg(not(target_arch = "wasm32"))]
fn mark_searched(s: &mut Session, photo: u64) {
    s.faces.scanned.insert(photo);
}

#[cfg(target_arch = "wasm32")]
fn mark_searched(_: &mut Session, _: u64) {}

/// `faces.removeBoxes {faces: [{photo, index}]}`: remove the face boxes, as one undo step. The indexes are positions in
/// each photo's regions as they are now: all of a photo's boxes go in one edit, highest index first, so removing one
/// never moves another. Entries that do not exist or are not faces are skipped (a face listed twice counts once);
/// the call is refused when none exists.
fn remove_boxes(s: &mut Session, p: &Value) -> Result<Value> {
    let mut by_photo: std::collections::BTreeMap<u64, std::collections::BTreeSet<usize>> = std::collections::BTreeMap::new();
    for (photo, index) in box_targets(p)? {
        by_photo.entry(photo).or_default().insert(index);
    }
    let (mut ops, mut removed) = (Vec::new(), 0usize);
    for (photo, indexes) in by_photo {
        let id = PhotoId(photo);
        let Some(ph) = s.catalog.photo(id) else { continue };
        let mut meta = ph.meta.clone();
        let mut gone = 0usize;
        for &i in indexes.iter().rev() {
            if meta.regions.get(i).is_some_and(|r| r.kind == RegionKind::Face) {
                meta.regions.remove(i);
                gone += 1;
            }
        }
        if gone > 0 {
            removed += gone;
            if !meta.regions.iter().any(|r| r.kind == RegionKind::Face) {
                mark_searched(s, photo);
            }
            ops.push(Op::SetMeta { id, meta: Box::new(meta) });
        }
    }
    if removed == 0 {
        return Err(bad("faces.removeBoxes", "none of those faces exist"));
    }
    let photos = ops.len();
    s.commit(if removed == 1 { "Remove Face Box" } else { "Remove Face Boxes" }, Op::Batch { ops })?;
    s.skip_auto_write = true;
    Ok(json!({"removed": removed, "photos": photos}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "faces.reject",
            "Reject Name for Faces",
            [],
            None,
            "{faces: [{photo, index, name?}], name?, clearName?: false} → {rejected, cleared, skipped, photos} — say the name is not these faces' (up to 5000, across photos), as one undoable step: it is kept on each face in the catalog only (never in a sidecar or a file), suggestions skip it, the person's More leaves the face out, and naming the face that name later drops it. An entry's own `name` wins over the shared one; `clearName` also clears the name of a face that is called the rejected name; without it such a face is left alone (a face would carry a name it rejects), counted in `skipped`. `rejected` counts faces whose list grew (a name already rejected, in any capitalization, changes nothing), `cleared` faces that lost their name, `skipped` faces left alone because they are called the name; missing or non-face entries are skipped and the call is refused when none exists",
            super::always,
            reject_faces
        ),
        cmd!(
            "faces.removeBoxes",
            "Remove Face Boxes",
            [],
            None,
            "{faces: [{photo, index}]} → {removed, photos} — remove the face boxes of up to 5000 faces across photos as one undoable step (the boxes of one photo go together, so indexes never shift under the call), name and rejections included; Undo brings every box back. Missing, repeated or non-face entries are skipped and the call is refused when none exists. The XMP sidecar is never rewritten for this, and a removed box is not detected again on the photo: the scan keeps a record of the photos it searched",
            super::always,
            remove_boxes
        ),
    ]
}
