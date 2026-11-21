//! Stacks of the Unnamed faces: faces without a suggestion that are virtually certain to be the same person, shown as one
//! tile. Computed, never stored: a stack is the result of stacking the unnamed faces as they are now, and it becomes a
//! person only when it is named (the name goes onto each face box, as for any face).
//!
//! Nothing here touches the session; [`crate::faces_arrange::arrange`] calls it on the thread of the run that puts the
//! Unnamed faces in order, so it is cancellable and reports in that run's activity row.
//!
//! **The bar** ([`Calibration`]) is what two faces must reach (cosine of their embeddings, the measure the suggestions use) to
//! be alike enough to stack. It comes from the library's own named faces: above the highest similarity found between two faces
//! named as different people, plus [`BAR_MARGIN`], and never below [`MIN_BAR`].
//!
//! **No chaining.** A face joins a stack only when it reaches the bar against *every* face in it, so A like B and B like C
//! with A not like C never end up together. Wherever the search for look-alikes misses a pair, the faces are left apart: the
//! mistakes this makes are stacks that are smaller than they could be, never a face that does not belong.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use lightcraft_faces::matching;
use rayon::prelude::*;

use crate::faces_arrange::{Face, Key, Person};

/// The lowest the bar ever is. The suggestion bars of the known models are 0.40 and 0.55 (cosine of a face against a person's
/// two closest faces); two faces of one person photographed in the same hour rarely reach 0.75 in the weaker model, so it is
/// clearly stricter than any suggestion, as stacking names faces nobody looked at one by one.
pub(crate) const MIN_BAR: f32 = 0.75;

/// What is added to the highest similarity between two faces named as different people, so that faces just under what
/// two different people reached never stack.
pub(crate) const BAR_MARGIN: f32 = 0.05;

/// Faces looked at between two looks at the cancel flag.
const CHUNK: usize = 256;

/// Most faces a stack is looked for among by comparing every pair (the number of comparisons); above it the faces are put in
/// cells first and each is compared with the faces of its two nearest cells only.
pub(crate) const BRUTE_LIMIT: usize = 4_000_000;

/// How many faces each face keeps as its look-alikes (a person in thousands of photos is one stack; this bounds what a
/// hostile library of identical embeddings costs).
const MAX_ALIKE: usize = 4096;

/// Most faces a stack holds: naming a stack is one `faces.nameFaces` command (one undo step), which takes at most 5,000 faces.
pub(crate) const MAX_STACK: usize = 4500;

/// What a person's faces reach against a face of another person: the highest cosine, `None` when no faces can be compared.
fn cross(a: &Person, b: &Person) -> Option<f32> {
    let mut best: Option<f32> = None;
    for x in &a.faces {
        for y in &b.faces {
            if let Some(c) = matching::cosine(x, y) {
                best = Some(best.map_or(c, |b| b.max(c)));
            }
        }
    }
    best
}

/// What is kept of one person: their fingerprint when the row was worked out, and the highest similarity of any of their
/// faces to a face of another person, with that person.
struct Row {
    fingerprint: u64,
    best: Option<f32>,
    partner: Option<String>,
}

/// The highest similarity between two faces named as different people, kept so that a naming costs what it changed: only the
/// rows of the people whose faces changed (and of the people whose best partner they were) are worked out again, against
/// everybody; everybody else only against the changed people.
#[derive(Default)]
pub(crate) struct Calibration {
    rows: HashMap<String, Row>,
    /// How many pairs of people the last [`Calibration::bar`] compared.
    pub last_pairs: usize,
}

