//! Putting the Unnamed faces in confidence order, without a session: the part that costs something (faces x named
//! faces comparisons, and the look-alike chain), made so that a change is paid for by what it changed.
//!
//! A face's name is suggested from the score of every named person against it ([`matching::person_score`]: the mean of
//! the person's two closest faces), and only the best and the second-best person matter ([`matching::suggest`]). So
//! the [`Scorer`] keeps for each unnamed face the few best people with their scores, and a bound that every other person
//! stays under. When some people's named faces change (a naming, an un-naming, a rename, a merge), only the scores
//! against those people are worked out again, for each face; a face whose own box, embedding or rejections changed is
//! scored against everybody. Whatever the changes, the suggestion, score and order of every face is what scoring it
//! against everybody from scratch gives (the tests compare, after every random change, with exactly that).
//!
//! Nothing here touches the session, so [`arrange`] runs on a thread of its own ([`crate::cmd`]'s `faces.unnamed`).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lightcraft_faces::matching;
use lightcraft_meta::Apart;
use rayon::prelude::*;

/// A face as the runs know it: its photo, its own box (as bits) and which of the faces of that photo with the very same box
/// it is (0 for nearly every face; an XMP import can carry two regions with one box, and they may differ in what they reject).
/// Not its region index: removing a box renumbers the later regions of the photo, and what was worked out for a face must
/// stay with the face.
pub(crate) type Key = (u64, [u64; 4], u32);

/// A face box as the runs key it: the bits of its four numbers (the same box is the same bits).
pub(crate) fn rect_bits(r: &lightcraft_geom::Rect) -> [u64; 4] {
    [r.x0.to_bits(), r.y0.to_bits(), r.x1.to_bits(), r.y1.to_bits()]
}

/// The box that [`rect_bits`] made `bits` from.
pub(crate) fn rect_from_bits(bits: [u64; 4]) -> lightcraft_geom::Rect {
    lightcraft_geom::Rect { x0: f64::from_bits(bits[0]), y0: f64::from_bits(bits[1]), x1: f64::from_bits(bits[2]), y1: f64::from_bits(bits[3]) }
}

/// How the catalogue's `apart` lists name the face of `photo` with the box `bits`: by the box in ten-thousandths.
pub(crate) fn apart_at(photo: u64, bits: [u64; 4]) -> Apart {
    Apart { photo, rect: crate::faces_index::region_key(&rect_from_bits(bits)) }
}

/// How many of the best people are kept for each face. Two are enough to suggest; the rest make a person whose score
/// falls (un-naming, a rename) replaceable without scoring the face against everybody again.
pub(crate) const KEPT: usize = 4;

/// Most faces put in order of likeness (the order is a pass over every pair of them).
pub(crate) const MAX_ARRANGED: usize = 1500;

/// Most people a face's kept entry may hold when many of them tie exactly (identical embeddings, one photo named under many
/// names): an entry over it is not kept for the next run, the face is scored against everybody again then (hostile input must
/// not make what is kept grow with the number of people).
pub(crate) const TIE_CAP: usize = 16;

/// Faces scored between two looks at the cancel flag.
const CHUNK: usize = 256;

/// A fingerprint of `x`, for noticing that something is the same as last time.
pub(crate) fn fingerprint<T: Hash + ?Sized>(x: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    x.hash(&mut h);
    h.finish()
}

/// The start of an embedding, as bits: enough to notice that a face was embedded again from other pixels (a new embedding
/// differs in every number, and hashing all 512 of every face would cost the window tens of milliseconds for tens of thousands).
fn head_bits(e: &[f32]) -> [u32; 16] {
    let mut out = [0u32; 16];
    for (o, x) in out.iter_mut().zip(e) {
        *o = x.to_bits();
    }
    out
}

/// A person named in the library with the embeddings of their faces.
pub(crate) struct Person {
    /// The lower-cased name.
    pub key: String,
    /// The name as first seen, which is what a suggestion shows.
    pub shown: String,
    pub faces: Vec<Arc<[f32]>>,
    sum: u64,
}

impl Person {
    pub fn new(key: String, shown: String) -> Person {
        Person { key, shown, faces: Vec::new(), sum: 0 }
    }

    /// One more face of theirs. `at` says which face it is: who it is named, in which photo and region, where (so that
    /// the same faces in any order make the same person, and a rename that only changes capitalization makes a
    /// different one).
    pub fn add(&mut self, at: u64, embedding: Arc<[f32]>) {
        self.sum = self.sum.wrapping_add(fingerprint(&(at, head_bits(&embedding))));
        self.faces.push(embedding);
    }

    /// What the scores against this person depend on.
    pub fn fingerprint(&self) -> u64 {
        fingerprint(&(self.sum, &self.shown, self.faces.len()))
    }
}

/// An unnamed face with an embedding.
pub(crate) struct Face {
    pub photo: u64,
    /// The face's own box, as bits.
    pub rect: [u64; 4],
    /// Which of the faces of the photo with this very box it is.
    pub ord: u32,
    /// What the face's own score depends on besides the named faces: its box, its embedding and its rejections.
    pub stamp: u64,
    pub embedding: Arc<[f32]>,
    /// The names it rejected, trimmed and lower-cased.
    rejected: Vec<String>,
    /// Where the face is as the catalogue names a face in `apart` lists: its photo and its box in ten-thousandths.
    pub at: Apart,
    /// The faces it was taken out of a stack with: it never stacks with them (nor they with it).
    pub apart: std::collections::HashSet<Apart>,
}

impl Face {
    pub fn new(photo: u64, bits: [u64; 4], ord: u32, embedding: Arc<[f32]>, rejected: &[String]) -> Face {
        let rejected: Vec<String> = rejected.iter().map(|r| r.trim().to_lowercase()).collect();
        let stamp = fingerprint(&(bits, &rejected, head_bits(&embedding)));
        Face { photo, rect: bits, ord, stamp, embedding, rejected, at: apart_at(photo, bits), apart: Default::default() }
    }

    /// The face with the faces it was taken out of a stack with. They are part of its stamp (what the run kept of it is
    /// worked out again), unless there are none, so that a face that was never taken out keeps its stamp.
    ///
    /// Only the first `MAX_STACK` are read: that is as many as `faces.takeOut` ever writes, and a catalogue that says more is
    /// corrupt or hostile (the list is sorted and put in a set here).
    pub fn with_apart(mut self, apart: &[Apart]) -> Face {
        let apart = apart.get(..crate::faces_stack::MAX_STACK).unwrap_or(apart);
        if !apart.is_empty() {
            let mut sorted: Vec<(u64, [i32; 4])> = apart.iter().map(|a| (a.photo, a.rect)).collect();
            sorted.sort_unstable();
            self.stamp = fingerprint(&(self.stamp, sorted));
            self.apart = apart.iter().copied().collect();
        }
        self
    }

    /// Whether the two faces may not share a stack: either was taken out of a stack with the other.
    pub fn apart_from(&self, other: &Face) -> bool {
        (!self.apart.is_empty() && self.apart.contains(&other.at)) || (!other.apart.is_empty() && other.apart.contains(&self.at))
    }

    pub fn key(&self) -> Key {
        (self.photo, self.rect, self.ord)
    }

    fn rejects(&self, key: &str) -> bool {
        self.rejected.iter().any(|r| r == key)
    }
}

