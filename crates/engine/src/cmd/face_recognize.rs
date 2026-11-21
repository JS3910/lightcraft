//! Recognition commands: `faces.index` (embed the faces in the library), `faces.suggest` (who might an unnamed
//! face be?), `faces.person` (one person's faces, and the unnamed faces that look like them), `faces.unnamed` (every
//! unnamed face, the likeliest suggestions first) and `faces.setName` / `faces.nameFaces` (name one face, or many at once, which is
//! what teaches the suggestions). `faces.renamePerson` renames every in-library face of one person, or merges two
//! people when asked.
//!
//! Everything here is a suggestion for the user to confirm; nothing is ever named automatically. All of it is
//! catalog-only (no sidecar is written) and naming is undoable.

use lightcraft_catalog::{Op, Photo};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd};
use crate::{Result, Session};

/// The `name` parameter of the naming commands: a name, or none (`null`, empty) to clear it.
fn name_param(p: &Value, c: &str) -> Result<Option<String>> {
    name_value(p.get("name"), c)
}

/// A name value as [`name_param`] reads it (`None`: nothing given).
pub(super) fn name_value(v: Option<&Value>, c: &str) -> Result<Option<String>> {
    let name = match v {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| bad(c, "`name` must be text or null"))?.trim().to_string()).filter(|n| !n.is_empty()),
    };
    if let Some(n) = &name
        && (n.chars().count() > 200 || n.chars().any(char::is_control))
    {
        return Err(bad(c, "a name is at most 200 characters, without control characters"));
    }
    Ok(name)
}

/// Give a face region its name. Naming makes the face yours: a later "Detect Faces" run no longer replaces it. A name
/// the face had rejected is no longer rejected: an explicit choice wins, in the same edit.
fn apply_name(region: &mut lightcraft_meta::Region, name: &Option<String>) {
    region.name = name.clone();
    if let Some(n) = name {
        let key = n.trim().to_lowercase();
        region.rejected.retain(|r| r.trim().to_lowercase() != key);
    }
    if name.is_some() && region.description.as_deref().is_some_and(|d| d.starts_with(super::face_detect::MARK)) {
        region.description = None;
    }
}

/// `faces.nameFaces {faces: [{photo, index, name?}], name?}`: name (or clear) many face regions at once, as one undo
/// step: what naming a group of look-alikes does. An entry's own `name` wins; an entry without one takes the top-level
/// `name` (none: clears it). Every name is checked before anything changes.
fn name_faces(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "faces.nameFaces";
    let default = name_param(p, C)?;
    let list = p.get("faces").and_then(Value::as_array).ok_or_else(|| bad(C, "missing `faces`"))?;
    if list.len() > 5000 {
        return Err(bad(C, "at most 5000 faces at a time"));
    }
    let mut by_photo: std::collections::BTreeMap<u64, Vec<(usize, Option<String>)>> = std::collections::BTreeMap::new();
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
        by_photo.entry(photo).or_default().push((index, name));
    }
    let (mut ops, mut named, mut cleared) = (Vec::new(), 0usize, 0usize);
    for (photo, faces) in by_photo {
        let id = lightcraft_catalog::PhotoId(photo);
        let Some(photo) = s.catalog.photo(id) else { continue };
        let mut meta = photo.meta.clone();
        let before = named;
        for (i, name) in faces {
            if let Some(region) = meta.regions.get_mut(i).filter(|r| r.kind == lightcraft_meta::RegionKind::Face) {
                apply_name(region, &name);
                named += 1;
                cleared += usize::from(name.is_none());
            }
        }
        if named > before {
            ops.push(Op::SetMeta { id, meta: Box::new(meta) });
        }
    }
    if ops.is_empty() {
        return Err(bad(C, "none of those faces exist"));
    }
    let photos = ops.len();
    s.commit(if cleared == named { "Clear Face Names" } else { "Name Faces" }, Op::Batch { ops })?;
    s.skip_auto_write = true;
    Ok(json!({"named": named, "photos": photos, "name": default}))
}

/// `faces.setName {id?, index, name}`: name (or, with `null` or an empty name, un-name) one face region. Naming
/// makes the face yours: a later "Detect Faces" run no longer replaces it.
fn set_name(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "faces.setName";
    let id = super::photo_id(p, C)?.or_else(|| s.active()).ok_or_else(|| bad(C, "no photo"))?;
    let index = p.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(C, "missing or invalid `index`"))?;
    let name = name_param(p, C)?;
    let mut meta = s.catalog.photo(id).ok_or_else(|| bad(C, "no such photo"))?.meta.clone();
    let region = meta.regions.get_mut(index).ok_or_else(|| bad(C, "no such region"))?;
    apply_name(region, &name);
    s.commit(if name.is_some() { "Name Face" } else { "Clear Face Name" }, Op::SetMeta { id, meta: Box::new(meta) })?;
    s.skip_auto_write = true;
    Ok(json!({"id": id.0, "index": index, "name": name}))
}

/// A person was renamed: the face's rejections of `from_key` (lower case) become rejections of `to`, each name once, and a
/// face that is now called `to` does not reject itself.
fn rewrite_rejections(region: &mut lightcraft_meta::Region, from_key: &str, to: &str) {
    let to_key = to.trim().to_lowercase();
    let own = region.name.as_deref().map(|n| n.trim().to_lowercase());
    let mut out: Vec<String> = Vec::new();
    for r in std::mem::take(&mut region.rejected) {
        let key = r.trim().to_lowercase();
        let (key, shown) = if key == from_key { (to_key.clone(), to.to_string()) } else { (key, r) };
        if own.as_deref() != Some(key.as_str()) && !out.iter().any(|o| o.trim().to_lowercase() == key) {
            out.push(shown);
        }
    }
    region.rejected = out;
}

/// `faces.renamePerson {from, to, merge?}`: give every in-library face of `from` the exact name `to`, as one undo
/// step. Names match the People view (trimmed, case-insensitive). A `to` that is already someone else is refused
/// unless `merge` is set, and then the two people become one. The sidecar is not touched, and a refusal changes nothing.
fn rename_person(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "faces.renamePerson";
    let from = match p.get("from") {
        Some(Value::String(v)) => {
            let t = v.trim();
            if t.is_empty() {
                return Err(bad(C, "missing `from`"));
            }
            t.to_string()
        }
        Some(_) => return Err(bad(C, "`from` must be text")),
        None => return Err(bad(C, "missing `from`")),
    };
    let to = match p.get("to") {
        Some(Value::String(v)) => {
            let t = v.trim();
            if t.is_empty() {
                return Err(bad(C, "`to` must be a name"));
            }
            if t.chars().count() > 200 || t.chars().any(char::is_control) {
                return Err(bad(C, "a name is at most 200 characters, without control characters"));
            }
            t.to_string()
        }
        Some(_) => return Err(bad(C, "`to` must be text")),
        None => return Err(bad(C, "missing `to`")),
    };
    let merge = super::bool_or(p, "merge", false);
    let from_key = from.to_lowercase();
    let to_key = to.to_lowercase();
    let mut matched = 0usize;
    let mut other = false;
    let mut faces = 0usize;
    let mut ops = Vec::new();
    let ids: Vec<_> = s.catalog.photos().filter(|ph| ph.in_library()).map(|ph| ph.id).collect();
    for id in ids {
        let Some(ph) = s.catalog.photo(id) else { continue };
        let (mut indexes, mut rewrites) = (Vec::new(), Vec::new());
        for (i, r) in ph.meta.regions.iter().enumerate() {
            if r.kind != lightcraft_meta::RegionKind::Face {
                continue;
            }
            // the names it says are not its own follow the person to the new name, whoever the face is
            if r.rejects(&from) {
                rewrites.push(i);
            }
            let Some(n) = r.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) else { continue };
            let key = n.to_lowercase();
            if key == from_key {
                matched += 1;
                if r.name.as_deref() != Some(to.as_str()) {
                    indexes.push(i);
                }
            } else if key == to_key {
                other = true;
            }
        }
        if indexes.is_empty() && rewrites.is_empty() {
            continue;
        }
        let mut meta = ph.meta.clone();
        for &i in &indexes {
            if let Some(region) = meta.regions.get_mut(i) {
                region.name = Some(to.clone());
                faces += 1;
            }
        }
        // (after the names, so a face that is now `to` does not reject itself; a renamed face also drops a rejection
        // of `to` it already held)
        rewrites.extend(indexes);
        rewrites.sort_unstable();
        rewrites.dedup();
        for i in rewrites {
            if let Some(region) = meta.regions.get_mut(i) {
                rewrite_rejections(region, &from_key, &to);
            }
        }
        ops.push(Op::SetMeta { id, meta: Box::new(meta) });
    }
    if matched == 0 {
        return Err(bad(C, format!("no person named {from}")));
    }
    if faces == 0 {
        return Err(bad(C, format!("{to} is already their name")));
    }
    if other && !merge {
        return Err(bad(C, format!("{to} is already another person; pass merge: true to combine them")));
    }
    let photos = ops.len();
    s.commit(if other { "Merge People" } else { "Rename Person" }, Op::Batch { ops })?;
    s.skip_auto_write = true;
    if s.filter.person.as_deref().is_some_and(|n| n.trim().to_lowercase() == from_key) {
        s.filter.person = Some(to.clone());
    }
    if let Some(f) = s.shelved_filter.as_mut()
        && f.person.as_deref().is_some_and(|n| n.trim().to_lowercase() == from_key)
    {
        f.person = Some(to.clone());
    }
    Ok(json!({"faces": faces, "photos": photos, "from": from, "to": to, "merged": other}))
}

/// Most faces `faces.unnamed` lists.
const MAX_UNNAMED: usize = 3000;

/// Whether `scope: "visible"` was asked for: the faces of the People view's photos only ([`Session::people_view_photos`]).
/// Any other value, or none, is the whole library (the convention `faces.evaluate` uses).
fn wants_visible(p: &Value) -> bool {
    p.get("scope").and_then(Value::as_str) == Some("visible")
}

/// How the unnamed faces were laid out.
struct Arranged {
    /// Some faces were put first because the named faces suggest a name for them.
    suggested: bool,
    /// At least two faces without a suggestion were put next to the ones they look like.
    look_alike: bool,
    /// Recognition is on and a model is loaded.
    ready: bool,
    /// The order is still being worked out on a thread of its own: what is listed is the best order there is now.
    ordering: bool,
    /// What is listed is the final order: the last run was for these very faces and named faces (or there is nothing to score).
    complete: bool,
    /// No run is going and the order is not complete: the run was cancelled, died or could not be started. It is not started
    /// again by itself (until a face or a name changes); `restart: true` asks for it.
    stopped: bool,
    /// A name for some faces, when the faces already named say so with confidence: `(photo, index)` to `(name, score)`.
    suggestions: std::collections::HashMap<(u64, usize), (String, f32)>,
    /// What to list, in order: a face or a stack. None: every face as it is.
    entries: Option<Vec<Entry>>,
    /// How many stacks `entries` holds.
    stacks: usize,
}