impl Calibration {
    /// The bar for `people`: [`MIN_BAR`] or the highest cross-person similarity plus [`BAR_MARGIN`], whichever is higher. `None`
    /// when `cancel` was set (the rows are then as they were).
    pub fn bar(&mut self, people: &[Person], cancel: &AtomicBool) -> Option<f32> {
        let prints: Vec<u64> = people.iter().map(Person::fingerprint).collect();
        let index: HashMap<&str, usize> = people.iter().enumerate().map(|(i, p)| (p.key.as_str(), i)).collect();
        let changed = |i: usize, p: &Person| self.rows.get(&p.key).map(|r| r.fingerprint) != prints.get(i).copied();
        let changed_keys: HashSet<&str> = people.iter().enumerate().filter(|(i, p)| changed(*i, p)).map(|(_, p)| p.key.as_str()).collect();
        // a row is worked out again when its person changed, or the person it was best against did (or went)
        let dirty: Vec<usize> = people
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                changed(*i, p)
                    || self.rows.get(&p.key).and_then(|r| r.partner.as_deref()).is_some_and(|q| changed_keys.contains(q) || !index.contains_key(q))
            })
            .map(|(i, _)| i)
            .collect();
        let is_dirty: HashSet<usize> = dirty.iter().copied().collect();
        let old_best = |j: usize| people.get(j).and_then(|p| self.rows.get(&p.key)).filter(|_| !is_dirty.contains(&j)).and_then(|r| r.best);
        let pairs = AtomicUsize::new(0);
        // each dirty person against every other, once per pair: (their own best, and what a clean person is brought to)
        let parts: Vec<Option<(usize, Option<(f32, usize)>, Vec<(usize, f32)>)>> = dirty
            .par_iter()
            .map(|&i| {
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
                let me = people.get(i)?;
                let mut best: Option<(f32, usize)> = None;
                let mut lifts: Vec<(usize, f32)> = Vec::new();
                for (j, other) in people.iter().enumerate() {
                    if j == i || (is_dirty.contains(&j) && j < i) {
                        continue;
                    }
                    pairs.fetch_add(1, Ordering::Relaxed);
                    let Some(c) = cross(me, other) else { continue };
                    if best.is_none_or(|(b, _)| c > b) {
                        best = Some((c, j));
                    }
                    if old_best(j).is_none_or(|b| c > b) {
                        lifts.push((j, c));
                    }
                }
                Some((i, best, lifts))
            })
            .collect();
        if parts.iter().any(Option::is_none) || cancel.load(Ordering::Relaxed) {
            return None;
        }
        self.last_pairs = pairs.load(Ordering::Relaxed);
        let mut rows: HashMap<String, Row> = HashMap::with_capacity(people.len());
        for (i, p) in people.iter().enumerate() {
            let kept = self.rows.get(&p.key).filter(|_| !is_dirty.contains(&i));
            let row = Row { fingerprint: prints.get(i).copied().unwrap_or(0), best: kept.and_then(|r| r.best), partner: kept.and_then(|r| r.partner.clone()) };
            rows.insert(p.key.clone(), row);
        }
        for (i, best, lifts) in parts.into_iter().flatten() {
            if let (Some(p), Some((c, j))) = (people.get(i), best) {
                let partner = people.get(j).map(|q| q.key.clone());
                if let Some(row) = rows.get_mut(&p.key)
                    && row.best.is_none_or(|b| c > b)
                {
                    (row.best, row.partner) = (Some(c), partner);
                }
            }
            for (j, c) in lifts {
                if let Some(q) = people.get(j)
                    && let Some(row) = rows.get_mut(&q.key)
                    && row.best.is_none_or(|b| c > b)
                {
                    (row.best, row.partner) = (Some(c), people.get(i).map(|p| p.key.clone()));
                }
            }
        }
        self.rows = rows;
        Some(bar_from(self.rows.values().filter_map(|r| r.best).fold(None, |m: Option<f32>, c| Some(m.map_or(c, |m| m.max(c))))))
    }
}

/// The bar a highest cross-person similarity gives.
pub(crate) fn bar_from(highest: Option<f32>) -> f32 {
    highest.filter(|h| h.is_finite()).map_or(MIN_BAR, |h| MIN_BAR.max(h + BAR_MARGIN))
}

/// What a run keeps of the stacks for the next one.
#[derive(Default)]
pub(crate) struct StackState {
    bar: u32,
    /// The stacks, each as (face, stamp) of its faces.
    stacks: Vec<Vec<(Key, u64)>>,
    /// The faces that were looked at against every other face without a stack (and found no stack).
    seen: HashSet<(Key, u64)>,
    /// How many comparisons of two faces the last [`stack`] made.
    pub last_compared: usize,
    /// How many faces the last [`stack`] had to look for look-alikes for (the others were kept as they were).
    pub last_new: usize,
}

/// The work of one [`stack`] call.
struct Work<'a> {
    faces: &'a [&'a Face],
    bar: f32,
    cancel: &'a AtomicBool,
    compared: AtomicUsize,
}