/// What the scores depend on besides the people and the faces.
#[derive(Clone, Copy)]
pub(crate) struct Settings {
    /// The model the embeddings were made with (another model, another scores).
    pub model: u64,
    pub threshold: f32,
    pub margin: f32,
}

/// What is kept for one face: the best people with their scores, best first, and a bound above every other person's
/// score (`None`: nobody else has one). Everything listed scores strictly above the bound.
#[derive(Clone)]
struct Entry {
    stamp: u64,
    top: Vec<(u32, f32)>,
    bound: Option<f32>,
    /// Too many people tied to keep them all ([`TIE_CAP`]): good for this run's suggestion, not kept for the next.
    stale: bool,
}

fn by_score(a: &(u32, f32), b: &(u32, f32)) -> std::cmp::Ordering {
    b.1.total_cmp(&a.1).then(a.0.cmp(&b.0))
}

impl Entry {
    /// Keep the best [`KEPT`] people (and anyone tied with the last of them, so that what is left out scores strictly below
    /// what is kept), remembering how high what was left out can be.
    fn trim(&mut self) {
        self.top.sort_by(by_score);
        let Some(last) = self.top.get(KEPT.saturating_sub(1)).map(|(_, s)| *s) else { return };
        let Some(cut) = self.top.iter().position(|(_, s)| *s < last) else { return };
        let Some(highest_left_out) = self.top.get(cut).map(|(_, s)| *s) else { return };
        self.top.truncate(cut);
        self.bound = Some(self.bound.map_or(highest_left_out, |b| b.max(highest_left_out)));
    }

    /// Cut what is kept to [`TIE_CAP`] people, marking the entry as not to be reused when that dropped any.
    fn shrink(&mut self) {
        if self.top.len() > TIE_CAP {
            self.top.truncate(TIE_CAP);
            self.stale = true;
        }
    }

    /// Whether the two best people are known for certain.
    fn usable(&self) -> bool {
        self.bound.is_none() || self.top.len() >= 2
    }
}

/// A person as a face is scored against: their faces, and the key a rejection names.
struct Crowd<'a> {
    pid: u32,
    key: &'a str,
    faces: Vec<&'a [f32]>,
}

impl Crowd<'_> {
    fn score(&self, face: &Face) -> Option<f32> {
        if face.rejects(self.key) {
            return None;
        }
        matching::person_score(&face.embedding, &self.faces, matching::TOP_FACES)
    }
}

/// `face` scored against everybody. `scratch` holds the scores meanwhile (it is reused from face to face, so that what is
/// kept is as small as it is: a list of every person's score per face would be most of a gigabyte for tens of thousands
/// of faces).
fn full(face: &Face, crowd: &[Crowd], scratch: &mut Vec<(u32, f32)>) -> Entry {
    scratch.clear();
    scratch.extend(crowd.iter().filter_map(|c| Some((c.pid, c.score(face)?))));
    scratch.sort_unstable_by(by_score);
    // the best KEPT, and anyone tied with the last of them (see `Entry::trim`)
    let cut = match scratch.get(KEPT.saturating_sub(1)) {
        Some((_, last)) => scratch.iter().position(|(_, s)| s < last).unwrap_or(scratch.len()),
        None => scratch.len(),
    };
    Entry { stamp: face.stamp, top: scratch.get(..cut).unwrap_or_default().to_vec(), bound: scratch.get(cut).map(|(_, s)| *s), stale: false }
}

/// `old` (what was kept for the same face) after the scores against the `changed` people were worked out again. `None`
/// when what is left no longer settles the two best people: the face is scored against everybody then.
fn delta(old: &Entry, face: &Face, changed: &[u32], crowd: &HashMap<u32, &Crowd>) -> Option<Entry> {
    let mut e = old.clone();
    e.top.retain(|(p, _)| !changed.contains(p));
    for pid in changed {
        let Some(score) = crowd.get(pid).and_then(|c| c.score(face)) else { continue };
        // a person who was left out scored at most the bound, so only a higher score can bring them back
        if e.bound.is_none_or(|b| score > b) {
            e.top.push((*pid, score));
        }
    }
    e.trim();
    e.usable().then_some(e)
}

/// The name the best person gives, when the bars say so: what [`matching::suggest`] says of the two best.
fn resolve(e: &Entry, names: &HashMap<u32, &str>, settings: Settings) -> Option<(String, f32)> {
    let mut ranked: Vec<(f32, &str)> = e.top.iter().filter_map(|(p, score)| Some((*score, *names.get(p)?))).collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    // (only two are looked at, so only two names are copied however many people tie)
    let two: Vec<matching::Ranked> =
        ranked.into_iter().take(2).map(|(score, name)| matching::Ranked { name: name.to_string(), score, faces: 0 }).collect();
    let x = matching::suggest(&two, settings.threshold, settings.margin)?;
    Some((x.name, x.score))
}

/// What is worked out about the unnamed faces and kept for the next time: the scores of every face against every person.
#[derive(Default)]
pub(crate) struct Scorer {
    model: u64,
    pids: HashMap<String, u32>,
    next_pid: u32,
    /// Each person's fingerprint when the entries were worked out.
    people: HashMap<u32, u64>,
    entries: HashMap<Key, Entry>,
    /// How many person scores the last [`Scorer::update`] worked out (a face against a person is one).
    pub last_scored: usize,
}

