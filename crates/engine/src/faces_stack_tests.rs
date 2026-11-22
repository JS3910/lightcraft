use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use lightcraft_meta::Apart;

use super::*;

/// A unit vector of 8 numbers from (place, value) pairs.
fn unit(parts: &[(usize, f32)]) -> Arc<[f32]> {
    let mut v = [0f32; 8];
    for (i, x) in parts {
        v[*i] = *x;
    }
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / n).collect()
}

fn face(id: u64, e: Arc<[f32]>) -> Face {
    Face::new(id, [id, 0, 0, 0], 0, e, &[])
}

fn person(name: &str, faces: &[Arc<[f32]>]) -> Person {
    let mut p = Person::new(name.to_lowercase(), name.to_string());
    for (i, e) in faces.iter().enumerate() {
        p.add(i as u64 + 1, e.clone());
    }
    p
}

fn run(state: &mut StackState, faces: &[Face], bar: f32, limit: usize) -> Vec<Vec<u64>> {
    let refs: Vec<&Face> = faces.iter().collect();
    let found = stack(state, &refs, bar, &AtomicBool::new(false), limit).expect("not cancelled");
    found.into_iter().map(|s| s.into_iter().filter_map(|i| faces.get(i).map(|f| f.photo)).collect()).collect()
}

/// A vector whose cosine with the unit vector of place `a` is `c`, in the plane of places `a` and `b`.
fn at(c: f32, a: usize, b: usize, sign: f32) -> Arc<[f32]> {
    unit(&[(a, c), (b, sign * (1.0 - c * c).sqrt())])
}

#[test]
fn with_nobody_or_one_person_named_the_bar_is_the_fixed_minimum() {
    let mut c = Calibration::default();
    let none = AtomicBool::new(false);
    assert_eq!(c.bar(&[], &none), Some(0.75));
    // one person, however alike their own faces are: nobody to be told apart from
    let ann = person("Ann", &[unit(&[(0, 1.0)]), unit(&[(0, 1.0), (1, 0.01)])]);
    assert_eq!(c.bar(&[ann], &none), Some(0.75));
    // the minimum is clearly stricter than the suggestion bars of the known models (0.55 and 0.40)
    const { assert!(MIN_BAR >= 0.55 + 0.15) };
}

#[test]
fn the_bar_is_above_the_highest_similarity_between_two_people_but_never_below_the_minimum() {
    let mut c = Calibration::default();
    let none = AtomicBool::new(false);
    // Ann and Bob each have two faces; the closest of Ann's to Bob's are 0.9 alike
    let ann = person("Ann", &[unit(&[(0, 1.0)]), at(0.2, 0, 3, 1.0)]);
    let bob = person("Bob", &[at(0.9, 0, 1, 1.0), unit(&[(5, 1.0)])]);
    let bar = c.bar(&[ann, bob], &none).expect("bar");
    assert!((bar - 0.95).abs() < 1e-4, "the highest similarity (0.9) and the margin (0.05): {bar}");
    // two people who are not alike leave the minimum
    let ann = person("Ann", &[unit(&[(0, 1.0)])]);
    let bob = person("Bob", &[at(0.5, 0, 1, 1.0)]);
    assert_eq!(c.bar(&[ann, bob], &none), Some(0.75));
}

/// The highest similarity between two people the long way.
fn reference(people: &[Person]) -> f32 {
    let mut best: Option<f32> = None;
    for (i, a) in people.iter().enumerate() {
        for b in people.iter().skip(i + 1) {
            if let Some(c) = closest_between(a, b) {
                best = Some(best.map_or(c, |m| m.max(c)));
            }
        }
    }
    bar_from(best)
}

