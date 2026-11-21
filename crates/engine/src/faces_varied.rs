//! Picking faces that look as different from each other as possible, for the top of a person's page: the most typical
//! face first (closest to the average of the faces), then each next face is the one least like the faces already picked.
//!
//! No session here: [`pick`] takes the embeddings and says which to show, so it is tested alone and the page asks for it
//! once per change of the person's faces, never per frame.

use lightcraft_faces::matching;
use rayon::prelude::*;

/// The indexes of up to `count` of `faces` to show first, in the order to show them. A face counts only with an embedding
/// of exactly `dim` finite numbers; the others are never picked (the caller lists them after). Ties go to the lower index,
/// so the same faces always give the same answer.
pub fn pick(faces: &[Option<&[f32]>], dim: usize, count: usize) -> Vec<usize> {
    if dim == 0 || count == 0 {
        return Vec::new();
    }
    // the usable faces (a NaN or infinite number makes the length not finite) as unit vectors in one block, so that comparing is a walk through memory
    let mut at: Vec<usize> = Vec::new();
    let mut unit: Vec<f32> = Vec::new();
    for (i, face) in faces.iter().enumerate() {
        let Some(e) = face.filter(|e| e.len() == dim) else { continue };
        let norm = e.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm.is_finite() && norm > 1e-12 {
            at.push(i);
            unit.extend(e.iter().map(|x| x / norm));
        }
    }
    let n = at.len();
    if n == 0 {
        return Vec::new();
    }
    let face = |k: usize| unit.get(k * dim..(k + 1) * dim).unwrap_or(&[]);
    let sim = |a: &[f32], b: &[f32]| matching::cosine(a, b).unwrap_or(-1.0);
    // the most typical face: closest to the average
    let mut mean = vec![0f32; dim];
    for row in unit.chunks_exact(dim) {
        for (m, x) in mean.iter_mut().zip(row) {
            *m += x;
        }
    }
    let mut first = 0;
    let mut best = f32::NEG_INFINITY;
    for k in 0..n {
        let s = sim(&mean, face(k));
        if s > best {
            (first, best) = (k, s);
        }
    }
    let mut out = Vec::with_capacity(count.min(n));
    // how close each face is to the nearest face already picked (a picked face is infinitely close, so is never taken twice)
    let mut nearest = vec![f32::NEG_INFINITY; n];
    let mut next = Some(first);
    while let Some(k) = next.filter(|_| out.len() < count) {
        out.push(at.get(k).copied().unwrap_or(0));
        let picked = face(k);
        nearest.par_iter_mut().enumerate().for_each(|(j, near)| {
            *near = if j == k { f32::INFINITY } else { near.max(sim(picked, face(j))) };
        });
        // the face least like the picked ones; the lower index wins a tie
        next = nearest.iter().enumerate().filter(|(_, s)| s.is_finite()).min_by(|a, b| a.1.total_cmp(b.1).then(a.0.cmp(&b.0))).map(|(j, _)| j);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit vector of 8 numbers with weight on `axis`, tilted a little by `tilt` towards the next axis.
    fn near(axis: usize, tilt: f32) -> Vec<f32> {
        let mut v = vec![0.0; 8];
        v[axis % 8] = 1.0;
        v[(axis + 1) % 8] = tilt;
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    fn refs(v: &[Vec<f32>]) -> Vec<Option<&[f32]>> {
        v.iter().map(|e| Some(e.as_slice())).collect()
    }

    #[test]
    fn the_first_picks_come_one_from_each_cluster_before_any_second_face_of_a_cluster() {
        // three clusters (axes 0, 2 and 4) of three faces, and two more faces in the cluster on axis 0: the biggest
        let faces: Vec<Vec<f32>> =
            (0..3).flat_map(|c| (0..3).map(move |i| near(c * 2, 0.02 * i as f32))).chain([near(0, 0.01), near(0, 0.03)]).collect();
        let picked = pick(&refs(&faces), 8, 4);
        let cluster = |i: usize| if i >= 9 { 0 } else { i / 3 };
        let mut first_three: Vec<usize> = picked.iter().take(3).map(|i| cluster(*i)).collect();
        assert_eq!(cluster(picked[0]), 0, "the most typical face is in the cluster with most faces: {picked:?}");
        first_three.sort_unstable();
        assert_eq!(first_three, [0, 1, 2], "one from each cluster first: {picked:?}");
        assert_eq!(picked.len(), 4);
    }

    #[test]
    fn the_first_pick_is_the_face_closest_to_the_average() {
        // the average of these three is nearest the middle one
        let faces = vec![near(0, -0.5), near(0, 0.0), near(0, 0.5)];
        assert_eq!(pick(&refs(&faces), 8, 3)[0], 1);
    }

    #[test]
    fn faces_without_a_usable_embedding_are_never_picked_and_nothing_panics() {
        let good = near(0, 0.0);
        let short = vec![1.0, 0.0];
        let nan = vec![f32::NAN; 8];
        let inf = vec![f32::INFINITY; 8];
        let zero = vec![0.0; 8];
        let faces: Vec<Option<&[f32]>> = vec![None, Some(&short), Some(&nan), Some(&good), Some(&inf), Some(&zero), Some(&good)];
        let picked = pick(&faces, 8, 10);
        assert!(picked.iter().all(|i| [3, 6].contains(i)), "only the usable ones: {picked:?}");
        assert_eq!(picked.len(), 2);
        assert!(pick(&[], 8, 5).is_empty() && pick(&refs(&[good.clone()]), 8, 0).is_empty() && pick(&refs(&[good]), 0, 5).is_empty());
    }

    #[test]
    fn identical_faces_are_picked_in_list_order() {
        let faces = vec![near(3, 0.0); 4];
        assert_eq!(pick(&refs(&faces), 8, 4), [0, 1, 2, 3]);
    }
}