impl Scorer {
    /// What a changed model or bar does: another model is another set of scores, so every face is scored against everybody
    /// again. The bars (threshold, margin) only decide which of the kept best people is suggested, never what a person
    /// scores, so a changed bar scores nobody again and the suggestions are read off the kept scores afresh (the result is
    /// what scoring everyone again gives; the tests compare).
    ///
    /// The name (and score) each of `faces` is suggested from `people`, in the faces' order; none when nobody passes the
    /// bars. Only what changed since the last call is worked out: the scores of the faces against the people whose named
    /// faces changed, and the faces that are new or whose own stamp changed against everybody. `None` when `cancel`
    /// was set (looked at between chunks of faces); the scorer is then as it was.
    ///
    /// Faces not in `faces` are forgotten, so what is kept is bounded by what is asked about. `progress(done, total)` is
    /// told as it goes. Run it inside the rayon pool it should use.
    pub fn update(
        &mut self,
        settings: Settings,
        people: &[Person],
        faces: &[Face],
        cancel: &AtomicBool,
        progress: &(dyn Fn(usize, usize) + Sync),
    ) -> Option<Vec<Option<(String, f32)>>> {
        let fresh = self.model != settings.model;
        let (mut pids, mut next_pid) = if fresh { (HashMap::new(), 0) } else { (self.pids.clone(), self.next_pid) };
        let known: HashMap<u32, u64> = if fresh { HashMap::new() } else { self.people.clone() };
        let mut now: HashMap<u32, u64> = HashMap::new();
        let mut crowd: Vec<Crowd> = Vec::with_capacity(people.len());
        for p in people {
            let pid = *pids.entry(p.key.clone()).or_insert_with(|| {
                next_pid += 1;
                next_pid
            });
            now.insert(pid, p.fingerprint());
            crowd.push(Crowd { pid, key: p.key.trim(), faces: p.faces.iter().map(|e| &e[..]).collect() });
        }
        let mut changed: Vec<u32> = now.iter().filter(|(pid, fp)| known.get(pid) != Some(fp)).map(|(pid, _)| *pid).collect();
        changed.extend(known.keys().filter(|pid| !now.contains_key(pid)));
        changed.sort_unstable();
        let by_pid: HashMap<u32, &Crowd> = crowd.iter().map(|c| (c.pid, c)).collect();
        let names: HashMap<u32, &str> = crowd.iter().zip(people).map(|(c, p)| (c.pid, p.shown.as_str())).collect();
        let old = std::mem::take(&mut self.entries);
        let mut scored = 0usize;
        let mut entries: HashMap<Key, Entry> = HashMap::with_capacity(faces.len());
        let mut out: Vec<Option<(String, f32)>> = Vec::with_capacity(faces.len());
        for (n, chunk) in faces.chunks(CHUNK).enumerate() {
            if cancel.load(Ordering::Relaxed) {
                // nothing was changed but the entries taken out above: put them back
                self.entries = old;
                return None;
            }
            progress(n * CHUNK, faces.len());
            let done: Vec<(Entry, Option<(String, f32)>, usize)> = chunk
                .par_iter()
                .map_init(Vec::new, |scratch, face| {
                    let kept = old.get(&face.key()).filter(|e| !fresh && e.stamp == face.stamp && !e.stale);
                    let (entry, work) = match kept {
                        Some(e) if changed.is_empty() => (e.clone(), 0),
                        Some(e) => match delta(e, face, &changed, &by_pid) {
                            Some(d) => (d, changed.len()),
                            None => (full(face, &crowd, scratch), changed.len() + crowd.len()),
                        },
                        None => (full(face, &crowd, scratch), crowd.len()),
                    };
                    let suggestion = resolve(&entry, &names, settings);
                    let mut entry = entry;
                    entry.shrink();
                    (entry, suggestion, work)
                })
                .collect();
            for (face, (entry, suggestion, work)) in chunk.iter().zip(done) {
                entries.insert(face.key(), entry);
                out.push(suggestion);
                scored += work;
            }
        }
        progress(faces.len(), faces.len());
        self.last_scored = scored;
        pids.retain(|_, pid| now.contains_key(pid));
        (self.model, self.pids, self.next_pid, self.people, self.entries) = (settings.model, pids, next_pid, now, entries);
        Some(out)
    }
}

/// The order of the faces in `embeddings` that puts look-alikes together: start anywhere, then always go to the closest
/// face not yet placed. `None` when `cancel` was set.
fn nearest_chain(embeddings: &[&[f32]], cancel: &AtomicBool) -> Option<Vec<usize>> {
    let n = embeddings.len();
    let (mut placed, mut order) = (vec![false; n], Vec::with_capacity(n));
    let Some(first) = placed.first_mut() else { return Some(order) };
    *first = true;
    order.push(0);
    let mut current = 0;
    for _ in 1..n {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let from = embeddings.get(current)?;
        let mut best: Option<(usize, f32)> = None;
        for (j, e) in embeddings.iter().enumerate() {
            if placed.get(j).copied().unwrap_or(true) {
                continue;
            }
            let c = matching::cosine(from, e).unwrap_or(-1.0);
            if best.is_none_or(|(_, b)| c > b) {
                best = Some((j, c));
            }
        }
        let Some((j, _)) = best else { break };
        if let Some(p) = placed.get_mut(j) {
            *p = true;
        }
        order.push(j);
        current = j;
    }
    Some(order)
}

/// Everything a run keeps for the next one.
#[derive(Default)]
pub(crate) struct State {
    pub scorer: Scorer,
    /// The highest similarity between two people, for the stacking bar.
    pub calibration: crate::faces_stack::Calibration,
    /// The stacks of the faces without a suggestion, as the last run left them.
    pub stacks: crate::faces_stack::StackState,
    /// The stacks of the faces suggested a name, a state for each name (faces suggested different names never share a stack).
    pub named_stacks: HashMap<String, crate::faces_stack::StackState>,
    /// A fingerprint of the faces in look-alike order, and their order: reused while those are the same faces.
    chain_of: u64,
    chain: Vec<Key>,
}

/// What a run is asked to arrange.
pub(crate) struct Input {
    pub settings: Settings,
    pub people: Vec<Person>,
    /// The embedded unnamed faces, in photo and region order.
    pub faces: Vec<Face>,
}

/// What a run worked out.
#[derive(Default)]
pub(crate) struct Output {
    /// A face to the stamp it was scored at, its suggested name and the score.
    pub suggestions: HashMap<Key, (u64, String, f32)>,
    /// The faces without a suggestion and in no stack (the first [`MAX_ARRANGED`]) in look-alike order.
    pub chain: Vec<Key>,
    /// The stacks of faces without a suggestion, the biggest first: each face with the stamp it was stacked at, the most typical first.
    pub stacks: Vec<Vec<(Key, u64)>>,
    /// The bar the stacks were made at.
    pub bar: f32,
    /// How many faces the stacking looked for look-alikes of, and how many comparisons it made (the rest was kept from the run before).
    pub stack_new: usize,
    pub stack_compared: usize,
    /// How many person scores the run worked out (a face against a person is one).
    pub scored: usize,
    /// The look-alike order of the last run was still the right one, so it was not worked out again.
    pub chain_reused: bool,
}

/// Score the faces and put those without a suggestion in look-alike order. `None` when `cancel` was set. Run it inside the
/// rayon pool it should use.
pub(crate) fn arrange(state: &mut State, input: &Input, cancel: &AtomicBool, progress: &(dyn Fn(usize, usize) + Sync)) -> Option<Output> {
    let scored = state.scorer.update(input.settings, &input.people, &input.faces, cancel, progress)?;
    let mut suggestions = HashMap::new();
    let mut plain: Vec<&Face> = Vec::new();
    let mut named: std::collections::BTreeMap<String, Vec<&Face>> = std::collections::BTreeMap::new();
    for (face, suggestion) in input.faces.iter().zip(scored) {
        match suggestion {
            Some((name, score)) => {
                named.entry(name.clone()).or_default().push(face);
                suggestions.insert(face.key(), (face.stamp, name, score));
            }
            None => plain.push(face),
        }
    }
    // faces that are virtually certain to be one person are one stack; the others are put next to their look-alikes
    let bar = state.calibration.bar(&input.people, cancel)?;
    let found = crate::faces_stack::stack(&mut state.stacks, &plain, bar, cancel, crate::faces_stack::BRUTE_LIMIT)?;
    let stacked: std::collections::HashSet<usize> = found.iter().flatten().copied().collect();
    let mut stacks: Vec<Vec<(Key, u64)>> =
        found.iter().map(|s| s.iter().filter_map(|i| plain.get(*i).map(|f| (f.key(), f.stamp))).collect()).collect();
    let (mut stack_new, mut stack_compared) = (state.stacks.last_new, state.stacks.last_compared);
    // the faces suggested one name are stacked among themselves, under the same bar and the same no-chaining rule
    for (name, faces) in &named {
        let kept = state.named_stacks.entry(name.clone()).or_default();
        let found = crate::faces_stack::stack(kept, faces, bar, cancel, crate::faces_stack::BRUTE_LIMIT)?;
        stack_new += kept.last_new;
        stack_compared += kept.last_compared;
        stacks.extend(found.iter().map(|s| s.iter().filter_map(|i| faces.get(*i).map(|f| (f.key(), f.stamp))).collect::<Vec<_>>()));
    }
    state.named_stacks.retain(|name, _| named.contains_key(name));
    let mut plain: Vec<&Face> = plain.into_iter().enumerate().filter(|(i, _)| !stacked.contains(i)).map(|(_, f)| f).collect();
    plain.truncate(MAX_ARRANGED);
    let chain_of = fingerprint(&plain.iter().map(|f| (f.key(), f.stamp)).collect::<Vec<_>>());
    let chain_reused = state.chain_of == chain_of && state.chain.len() == plain.len();
    if !chain_reused {
        let order = nearest_chain(&plain.iter().map(|f| &f.embedding[..]).collect::<Vec<_>>(), cancel)?;
        state.chain = order.into_iter().filter_map(|i| plain.get(i).map(|f| f.key())).collect();
        state.chain_of = chain_of;
    }
    Some(Output { suggestions, chain: state.chain.clone(), stacks, bar, stack_new, stack_compared, scored: state.scorer.last_scored, chain_reused })
}

