//! `faces.takeOut`: take a face out of the stack of Unnamed faces it is in. The face becomes a tile of its own and is
//! remembered, on its face box and in the catalogue only (`Region::apart`), as one that never stacks with the faces it
//! was stacked with: not after the next run, and not after a restart. One undo step for however many faces.

use std::collections::BTreeMap;

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_meta::{Apart, RegionKind};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd};
use crate::faces_stack::MAX_STACK;
use crate::{Result, Session};

/// Most faces one call takes.
const MAX_FACES: usize = 5000;

/// The `(photo, index)` pairs of a call, checked before anything changes.
fn targets(p: &Value) -> Result<Vec<(u64, usize)>> {
    const C: &str = "faces.takeOut";
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

fn bits(r: &lightcraft_geom::Rect) -> [u64; 4] {
    [r.x0.to_bits(), r.y0.to_bits(), r.x1.to_bits(), r.y1.to_bits()]
}

/// What the face at `index` of `photo` is to the arrangement: its photo, its box and which of the unnamed faces of the photo with
/// that very box it is.
fn key_of(s: &Session, photo: u64, index: usize) -> Option<crate::faces_arrange::Key> {
    let regions = &s.catalog.photo(PhotoId(photo))?.meta.regions;
    let region = regions.get(index).filter(|r| r.kind == RegionKind::Face && r.name.is_none())?;
    let ord = regions.iter().take(index).filter(|r| r.kind == RegionKind::Face && r.name.is_none() && bits(&r.rect) == bits(&region.rect)).count();
    Some((photo, bits(&region.rect), u32::try_from(ord).ok()?))
}

/// `faces.takeOut {faces: [{photo, index}]}`: take each face out of the stack it is in (as the last run that put the Unnamed
/// faces in order found it), as one undo step. Faces that are in no stack are skipped; the call is refused when none is in one.
fn take_out(s: &mut Session, p: &Value) -> Result<Value> {
    let mut by_photo: BTreeMap<u64, Vec<(usize, Vec<Apart>)>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for (photo, index) in targets(p)? {
        let Some(key) = key_of(s, photo, index) else { continue };
        let Some(members) = s.faces.arrange.stack_members(&key) else { continue };
        if !seen.insert((photo, index)) {
            continue;
        }
        let apart: Vec<Apart> = members
            .iter()
            .filter(|m| **m != key)
            .map(|(ph, rect, _)| Apart {
                photo: *ph,
                rect: crate::faces_index::region_key(&lightcraft_geom::Rect {
                    x0: f64::from_bits(rect[0]),
                    y0: f64::from_bits(rect[1]),
                    x1: f64::from_bits(rect[2]),
                    y1: f64::from_bits(rect[3]),
                }),
            })
            .collect();
        by_photo.entry(photo).or_default().push((index, apart));
    }
    let (mut ops, mut taken) = (Vec::new(), 0usize);
    for (photo, faces) in by_photo {
        let id = PhotoId(photo);
        let Some(ph) = s.catalog.photo(id) else { continue };
        let mut meta = ph.meta.clone();
        for (index, apart) in faces {
            let Some(region) = meta.regions.get_mut(index) else { continue };
            for a in apart {
                if region.apart.len() < MAX_STACK && !region.apart.contains(&a) {
                    region.apart.push(a);
                }
            }
            taken += 1;
        }
        ops.push(Op::SetMeta { id, meta: Box::new(meta) });
    }
    if taken == 0 {
        return Err(bad("faces.takeOut", "none of those faces is in a stack"));
    }
    let photos = ops.len();
    s.commit(if taken == 1 { "Take Face Out of Stack" } else { "Take Faces Out of Stacks" }, Op::Batch { ops })?;
    s.skip_auto_write = true;
    Ok(json!({"taken": taken, "photos": photos}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "faces.takeOut",
        "Take Faces Out of Their Stacks",
        [],
        None,
        "{faces: [{photo, index}]} → {taken, photos} — take up to 5000 faces out of the stacks of the Unnamed faces they are in (as the last run that put the faces in order found them; `faces.unnamed` lists a stack's faces), as one undoable step (stacks with a suggested name too). Each face becomes a tile of its own and never stacks again with the faces it was stacked with, also after a restart: this is kept on the face's box in the catalogue only (never in a sidecar or a file), carried over a metadata re-read like rejected names, and is nothing in a catalogue written before. Native builds only: the web build has no recognition and so no stacks. A stack left with one face is no stack. Faces that are in no stack (a single face, one the last run did not stack, one that does not exist) are skipped and the call is refused when none is in a stack",
        super::always,
        take_out
    )]
}