impl Work<'_> {
    fn sim(&self, a: usize, b: usize) -> f32 {
        self.compared.fetch_add(1, Ordering::Relaxed);
        match (self.faces.get(a), self.faces.get(b)) {
            (Some(x), Some(y)) => matching::cosine(&x.embedding, &y.embedding).unwrap_or(-1.0),
            _ => -1.0,
        }
    }

    /// Whether `c` reaches the bar against every one of `members`.
    fn fits(&self, members: &[usize], c: usize) -> bool {
        members.iter().all(|m| self.sim(*m, c) >= self.bar)
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The mean of the faces' embeddings, as a unit vector (`None` when they are not of one length).
    fn centroid(&self, members: &[usize]) -> Option<Vec<f32>> {
        let dim = self.faces.get(*members.first()?)?.embedding.len();
        let mut sum = vec![0f32; dim];
        for m in members {
            let e = &self.faces.get(*m)?.embedding;
            if e.len() != dim {
                return None;
            }
            for (s, x) in sum.iter_mut().zip(e.iter()) {
                *s += x;
            }
        }
        let norm = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
        (norm.is_finite() && norm > 1e-9).then(|| sum.into_iter().map(|x| x / norm).collect())
    }

    /// The stack among `stacks` (with their centroids) that `f` fits, the one whose centre it is closest to. A face that
    /// reaches the bar against every face of a stack reaches it against the stack's centre too (the centre is their mean), so the
    /// centre tells which stacks are worth comparing face by face.
    fn home(&self, f: usize, stacks: &[Vec<usize>], centres: &[Option<Vec<f32>>]) -> Option<usize> {
        let e = &self.faces.get(f)?.embedding;
        let mut best: Option<(usize, f32)> = None;
        for (s, c) in centres.iter().enumerate() {
            let Some(c) = c else { continue };
            self.compared.fetch_add(1, Ordering::Relaxed);
            let Some(near) = matching::cosine(e, c).filter(|n| *n >= self.bar) else { continue };
            if best.is_none_or(|(_, b)| near > b) && stacks.get(s).is_some_and(|m| self.fits(m, f)) {
                best = Some((s, near));
            }
        }
        best.map(|(s, _)| s)
    }
}

/// Which faces look like which: for every one of `from`, the faces of `among` it reaches the bar against, the most alike first.
/// By comparing every pair when that is a few million comparisons, else through cells (each face against the faces of its
/// two nearest centres). `None` when cancelled.
fn alikes(w: &Work, from: &[usize], among: &[usize], brute_limit: usize) -> Option<Vec<Vec<usize>>> {
    let cells = (from.len().saturating_mul(among.len()) > brute_limit).then(|| Cells::new(w, among)).flatten();
    let mut out: Vec<Vec<usize>> = Vec::with_capacity(from.len());
    for chunk in from.chunks(CHUNK) {
        if w.cancelled() {
            return None;
        }
        let part: Vec<Vec<usize>> = chunk
            .par_iter()
            .map(|&f| {
                let near: Vec<usize> = match &cells {
                    Some(c) => c.around(w, f),
                    None => among.to_vec(),
                };
                let mut scored: Vec<(f32, usize)> = near.into_iter().filter(|g| *g != f).map(|g| (w.sim(f, g), g)).filter(|(c, _)| *c >= w.bar).collect();
                scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
                scored.truncate(MAX_ALIKE);
                scored.into_iter().map(|(_, g)| g).collect()
            })
            .collect();
        out.extend(part);
    }
    Some(out)
}

/// The faces `among` put in cells around centres chosen from among them, each face in the cells of its two nearest centres.
struct Cells {
    centres: Vec<usize>,
    members: Vec<Vec<usize>>,
}