/// A fingerprint of what a run is asked: the model and bars, the people with their faces, and the faces with their stamps.
/// Two runs with the same one give the same order.
pub(crate) fn signature(settings: Settings, people: &[Person], faces: &[Face]) -> u64 {
    let who = people.iter().fold(0u64, |sum, p| sum.wrapping_add(fingerprint(&(&p.key, p.fingerprint()))));
    let which: Vec<(Key, u64)> = faces.iter().map(|f| (f.key(), f.stamp)).collect();
    fingerprint(&(settings.model, settings.threshold.to_bits(), settings.margin.to_bits(), who, people.len(), which))
}

/// What the last finished run worked out, and for which [`signature`].
#[derive(Default)]
struct Last {
    sig: u64,
    suggestions: HashMap<Key, (u64, String, f32)>,
    chain: HashMap<Key, usize>,
    /// For each stacked face: where the last run put it.
    stack_of: HashMap<Key, StackSlot>,
    /// The faces of each stack, the most typical first: what `stack_of` points into.
    stacks: Vec<Vec<Key>>,
}

/// Where the last run put a stacked face.
struct StackSlot {
    /// Which stack it is in (an index into `Last::stacks`).
    stack: usize,
    /// The stamp the face was stacked at.
    stamp: u64,
    /// Its place in the stack (the most typical first).
    place: usize,
}

impl Last {
    /// What a finished run worked out: its suggestions, its look-alike order and its stacks (each as the faces in it with the stamp
    /// they were stacked at, the most typical first).
    fn new(sig: u64, suggestions: HashMap<Key, (u64, String, f32)>, chain: &[Key], stacks: &[Vec<(Key, u64)>]) -> Last {
        let chain = chain.iter().enumerate().map(|(i, k)| (*k, i)).collect();
        let stack_of = stacks
            .iter()
            .enumerate()
            .flat_map(|(stack, m)| m.iter().enumerate().map(move |(place, (k, stamp))| (*k, StackSlot { stack, stamp: *stamp, place })))
            .collect();
        let stacks = stacks.iter().map(|m| m.iter().map(|(k, _)| *k).collect()).collect();
        Last { sig, suggestions, chain, stack_of, stacks }
    }
}

/// A run on its thread: what it is for, how to stop it, and where its result comes.
struct Running {
    sig: u64,
    cancel: Arc<AtomicBool>,
    rx: std::sync::mpsc::Receiver<Done>,
}

impl Drop for Running {
    /// Dropped before it was taken in (another library, the session going away): the thread stops at its next chunk.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// What a run hands back: what it kept for the next run, and its result unless it was cancelled.
struct Done {
    state: State,
    output: Option<Output>,
}

/// The runs that put the Unnamed faces in order, one at a time, and what the last one worked out.
#[derive(Default)]
pub(crate) struct Jobs {
    /// What the next run starts from; away with the run while it runs.
    state: Option<State>,
    running: Option<Running>,
    last: Option<Last>,
    /// How many runs were taken in (a frontend compares it to know when to ask again).
    pub taken: u64,
    /// The signature of a run that was cancelled or failed: it is not started again until something changes.
    stopped: Option<u64>,
    /// What the last result's run cost, in person scores (a face against a person is one).
    pub last_scored: usize,
    /// The last result's run took the look-alike order of the run before it instead of working it out again.
    pub chain_reused: bool,
    /// What the last result's stacking cost: the faces it looked for look-alikes of, and the comparisons of two faces it made.
    pub stack_new: usize,
    pub stack_compared: usize,
    /// The bar the last result's stacks were made at.
    pub bar: f32,
    /// Tests only: a run waits here until this is false (or half a minute has passed, so a regression fails instead of hanging).
    #[cfg(any(test, feature = "test-hooks"))]
    pub hold: Option<Arc<AtomicBool>>,
    /// Tests only: how the next runs go wrong.
    #[cfg(test)]
    pub fail: Option<Fail>,
}

/// How a test makes a run go wrong.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fail {
    /// The run's row is cancelled as the run starts.
    Cancel,
    /// The thread panics.
    Panic,
    /// No thread can be started.
    Spawn,
}

impl Jobs {
    /// Another library was opened: whatever ran is stopped and forgotten.
    pub fn reset(&mut self) {
        *self = Jobs { taken: self.taken.wrapping_add(1), ..Jobs::default() };
    }

    /// Nothing is kept: no run, no result, no state, nothing stopped (what opening another library leaves).
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.running.is_none() && self.last.is_none() && self.state.is_none() && self.stopped.is_none() && self.last_scored == 0
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Whether the last result is for `sig`.
    pub fn is_current(&self, sig: u64) -> bool {
        self.last.as_ref().is_some_and(|l| l.sig == sig)
    }

    /// Whether the run for `sig` was cancelled or failed.
    pub fn was_stopped(&self, sig: u64) -> bool {
        self.stopped == Some(sig)
    }

    /// A caller that waits starts a run of its own, whatever became of an earlier one for the same inputs.
    pub fn forget_stop(&mut self, sig: u64) {
        if self.stopped == Some(sig) {
            self.stopped = None;
        }
    }

    /// Take in the run's result when it is there (`block`: wait for it). Whether a run ended.
    pub fn take_in(&mut self, block: bool) -> bool {
        let Some(running) = &self.running else { return false };
        let got = if block {
            running.rx.recv().map_err(|_| ())
        } else {
            match running.rx.try_recv() {
                Ok(done) => Ok(done),
                Err(std::sync::mpsc::TryRecvError::Empty) => return false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(()),
            }
        };
        let sig = running.sig;
        self.running = None;
        self.taken = self.taken.wrapping_add(1);
        match got {
            Ok(Done { state, output: Some(output) }) => {
                self.state = Some(state);
                self.stopped = None;
                (self.last_scored, self.chain_reused) = (output.scored, output.chain_reused);
                (self.stack_new, self.stack_compared, self.bar) = (output.stack_new, output.stack_compared, output.bar);
                self.last = Some(Last::new(sig, output.suggestions, &output.chain, &output.stacks));
            }
            Ok(Done { state, output: None }) => {
                self.state = Some(state);
                self.stopped = Some(sig);
            }
            // the thread ended without a result (it panicked): what it kept is lost
            Err(()) => {
                self.state = None;
                self.stopped = Some(sig);
            }
        }
        true
    }