#[test]
fn after_every_change_the_bar_is_what_comparing_everyone_gives() {
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |n: usize| {
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        (rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % n
    };
    let mut named: Vec<(usize, Arc<[f32]>)> = Vec::new();
    let mut c = Calibration::default();
    for step in 0..120 {
        match next(4) {
            0 if !named.is_empty() => {
                let i = next(named.len());
                named.remove(i);
            }
            1 if !named.is_empty() => {
                let i = next(named.len());
                named[i].0 = next(6);
            }
            _ => {
                let who = next(6);
                let e = unit(&[(next(8), 1.0), (next(8), next(100) as f32 / 100.0)]);
                named.push((who, e));
            }
        }
        let people: Vec<Person> = (0..6)
            .filter_map(|p| {
                let mine: Vec<Arc<[f32]>> = named.iter().filter(|(who, _)| *who == p).map(|(_, e)| e.clone()).collect();
                (!mine.is_empty()).then(|| person(&format!("P{p}"), &mine))
            })
            .collect();
        let got = c.bar(&people, &AtomicBool::new(false)).expect("bar");
        assert!((got - reference(&people)).abs() < 1e-5, "step {step}: {got} against {}", reference(&people));
    }
}

#[test]
fn a_chain_of_look_alikes_never_puts_the_ends_in_one_stack() {
    // A and B are 0.9 alike, B and C are 0.9 alike, A and C only 0.62
    // (whichever of them comes first: the search starts from the first one)
    for order in [[1, 2, 3], [1, 3, 2], [2, 1, 3], [2, 3, 1], [3, 1, 2], [3, 2, 1]] {
        let make = |id: u64| match id {
            1 => face(1, at(0.9, 0, 1, 1.0)),
            2 => face(2, unit(&[(0, 1.0)])),
            _ => face(3, at(0.9, 0, 1, -1.0)),
        };
        let faces: Vec<Face> = order.iter().map(|id| make(*id)).collect();
        let stacks = run(&mut StackState::default(), &faces, 0.8, BRUTE_LIMIT);
        assert!(stacks.iter().all(|s| !(s.contains(&1) && s.contains(&3))), "{order:?}: {stacks:?}");
        assert_eq!(stacks.len(), 1, "{order:?}: one of the two pairs is still a stack: {stacks:?}");
        assert_eq!(stacks[0].len(), 2, "{order:?}");
    }
}

#[test]
fn two_faces_are_a_stack_one_is_not_and_a_face_that_cannot_be_compared_is_left_alone() {
    let faces = [
        face(1, unit(&[(0, 1.0)])),
        face(2, at(0.95, 0, 1, 1.0)),
        face(3, unit(&[(4, 1.0)])),
        // an embedding of another length (another model) and one with a NaN
        face(4, Arc::from(vec![1.0f32, 0.0])),
        face(5, Arc::from(vec![f32::NAN; 8])),
    ];
    let stacks = run(&mut StackState::default(), &faces, 0.9, BRUTE_LIMIT);
    let mut one = stacks.first().cloned().unwrap_or_default();
    one.sort_unstable();
    assert_eq!((stacks.len(), one), (1, vec![1, 2]), "{stacks:?}");
}

#[test]
fn a_stack_has_its_most_typical_face_first_and_the_biggest_stack_comes_first() {
    // a pair far from everything, and three faces around a middle one
    let faces = [
        face(1, unit(&[(5, 1.0)])),
        face(2, unit(&[(5, 1.0), (6, 0.01)])),
        face(3, at(0.99, 0, 1, 1.0)),
        face(4, unit(&[(0, 1.0)])),
        face(5, at(0.99, 0, 1, -1.0)),
    ];
    let stacks = run(&mut StackState::default(), &faces, 0.9, BRUTE_LIMIT);
    assert_eq!(stacks.len(), 2, "{stacks:?}");
    assert_eq!(stacks[0].len(), 3, "the biggest first");
    assert_eq!(stacks[0][0], 4, "the one in the middle is the most typical");
}

#[test]
fn a_new_face_joins_without_comparing_the_old_faces_with_each_other_and_a_face_that_left_dissolves_its_stack() {
    // 40 unrelated faces, and a pair
    let mut faces: Vec<Face> =
        (0..40).map(|i| face(i + 10, unit(&[(i as usize % 6, 1.0), (((i as usize / 6) % 5) + 1, 0.3 * (i % 5) as f32)]))).collect();
    faces.push(face(1, unit(&[(7, 1.0), (6, 0.05)])));
    faces.push(face(2, unit(&[(7, 1.0), (6, 0.06)])));
    let mut state = StackState::default();
    let first = run(&mut state, &faces, 0.97, BRUTE_LIMIT);
    let cold = state.last_compared;
    assert!(first.iter().any(|s| s.contains(&1) && s.contains(&2)), "{first:?}");
    // the same faces again: nothing is new, nothing is compared
    assert_eq!(run(&mut state, &faces, 0.97, BRUTE_LIMIT), first);
    assert_eq!((state.last_new, state.last_compared), (0, 0));
    // one more face like the pair: it joins their stack, at a fraction of the first cost
    faces.push(face(3, unit(&[(7, 1.0), (6, 0.055)])));
    let grown = run(&mut state, &faces, 0.97, BRUTE_LIMIT);
    assert!(grown.iter().any(|s| s.len() == 3 && s.contains(&3)), "{grown:?}");
    assert!(state.last_compared * 10 < cold, "{} against {cold}", state.last_compared);
    // a face of the stack goes: the stack is looked at again and the two left are still a stack
    faces.retain(|f| f.photo != 1);
    let shrunk = run(&mut state, &faces, 0.97, BRUTE_LIMIT);
    assert!(shrunk.iter().any(|s| s.len() == 2 && s.contains(&2) && s.contains(&3)), "{shrunk:?}");
    // another bar starts afresh
    let strict = run(&mut state, &faces, 1.5, BRUTE_LIMIT);
    assert!(strict.is_empty() && state.last_new == faces.len(), "{strict:?}");
}

#[test]
fn looking_among_cells_finds_the_same_people_as_comparing_every_pair() {
    // five people of ten faces each, a little noise around a direction of their own, and 100 faces that are nobody in particular
    let mut faces: Vec<Face> = Vec::new();
    for who in 0..5usize {
        for k in 0..10usize {
            faces.push(face((who * 10 + k) as u64 + 1, unit(&[(who, 1.0), (7, 0.02 * k as f32)])));
        }
    }
    for i in 0..100usize {
        faces.push(face(1000 + i as u64, unit(&[(5 + i % 3, 1.0), (i % 5, 0.4 + 0.05 * (i % 7) as f32), (7, 0.9)])));
    }
    let norm = |mut v: Vec<Vec<u64>>| {
        v.iter_mut().for_each(|s| s.sort_unstable());
        v.sort();
        v.into_iter().filter(|s| s.iter().all(|p| *p < 1000)).collect::<Vec<_>>()
    };
    let all = norm(run(&mut StackState::default(), &faces, 0.97, BRUTE_LIMIT));
    let cells = norm(run(&mut StackState::default(), &faces, 0.97, 0));
    assert_eq!(all.len(), 5, "{all:?}");
    assert_eq!(cells, all);
}

#[test]
fn a_cancelled_call_leaves_what_was_kept_as_it_was() {
    let faces = [face(1, unit(&[(0, 1.0)])), face(2, unit(&[(0, 1.0), (1, 0.01)]))];
    let mut state = StackState::default();
    assert_eq!(run(&mut state, &faces, 0.9, BRUTE_LIMIT).len(), 1);
    let refs: Vec<&Face> = faces.iter().collect();
    assert!(stack(&mut state, &refs, 0.9, &AtomicBool::new(true), BRUTE_LIMIT).is_none());
    assert_eq!(state.stacks.len(), 1, "the stack that was kept is still kept");
}

#[test]
fn a_catalogue_with_an_endless_list_of_faces_taken_out_is_read_up_to_the_cap() {
    let too_many: Vec<Apart> = (0..MAX_STACK as i32 + 2000).map(|i| Apart { photo: 0, rect: [i, 0, 0, 0] }).collect();
    let f = face(1, unit(&[(0, 1.0)])).with_apart(&too_many);
    assert!(f.apart.len() <= MAX_STACK, "{} faces kept", f.apart.len());
    // the ones kept are still honoured
    let kept = face(2, unit(&[(0, 1.0)]));
    let first = face(0, unit(&[(0, 1.0)])); // at photo 0, box 0: the first of the list
    assert!(f.apart_from(&first) && !f.apart_from(&kept));
}
