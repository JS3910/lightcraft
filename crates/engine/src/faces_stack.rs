//! Stacks of the Unnamed faces: faces without a suggestion that are virtually certain to be the same person, shown as one
//! tile. Computed, never stored: a stack is the result of stacking the unnamed faces as they are now, and it becomes a
//! person only when it is named (the name goes onto each face box, as for any face).
//!
//! Nothing here touches the session; [`crate::faces_arrange::arrange`] calls it on the thread of the run that puts the
//! Unnamed faces in order, so it is cancellable and reports in that run's activity row.
//!
//! **The bar** ([`Calibration`]) is what two faces must reach (cosine of their embeddings, the measure the suggestions use) to
//! be alike enough to stack. It comes from the library's own named faces: the 99.9th percentile ([`BAR_PERCENTILE`]) of the
//! similarities between faces named as different people, plus [`BAR_MARGIN`], kept between [`MIN_BAR`] and [`MAX_BAR`] (and
//! [`MIN_BAR`] while there are fewer than [`MIN_PAIRS`] such pairs).
//!
//! **No chaining.** A face joins a stack only when it reaches the bar against *every* face in it, so A like B and B like C
//! with A not like C never end up together. Wherever the search for look-alikes misses a pair, the faces are left apart: the
//! mistakes this makes are stacks that are smaller than they could be, never a face that does not belong.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use lightcraft_faces::matching;
use rayon::prelude::*;

use crate::faces_arrange::{Face, Key, Person};

/// The lowest the bar ever is. The suggestion bars of the known models are 0.40 and 0.55 (cosine of a face against a person's
/// two closest faces); two faces of one person photographed in the same hour rarely reach 0.75 in the weaker model, so it is
/// clearly stricter than any suggestion, as stacking names faces nobody looked at one by one.
pub(crate) const MIN_BAR: f32 = 0.75;

/// The highest the bar ever is: one lookalike or mislabelled pair in a large library must not make stacking impossible.
pub(crate) const MAX_BAR: f32 = 0.85;

/// What is added to the percentile of the similarities between faces named as different people, so that faces just under what
/// two different people reach never stack.
pub(crate) const BAR_MARGIN: f32 = 0.05;

/// Which share of the similarities between faces of different people the bar is above (the 99.9th percentile): a handful of
/// lookalike or mislabelled pairs in a large library do not move it.
pub(crate) const BAR_PERCENTILE: f64 = 0.999;

/// Fewest pairs of faces of different people a percentile is taken of; with fewer the bar is [`MIN_BAR`].
pub(crate) const MIN_PAIRS: u64 = 1000;

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

/// How many bins the similarities of different people are counted in, from -1 to 1 (a bin is 0.001 wide).
const BINS: usize = 2000;

/// Which of the [`BINS`] the cosine `c` is counted in (`None` when it is not a number).
fn bin_of(c: f32) -> Option<usize> {
    c.is_finite().then(|| (((c.clamp(-1.0, 1.0) + 1.0) / 2.0 * BINS as f32) as usize).min(BINS - 1))
}

/// The cosine at the top of bin `i`.
fn bin_top(i: usize) -> f32 {
    (-1.0 + 2.0 * (i + 1) as f32 / BINS as f32).min(1.0)
}

/// How many pairs of faces of two people fall in each bin, added to `into` (`sign` is +1 to count them, -1 to take them away
/// again).
fn count_between(into: &mut [i64], sign: i64, a: &[Arc<[f32]>], b: &[Arc<[f32]>]) {
    for x in a {
        for y in b {
            if let Some(i) = matching::cosine(x, y).and_then(bin_of)
                && let Some(n) = into.get_mut(i)
            {
                *n += sign;
            }
        }
    }
}

/// What is kept of one person: their fingerprint when they were counted, and their faces then.
struct Counted {
    fingerprint: u64,
    faces: Vec<Arc<[f32]>>,
}

/// How alike the faces of different people are, counted for the stacking bar: the similarity of every face of a person to every
/// face of every other person, in a histogram of [`BINS`] bins. A naming costs what it changed: only the pairs that
/// involve a person whose faces changed are taken away (as they were) and counted again (as they are).
#[derive(Default)]
pub(crate) struct Calibration {
    people: HashMap<String, Counted>,
    bins: Vec<u64>,
}