    /// The faces of the stack `key` is in, as the last finished run found it (the most typical first); none when it is in no stack.
    pub fn stack_members(&self, key: &Key) -> Option<Vec<Key>> {
        let last = self.last.as_ref()?;
        let slot = last.stack_of.get(key)?;
        last.stacks.get(slot.stack).cloned()
    }

    /// Start a run on a thread of its own, with a row in the activity stack for as long as it runs. `all_threads`: use
    /// every thread rayon has (the caller waits), else one is left for the window. False when no thread could be started.
    ///
    /// When no thread can be started nothing is kept of the attempt (no row, what the last run kept is still there) and the run is
    /// marked as stopped for these inputs, so that it is not tried again on every ask; [`Jobs::forget_stop`] allows it.
    pub fn start(&mut self, activity: &crate::activity::Activity, input: Input, all_threads: bool) -> bool {
        let sig = signature(input.settings, &input.people, &input.faces);
        #[cfg(test)]
        if self.fail == Some(Fail::Spawn) {
            self.stopped = Some(sig);
            return false;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let task = activity.start("faceOrder", "Putting unnamed faces in order", crate::activity::Cancel::Flag(cancel.clone()));
        let (tx, rx) = std::sync::mpsc::channel();
        // what the last run kept goes with the thread through a slot, so that a thread that cannot be started gives it back
        let slot = Arc::new(std::sync::Mutex::new(self.state.take()));
        let thread_slot = slot.clone();
        let flag = cancel.clone();
        #[cfg(any(test, feature = "test-hooks"))]
        let hold = self.hold.clone();
        #[cfg(test)]
        let fail = self.fail;
        let job = move || {
            let mut state = thread_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take().unwrap_or_default();
            #[cfg(any(test, feature = "test-hooks"))]
            {
                let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
                while hold.as_ref().is_some_and(|h| h.load(Ordering::Relaxed)) && !flag.load(Ordering::Relaxed) && std::time::Instant::now() < until {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            #[cfg(test)]
            match fail {
                Some(Fail::Cancel) => flag.store(true, Ordering::Relaxed),
                Some(Fail::Panic) => panic!("a run that panics"),
                _ => {}
            }
            let progress = |done: usize, total: usize| task.progress(done as u64, total as u64);
            let threads = rayon::current_num_threads();
            let threads = if all_threads { threads } else { threads.saturating_sub(1).max(1) };
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("face-order-{i}")).build().ok();
            let output = match &pool {
                Some(pool) => pool.install(|| arrange(&mut state, &input, &flag, &progress)),
                None => arrange(&mut state, &input, &flag, &progress),
            };
            // the row goes with the run, before the result is announced
            drop(task);
            let _ = tx.send(Done { state, output });
        };
        match std::thread::Builder::new().name("face-order".into()).spawn(job) {
            Ok(_) => {
                self.running = Some(Running { sig, cancel, rx });
                true
            }
            Err(e) => {
                log::warn!("could not start the thread that puts the unnamed faces in order: {e}");
                // (the job was dropped with its row; what it was to start from is put back)
                self.state = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                self.stopped = Some(sig);
                false
            }
        }
    }

    /// The faces `listed` (photo, region and the stamp they were embedded and scored at, none when not embedded; in
    /// photo and region order) as they are put now: the positions in the order, which of them carry a suggestion, and
    /// whether any were put next to look-alikes.
    ///
    /// Faces the last run scored at the same stamp keep their suggestion, the likeliest first. Then the stacks the last
    /// run made, the biggest first, with the faces of each that are listed (a stack with one face listed is that face alone,
    /// after the look-alikes), then the others without a suggestion in look-alike order where the last run had them in it,
    /// then in photo order, then the faces not embedded.
    ///
    /// The names of the suggestions are copied for the first `limit` entries of the order only.
    pub fn arrange(&self, listed: &[Listed], limit: usize) -> Arrangement {
        let last = self.last.as_ref();
        // the listed faces of each stack: (place in the stack, place in `listed`)
        let mut in_stack: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        let mut slots: Vec<Slot> = Vec::with_capacity(listed.len());
        for (place, face) in listed.iter().enumerate() {
            let key = face.key();
            let Some(stamp) = face.stamp else {
                slots.push(Slot::new(Group::Unembedded, place));
                continue;
            };
            if let Some(s) = last.and_then(|l| l.stack_of.get(&key)).filter(|s| s.stamp == stamp) {
                in_stack.entry(s.stack).or_default().push((s.place, place));
            } else if let Some((_, _, score)) = last.and_then(|l| l.suggestions.get(&key)).filter(|(was, _, _)| *was == stamp) {
                slots.push(Slot { score: *score, ..Slot::new(Group::Suggested, place) });
            } else if let Some(pos) = last.and_then(|l| l.chain.get(&key)) {
                slots.push(Slot { chain: *pos, ..Slot::new(Group::LookAlike, place) });
            } else {
                slots.push(Slot::new(Group::Unscored, place));
            }
        }
        let mut stacks: HashMap<usize, Vec<usize>> = HashMap::new();
        // the score a face was suggested its name with, when the last run scored it at the stamp it has now
        let suggestion_at = |place: usize| -> Option<f32> {
            let face = listed.get(place)?;
            last?.suggestions.get(&face.key()).filter(|(was, _, _)| Some(was) == face.stamp.as_ref()).map(|(_, _, score)| *score)
        };
        for (_, mut faces) in in_stack {
            faces.sort_unstable();
            let places: Vec<usize> = faces.into_iter().map(|(_, place)| place).collect();
            match places.as_slice() {
                // a stack with one face in scope is that face alone: where it goes with its suggestion, else after the look-alikes
                [only] => match suggestion_at(*only) {
                    Some(score) => slots.push(Slot { score, ..Slot::new(Group::Suggested, *only) }),
                    None => slots.push(Slot { chain: ALONE, ..Slot::new(Group::LookAlike, *only) }),
                },
                // a stack with a suggestion (all its faces share it) sorts among the suggested faces by its representative's score
                [first, ..] => {
                    match suggestion_at(*first) {
                        Some(score) => slots.push(Slot { score, ..Slot::new(Group::Suggested, *first) }),
                        None => slots.push(Slot { size_order: usize::MAX - places.len(), ..Slot::new(Group::Stack, *first) }),
                    }
                    stacks.insert(*first, places);
                }
                [] => {}
            }
        }
        // `listed` is in photo and region order, so the place in it is that order
        slots.sort_by(Slot::order);
        let mut suggestions = HashMap::new();
        // (every face of a stack with a suggestion carries it, not only the one that shows it)
        let suggested = slots
            .iter()
            .take(limit)
            .filter(|s| s.group == Group::Suggested)
            .flat_map(|s| stacks.get(&s.place).cloned().unwrap_or_else(|| vec![s.place]));
        for face in suggested.filter_map(|place| listed.get(place)) {
            if let Some((_, name, score)) = last.and_then(|l| l.suggestions.get(&face.key())) {
                suggestions.insert((face.photo, face.index), (name.clone(), *score));
            }
        }
        Arrangement {
            suggested: slots.first().is_some_and(|s| s.group == Group::Suggested),
            look_alike: slots.iter().filter(|s| s.group == Group::LookAlike && s.chain != ALONE).count() >= 2,
            stacks: slots.iter().filter(|s| stacks.contains_key(&s.place)).count(),
            bar: last.map(|_| self.bar),
            order: slots
                .into_iter()
                .map(|s| match stacks.remove(&s.place) {
                    Some(places) => Item::Stack(places),
                    None => Item::Face(s.place),
                })
                .collect(),
            suggestions,
        }
    }
}

/// The groups the faces are put in, in the order of the groups (the first goes first).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    /// Faces and stacks with a suggestion, the likeliest first.
    Suggested,
    /// Stacks without a suggestion, the biggest first.
    Stack,
    /// Faces without one, next to their look-alikes where the last run had them in its look-alike order.
    LookAlike,
    /// Faces the last run did not score at their stamp yet, in photo order.
    Unscored,
    /// Faces that are not embedded.
    Unembedded,
}

/// The place of a [`Slot`] in the look-alike order of a face that is the only one of its stack in scope: after the others.
const ALONE: usize = usize::MAX;

/// Where one tile (a face, or a stack shown by its first face) goes in the order [`Jobs::arrange`] makes.
struct Slot {
    group: Group,
    /// Within [`Group::Stack`]: the bigger the stack the smaller this is.
    size_order: usize,
    /// Within [`Group::Suggested`]: the score of the suggestion, the highest first.
    score: f32,
    /// Within [`Group::LookAlike`]: the position in the look-alike order ([`ALONE`] for a face that has none).
    chain: usize,
    /// The place in the list that was asked about (the tile's first face): photo and region order, which settles the rest.
    place: usize,
}

impl Slot {
    fn new(group: Group, place: usize) -> Slot {
        Slot { group, size_order: 0, score: 0.0, chain: 0, place }
    }

