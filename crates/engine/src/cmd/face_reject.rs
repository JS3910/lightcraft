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
}

/// Reject `name` for `region`; with `clear`, a face called `name` loses its name too.
fn reject(region: &mut Region, name: &str, clear: bool) -> Result<Effect> {
    const C: &str = "faces.reject";
    let mut effect = Effect::default();
    if !region.rejects(name) {
        if region.rejected.len() >= MAX_REJECTED {
            return Err(bad(C, format!("a face keeps at most {MAX_REJECTED} rejected names")));
        }
        region.rejected.push(name.to_string());
        effect.listed = true;
    }
    if clear && region.name.as_deref().is_some_and(|n| n.trim().to_lowercase() == name.trim().to_lowercase()) {
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
    let (mut ops, mut found, mut listed, mut cleared) = (Vec::new(), 0usize, 0usize, 0usize);
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
    Ok(json!({"rejected": listed, "cleared": cleared, "photos": photos}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "faces.reject",
        "Reject Name for Faces",
        [],
        None,
        "{faces: [{photo, index, name?}], name?, clearName?: false} → {rejected, cleared, photos} — say the name is not these faces' (up to 5000, across photos), as one undoable step: it is kept on each face in the catalog only (never in a sidecar or a file), suggestions skip it, the person's More leaves the face out, and naming the face that name later drops it. An entry's own `name` wins over the shared one; `clearName` also clears the name of a face that is called the rejected name. `rejected` counts faces whose list grew (a name already rejected, in any capitalization, changes nothing), `cleared` faces that lost their name; missing or non-face entries are skipped and the call is refused when none exists",
        super::always,
        reject_faces
    )]
}