impl Calibration {
    /// The bar for `people` (see [`bar_from`]). `None` when `cancel` was set (the counts are then as they were).
    pub fn bar(&mut self, people: &[Person], cancel: &AtomicBool) -> Option<f32> {
        let prints: HashMap<&str, u64> = people.iter().map(|p| (p.key.as_str(), p.fingerprint())).collect();
        let same = |key: &str, print: u64| self.people.get(key).is_some_and(|c| c.fingerprint == print);
        // the people whose pairs are no longer true: changed, or gone; and those whose pairs are not counted yet
        let stale: Vec<(&str, &Counted)> =
            self.people.iter().filter(|(k, _)| !prints.get(k.as_str()).is_some_and(|p| same(k, *p))).map(|(k, c)| (k.as_str(), c)).collect();
        let fresh: Vec<&Person> = people.iter().filter(|p| !same(&p.key, p.fingerprint())).collect();
        let stale_keys: HashSet<&str> = stale.iter().map(|(k, _)| *k).collect();
        let fresh_keys: HashSet<&str> = fresh.iter().map(|p| p.key.as_str()).collect();
        let stopped = AtomicBool::new(false);
        let work = |old: bool| {
            let zero = || vec![0i64; BINS];
            let add = |mut a: Vec<i64>, b: Vec<i64>| {
                a.iter_mut().zip(b).for_each(|(x, y)| *x += y);
                a
            };
            // (a pair of two people who both changed is counted once, by the one with the lower key)
            let each = |key: &str, faces: &[Arc<[f32]>], acc: &mut Vec<i64>| {
                if cancel.load(Ordering::Relaxed) {
                    stopped.store(true, Ordering::Relaxed);
                    return;
                }
                if old {
                    for (other, c) in &self.people {
                        if other != key && !(stale_keys.contains(other.as_str()) && other.as_str() < key) {
                            count_between(acc, -1, faces, &c.faces);
                        }
                    }
                } else {
                    for q in people {
                        if q.key != key && !(fresh_keys.contains(q.key.as_str()) && q.key.as_str() < key) {
                            count_between(acc, 1, faces, &q.faces);
                        }
                    }
                }
            };
            if old {
                stale
                    .par_iter()
                    .fold(zero, |mut acc, (k, c)| {
                        each(k, &c.faces, &mut acc);
                        acc
                    })
                    .reduce(zero, add)
            } else {
                fresh
                    .par_iter()
                    .fold(zero, |mut acc, p| {
                        each(&p.key, &p.faces, &mut acc);
                        acc
                    })
                    .reduce(zero, add)
            }
        };
        let (gone, new) = (work(true), work(false));
        if stopped.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
            return None;
        }
        if self.bins.len() != BINS {
            self.bins = vec![0; BINS];
        }
        for ((bin, a), b) in self.bins.iter_mut().zip(gone).zip(new) {
            *bin = bin.saturating_add_signed(a + b);
        }
        self.people = people.iter().map(|p| (p.key.clone(), Counted { fingerprint: p.fingerprint(), faces: p.faces.clone() })).collect();
        Some(bar_from(&self.bins))
    }

    /// The counts, for the tests: how many pairs of faces of different people are in each bin.
    #[cfg(test)]
    pub fn counts(&self) -> &[u64] {
        &self.bins
    }
}

/// The bar the similarities of different people give (`bins`: how many pairs fall in each [`BINS`] bin): [`BAR_PERCENTILE`] of
/// them (the top of the bin it falls in) plus [`BAR_MARGIN`], kept between [`MIN_BAR`] and [`MAX_BAR`]. With fewer than
/// [`MIN_PAIRS`] pairs a percentile says nothing, so it is [`MIN_BAR`].
pub(crate) fn bar_from(bins: &[u64]) -> f32 {
    let pairs: u64 = bins.iter().sum();
    if pairs < MIN_PAIRS {
        return MIN_BAR;
    }
    let want = (pairs as f64 * BAR_PERCENTILE).ceil() as u64;
    let mut seen = 0u64;
    let at = bins.iter().position(|n| {
        seen += n;
        seen >= want
    });
    at.map_or(MIN_BAR, |i| (bin_top(i) + BAR_MARGIN).clamp(MIN_BAR, MAX_BAR))
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
                let mut scored: Vec<(f32, usize)> =
                    near.into_iter().filter(|g| *g != f).map(|g| (w.sim(f, g), g)).filter(|(c, _)| *c >= w.bar).collect();
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
    let fresh: Vec<usize> =
        free.iter().copied().filter(|i| cold || faces.get(*i).is_none_or(|f| !state.seen.contains(&(f.key(), f.stamp)))).collect();
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