    /// The order of the tiles: group, then within the group as each says, then by place.
    fn order(a: &Slot, b: &Slot) -> std::cmp::Ordering {
        a.group
            .cmp(&b.group)
            .then(a.size_order.cmp(&b.size_order))
            .then(b.score.total_cmp(&a.score))
            .then(a.chain.cmp(&b.chain))
            .then(a.place.cmp(&b.place))
    }
}

/// A face as [`Jobs::arrange`] is asked about it.
pub(crate) struct Listed {
    pub photo: u64,
    /// Its region.
    pub index: usize,
    /// Its own box, as bits.
    pub rect: [u64; 4],
    /// Which of the photo's faces with that very box it is.
    pub ord: u32,
    /// The stamp it was embedded and scored at (none when not embedded).
    pub stamp: Option<u64>,
}

impl Listed {
    fn key(&self) -> Key {
        (self.photo, self.rect, self.ord)
    }
}

/// One tile of the arrangement: a face, or a stack of faces (the most typical first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Face(usize),
    Stack(Vec<usize>),
}

/// How the faces were put by [`Jobs::arrange`].
pub(crate) struct Arrangement {
    /// What to show, in order: a face or a stack, as places in the list that was asked about.
    pub order: Vec<Item>,
    pub suggested: bool,
    pub look_alike: bool,
    /// How many stacks there are in `order`.
    pub stacks: usize,
    /// The bar the stacks were made at, when a run has finished.
    pub bar: Option<f32>,
    pub suggestions: HashMap<(u64, usize), (String, f32)>,
}