impl Cells {
    fn new(w: &Work, among: &[usize]) -> Option<Cells> {
        let k = (among.len() as f64).sqrt().ceil() as usize;
        let centres: Vec<usize> = (0..k.max(1)).filter_map(|i| among.get(i * among.len() / k.max(1)).copied()).collect();
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); centres.len()];
        for chunk in among.chunks(CHUNK) {
            if w.cancelled() {
                return None;
            }
            let near: Vec<(usize, [usize; 2])> = chunk.par_iter().map(|&f| (f, Self::nearest(w, &centres, f))).collect();
            for (f, two) in near {
                for c in two.into_iter().take(2) {
                    if let Some(m) = members.get_mut(c) {
                        m.push(f);
                    }
                }
            }
        }
        Some(Cells { centres, members })
    }

    /// The indexes of the two centres nearest to `f` (the same one twice when there is one centre).
    fn nearest(w: &Work, centres: &[usize], f: usize) -> [usize; 2] {
        let mut best: [(usize, f32); 2] = [(0, f32::NEG_INFINITY); 2];
        for (c, centre) in centres.iter().enumerate() {
            let s = w.sim(f, *centre);
            if s > best[0].1 {
                best = [(c, s), best[0]];
            } else if s > best[1].1 {
                best[1] = (c, s);
            }
        }
        [best[0].0, best[1].0]
    }

    /// The faces in the two cells `f` is nearest to.
    fn around(&self, w: &Work, f: usize) -> Vec<usize> {
        let two = Self::nearest(w, &self.centres, f);
        let mut out: Vec<usize> = Vec::new();
        for c in two {
            if let Some(m) = self.members.get(c) {
                out.extend(m);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Stacks of `faces` (the faces without a suggestion, in photo and region order) that are within `bar` of each other, every
/// face of a stack with every other. Each stack has at least two faces, its most typical face first; the biggest stacks
/// come first. `None` when `cancel` was set (`state` is then as it was).
///
/// What the last call found is reused: a stack whose faces are all still there stays as it is (a face that left dissolves its
/// stack, and the faces are looked at again), a new face joins the stack it fits or looks for look-alikes among the faces
/// without a stack, and the faces looked at before are not compared with each other again. A changed `bar` starts afresh.
/// `brute_limit`: see [`alikes`].
pub(crate) fn stack(state: &mut StackState, faces: &[&Face], bar: f32, cancel: &AtomicBool, brute_limit: usize) -> Option<Vec<Vec<usize>>> {
    let w = Work { faces, bar, cancel, compared: AtomicUsize::new(0) };
    let cold = state.bar != bar.to_bits();
    let at: HashMap<(Key, u64), usize> = faces.iter().enumerate().map(|(i, f)| ((f.key(), f.stamp), i)).collect();
    let mut stacks: Vec<Vec<usize>> = Vec::new();
    if !cold {
        for old in &state.stacks {
            let members: Vec<usize> = old.iter().filter_map(|m| at.get(m).copied()).collect();
            if members.len() == old.len() {
                stacks.push(members);
            }
        }
    }
    let mut stacked: HashSet<usize> = stacks.iter().flatten().copied().collect();
    let free: Vec<usize> = (0..faces.len()).filter(|i| !stacked.contains(i)).collect();
    let fresh: Vec<usize> = free.iter().copied().filter(|i| cold || faces.get(*i).is_none_or(|f| !state.seen.contains(&(f.key(), f.stamp)))).collect();
    let new_count = fresh.len();
    // new faces join the stacks they fit
    let centres: Vec<Option<Vec<f32>>> = stacks.iter().map(|m| w.centroid(m)).collect();
    let mut homes: Vec<Option<usize>> = Vec::with_capacity(fresh.len());
    for chunk in fresh.chunks(CHUNK) {
        if w.cancelled() {
            return None;
        }
        let part: Vec<Option<usize>> = chunk.par_iter().map(|&f| w.home(f, &stacks, &centres)).collect();
        homes.extend(part);
    }
    let mut left: Vec<usize> = Vec::new();
    for (&f, home) in fresh.iter().zip(homes) {
        // (two new faces that fit the stack may not fit each other)
        match home.and_then(|s| stacks.get_mut(s)) {
            Some(members) if members.len() < MAX_STACK && w.fits(members, f) => {
                members.push(f);
                stacked.insert(f);
            }
            _ => left.push(f),
        }
    }
    // the others look for look-alikes among all the faces that have no stack
    let pool: Vec<usize> = free.iter().copied().filter(|i| !stacked.contains(i)).collect();
    let alike = alikes(&w, &left, &pool, brute_limit)?;
    for (f, near) in left.iter().zip(alike) {
        if stacked.contains(f) {
            continue;
        }
        let mut members = vec![*f];
        for g in near.into_iter().filter(|g| !stacked.contains(g)) {
            if members.len() < MAX_STACK && w.fits(&members, g) {
                members.push(g);
            }
        }
        if members.len() >= 2 {
            stacked.extend(members.iter().copied());
            stacks.push(members);
        }
    }
    if w.cancelled() {
        return None;
    }
    for members in &mut stacks {
        order_typical_first(&w, members);
    }
    stacks.sort_by(|a, b| b.len().cmp(&a.len()).then(a.first().cmp(&b.first())));
    state.bar = bar.to_bits();
    state.stacks = stacks.iter().map(|m| m.iter().filter_map(|i| faces.get(*i).map(|f| (f.key(), f.stamp))).collect()).collect();
    state.seen = (0..faces.len()).filter(|i| !stacked.contains(i)).filter_map(|i| faces.get(i).map(|f| (f.key(), f.stamp))).collect();
    state.last_compared = w.compared.load(Ordering::Relaxed);
    state.last_new = new_count;
    Some(stacks)
}

/// `members` put with the face closest to the stack's centre first (ties: the lower index).
fn order_typical_first(w: &Work, members: &mut [usize]) {
    let centre = w.centroid(members);
    let closeness = |m: &usize| -> f32 {
        match (centre.as_deref(), w.faces.get(*m)) {
            (Some(c), Some(f)) => matching::cosine(&f.embedding, c).unwrap_or(-1.0),
            _ => -1.0,
        }
    };
    members.sort_by(|a, b| closeness(b).total_cmp(&closeness(a)).then(a.cmp(b)));
}

#[cfg(test)]
#[path = "faces_stack_tests.rs"]
mod tests;