/// An unnamed face as `faces.unnamed` lists it: photo, region index and its own box.
type Triple = (u64, usize, lightcraft_geom::Rect);

/// One entry of the list `faces.unnamed` answers with.
enum Entry {
    Face(Triple),
    /// Faces that are virtually certain to be the same person (at least two), the most typical first.
    Stack(Vec<Triple>),
}

/// `faces.unnamed {limit?: 3000, scope?, wait?: true, restart?: false}`: the faces nobody has named, to be named a group at a time. With recognition
/// running, the faces the named faces recognise come first, the most confident first, each carrying the name they suggest;
/// then the faces that suggest nothing, look-alikes next to each other; then the faces not embedded yet, in photo order.
/// With `scope: "visible"` only the faces in the People view's photos are listed (and counted); what they are suggested
/// still compares against every named face in the library. Scoring runs on a thread of its own; by default the call waits
/// for the complete order, with `wait: false` it answers at once with the best order there is (`ordering` says more is coming;
/// `complete` says the order is final; neither: the run was stopped, `stopped`, and `restart: true` starts it again).
fn unnamed(s: &mut Session, p: &Value) -> Result<Value> {
    let limit = p.get("limit").and_then(Value::as_u64).map_or(MAX_UNNAMED, |n| usize::try_from(n).unwrap_or(MAX_UNNAMED)).clamp(1, MAX_UNNAMED);
    let mut all: Vec<(u64, usize, lightcraft_geom::Rect)> = Vec::new();
    let scope: Option<std::collections::HashSet<lightcraft_catalog::PhotoId>> =
        wants_visible(p).then(|| s.people_view_photos().into_iter().collect());
    for ph in s.catalog.photos().filter(|ph| ph.in_library() && scope.as_ref().is_none_or(|v| v.contains(&ph.id))) {
        for (index, r) in ph.meta.regions.iter().enumerate() {
            if r.kind == lightcraft_meta::RegionKind::Face && r.name.is_none() {
                all.push((ph.id.0, index, r.rect));
            }
        }
    }
    all.sort_by_key(|(photo, index, _)| (*photo, *index));
    let total = all.len();
    // a caller that does not say otherwise gets the complete order, worked out on a thread of its own
    let wait = match p.get("wait") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad("faces.unnamed", "`wait` must be true or false")),
    };
    let restart = p.get("restart").and_then(Value::as_bool).unwrap_or(false);
    let arranged = imp::arrange_unnamed(s, &mut all, wait, restart, limit)?;
    let mut entries = arranged.entries.unwrap_or_else(|| std::mem::take(&mut all).into_iter().map(Entry::Face).collect());
    entries.truncate(limit);
    let face = |(photo, index, rect): &Triple| {
        let view = s.face_view(lightcraft_catalog::PhotoId(*photo), *rect);
        let suggestion = arranged.suggestions.get(&(*photo, *index)).map(|(name, score)| json!({"name": name, "score": score}));
        json!({"photo": photo, "index": index, "rect": rect_json(rect), "view": rect_json(&view), "suggestion": suggestion})
    };
    let faces: Vec<Value> = entries
        .iter()
        .map(|e| match e {
            Entry::Face(f) => face(f),
            // a stack is shown by its first face (the most typical); the others are only in `stack.faces`
            Entry::Stack(members) => {
                let mut head = members.first().map_or(Value::Null, face);
                let listed: Vec<Value> =
                    members.iter().map(|(photo, index, rect)| json!({"photo": photo, "index": index, "rect": rect_json(rect)})).collect();
                head["stack"] = json!({"count": listed.len(), "faces": listed});
                head
            }
        })
        .collect();
    Ok(json!({
        "total": total,
        "stacks": arranged.stacks,
        "faces": faces,
        "ordered": arranged.suggested || arranged.look_alike,
        "lookAlike": arranged.look_alike,
        "ready": arranged.ready,
        "ordering": arranged.ordering,
        "complete": arranged.complete,
        "stopped": arranged.stopped,
    }))
}

/// What looking for faces that resemble a person found.
struct Similar {
    /// `{photo, index, rect, score}` of unnamed faces, the most alike first.
    items: Vec<serde_json::Value>,
    /// Recognition is on and a model is loaded, so an empty list means "none yet" rather than "not looking".
    ready: bool,
    /// Photos the background scan has still to do.
    pending: usize,
}

fn rect_json(r: &lightcraft_geom::Rect) -> Value {
    json!({"x0": r.x0, "y0": r.y0, "x1": r.x1, "y1": r.y1})
}

/// The Named People cards, most photos first then by name, and how many faces (named or unnamed) are in library
/// photos that `visible` leaves out. With `visible` the cards cover [`Session::people_view_photos`], else the whole
/// library (and nothing is left out).
fn people_cards(s: &mut Session, visible: bool) -> (Vec<lightcraft_catalog::Person>, usize) {
    if !visible {
        return (s.catalog.people(), 0);
    }
    let ids = s.people_view_photos();
    let cards = s.catalog.people_among(&ids);
    let inside: std::collections::HashSet<lightcraft_catalog::PhotoId> = ids.into_iter().collect();
    let faces = |ph: &std::sync::Arc<Photo>| ph.meta.regions.iter().filter(|r| r.kind == lightcraft_meta::RegionKind::Face).count();
    let hidden = s.catalog.photos().filter(|ph| ph.in_library() && !inside.contains(&ph.id)).map(faces).sum();
    (cards, hidden)
}

/// `faces.people {scope?}`: the Named People cards as data.
fn people(s: &mut Session, p: &Value) -> Result<Value> {
    let visible = wants_visible(p);
    let (cards, hidden) = people_cards(s, visible);
    let people: Vec<Value> =
        cards.iter().map(|c| json!({"name": c.name, "photos": c.count, "photo": c.photo.0, "face": rect_json(&c.face)})).collect();
    Ok(json!({"people": people, "hidden": hidden}))
}

/// Most confirmed faces `faces.person` lists.
const MAX_CONFIRMED: usize = 5000;

/// `faces.person {name, more?, scope?, varied?}`: the faces named `name` (case-insensitively) across the library, and, when
/// recognition is running, the unnamed faces that look most like them, to be confirmed one by one. With
/// `scope: "visible"` the confirmed faces (and `total`) are those in the People view's photos, and More looks in those
/// photos too, except that a person filter naming this same person is ignored for More. More's scores always compare
/// against all of the person's named faces in the library, in scope or not.
fn person(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "faces.person";
    let name = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(C, "missing `name`"))?;
    let key = name.to_lowercase();
    let limit = p.get("more").and_then(Value::as_u64).map_or(120, |n| usize::try_from(n).unwrap_or(120)).min(1000);
    let visible = wants_visible(p);
    let scope: Option<std::collections::HashSet<lightcraft_catalog::PhotoId>> = visible.then(|| s.people_view_photos().into_iter().collect());
    let varied = p.get("varied").and_then(Value::as_u64).map_or(0, |n| usize::try_from(n).unwrap_or(MAX_CONFIRMED)).min(MAX_CONFIRMED);
    let (mut shown, mut total, mut confirmed, mut in_library) = (name.to_string(), 0usize, Vec::new(), 0usize);
    for ph in s.catalog.photos().filter(|ph| ph.in_library()) {
        let in_scope = scope.as_ref().is_none_or(|v| v.contains(&ph.id));
        for (index, r) in ph.meta.regions.iter().enumerate() {
            let Some(n) = r.name.as_deref().filter(|n| r.kind == lightcraft_meta::RegionKind::Face && n.trim().to_lowercase() == key) else {
                continue;
            };
            in_library += 1;
            if !in_scope {
                continue;
            }
            if total == 0 {
                shown = n.trim().to_string();
            }
            total += 1;
            if confirmed.len() < MAX_CONFIRMED {
                confirmed.push((ph.id, index, r.rect));
            }
        }
    }
    let confirmed: Vec<Value> = imp::varied_first(s, confirmed, varied)
        .into_iter()
        .map(|(photo, index, rect)| json!({"photo": photo.0, "index": index, "rect": rect_json(&rect), "view": rect_json(&s.face_view(photo, rect))}))
        .collect();
    let more_in: Option<std::collections::HashSet<lightcraft_catalog::PhotoId>> =
        visible.then(|| s.people_view_photos_for_more(&key).into_iter().collect());
    let similar =
        if in_library > 0 { imp::similar(s, &key, limit, more_in.as_ref()) } else { Similar { items: Vec::new(), ready: false, pending: 0 } };
    Ok(
        json!({"name": shown, "total": total, "inLibrary": in_library, "confirmed": confirmed, "more": similar.items, "ready": similar.ready, "pendingPhotos": similar.pending}),
    )
}