/// The faces nobody has scored against everybody, as a set: for the tests.
#[cfg(test)]
impl Scorer {
    fn kept(&self) -> std::collections::HashSet<Key> {
        self.entries.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator (xorshift64*), so a failing seed can be run again.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() >> 33) as usize % n.max(1)
        }

        fn unit(&mut self, dim: usize) -> Arc<[f32]> {
            let v: Vec<f32> = (0..dim).map(|_| (self.next() >> 40) as f32 / (1u64 << 24) as f32 - 0.5).collect();
            let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
            v.into_iter().map(|x| x / n).collect()
        }
    }

    const NAMES: [&str; 9] = ["Ann", "Bob", "Cy", "Di", "Eve", "Flo", "Gus", "Hal", "Ida"];

    struct Named {
        name: String,
        id: u64,
        embedding: Arc<[f32]>,
    }

    struct Loose {
        id: u64,
        rect: u64,
        embedding: Arc<[f32]>,
        rejected: Vec<String>,
    }

    #[derive(Default)]
    struct World {
        named: Vec<Named>,
        loose: Vec<Loose>,
        next: u64,
    }

    fn cased(rng: &mut Rng, name: &str) -> String {
        if rng.below(2) == 0 { name.to_uppercase() } else { name.to_lowercase() }
    }

    impl World {
        fn people(&self) -> Vec<Person> {
            let mut out: Vec<Person> = Vec::new();
            for n in &self.named {
                let key = n.name.to_lowercase();
                let at = fingerprint(&(&n.name, n.id));
                match out.iter_mut().find(|p| p.key == key) {
                    Some(p) => p.add(at, n.embedding.clone()),
                    None => {
                        let mut p = Person::new(key, n.name.clone());
                        p.add(at, n.embedding.clone());
                        out.push(p);
                    }
                }
            }
            out
        }

        fn faces(&self) -> Vec<Face> {
            self.loose.iter().map(|l| Face::new(l.id, [l.rect, 0, 0, 0], 0, l.embedding.clone(), &l.rejected)).collect()
        }

        /// What scoring every face against everybody from scratch gives, as the suggestion code did before there was a scorer.
        fn reference(&self, settings: Settings) -> Vec<Option<(String, f32)>> {
            let gallery: Vec<(&str, &[f32])> = self.named.iter().map(|n| (n.name.as_str(), &n.embedding[..])).collect();
            self.loose
                .iter()
                .map(|l| {
                    let keys: Vec<String> = l.rejected.iter().map(|r| r.trim().to_lowercase()).collect();
                    let kept: Vec<(&str, &[f32])> = gallery.iter().filter(|(n, _)| !keys.contains(&n.trim().to_lowercase())).copied().collect();
                    let x = matching::suggest(&matching::rank(&l.embedding, &kept), settings.threshold, settings.margin)?;
                    Some((x.name, x.score))
                })
                .collect()
        }

        /// One random change of the kinds the People view makes.
        fn change(&mut self, rng: &mut Rng, dim: usize) {
            self.next += 1;
            let id = self.next;
            match rng.below(9) {
                // a naming
                0 | 1 if !self.loose.is_empty() => {
                    let l = self.loose.remove(rng.below(self.loose.len()));
                    let base = NAMES[rng.below(NAMES.len())];
                    // the name of someone who is already named keeps their spelling, as the commands do not: any spelling
                    let name = cased(rng, base);
                    self.named.push(Named { name, id, embedding: l.embedding });
                }
                // an un-naming
                2 if !self.named.is_empty() => {
                    let n = self.named.remove(rng.below(self.named.len()));
                    self.loose.push(Loose { id, rect: 1, embedding: n.embedding, rejected: Vec::new() });
                }
                // a rename that only changes the capitalization of one person
                3 if !self.named.is_empty() => {
                    let from = self.named[rng.below(self.named.len())].name.to_lowercase();
                    let to = cased(rng, &from);
                    for n in self.named.iter_mut().filter(|n| n.name.to_lowercase() == from) {
                        n.name = to.clone();
                    }
                }
                // a merge of two people
                4 if !self.named.is_empty() => {
                    let from = self.named[rng.below(self.named.len())].name.to_lowercase();
                    let to = NAMES[rng.below(NAMES.len())];
                    for n in self.named.iter_mut().filter(|n| n.name.to_lowercase() == from) {
                        n.name = to.to_string();
                    }
                }
                // a rejected name
                5 | 6 if !self.loose.is_empty() => {
                    let i = rng.below(self.loose.len());
                    let base = NAMES[rng.below(NAMES.len())];
                    let name = cased(rng, base);
                    self.loose[i].rejected.push(format!(" {name}"));
                }
                // a box moved, or removed
                7 if !self.loose.is_empty() => {
                    let i = rng.below(self.loose.len());
                    self.loose[i].rect += 1;
                }
                8 if !self.loose.is_empty() => {
                    self.loose.remove(rng.below(self.loose.len()));
                }
                // a face the scan found
                _ => self.loose.push(Loose { id, rect: 0, embedding: rng.unit(dim), rejected: Vec::new() }),
            }
        }
    }

    fn settings() -> Settings {
        Settings { model: 1, threshold: 0.2, margin: 0.05 }
    }

    /// The faces in the order a list of them is shown, as `outcome` puts them (they are listed in photo and region order).
    fn shown_order(outcome: &Output, faces: &[Face]) -> Vec<usize> {
        let chain = outcome.chain.iter().enumerate().map(|(i, k)| (*k, i)).collect();
        let jobs = Jobs { last: Some(Last { sig: 0, suggestions: outcome.suggestions.clone(), chain, ..Last::default() }), ..Jobs::default() };
        let listed: Vec<Listed> =
            faces.iter().enumerate().map(|(i, f)| Listed { photo: f.photo, index: i, rect: f.rect, ord: f.ord, stamp: Some(f.stamp) }).collect();
        jobs.arrange(&listed, usize::MAX)
            .order
            .into_iter()
            .flat_map(|i| match i {
                Item::Face(i) => vec![i],
                Item::Stack(s) => s,
            })
            .collect()
    }

    /// What `state` (which has worked through the changes so far) gives for `w`, against what a fresh state gives for the same
    /// input and what scoring everyone against every face gives: the same suggestions, scores (to the last bit), look-alike
    /// order and face order. How many faces have a suggestion.
    fn same_as_from_scratch(state: &mut State, w: &World, settings: Settings, seed: u64, step: usize) -> usize {
        let input = Input { settings, people: w.people(), faces: w.faces() };
        let quiet = AtomicBool::new(false);
        let got = arrange(state, &input, &quiet, &|_, _| {}).expect("not cancelled");
        let want = arrange(&mut State::default(), &input, &quiet, &|_, _| {}).expect("not cancelled");
        assert_eq!(got.suggestions, want.suggestions, "seed {seed} step {step}");
        assert_eq!(got.chain, want.chain, "seed {seed} step {step}: the look-alike order");
        assert_eq!(shown_order(&got, &input.faces), shown_order(&want, &input.faces), "seed {seed} step {step}: the order of the faces");
        let by_face: Vec<Option<(String, f32)>> =
            input.faces.iter().map(|f| got.suggestions.get(&f.key()).map(|(_, name, score)| (name.clone(), *score))).collect();
        same(&by_face, &w.reference(settings), seed, step);
        assert_eq!(state.scorer.kept().len(), w.loose.len(), "what is kept is what was asked about");
        got.suggestions.len()
    }

    fn run(scorer: &mut Scorer, w: &World) -> Vec<Option<(String, f32)>> {
        scorer.update(settings(), &w.people(), &w.faces(), &AtomicBool::new(false), &|_, _| {}).expect("not cancelled")
    }

    fn same(a: &[Option<(String, f32)>], b: &[Option<(String, f32)>], seed: u64, step: usize) {
        assert_eq!(a.len(), b.len(), "seed {seed} step {step}");
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            match (x, y) {
                (None, None) => {}
                (Some((n, s)), Some((m, t))) => assert!(n == m && (s - t).abs() < 1e-6, "seed {seed} step {step} face {i}: {x:?} vs {y:?}"),
                _ => panic!("seed {seed} step {step} face {i}: {x:?} vs {y:?}"),
            }
        }
    }

    #[test]
    fn a_change_gives_what_scoring_everyone_again_gives_for_every_face() {
        let dim = 5;
        for seed in 1..=300u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut w = World::default();
            for _ in 0..14 {
                w.next += 1;
                let id = w.next;
                w.named.push(Named { name: NAMES[rng.below(NAMES.len())].to_string(), id, embedding: rng.unit(dim) });
            }
            for _ in 0..10 {
                w.next += 1;
                let id = w.next;
                w.loose.push(Loose { id, rect: 0, embedding: rng.unit(dim), rejected: Vec::new() });
            }
            let mut state = State::default();
            let mut now = settings();
            let mut suggested = same_as_from_scratch(&mut state, &w, now, seed, 0);
            for step in 1..=30 {
                match rng.below(12) {
                    // a changed model scores everything again
                    0 => now.model = if now.model == 1 { 2 } else { 1 },
                    // a changed bar only changes what is suggested from the scores
                    1 => now.threshold = [0.1, 0.2, 0.35][rng.below(3)],
                    _ => w.change(&mut rng, dim),
                }
                suggested += same_as_from_scratch(&mut state, &w, now, seed, step);
            }
            assert!(suggested > 0, "seed {seed}: no face was ever suggested a name, so nothing was compared");
        }
    }

    #[test]
    fn a_change_scores_only_the_people_whose_faces_changed() {
        let mut rng = Rng(7);
        let mut w = World::default();
        for i in 0..40usize {
            w.next += 1;
            let id = w.next;
            w.named.push(Named { name: NAMES[i % NAMES.len()].to_string(), id, embedding: rng.unit(16) });
        }
        for _ in 0..50 {
            w.next += 1;
            let id = w.next;
            w.loose.push(Loose { id, rect: 0, embedding: rng.unit(16), rejected: Vec::new() });
        }
        let mut scorer = Scorer::default();
        run(&mut scorer, &w);
        assert_eq!(scorer.last_scored, 50 * NAMES.len(), "the first time every face is scored against every person");
        run(&mut scorer, &w);
        assert_eq!(scorer.last_scored, 0, "nothing changed, nothing is scored");
        // a naming: one person's faces changed
        let l = w.loose.remove(0);
        w.named.push(Named { name: "Ann".into(), id: 999, embedding: l.embedding });
        same(&run(&mut scorer, &w), &w.reference(settings()), 7, 1);
        assert_eq!(scorer.last_scored, 49, "each face that is left against Ann alone");
        // a rename that only changes the capitalization
        for n in w.named.iter_mut().filter(|n| n.name.to_lowercase() == "bob") {
            n.name = "BOB".into();
        }
        same(&run(&mut scorer, &w), &w.reference(settings()), 7, 2);
        assert_eq!(scorer.last_scored, 49);
        // a face whose box moved is scored against everybody, the others against nobody
        w.loose[3].rect += 1;
        same(&run(&mut scorer, &w), &w.reference(settings()), 7, 3);
        assert_eq!(scorer.last_scored, NAMES.len());
        // a rejection too
        w.loose[4].rejected.push("cy".into());
        same(&run(&mut scorer, &w), &w.reference(settings()), 7, 4);
        assert_eq!(scorer.last_scored, NAMES.len());
        // a merge of two people changes both
        for n in w.named.iter_mut().filter(|n| n.name.to_lowercase() == "di") {
            n.name = "Eve".into();
        }
        same(&run(&mut scorer, &w), &w.reference(settings()), 7, 5);
        assert_eq!(scorer.last_scored, 49 * 2);
    }

    #[test]
    fn a_changed_model_scores_everything_again_and_a_cancel_leaves_the_scorer_as_it_was() {
        let mut rng = Rng(11);
        let mut w = World::default();
        for i in 0..12usize {
            w.next += 1;
            let id = w.next;
            w.named.push(Named { name: NAMES[i % 5].to_string(), id, embedding: rng.unit(8) });
            w.next += 1;
            let id = w.next;
            w.loose.push(Loose { id, rect: 0, embedding: rng.unit(8), rejected: Vec::new() });
        }
        let mut scorer = Scorer::default();
        run(&mut scorer, &w);
        let cancelled = AtomicBool::new(true);
        assert!(scorer.update(settings(), &w.people(), &w.faces(), &cancelled, &|_, _| {}).is_none());
        assert_eq!(scorer.entries.len(), 12, "a cancelled run keeps what was worked out");
        let other = Settings { model: 2, ..settings() };
        let again = scorer.update(other, &w.people(), &w.faces(), &AtomicBool::new(false), &|_, _| {}).expect("not cancelled");
        same(&again, &w.reference(other), 11, 0);
        assert_eq!(scorer.last_scored, 12 * 5, "another model: every face against every person again");
        // another bar scores nobody again (it only decides which kept score is suggested), and still gives what scoring
        // everyone again gives
        let strict = Settings { threshold: 0.9, ..other };
        let read_off = scorer.update(strict, &w.people(), &w.faces(), &AtomicBool::new(false), &|_, _| {}).expect("not cancelled");
        same(&read_off, &w.reference(strict), 11, 1);
        assert_eq!(scorer.last_scored, 0);
    }

    #[test]
    fn two_faces_with_one_box_are_each_scored_for_themselves() {
        let mut ann = vec![0.0f32; 8];
        ann[0] = 1.0;
        let mut bob = vec![0.0f32; 8];
        bob[1] = 1.0;
        let (ann, bob): (Arc<[f32]>, Arc<[f32]>) = (ann.into(), bob.into());
        let mut a = Person::new("ann".into(), "Ann".into());
        a.add(1, ann.clone());
        let mut b = Person::new("bob".into(), "Bob".into());
        b.add(2, bob);
        let people = vec![a, b];
        // one photo, one box, two regions: the second has rejected Ann
        let faces = vec![Face::new(9, [1, 2, 3, 4], 0, ann.clone(), &[]), Face::new(9, [1, 2, 3, 4], 1, ann, &["Ann".to_string()])];
        assert_ne!(faces[0].key(), faces[1].key());
        let input = Input { settings: settings(), people, faces };
        let mut state = State::default();
        let quiet = AtomicBool::new(false);
        let out = arrange(&mut state, &input, &quiet, &|_, _| {}).expect("not cancelled");
        assert_eq!(out.suggestions.get(&input.faces[0].key()).map(|s| s.1.as_str()), Some("Ann"));
        assert!(!out.suggestions.contains_key(&input.faces[1].key()), "the one that rejected Ann is not suggested her");
        let jobs = Jobs { last: Some(Last { sig: 0, suggestions: out.suggestions.clone(), ..Last::default() }), ..Jobs::default() };
        let listed: Vec<Listed> = input
            .faces
            .iter()
            .enumerate()
            .map(|(i, f)| Listed { photo: f.photo, index: i, rect: f.rect, ord: f.ord, stamp: Some(f.stamp) })
            .collect();
        let a = jobs.arrange(&listed, usize::MAX);
        assert_eq!(a.order, vec![Item::Face(0), Item::Face(1)]);
        assert_eq!(a.suggestions.get(&(9, 0)).map(|s| s.0.as_str()), Some("Ann"));
        assert!(!a.suggestions.contains_key(&(9, 1)));
        // and neither is scored again on the next run
        arrange(&mut state, &input, &quiet, &|_, _| {}).expect("not cancelled");
        assert_eq!(state.scorer.last_scored, 0);
    }

    #[test]
    fn many_people_tied_do_not_make_what_is_kept_grow() {
        let mut rng = Rng(3);
        let same = rng.unit(8);
        let mut w = World::default();
        for i in 0..40 {
            w.next += 1;
            let id = w.next;
            w.named.push(Named { name: format!("p{i}"), id, embedding: same.clone() });
        }
        for _ in 0..5 {
            w.next += 1;
            let id = w.next;
            w.loose.push(Loose { id, rect: 0, embedding: same.clone(), rejected: Vec::new() });
        }
        w.next += 1;
        let id = w.next;
        w.loose.push(Loose { id, rect: 0, embedding: rng.unit(8), rejected: Vec::new() });
        let mut scorer = Scorer::default();
        same_as(&run(&mut scorer, &w), &w);
        assert!(scorer.entries.values().all(|e| e.top.len() <= TIE_CAP), "at most {TIE_CAP} people are kept for a face");
        // the faces whose entry was cut are scored again, in full, and the answer is the same
        same_as(&run(&mut scorer, &w), &w);
        assert!(scorer.last_scored >= 5 * 40, "{}", scorer.last_scored);
    }

    fn same_as(got: &[Option<(String, f32)>], w: &World) {
        same(got, &w.reference(settings()), 3, 0);
    }

    /// What a take-out of 5,000 faces asks of the dispatch thread: the stack of each, looked up among 100,000 stacked faces.
    #[test]
    fn the_members_of_a_stack_are_looked_up_not_searched_for_among_every_stacked_face() {
        let key = |n: usize| -> Key { (n as u64, [n as u64; 4], 0) };
        let stacks: Vec<Vec<(Key, u64)>> = (0..20_000).map(|s| (0..5).map(|m| (key(s * 5 + m), 1)).collect()).collect();
        let jobs = Jobs { last: Some(Last::new(1, HashMap::new(), &[], &stacks)), ..Default::default() };
        let started = std::time::Instant::now();
        for n in 0..5000 {
            let members = jobs.stack_members(&key(n * 8)).expect("in a stack");
            assert_eq!(members.len(), 5);
            // (the most typical first, as the run listed them)
            assert_eq!(members[0], key(n * 8 / 5 * 5));
        }
        assert!(jobs.stack_members(&key(1_000_000)).is_none(), "a face in no stack");
        let took = started.elapsed();
        assert!(took < std::time::Duration::from_millis(250), "5,000 faces took {took:?}");
    }

    #[test]
    fn the_chain_puts_look_alikes_together_and_stops_when_cancelled() {
        let a: Arc<[f32]> = Arc::from(vec![1.0, 0.0]);
        let b: Arc<[f32]> = Arc::from(vec![0.0, 1.0]);
        let a2: Arc<[f32]> = Arc::from(vec![0.99, 0.1]);
        let order = nearest_chain(&[&a[..], &b[..], &a2[..]], &AtomicBool::new(false)).expect("not cancelled");
        assert_eq!(order, vec![0, 2, 1]);
        assert!(nearest_chain(&[&a[..], &b[..], &a2[..]], &AtomicBool::new(true)).is_none());
        assert_eq!(nearest_chain(&[], &AtomicBool::new(false)), Some(Vec::new()));
    }
}