/// `faces.rank {id?, index}`: every named person ranked by how much they look like this one face (best first, `score`
/// is the same similarity `faces.suggest` reports as a candidate's), for the name box. Read-only: it uses what the
/// background scan has embedded and never embeds anything, so a face the scan has not looked at, or recognition being
/// off, gives an empty ranking. The face's own embedding is left out of the gallery.
fn rank_face(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "faces.rank";
    let id = super::photo_id(p, C)?.or_else(|| s.active()).ok_or_else(|| bad(C, "no photo"))?;
    let index = p.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(C, "missing or invalid `index`"))?;
    let photo = s.catalog.photo(id).ok_or_else(|| bad(C, "no such photo"))?;
    let region = photo.meta.regions.get(index).filter(|r| r.kind == lightcraft_meta::RegionKind::Face).ok_or_else(|| bad(C, "no such face"))?;
    let rect = region.rect;
    let rejected = region.rejected.clone();
    let ranking: Vec<Value> = imp::ranking(s, id, index, &rect)
        .into_iter()
        .map(|(name, score, faces)| {
            let no = rejected.iter().any(|r| r.trim().to_lowercase() == name.trim().to_lowercase());
            json!({"name": name, "score": score, "faces": faces, "rejected": no})
        })
        .collect();
    Ok(json!({"photo": id.0, "index": index, "ranking": ranking}))
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use lightcraft_catalog::{Op, Photo, PhotoId};
    use lightcraft_develop::DevelopSettings;
    use lightcraft_faces::matching;
    use lightcraft_faces::runtime::Embedder;
    use lightcraft_geom::Rect;
    use lightcraft_meta::{Region, RegionKind};
    use rayon::prelude::*;
    use serde_json::{Value, json};

    use super::super::bad;
    use super::super::face_detect::{LABEL, MARK};
    use super::super::face_models::{finish_downloads, installed_models, read_settings};
    use super::Similar;
    use crate::faces_arrange::{Face, Input, Item, Listed, Person, Settings, fingerprint, signature};
    use crate::faces_worker::{Done, EDGE, PREFETCH, Pace, Prepared, Worker, max_workers, process, target_workers};
    use crate::{Result, Session};

    /// Defaults for suggestions, until a model's own threshold is known.
    const DEFAULT_THRESHOLD: f32 = 0.45;
    const DEFAULT_MARGIN: f32 = 0.06;
    /// How long a look at the models folder is trusted by the per-frame `faces.pump`.
    const RECHECK: Duration = Duration::from_secs(1);
    /// How often the background pump writes new embeddings to the cache file (appending is cheap; this keeps it to a few
    /// writes a minute, and a session that ends writes the rest).
    const SAVE_EVERY: Duration = Duration::from_secs(3);

    /// The chosen recognition model, loaded, with the index reset to it when it changed. `fresh` looks at the models
    /// folder again; otherwise a look made in the last second is trusted (the UI asks every frame).
    fn current_with(s: &mut Session, cmd: &str, fresh: bool) -> Result<Arc<Embedder>> {
        if !fresh
            && let Some((_, e)) = &s.faces.embedder
            && s.faces.checked.is_some_and(|t| t.elapsed() < RECHECK)
        {
            return Ok(e.clone());
        }
        let dir = s.face_models_dir.clone().ok_or_else(|| bad(cmd, "this build has nowhere to keep face models"))?;
        let id = read_settings(&dir).embedder.ok_or_else(|| bad(cmd, "no recognition model is chosen: add one in Settings > Faces and press Use"))?;
        let installed = installed_models(&dir)
            .into_iter()
            .find(|i| i.manifest.id == id)
            .ok_or_else(|| bad(cmd, "the chosen recognition model is not installed"))?;
        let tag = format!("{id}@{}", installed.manifest.sha256.as_deref().unwrap_or("unknown"));
        s.faces.checked = Some(Instant::now());
        // which photos the detector has searched is kept beside the library (or in memory when it is not on disk)
        let scan_path = s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.join("face-scanned.bin"));
        if s.faces.scanned.path() != scan_path.as_deref() {
            s.faces.scanned.reset(scan_path);
        }
        if let Some((have, e)) = &s.faces.embedder
            && *have == tag
        {
            return Ok(e.clone());
        }
        let embedder = Arc::new(Embedder::load(&dir.join(&id).join("model.onnx"), &installed.manifest).map_err(|e| bad(cmd, e.to_string()))?);
        let net = embedder.clone();
        super::super::face_detect::offer_gpu(s, "recognition", move |gpu| net.accelerate(gpu, true).map_err(|e| e.to_string()));
        let dim = match installed.manifest.output {
            lightcraft_faces::OutputSpec::Embedding { dim } => dim as usize,
            _ => return Err(bad(cmd, "the chosen model is not a recognition model")),
        };
        let cache = s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.join(format!("face-embeddings-{id}.bin")));
        s.faces.index.reset(&tag, dim, cache);
        s.faces.queue.clear();
        s.faces.queue_stamp = None;
        s.faces.in_flight.clear();
        s.faces.embedder = Some((tag, embedder.clone()));
        Ok(embedder)
    }

    fn current(s: &mut Session, cmd: &str) -> Result<Arc<Embedder>> {
        current_with(s, cmd, true)
    }

    /// Whether the background scan should look for faces in this photo: it is in the library, is not a video (the image
    /// decoders can't read one, [`Photo::is_video`]), has no face regions at all (nothing from XMP, nothing drawn or
    /// named, no earlier detection) and has not been searched yet.
    fn needs_search(s: &Session, photo: &Photo) -> bool {
        photo.in_library()
            && !photo.is_video()
            && !photo.meta.regions.iter().any(|r| r.kind == RegionKind::Face)
            && !s.faces.scanned.contains(photo.id.0)
            && !s.faces.scan_failed.contains(&photo.id.0)
    }

    /// The photo's faces that still need embedding (and, with `search`, the faces to find in it first), with the render job
    /// for its picture. `None` when there is nothing to do.
    fn prepare(s: &mut Session, id: PhotoId, search: bool) -> Option<Prepared> {
        let tag = s.faces.index.tag.clone();
        let photo = s.catalog.photo(id)?.clone();
        let todo: Vec<Rect> =
            photo.meta.regions.iter().filter(|r| r.kind == RegionKind::Face && s.faces.index.needs(id.0, &r.rect)).map(|r| r.rect).collect();
        // finding faces needs the detector the user may not have downloaded: such a photo is left unsearched, not marked as searched
        let detector = crate::cmd::face_detect::detector(s).ok();
        let detect = search && detector.is_some() && needs_search(s, &photo);
        if todo.is_empty() && !detect {
            return None;
        }
        let job = s.preview_job(id, EDGE, EDGE, false, &DevelopSettings::default())?;
        let preview = s.scan_preview_of(&photo);
        Some(Prepared { tag, epoch: s.faces.epoch, id, content: crate::media::content_key(&photo), job, preview, todo, detect, detector })
    }

    /// The faces the detector found in a photo that had none become its face regions: unnamed, marked as detected (so a
    /// manual Detect Faces replaces them), and not an undo step, since the user did not do this.
    fn add_found(s: &mut Session, id: PhotoId, found: &[Rect]) {
        if found.is_empty() {
            return;
        }
        let Some(photo) = s.catalog.photo(id) else { return };
        // the user (or an import) got there first: leave their faces alone
        if photo.meta.regions.iter().any(|r| r.kind == RegionKind::Face) {
            return;
        }
        let mut meta = photo.meta.clone();
        for rect in found {
            meta.regions.push(Region {
                rect: *rect,
                kind: RegionKind::Face,
                name: None,
                description: Some(format!("{MARK}{LABEL}")),
                rejected: Vec::new(),
            });
        }
        if s.apply_system(Op::SetMeta { id, meta: Box::new(meta) }).is_ok() {
            // regions are not written to XMP, so no sidecar may be rewritten for this
            s.skip_auto_write = true;
        }
    }

    /// Take a finished photo into the index (ignored when it was made for another model or another library, or from
    /// a file the photo no longer has: issue #1524). Returns (embedded, failed).
    fn ingest(s: &mut Session, d: Done) -> (usize, usize) {
        s.faces.in_flight.remove(&d.id);
        if d.epoch != s.faces.epoch || d.tag != s.faces.index.tag {
            return (0, 0);
        }
        if s.catalog.photo(d.id).is_none_or(|p| crate::media::content_key(p) != d.content) {
            return (0, 0);
        }
        if d.detect_wanted {
            match &d.found {
                Some(found) => {
                    s.faces.scanned.insert(d.id.0);
                    add_found(s, d.id, found);
                }
                None => {
                    s.faces.scan_failed.insert(d.id.0);
                }
            }
        }
        let (mut embedded, mut failed) = (0, 0);
        for (rect, v, view) in d.results {
            match v {
                Some(v) if v.len() == s.faces.index.dim => {
                    s.faces.index.insert_with_view(d.id.0, &rect, v, &view);
                    embedded += 1;
                }
                _ => {
                    s.faces.index.skip(d.id.0, &rect);
                    failed += 1;
                }
            }
        }
        (embedded, failed)
    }

    /// Take in whatever the worker has finished.
    fn collect(s: &mut Session) -> (usize, usize) {
        let finished = s.faces.worker.as_ref().map(Worker::finished).unwrap_or_default();
        finished.into_iter().fold((0, 0), |(e, f), d| {
            let (de, df) = ingest(s, d);
            (e + de, f + df)
        })
    }

    /// Embed every face region of `id` the index does not have yet, here and now. Returns (embedded, failed).
    fn embed_photo(s: &mut Session, id: PhotoId, e: &Embedder) -> Result<(usize, usize)> {
        let Some(job) = prepare(s, id, false) else { return Ok((0, 0)) };
        Ok(ingest(s, process(job, e)))
    }

    /// Photos with a face that needs embedding (and, with `search`, photos to look for faces in): those the caller asked
    /// for first; then those of the People view (`view`, see [`Session::people_view_photos`]), the work the user is
    /// looking at; then everything else. Within the view and within the rest the order is the same: those with named faces
    /// (they are the gallery everything else is compared with), then the rest, and last the photos that only need
    /// searching. With an empty `view` that is the order for the whole library.
    fn pending(s: &Session, first: &[PhotoId], view: &std::collections::HashSet<PhotoId>, search: bool) -> Vec<PhotoId> {
        let mut out: Vec<(u8, PhotoId)> = Vec::new();
        for p in s.catalog.photos().filter(|p| p.in_library()) {
            let faces: Vec<_> = p.meta.regions.iter().filter(|r| r.kind == RegionKind::Face).collect();
            let embed = faces.iter().any(|r| s.faces.index.needs(p.id.0, &r.rect));
            if embed || (search && needs_search(s, p)) {
                let by_work = if faces.iter().any(|r| r.name.is_some()) {
                    1
                } else if embed {
                    2
                } else {
                    3
                };
                let rank = if first.contains(&p.id) {
                    0
                } else if view.contains(&p.id) {
                    by_work
                } else {
                    by_work + 3
                };
                out.push((rank, p.id));
            }
        }
        out.sort_by_key(|(rank, id)| (*rank, id.0));
        out.into_iter().map(|(_, id)| id).collect()
    }

    pub fn index(s: &mut Session, p: &Value) -> Result<Value> {
        const C: &str = "faces.index";
        let embedder = current(s, C)?;
        collect(s);
        let budget = Duration::from_millis(p.get("budgetMs").and_then(Value::as_u64).unwrap_or(200).min(600_000));
        let first: Vec<PhotoId> = super::super::photo_ids(p, C)?.unwrap_or_default();
        // the photos the grid shows are done before the others, as the background scan does them
        let view: std::collections::HashSet<PhotoId> = s.people_view_photos().into_iter().collect();
        let todo = pending(s, &first, &view, false);
        let started = Instant::now();
        let (mut embedded, mut failed, mut done) = (0, 0, 0);
        for id in &todo {
            // at least one photo per call, so a slow model still makes progress; none when only the status is asked for
            if budget.is_zero() || (done > 0 && started.elapsed() >= budget) {
                break;
            }
            let (e, f) = embed_photo(s, *id, &embedder)?;
            (embedded, failed, done) = (embedded + e, failed + f, done + 1);
        }
        if let Err(e) = s.faces.index.save() {
            log::warn!("could not save the face embeddings: {e}");
        }
        Ok(json!({
            "model": s.faces.index.tag,
            "embedded": embedded,
            "failed": failed,
            "photosDone": done,
            "pendingPhotos": todo.len().saturating_sub(done),
            "indexedFaces": s.faces.index.len(),
            "ms": started.elapsed().as_secs_f64() * 1000.0,
        }))
    }

    fn str_scope(p: &Value) -> Option<&str> {
        p.get("scope").and_then(Value::as_str)
    }

    /// Seconds since 1970 of an ISO capture time (`2024-09-20T10:48:45…`, any zone suffix ignored): only for telling
    /// shots taken close together, so the zone does not matter.
    fn capture_secs(iso: &str) -> Option<i64> {
        let n = |a: usize, b: usize| iso.get(a..b)?.parse::<i64>().ok();
        let (y, m, d, hh, mm, ss) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        // Days since 1970-01-01 (proleptic Gregorian); a day past the month's length rolls over as
        // the old Hinnant arithmetic did.
        let first = chrono::NaiveDate::from_ymd_opt(i32::try_from(y).ok()?, u32::try_from(m).ok()?, 1)?;
        let date = first.checked_add_days(chrono::Days::new(u64::try_from(d - 1).ok()?))?;
        let days = (date - chrono::NaiveDate::from_ymd_opt(1970, 1, 1)?).num_days();
        Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
    }

    /// `faces.evaluate`: leave-one-out over the library's named faces, to see how well recognition works on *your*
    /// photos and which threshold to trust. Each named face in turn is treated as unknown and ranked against the
    /// other named faces, ignoring those from shots taken within `burstSecs` of it (near-duplicates would make
    /// everything look easy). Reports counts only.
    pub fn evaluate(s: &mut Session, p: &Value) -> Result<Value> {
        const C: &str = "faces.evaluate";
        let embedder = current(s, C)?;
        collect(s);
        let burst = p.get("burstSecs").and_then(Value::as_i64).unwrap_or(5).clamp(0, 86_400 * 7);
        let margin = p.get("margin").and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(DEFAULT_MARGIN, |v| v as f32);
        let thresholds: Vec<f32> = p
            .get("thresholds")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32).take(64).collect())
            .filter(|v: &Vec<f32>| !v.is_empty())
            .unwrap_or_else(|| (4..=16).map(|i| i as f32 * 0.05).collect());
        let budget = Duration::from_millis(p.get("budgetMs").and_then(Value::as_u64).unwrap_or(0).min(3_600_000));
        let started = Instant::now();
        // `scope: "visible"` looks only at the People view's photos (see `Session::people_view_photos`)
        let scope: Option<std::collections::HashSet<PhotoId>> =
            (str_scope(p) == Some("visible")).then(|| s.people_view_photos().into_iter().collect());
        let in_scope = |id: PhotoId| scope.as_ref().is_none_or(|v| v.contains(&id));
        let todo: Vec<PhotoId> = pending(s, &[], &Default::default(), false).into_iter().filter(|id| in_scope(*id)).collect();
        let mut done = 0;
        for id in &todo {
            if started.elapsed() >= budget {
                break;
            }
            embed_photo(s, *id, &embedder)?;
            done += 1;
        }
        // every named face with its embedding, the photo it is in, and when that was taken
        struct Named {
            name: String,
            emb: Arc<[f32]>,
            photo: PhotoId,
            when: Option<i64>,
        }
        let named: Vec<Named> = s
            .catalog
            .photos()
            .filter(|ph| ph.in_library() && in_scope(ph.id))
            .flat_map(|ph| {
                let when = ph.captured.as_deref().and_then(capture_secs);
                ph.meta
                    .regions
                    .iter()
                    .filter(|r| r.kind == RegionKind::Face)
                    .filter_map(|r| Some(Named { name: r.name.clone()?, emb: s.faces.index.get(ph.id.0, &r.rect)?.clone(), photo: ph.id, when }))
                    .collect::<Vec<_>>()
            })
            .collect();
        let near = |a: &Named, b: &Named| a.photo == b.photo || matches!((a.when, b.when), (Some(x), Some(y)) if (x - y).abs() <= burst);
        let mut sweep: Vec<(f32, usize, usize)> = thresholds.iter().map(|t| (*t, 0, 0)).collect();
        let (mut queries, mut top1, mut unmatchable) = (0usize, 0usize, 0usize);
        for q in &named {
            let others: Vec<&Named> = named.iter().filter(|g| !near(q, g)).collect();
            if !others.iter().any(|g| g.name.eq_ignore_ascii_case(&q.name)) {
                unmatchable += 1;
                continue;
            }
            let refs: Vec<(&str, &[f32])> = others.iter().map(|g| (g.name.as_str(), &g.emb[..])).collect();
            let ranked = matching::rank(&q.emb, &refs);
            queries += 1;
            if ranked.first().is_some_and(|r| r.name.eq_ignore_ascii_case(&q.name)) {
                top1 += 1;
            }
            for (t, suggested, correct) in sweep.iter_mut() {
                if let Some(sg) = matching::suggest(&ranked, *t, margin) {
                    *suggested += 1;
                    if sg.name.eq_ignore_ascii_case(&q.name) {
                        *correct += 1;
                    }
                }
            }
        }
        let pct = |n: usize, d: usize| if d == 0 { Value::Null } else { json!((n as f64 / d as f64 * 1000.0).round() / 1000.0) };
        Ok(json!({
            "model": s.faces.index.tag,
            "namedFaces": named.len(),
            "queries": queries,
            "unmatchable": unmatchable,
            "pendingPhotos": todo.len().saturating_sub(done),
            "top1": pct(top1, queries),
            "margin": margin,
            "burstSecs": burst,
            "sweep": sweep.iter().map(|(t, sg, ok)| json!({"threshold": t, "suggested": sg, "correct": ok, "precision": pct(*ok, *sg), "recall": pct(*ok, queries)})).collect::<Vec<_>>(),
        }))
    }

    /// The scan's row in the activity stack: there while `pending` photos are left, its whole the most that were left at
    /// once since the scan last finished (which it returns; 0 when none are left). It can't be stopped from there: the
    /// scan is switched off in Settings.
    fn track(s: &mut Session, pending: u64) -> u64 {
        if pending == 0 {
            s.faces.task = None;
            s.faces.peak = 0;
            return 0;
        }
        s.faces.peak = s.faces.peak.max(pending);
        let task = s.faces.task.get_or_insert_with(|| s.activity.start("faces", "Finding faces", crate::activity::Cancel::No));
        task.progress(s.faces.peak.saturating_sub(pending), s.faces.peak);
        s.faces.peak
    }

    /// What the grid asks for (its source and filters), which the scan's queue is made for.
    fn view_key(s: &Session) -> String {
        s.view_question()
    }

    /// Make the scan's queue again when it ran dry with the catalog changed since, or at once when the grid's source or
    /// filters changed since it was made: the People view's photos that need work come first (named faces first among
    /// them), then the rest as [`pending`] orders them. The photos a worker is busy with are not queued twice.
    fn fill_queue(s: &mut Session) {
        let key = view_key(s);
        let view_changed = s.faces.queue_view.as_ref() != Some(&key);
        let ran_dry = s.faces.in_flight.is_empty() && s.faces.queue.is_empty() && s.faces.queue_stamp != Some(s.catalog.revision);
        if !view_changed && !ran_dry {
            return;
        }
        let view: std::collections::HashSet<PhotoId> = s.people_view_photos().into_iter().collect();
        let mut queue = pending(s, &[], &view, true);
        queue.retain(|id| !s.faces.in_flight.contains(id));
        // the queue is taken from the end, so it is made last first
        queue.reverse();
        s.faces.queue = queue;
        s.faces.queue_stamp = Some(s.catalog.revision);
        s.faces.queue_view = Some(key);
    }

    /// `faces.pump`: cheap to call every frame. Takes in what the background worker has finished and, when it is idle,
    /// hands it the next photo with faces to embed. Does nothing (and says so) until recognition is switched on in
    /// Settings with a model chosen.
    pub fn pump(s: &mut Session, p: &Value) -> Result<Value> {
        const C: &str = "faces.pump";
        // how hard the scan may work now, from what the user is doing (the app says); nothing new is started while paused,
        // and what is already running finishes
        let pace = Pace::parse(p.get("pace").and_then(Value::as_str));
        let target = target_workers(pace);
        // a model that has finished downloading is installed and switched on here, whether or not recognition was on
        finish_downloads(s);
        // what the thread that puts the Unnamed faces in order has finished is taken in here (the window asks again when `arranged` moves)
        s.faces.arrange.take_in(false);
        let enabled = s.face_models_dir.clone().is_some_and(|d| enabled_cached(s, &d));
        if !enabled {
            track(s, 0);
            return Ok(json!({"active": false, "arranged": s.faces.arrange.taken}));
        }
        if s.faces.retry_at.is_some_and(|t| Instant::now() < t) {
            track(s, 0);
            return Ok(json!({"active": false, "arranged": s.faces.arrange.taken}));
        }
        let Ok(embedder) = current_with(s, C, false) else {
            // the chosen model is gone or broken: look again in a few seconds, not every frame
            s.faces.retry_at = Some(Instant::now() + Duration::from_secs(3));
            track(s, 0);
            return Ok(json!({"active": false, "arranged": s.faces.arrange.taken}));
        };
        s.faces.retry_at = None;
        let (embedded, _) = collect(s);
        if (s.faces.index.has_unsaved() || s.faces.scanned.has_unsaved()) && s.faces.saved_at.is_none_or(|t| t.elapsed() >= SAVE_EVERY) {
            if let Err(e) = s.faces.index.save() {
                log::warn!("could not save the face embeddings: {e}");
            }
            if let Err(e) = s.faces.scanned.save() {
                log::warn!("could not save the list of searched photos: {e}");
            }
            s.faces.saved_at = Some(Instant::now());
        }
        fill_queue(s);
        if target > 0 && s.faces.worker.is_none() && !s.faces.queue.is_empty() {
            s.faces.worker = Worker::start(max_workers());
        }
        // the workers are allowed `target` photos at once; a couple more wait behind them, so a worker that finishes has its
        // next photo at once instead of waiting for the next frame to hand it one
        if let Some(w) = &s.faces.worker {
            w.allow(target, pace);
        }
        // the GPU is held back the same way: a light pace leaves it mostly free for the window's own renders
        let (runs, share) = crate::faces_worker::gpu_pace(pace);
        lightcraft_gpu::faces::set_pace(runs, share);
        while target > 0 && s.faces.in_flight.len() < target + PREFETCH {
            let Some(id) = s.faces.queue.pop() else { break };
            let Some(job) = prepare(s, id, true) else { continue };
            if s.faces.worker.as_ref().is_some_and(|w| w.submit(embedder.clone(), job)) {
                s.faces.in_flight.insert(id);
            } else {
                break;
            }
        }
        let pending = s.faces.queue.len() + s.faces.in_flight.len();
        let peak = track(s, pending as u64);
        Ok(json!({
            "active": true,
            "inFlight": s.faces.in_flight.len(),
            "pendingPhotos": pending,
            "peak": peak,
            "indexedFaces": s.faces.index.len(),
            "searchedPhotos": s.faces.scanned.len(),
            "workers": target,
            "embedded": embedded,
            "arranged": s.faces.arrange.taken,
            "ordering": s.faces.arrange.is_running(),
        }))
    }

    /// `faces` ranked against `query` without the people the face rejected (compared like people are).
    fn rank_without(query: &[f32], gallery: &[(&str, &[f32])], rejected: &[String]) -> Vec<matching::Ranked> {
        if rejected.is_empty() {
            return matching::rank(query, gallery);
        }
        let keys: Vec<String> = rejected.iter().map(|r| r.trim().to_lowercase()).collect();
        let kept: Vec<(&str, &[f32])> = gallery.iter().filter(|(n, _)| !keys.contains(&n.trim().to_lowercase())).copied().collect();
        matching::rank(query, &kept)
    }

    /// Everyone named in the library, ranked against the face `index` of photo `id` (`rect`): (name, similarity, faces
    /// compared), best first. Empty while recognition is off or the face is not embedded; nothing is embedded here.
    pub fn ranking(s: &mut Session, id: PhotoId, index: usize, rect: &Rect) -> Vec<(String, f32, usize)> {
        let Some(dir) = s.face_models_dir.clone() else { return Vec::new() };
        if !enabled_cached(s, &dir) {
            return Vec::new();
        }
        let Some(query) = s.faces.index.get(id.0, rect).cloned() else { return Vec::new() };
        let mut gallery: Vec<(String, Arc<[f32]>)> = Vec::new();
        for ph in s.catalog.photos().filter(|ph| ph.in_library()) {
            for (i, r) in ph.meta.regions.iter().enumerate().filter(|(_, r)| r.kind == RegionKind::Face) {
                let Some(name) = r.name.clone() else { continue };
                if (ph.id, i) == (id, index) {
                    continue;
                }
                if let Some(e) = s.faces.index.get(ph.id.0, &r.rect) {
                    gallery.push((name, e.clone()));
                }
            }
        }
        let refs: Vec<(&str, &[f32])> = gallery.iter().map(|(n, e)| (n.as_str(), &e[..])).collect();
        matching::rank(&query, &refs).into_iter().map(|r| (r.name, r.score, r.faces)).collect()
    }

    /// The unnamed faces that look most like the person whose lower-cased name is `key`, best first, at most `limit`: each
    /// is scored like a suggestion (the mean of its two closest faces of theirs) and kept above four fifths of the
    /// model's suggestion bar, since a person looks at these before anything is named. Only faces already embedded can be
    /// found; `ready` says whether recognition is running at all.
    ///
    /// With `scope`, only faces in those photos are candidates; the person's gallery is always all their faces in the library.
    pub fn similar(s: &mut Session, key: &str, limit: usize, scope: Option<&std::collections::HashSet<PhotoId>>) -> Similar {
        let none = Similar { items: Vec::new(), ready: false, pending: 0 };
        let Some(dir) = s.face_models_dir.clone() else { return none };
        if !enabled_cached(s, &dir) {
            return none;
        }
        let Ok(embedder) = current_with(s, "faces.person", false) else { return none };
        let pending = s.faces.queue.len() + s.faces.in_flight.len();
        let (mut gallery, mut candidates): (Vec<Arc<[f32]>>, Vec<(u64, usize, Rect, Arc<[f32]>)>) = (Vec::new(), Vec::new());
        for ph in s.catalog.photos().filter(|ph| ph.in_library()) {
            for (index, r) in ph.meta.regions.iter().enumerate().filter(|(_, r)| r.kind == RegionKind::Face) {
                let Some(e) = s.faces.index.get(ph.id.0, &r.rect) else { continue };
                match r.name.as_deref() {
                    Some(n) if n.trim().to_lowercase() == key => gallery.push(e.clone()),
                    Some(_) => {}
                    // a face that rejected this person is not offered to them again
                    None if !r.rejects(key) && scope.is_none_or(|v| v.contains(&ph.id)) => candidates.push((ph.id.0, index, r.rect, e.clone())),
                    None => {}
                }
            }
        }
        let ready = Similar { items: Vec::new(), ready: true, pending };
        if gallery.is_empty() {
            return ready;
        }
        let bar = embedder.manifest().thresholds.match_cosine.unwrap_or(DEFAULT_THRESHOLD) * 0.8;
        let refs: Vec<&[f32]> = gallery.iter().map(|e| &e[..]).collect();
        // every unnamed face against every face of theirs: some 400 million comparisons for a person with 4 000 faces
        // in a library of 90 000, so on every core (minutes on one)
        let mut scored: Vec<(f32, u64, usize, Rect)> = candidates
            .into_par_iter()
            .filter_map(|(photo, index, rect, e)| {
                let score = matching::person_score(&e, &refs, matching::TOP_FACES)?;
                (score >= bar).then_some((score, photo, index, rect))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        scored.truncate(limit);
        let items = scored
            .into_iter()
            .map(|(score, photo, index, rect)| {
                let view = s.face_view(PhotoId(photo), rect);
                json!({"photo": photo, "index": index, "rect": super::rect_json(&rect), "view": super::rect_json(&view), "score": score})
            })
            .collect();
        Similar { items, ..ready }
    }

    /// `faces` (a person's confirmed faces, in the order the catalog lists them) with up to `count` faces that look as different
    /// from each other as possible first (the most typical one first, see [`crate::faces_varied`]), then all the others in the order
    /// they came. With a `count` of 0, or no embeddings, the faces are as they were.
    pub fn varied_first(s: &Session, faces: Vec<(PhotoId, usize, Rect)>, count: usize) -> Vec<(PhotoId, usize, Rect)> {
        if count == 0 {
            return faces;
        }
        let embeddings: Vec<Option<&[f32]>> = faces.iter().map(|(photo, _, rect)| s.faces.index.get(photo.0, rect).map(|e| &e[..])).collect();
        let picks = crate::faces_varied::pick(&embeddings, s.faces.index.dim, count);
        let mut taken = vec![false; faces.len()];
        for i in &picks {
            if let Some(t) = taken.get_mut(*i) {
                *t = true;
            }
        }
        let mut out: Vec<(PhotoId, usize, Rect)> = picks.iter().filter_map(|i| faces.get(*i).copied()).collect();
        out.extend(faces.iter().zip(&taken).filter(|(_, t)| !**t).map(|(f, _)| *f));
        out
    }

    fn rect_bits(r: &Rect) -> [u64; 4] {
        [r.x0.to_bits(), r.y0.to_bits(), r.x1.to_bits(), r.y1.to_bits()]
    }

    /// Everyone named in the library with the embeddings of their faces, grouped once so that scoring a face does no
    /// string work. A person's fingerprint (who, in which photo, where, what spelling; not the region number, which removing
    /// another box of the photo changes without any face of theirs changing) is the same for the same
    /// faces in any order the catalog lists them, and changes with any naming, un-naming, rename or merge of theirs.
    fn named_people(s: &Session) -> Vec<Person> {
        let mut people: Vec<Person> = Vec::new();
        let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for ph in s.catalog.photos().filter(|ph| ph.in_library()) {
            for r in ph.meta.regions.iter().filter(|r| r.kind == RegionKind::Face) {
                // (the name first: most faces are unnamed, and looking up the embedding of each would be most of this)
                let Some(name) = r.name.as_deref() else { continue };
                let Some(e) = s.faces.index.get(ph.id.0, &r.rect) else { continue };
                let key = name.to_lowercase();
                let i = *at.entry(key.clone()).or_insert_with(|| {
                    people.push(Person::new(key, name.to_string()));
                    people.len() - 1
                });
                if let Some(p) = people.get_mut(i) {
                    // (the spelling counts: a rename that only changes the capitalization changes what is suggested)
                    p.add(fingerprint(&(name, ph.id.0, rect_bits(&r.rect))), e.clone());
                }
            }
        }
        people
    }

    /// Put the unnamed faces `all` in order of confidence: the faces the named faces suggest a name for, most confident first;
    /// then the other embedded faces in look-alike order; then the faces not embedded yet, in photo order.
    ///
    /// Scoring every embedded face against every named person is the expensive part (faces x named faces comparisons), and so
    /// is the look-alike pass over the faces without a suggestion, so neither is done here: they are a run on a thread of
    /// its own ([`crate::faces_arrange::Jobs`]) that scores only what changed since the last one. Here the faces are listed
    /// as the last finished run put them: each keeps the suggestion it was scored at (unless its box or rejections changed
    /// since), the faces not scored yet go with those that have none. With `wait` the run for these very faces is started
    /// if there is none and waited for, so the answer is complete; without it the run is started and the answer says
    /// (`ordering`) that it is not done, and `faces.pump` takes the result in.
    pub fn arrange_unnamed(s: &mut Session, all: &mut Vec<(u64, usize, Rect)>, wait: bool, restart: bool, limit: usize) -> Result<super::Arranged> {
        let none = super::Arranged {
            suggested: false,
            look_alike: false,
            ready: false,
            ordering: false,
            complete: true,
            stopped: false,
            suggestions: Default::default(),
            entries: None,
            stacks: 0,
        };
        let Some(dir) = s.face_models_dir.clone() else { return Ok(none) };
        if !enabled_cached(s, &dir) {
            return Ok(none);
        }
        let Ok(embedder) = current_with(s, "faces.unnamed", false) else { return Ok(none) };
        let threshold = embedder.manifest().thresholds.match_cosine.unwrap_or(DEFAULT_THRESHOLD);
        let mut faces: Vec<Face> = Vec::new();
        let mut listed: Vec<Listed> = Vec::with_capacity(all.len());
        // which of a photo's faces with one very box each is (nearly always the only one)
        let mut same_box: std::collections::HashMap<(u64, [u64; 4]), u32> = std::collections::HashMap::new();
        for (photo, index, rect) in all.iter() {
            let ord = {
                let n = same_box.entry((*photo, rect_bits(rect))).or_insert(0);
                let ord = *n;
                *n = n.saturating_add(1);
                ord
            };
            let stamp = s.faces.index.get(*photo, rect).map(|e| {
                let rejected = s.catalog.photo(PhotoId(*photo)).and_then(|p| p.meta.regions.get(*index)).map_or(&[][..], |r| &r.rejected[..]);
                let face = Face::new(*photo, rect_bits(rect), ord, e.clone(), rejected);
                let stamp = face.stamp;
                faces.push(face);
                stamp
            });
            listed.push((*photo, *index, rect_bits(rect), ord, stamp));
        }
        let settings = Settings { model: fingerprint(&s.faces.index.tag), threshold, margin: DEFAULT_MARGIN };
        let people = named_people(s);
        let sig = signature(settings, &people, &faces);
        s.faces.arrange.take_in(false);
        if restart {
            s.faces.arrange.forget_stop(sig);
        }
        // (with no embedded face there is nothing to score or to put next to look-alikes)
        let mut input = Some(Input { settings, people, faces }).filter(|i| !i.faces.is_empty());
        let scoring = input.is_some();
        if wait && scoring {
            let mut started = false;
            while !s.faces.arrange.is_current(sig) {
                if s.faces.arrange.is_running() {
                    s.faces.arrange.take_in(true);
                    // the run this call started ended without a result: its row was cancelled, or its thread died
                    if started && !s.faces.arrange.is_current(sig) {
                        return Err(bad("faces.unnamed", "putting the faces in order was cancelled or stopped before it finished"));
                    }
                    continue;
                }
                let Some(input) = input.take() else {
                    return Err(bad("faces.unnamed", "putting the faces in order did not finish"));
                };
                s.faces.arrange.forget_stop(sig);
                if !s.faces.arrange.start(&s.activity, input, true) {
                    return Err(bad("faces.unnamed", "could not start the thread that puts the faces in order"));
                }
                started = true;
            }
        } else if !s.faces.arrange.is_running()
            && !s.faces.arrange.is_current(sig)
            && !s.faces.arrange.was_stopped(sig)
            && let Some(input) = input.take()
        {
            s.faces.arrange.start(&s.activity, input, false);
        }
        let complete = !scoring || s.faces.arrange.is_current(sig);
        let ordering = !complete && s.faces.arrange.is_running();
        // (a run that could not be started is marked stopped for these inputs, so no run and no result is a stop)
        let stopped = !complete && !ordering;
        let a = s.faces.arrange.arrange(&listed, limit);
        let mut by_place: Vec<Option<(u64, usize, Rect)>> = all.drain(..).map(Some).collect();
        let mut take = |i: &usize| by_place.get_mut(*i).and_then(Option::take);
        let entries: Vec<super::Entry> = a
            .order
            .iter()
            .filter_map(|item| match item {
                Item::Face(i) => take(i).map(super::Entry::Face),
                Item::Stack(places) => Some(super::Entry::Stack(places.iter().filter_map(&mut take).collect())),
            })
            .collect();
        Ok(super::Arranged {
            suggested: a.suggested,
            look_alike: a.look_alike,
            ready: true,
            ordering,
            complete,
            stopped,
            suggestions: a.suggestions,
            entries: Some(entries),
            stacks: a.stacks,
        })
    }

    /// Whether face recognition is switched on, looked up at most once a second.
    fn enabled_cached(s: &mut Session, dir: &std::path::Path) -> bool {
        if let Some((t, on)) = s.faces.enabled_seen
            && t.elapsed() < RECHECK
        {
            return on;
        }
        let on = read_settings(dir).enabled;
        s.faces.enabled_seen = Some((Instant::now(), on));
        on
    }

    pub fn suggest(s: &mut Session, p: &Value) -> Result<Value> {
        const C: &str = "faces.suggest";
        let embedder = current(s, C)?;
        collect(s);
        let targets = s.targets(p)?;
        let model_threshold = embedder.manifest().thresholds.match_cosine;
        let num = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32);
        let threshold = num("threshold").or(model_threshold).unwrap_or(DEFAULT_THRESHOLD);
        let margin = num("margin").unwrap_or(DEFAULT_MARGIN);
        // the photos asked about are embedded first, then the named faces they are compared with, within the budget
        let budget = Duration::from_millis(p.get("budgetMs").and_then(Value::as_u64).unwrap_or(1500).min(600_000));
        let started = Instant::now();
        let todo = pending(s, &targets, &Default::default(), false);
        let mut done = 0;
        for id in &todo {
            // a budget of zero only looks at what is already embedded
            if budget.is_zero() || (done > 0 && started.elapsed() >= budget) {
                break;
            }
            embed_photo(s, *id, &embedder)?;
            done += 1;
        }
        let pending_photos: Vec<u64> = todo.iter().skip(done).map(|id| id.0).collect();
        let gallery: Vec<(String, Arc<[f32]>)> = s
            .catalog
            .photos()
            .filter(|ph| ph.in_library())
            .flat_map(|ph| {
                ph.meta
                    .regions
                    .iter()
                    .filter(|r| r.kind == RegionKind::Face)
                    .filter_map(|r| Some((r.name.clone()?, s.faces.index.get(ph.id.0, &r.rect)?.clone())))
                    .collect::<Vec<_>>()
            })
            .collect();
        let refs: Vec<(&str, &[f32])> = gallery.iter().map(|(n, e)| (n.as_str(), &e[..])).collect();
        let mut photos = Vec::new();
        for id in &targets {
            let Some(photo) = s.catalog.photo(*id) else { continue };
            let mut faces = Vec::new();
            for (index, r) in photo.meta.regions.iter().enumerate().filter(|(_, r)| r.kind == RegionKind::Face && r.name.is_none()) {
                let Some(q) = s.faces.index.get(id.0, &r.rect) else {
                    faces.push(json!({"index": index, "pending": true}));
                    continue;
                };
                let ranked = rank_without(q, &refs, &r.rejected);
                let suggestion = matching::suggest(&ranked, threshold, margin);
                faces.push(json!({
                    "index": index,
                    "suggestion": suggestion.as_ref().map(|x| json!({"name": x.name, "score": x.score, "runnerUp": x.runner_up.as_ref().map(|(n, sc)| json!({"name": n, "score": sc}))})),
                    "candidates": ranked.iter().take(3).map(|c| json!({"name": c.name, "score": c.score, "faces": c.faces})).collect::<Vec<_>>(),
                }));
            }
            photos.push(json!({"id": id.0, "faces": faces}));
        }
        Ok(
            json!({"model": s.faces.index.tag, "threshold": threshold, "margin": margin, "galleryFaces": gallery.len(), "pendingPhotos": pending_photos, "photos": photos}),
        )
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::LibrarySource;
        use crate::tests_face_recognize::{install_yunet, region, set_regions, setup, temp, two_photos};

        fn unit(parts: &[(usize, f32)]) -> Vec<f32> {
            let mut v = vec![0.0; 64];
            for (i, x) in parts {
                v[*i] = *x;
            }
            let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.iter().map(|x| x / n).collect()
        }

        /// A session with a tiny model loaded (so the index has its model's tag).
        fn loaded(name: &str) -> (Session, std::path::PathBuf) {
            let d = temp(name);
            let (mut s, _) = setup(&d, 64);
            s.execute("faces.index", &json!({"budgetMs": 0})).unwrap();
            (s, d)
        }

        fn finished(s: &Session, id: PhotoId, found: Option<Vec<Rect>>, results: Vec<(Rect, Option<Vec<f32>>, Rect)>) -> Done {
            let content = s.catalog.photo(id).map(|p| crate::media::content_key(p)).unwrap_or_default();
            Done { tag: s.faces.index.tag.clone(), epoch: s.faces.epoch, id, content, results, detect_wanted: true, found }
        }

        #[test]
        fn faces_found_by_the_scan_become_unnamed_regions_without_an_undo_step() {
            let (mut s, d) = loaded("found");
            let (a, b) = two_photos(&s);
            set_regions(&mut s, b, vec![region(0.1, Some("Ann"))]);
            let (undo, redo) = (s.undo.len(), s.redo.len());
            let found = region(0.3, None).rect;
            let done = finished(&s, a, Some(vec![found]), vec![(found, Some(unit(&[(0, 1.0)])), found)]);
            assert_eq!(ingest(&mut s, done), (1, 0));
            let regions = &s.catalog.photo(a).unwrap().meta.regions;
            assert_eq!(regions.len(), 1);
            assert!(regions[0].kind == RegionKind::Face && regions[0].name.is_none() && regions[0].rect == found);
            assert!(regions[0].description.as_deref().unwrap().starts_with(MARK), "marked as detected");
            // not something the user did, and its embedding is already there
            assert_eq!((s.undo.len(), s.redo.len()), (undo, redo));
            assert!(s.skip_auto_write, "no sidecar is rewritten for it");
            assert!(s.faces.scanned.contains(a.0) && s.faces.index.contains(a.0, &found));
            // a photo that has faces by now (the user, an import) keeps exactly those
            let other = region(0.6, None).rect;
            let done = finished(&s, b, Some(vec![other]), vec![(other, None, other)]);
            ingest(&mut s, done);
            assert_eq!(s.catalog.photo(b).unwrap().meta.regions.len(), 1);
            assert!(s.faces.scanned.contains(b.0));
            let _ = std::fs::remove_dir_all(&d);
        }

        #[test]
        fn a_photo_that_could_not_be_searched_is_left_alone_for_the_session() {
            let (mut s, d) = loaded("scanfail");
            let (a, _) = two_photos(&s);
            assert!(needs_search(&s, s.catalog.photo(a).unwrap()));
            let done = finished(&s, a, None, vec![]);
            ingest(&mut s, done);
            assert!(!s.faces.scanned.contains(a.0), "not remembered as searched: the file may come back");
            assert!(!needs_search(&s, s.catalog.photo(a).unwrap()), "but not retried on every pass");
            assert!(!pending(&s, &[], &Default::default(), true).contains(&a));
            let _ = std::fs::remove_dir_all(&d);
        }

        /// A video went to the image decoders to be searched for faces and failed, once each launch: it is skipped, as the
        /// grid's thumbnails skip it (issue #2682 is about those).
        #[test]
        fn a_video_is_not_searched_for_faces() {
            let (mut s, d) = loaded("video");
            let (a, _) = two_photos(&s);
            let id = s.catalog.alloc_photo_id();
            let path = d.join("clip.mp4").to_string_lossy().into_owned();
            let clip = Photo::new(id, lightcraft_catalog::Source::File { path }, "clip.mp4", "VIDEO", 1920, 1080, "2026-10-24T00:00:00");
            s.catalog.apply(Op::AddPhoto { photo: Box::new(clip) }).unwrap();
            assert!(needs_search(&s, s.catalog.photo(a).unwrap()), "a photo is searched");
            assert!(!needs_search(&s, s.catalog.photo(id).unwrap()));
            assert!(!pending(&s, &[], &Default::default(), true).contains(&id));
            let _ = std::fs::remove_dir_all(&d);
        }

        #[test]
        fn only_the_background_scan_looks_for_faces_and_only_in_photos_without_any() {
            let (mut s, d) = loaded("search");
            // finding faces needs the detector, which is not bundled; without it the scan only embeds
            let detector = install_yunet(&mut s);
            let (a, b) = two_photos(&s);
            set_regions(&mut s, b, vec![region(0.1, Some("Ann")), region(0.5, None)]);
            // `faces.index` embeds what is boxed; only the scan (search = true) also searches the rest, last
            assert_eq!(pending(&s, &[], &Default::default(), false), vec![b]);
            let scan = pending(&s, &[], &Default::default(), true);
            assert_eq!(scan.first(), Some(&b), "named faces first");
            assert!(scan.contains(&a) && scan.len() > 2, "the unsearched photos follow");
            assert!(prepare(&mut s, a, false).is_none(), "nothing to embed in a photo without boxes");
            if detector {
                assert!(prepare(&mut s, a, true).is_some_and(|p| p.detect && p.todo.is_empty()));
            } else {
                assert!(prepare(&mut s, a, true).is_none(), "no detector: nothing to do, and not marked as searched");
            }
            assert!(prepare(&mut s, b, true).is_some_and(|p| !p.detect && p.todo.len() == 2), "a photo with boxes is not searched");
            // once searched, a photo is not searched again
            s.faces.scanned.insert(a.0);
            assert!(!pending(&s, &[], &Default::default(), true).contains(&a));
            let _ = std::fs::remove_dir_all(&d);
        }

        /// Four photos with faces to embed (`p0` unnamed, `p1` Ann, `p2` unnamed, `p3` Bob) and `p4` that only needs searching.
        fn five_photos(s: &mut Session) -> Vec<PhotoId> {
            let mut ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
            ids.sort();
            set_regions(s, ids[0], vec![region(0.1, None)]);
            set_regions(s, ids[1], vec![region(0.1, Some("Ann"))]);
            set_regions(s, ids[2], vec![region(0.1, None)]);
            set_regions(s, ids[3], vec![region(0.1, Some("Bob"))]);
            ids.truncate(5);
            ids
        }

        fn album_of(s: &mut Session, name: &str, photos: &[PhotoId]) -> LibrarySource {
            let id = s.execute("album.create", &json!({"name": name})).unwrap()["id"].as_u64().unwrap();
            let ids: Vec<u64> = photos.iter().map(|p| p.0).collect();
            s.execute("album.addPhotos", &json!({"id": id, "ids": ids})).unwrap();
            LibrarySource::Album(lightcraft_catalog::AlbumId(id))
        }

        /// The queue in the order the scan takes it.
        fn next_up(s: &Session) -> Vec<PhotoId> {
            s.faces.queue.iter().rev().copied().collect()
        }

        #[test]
        fn the_scan_does_the_photos_of_the_view_first_those_with_named_faces_first_among_them() {
            let (mut s, d) = loaded("scan-view");
            let p = five_photos(&mut s);
            // no source narrowing: today's order, named faces first, then the rest, then the photos that only need searching
            fill_queue(&mut s);
            assert_eq!(&next_up(&s)[..4], &[p[1], p[3], p[0], p[2]]);
            // an album of the unnamed p2, Bob's p3 and the search-only p4: they come first, p3 before p2 before p4,
            // then everything else as before
            let album = album_of(&mut s, "Set", &[p[2], p[3], p[4]]);
            s.source = album;
            fill_queue(&mut s);
            let queue = next_up(&s);
            assert_eq!(&queue[..5], &[p[3], p[2], p[4], p[1], p[0]], "{queue:?}");
            assert!(queue.len() > 5, "the other photos that need searching follow");
            let _ = std::fs::remove_dir_all(&d);
        }

        #[test]
        fn the_next_fill_after_a_source_or_filter_change_follows_the_new_photos() {
            let (mut s, d) = loaded("scan-view-change");
            let p = five_photos(&mut s);
            let first = album_of(&mut s, "First", &[p[2]]);
            let second = album_of(&mut s, "Second", &[p[0]]);
            s.source = first;
            fill_queue(&mut s);
            assert_eq!(next_up(&s).first(), Some(&p[2]));
            // the queue is not empty and nothing else changed: it is left alone
            let before = next_up(&s);
            s.faces.queue.pop();
            fill_queue(&mut s);
            assert_eq!(next_up(&s).len(), before.len() - 1, "not made again while the grid asks the same");
            // the grid asks for something else: the queue is made again at once
            s.source = second;
            fill_queue(&mut s);
            assert_eq!(next_up(&s).first(), Some(&p[0]));
            // so does a filter: only Ann's photo is shown now
            s.source = LibrarySource::All;
            s.filter.person = Some("Ann".into());
            fill_queue(&mut s);
            assert_eq!(next_up(&s).first(), Some(&p[1]));
            // a photo a worker is busy with is not queued again
            s.faces.in_flight.insert(p[1]);
            s.filter.person = Some("Bob".into());
            fill_queue(&mut s);
            assert!(next_up(&s).contains(&p[3]) && !next_up(&s).contains(&p[1]));
            let _ = std::fs::remove_dir_all(&d);
        }

        #[test]
        fn opening_another_library_forgets_the_old_ones_faces() {
            let (mut s, d) = loaded("newlib");
            let (a, _) = two_photos(&s);
            let rect = region(0.2, None).rect;
            s.faces.index.insert(a.0, &rect, unit(&[(0, 1.0)]));
            s.faces.scanned.insert(a.0);
            let epoch = s.faces.epoch;
            let old = finished(&s, a, Some(vec![rect]), vec![(rect, Some(unit(&[(1, 1.0)])), rect)]);
            s.open_library(d.join("second"), true).unwrap();
            assert_eq!((s.faces.index.len(), s.faces.scanned.len(), s.faces.queue.len(), s.faces.in_flight.len()), (0, 0, 0, 0));
            assert!(s.faces.epoch != epoch && s.faces.embedder.is_none());
            // a worker that was still busy with the old library cannot put its faces into the new one
            assert_eq!(ingest(&mut s, old), (0, 0));
            assert!(s.faces.index.len() == 0 && !s.faces.scanned.contains(a.0));
            let _ = std::fs::remove_dir_all(&d);
        }

        /// A Reload that finds the file replaced (an outside edit) gives the photo other pixels: what a worker made from
        /// the old file is dropped when it comes back, the faces already embedded from it are forgotten and the photo is
        /// searched again (issue #1524).
        #[test]
        fn faces_made_from_a_replaced_file_are_dropped() {
            let (mut s, d) = loaded("replaced");
            let (a, b) = two_photos(&s);
            let rect = region(0.2, None).rect;
            s.faces.index.insert(a.0, &rect, unit(&[(0, 1.0)]));
            s.faces.scanned.insert(a.0);
            s.faces.index.insert(b.0, &rect, unit(&[(1, 1.0)]));
            s.faces.scanned.insert(b.0);
            let old = finished(&s, a, Some(vec![rect]), vec![(rect, Some(unit(&[(2, 1.0)])), rect)]);
            let (width, height) = s.catalog.photo(a).map(|p| (p.width, p.height)).unwrap();
            let replace = Op::SetContent {
                id: a,
                width,
                height,
                file_size: 1234,
                content_hash: Some("edited".into()),
                preview_only: None,
                preview_checked: String::new(),
            };
            s.commit("Reload Changed Files", replace).unwrap();
            assert!(s.faces.index.get(a.0, &rect).is_none() && !s.faces.scanned.contains(a.0), "the old file's faces are forgotten");
            assert!(needs_search(&s, s.catalog.photo(a).unwrap()), "and the photo is searched again");
            assert!(s.faces.index.get(b.0, &rect).is_some() && s.faces.scanned.contains(b.0), "other photos keep theirs");
            assert_eq!(ingest(&mut s, old), (0, 0), "a result made from the old file is dropped");
            assert!(s.catalog.photo(a).unwrap().meta.regions.is_empty() && !s.faces.scanned.contains(a.0));
            // what is made from the new file is taken in
            let new = finished(&s, a, Some(vec![rect]), vec![(rect, Some(unit(&[(3, 1.0)])), rect)]);
            assert_eq!(ingest(&mut s, new), (1, 0));
            assert!(s.faces.scanned.contains(a.0) && s.faces.index.get(a.0, &rect).is_some());
            let _ = std::fs::remove_dir_all(&d);
        }

        #[test]
        fn the_unnamed_faces_that_look_like_a_person_come_best_first() {
            let (mut s, d) = loaded("similar");
            let (a, b) = two_photos(&s);
            let c = s.catalog.photos().map(|p| p.id).find(|id| *id != a && *id != b).unwrap();
            set_regions(&mut s, a, vec![region(0.1, Some("Ann")), region(0.5, Some("Bob"))]);
            set_regions(&mut s, b, vec![region(0.1, None), region(0.5, None)]);
            set_regions(&mut s, c, vec![region(0.3, None)]);
            let put = |s: &mut Session, id: PhotoId, x: f64, v: Vec<f32>| s.faces.index.insert(id.0, &region(x, None).rect, v);
            put(&mut s, a, 0.1, unit(&[(0, 1.0)])); // Ann
            put(&mut s, a, 0.5, unit(&[(1, 1.0)])); // Bob
            put(&mut s, b, 0.1, unit(&[(0, 0.95), (1, 0.05)])); // looks just like Ann
            put(&mut s, b, 0.5, unit(&[(1, 1.0)])); // looks like Bob, not Ann
            put(&mut s, c, 0.3, unit(&[(0, 0.8), (1, 0.6)])); // somewhat like Ann
            let r = similar(&mut s, "ann", 10, None);
            assert!(r.ready);
            let found: Vec<(u64, u64)> = r.items.iter().map(|i| (i["photo"].as_u64().unwrap(), i["index"].as_u64().unwrap())).collect();
            assert_eq!(found, vec![(b.0, 0), (c.0, 0)], "best first; the face that looks like Bob is left out");
            assert!(r.items[0]["score"].as_f64().unwrap() > r.items[1]["score"].as_f64().unwrap());
            assert_eq!(similar(&mut s, "ann", 1, None).items.len(), 1, "capped");
            assert!(similar(&mut s, "nobody", 10, None).items.is_empty());
            let _ = std::fs::remove_dir_all(&d);
        }

        /// Ann's confirmed faces as `faces.person` lists them, as (photo, region) pairs.
        fn confirmed(s: &mut Session, p: Value) -> Vec<(u64, u64)> {
            let r = s.execute("faces.person", &p).unwrap();
            r["confirmed"].as_array().unwrap().iter().map(|f| (f["photo"].as_u64().unwrap(), f["index"].as_u64().unwrap())).collect()
        }

        #[test]
        fn a_persons_confirmed_faces_start_with_the_varied_picks_then_the_rest_as_before() {
            let (mut s, d) = loaded("varied");
            let (a, _) = two_photos(&s);
            // eight faces of Ann in one photo, in three looks: regions 0 to 3 alike, 4 and 5 alike, 6 on its own, 7 not embedded
            let xs = [0.0, 0.05, 0.1, 0.15, 0.3, 0.35, 0.5, 0.7];
            set_regions(&mut s, a, xs.iter().map(|x| region(*x, Some("Ann"))).collect());
            let looks = [0, 0, 0, 0, 1, 1, 2];
            for (i, look) in looks.iter().enumerate() {
                s.faces.index.insert(a.0, &region(xs[i], None).rect, unit(&[(*look, 1.0), (7, 0.01 * i as f32)]));
            }
            let today: Vec<(u64, u64)> = (0..8).map(|i| (a.0, i)).collect();
            assert_eq!(confirmed(&mut s, json!({"name": "Ann"})), today, "without `varied` the list is as it was");
            let all = confirmed(&mut s, json!({"name": "Ann", "varied": 3}));
            let look_of = |(_, i): (u64, u64)| looks[i as usize];
            let mut firsts: Vec<usize> = all[..3].iter().map(|f| look_of(*f)).collect();
            assert_eq!(look_of(all[0]), 0, "the most typical face first: {all:?}");
            firsts.sort_unstable();
            assert_eq!(firsts, [0, 1, 2], "then the least alike: one of each look: {all:?}");
            assert_eq!(all.len(), 8, "every face is listed once");
            assert!(all[3..].windows(2).all(|w| w[0].1 < w[1].1), "the rest in today's order: {all:?}");
            let mut sorted = all.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, today);
            // more picks than faces with an embedding: the face without one comes last
            let many = confirmed(&mut s, json!({"name": "Ann", "varied": 500}));
            assert_eq!(many.last(), Some(&(a.0, 7)), "a face without an embedding after the embedded ones: {many:?}");
            assert_eq!(many.len(), 8);
            let _ = std::fs::remove_dir_all(&d);
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use serde_json::Value;

    use super::super::bad;
    use crate::{Result, Session};

    pub fn index(_: &mut Session, _: &Value) -> Result<Value> {
        Err(bad("faces.index", "this build cannot run face recognition"))
    }

    pub fn suggest(_: &mut Session, _: &Value) -> Result<Value> {
        Err(bad("faces.suggest", "this build cannot run face recognition"))
    }

    pub fn evaluate(_: &mut Session, _: &Value) -> Result<Value> {
        Err(bad("faces.evaluate", "this build cannot run face recognition"))
    }

    pub fn pump(_: &mut Session, _: &Value) -> Result<Value> {
        Ok(serde_json::json!({"active": false}))
    }

    pub fn ranking(_: &mut Session, _: lightcraft_catalog::PhotoId, _: usize, _: &lightcraft_geom::Rect) -> Vec<(String, f32, usize)> {
        Vec::new()
    }

    pub fn similar(_: &mut Session, _: &str, _: usize, _: Option<&std::collections::HashSet<lightcraft_catalog::PhotoId>>) -> super::Similar {
        super::Similar { items: Vec::new(), ready: false, pending: 0 }
    }

    pub fn varied_first(
        _: &Session,
        faces: Vec<(lightcraft_catalog::PhotoId, usize, lightcraft_geom::Rect)>,
        _: usize,
    ) -> Vec<(lightcraft_catalog::PhotoId, usize, lightcraft_geom::Rect)> {
        faces
    }

    pub fn arrange_unnamed(_: &mut Session, _: &mut Vec<(u64, usize, lightcraft_geom::Rect)>, _: bool, _: bool, _: usize) -> Result<super::Arranged> {
        Ok(super::Arranged {
            suggested: false,
            look_alike: false,
            ready: false,
            ordering: false,
            complete: true,
            stopped: false,
            suggestions: Default::default(),
            entries: None,
            stacks: 0,
        })
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "faces.index",
            "Index Faces",
            [],
            None,
            "{budgetMs?: 200, ids?: [photo ids to do first]} → {embedded, photosDone, pendingPhotos, indexedFaces, ms} — embed the faces in the library with the chosen recognition model, as many photos as fit in the time (at least one), the photos the grid shows first; call again until `pendingPhotos` is 0. `budgetMs: 0` only reports",
            super::always,
            imp::index
        ),
        cmd!(
            "faces.suggest",
            "Suggest Names for Faces",
            [],
            None,
            "{ids?, threshold?, margin?, budgetMs?} → {photos: [{id, faces: [{index, suggestion: {name, score, runnerUp} | null, candidates: [{name, score}]}]}]} — who the unnamed faces in these photos might be, from the faces already named in the library. Only suggestions: nothing is named",
            super::always,
            imp::suggest
        ),
        cmd!(
            query "faces.pump",
            "Index Faces in the Background",
            [],
            None,
            "{pace?: pause | light | normal | full} → {active, inFlight, pendingPhotos, indexedFaces, searchedPhotos, workers, arranged, ordering} — cheap to call every frame: takes in what the background workers have finished and gives them the next photos, as many at once as the pace allows (none when paused, one when light, half the machine's threads when normal, four fifths when full); it also takes in the result of the thread that puts the Unnamed faces in order (`arranged` counts the results taken in, so a window knows to ask `faces.unnamed` again; `ordering` says that thread is at work); inactive until face recognition is on in Settings with a model chosen",
            super::always,
            imp::pump
        ),
        cmd!(
            "faces.evaluate",
            "Evaluate Face Recognition",
            [],
            None,
            "{budgetMs?: 0, burstSecs?: 5, margin?, thresholds?: [..], scope?: \"visible\"} → {queries, top1, sweep: [{threshold, suggested, correct, precision, recall}]} — leave-one-out over your named faces: each is ranked against the others (ignoring shots within burstSecs of it) to show how well the chosen model recognises people in *your* photos and which threshold to trust. Faces not yet embedded are done within budgetMs (`faces.index` does them all); counts only",
            super::always,
            imp::evaluate
        ),
        cmd!(
            query "faces.unnamed",
            "Unnamed Faces",
            [],
            None,
            "{limit?: 3000, scope?: \"visible\", wait?: true, restart?: false} → {total, stacks, faces: [{photo, index, rect, view, suggestion: {name, score} | null, stack?: {count, faces: [{photo, index, rect}]}}], ordered, lookAlike, ready, ordering, complete, stopped} — the faces nobody has named; with recognition running, in this order: the faces the named ones recognise, the most confident suggestion first (names mixed, ties by photo then region); then the stacks, the biggest first; then the other faces without a suggestion, look-alikes next to each other (`lookAlike` says some are); then the faces not embedded yet, in photo order. A stack is one entry of `faces` for faces without a suggestion that are virtually certain to be the same person: its `stack.faces` are all its faces in scope, the most typical first, `stack.count` their number (at least two), and the entry's own `photo`, `index`, `rect`, `view` are its representative's (the first face); `suggestion` is null. The faces of a stack are listed nowhere else, and `total` counts faces, not entries; `stacks` is how many entries are stacks. Two faces stack only when each is at least as alike as the bar to every other face of the stack (the bar is the highest similarity between two faces named as different people plus a margin, never below a fixed minimum; no chaining: A like B and B like C never puts A and C together), so naming a stack (`faces.nameFaces` with its faces, one undo step) puts no wrong name on a face; a stack with one face in scope is that face alone. Stacks are worked out in the same background run (and appear in the answer once it has finished; until then every face is its own entry), and recognition off or no model means none. Every embedded face is scored for the order, not only the ones listed, on a thread of its own (a `faceOrder` row in `activity.list`, which can be cancelled): a change (a naming, un-naming, rename, merge, rejected name, moved or removed box) scores again only the people whose named faces changed and the faces whose own box or rejections changed, with the result of scoring everything again. By default the call waits for the complete order. With `wait: false` it answers at once with the best order there is (faces keep the order and suggestions already worked out; a face whose box or rejections changed has no suggestion; faces not scored yet go with the faces that have none; a face just named is gone) and `ordering: true` says the rest is still being worked out; `faces.pump` takes the result in (its `arranged` count moves), and asking again then gives the order. `complete` is true when what is listed is the final order (the run was for these very faces and names, or there is nothing to score); with neither `ordering` nor `complete` the run was cancelled, died or could not be started: `stopped` is true, nothing starts it again until a face or a name changes, and `restart: true` starts it again (a cancelled run is not lost work: the order is worked out again when asked for) With `scope: \"visible\"` only the faces in the photos the People view shows are listed and counted in `total` (the source chosen in the sidebar with its subfolders, every filter, the person filter, a held view, collapsed stacks with all their photos, library photos only); suggestions still compare against every named face in the library. Without a scope the whole library. `view` is the box to show the face by. Name them with `faces.nameFaces`",
            super::always,
            unnamed
        ),
        cmd!(
            "faces.nameFaces",
            "Name Faces",
            [],
            None,
            "{faces: [{photo, index, name?}], name?} → {named, photos} — name (or, with a null or empty name, clear) many face regions as one undoable step; a face's own `name` wins over the top-level one, so each face can get its own; the sidecar is not touched",
            super::always,
            name_faces
        ),
        cmd!(
            query "faces.rank",
            "Rank Names for a Face",
            [],
            None,
            "{id?, index} → {photo, index, ranking: [{name, score, faces}]} — every named person ranked by how much they look like this face (best first; `score` is the similarity the name box shows as a percentage). Read-only: it uses what the background scan has embedded and embeds nothing, so a face the scan has not looked at, or recognition off, gives an empty `ranking`; errors for a missing photo or an index that is not a face",
            super::always,
            rank_face
        ),
        cmd!(
            query "faces.people",
            "Named People",
            [],
            None,
            "{scope?: \"visible\"} → {people: [{name, photos, photo, face: {x0, y0, x1, y1}}], hidden} — the Named People cards, most photos first then by name: `photos` is how many photos they are in, `photo` and `face` their largest face. With `scope: \"visible\"` only the photos the People view shows count (the source chosen in the sidebar with its subfolders, every filter, the person filter, a held view, collapsed stacks with all their photos, library photos only) and `hidden` is how many faces, named or unnamed, are in library photos they leave out; without a scope the whole library (`hidden` 0)",
            super::always,
            people
        ),
        cmd!(
            query "faces.person",
            "Faces of a Person",
            [],
            None,
            "{name, more?: 120, scope?: \"visible\", varied?: 0} → {name, total, inLibrary, confirmed: [{photo, index, rect, view}], more: [{photo, index, rect, score}], ready, pendingPhotos} — the faces named `name` across the library (`inLibrary` is how many there are in all, in scope or not), and the unnamed faces that look most like them (best first, already embedded ones only, once recognition is on); confirm one with `faces.setName`. With `scope: \"visible\"` the confirmed faces and `total` are those in the photos the People view shows, and More looks in those photos too, except that a person filter naming this same person (trimmed, any capitalization) is ignored for More, so filtering to them still shows faces that might be them; a person filter naming anyone else still applies. More always scores against all of the person's named faces in the library, so a person with no named faces in the scope (`total` 0) still gets their possible faces from it. Without a scope the whole library. With `varied: N` (at most 5000) the first N faces of `confirmed` (fewer when fewer have an embedding) are the ones that look as different from each other as possible: the most typical face first (closest to the average of the person's embedded faces), then each next face the one least like those already picked; faces without a usable embedding are never picked; all the other faces follow in the order they have without `varied`. Every face is listed once, and `confirmed` is still capped at 5000",
            super::always,
            person
        ),
        cmd!(
            "faces.setName",
            "Name Face",
            [],
            None,
            "{id?, index, name, keepActive?: bool} — name a face region (null or empty clears it); undoable; the sidecar is not touched; when this takes the active photo out of the view, the selection moves on unless `keepActive` is true",
            super::always,
            set_name
        ),
        cmd!(
            "faces.renamePerson",
            "Rename Person",
            [],
            None,
            "{from, to, merge?: bool} → {faces, photos, from, to, merged} — give every in-library face of `from` the name `to` (trimmed, any capitalization), one undo step; refused when `to` is already someone else unless `merge` is true, which combines the two; the sidecar is not touched",
            super::always,
            rename_person
        ),
    ]
}
