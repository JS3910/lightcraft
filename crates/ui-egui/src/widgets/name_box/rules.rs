//! The name box's rules, free of any drawing: which names it offers for the text typed, what it completes to, and
//! which spelling of a typed name is stored. Names are compared trimmed and ignoring capitalization, as people are.

/// The most names the list below the box shows.
pub const MAX_OFFERS: usize = 7;

/// A name the box offers, with how much the face looks like that person when the engine said so.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub name: String,
    pub score: Option<f32>,
}

/// How a name is compared: trimmed, lower case.
pub fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// 0 when `name` begins with `typed` (both as keys), 1 when a later word of it does, else `None`.
fn rank(name: &str, typed: &str) -> Option<u8> {
    let name = key(name);
    if name.starts_with(typed) {
        return Some(0);
    }
    let later = name.match_indices(char::is_whitespace).any(|(i, w)| name.get(i + w.len()..).is_some_and(|rest| rest.starts_with(typed)));
    later.then_some(1)
}

/// The names to offer for `typed`: at most [`MAX_OFFERS`]. With nothing typed, the closest people (`candidates`, best
/// first, as the engine returned them). Otherwise the names that match: those that begin with the text before those
/// where a later word begins with it; within each, the people the face looks like (best first), then the rest
/// alphabetically. A name appears once, whatever its capitalization.
#[cfg(test)]
pub fn offered(typed: &str, candidates: &[(String, f32)], people: &[String]) -> Vec<Offer> {
    offered_for(typed, candidates, people, &[])
}

/// What a listed name says: the name, and when the face was ranked, how much it looks like them as a percentage.
pub fn label(offer: &Offer) -> String {
    match offer.score {
        Some(s) => format!("{}   {:.0}%", offer.name, (s * 100.0).max(0.0)),
        None => offer.name.clone(),
    }
}

/// [`offered`] for a face that rejected some names (trimmed, ignoring capitalization): those come last, after everyone
/// else, in the order they would have had.
pub fn offered_for(typed: &str, candidates: &[(String, f32)], people: &[String], rejected: &[String]) -> Vec<Offer> {
    let typed = key(typed);
    let bad: std::collections::HashSet<String> = rejected.iter().map(|r| key(r)).collect();
    let last = |offers: &mut Vec<Offer>| offers.sort_by_key(|o| bad.contains(&key(&o.name)));
    let mut seen = std::collections::HashSet::new();
    let mut known: Vec<Offer> = Vec::new();
    for (name, score) in candidates {
        if seen.insert(key(name)) {
            known.push(Offer { name: name.clone(), score: Some(*score) });
        }
    }
    if typed.is_empty() {
        last(&mut known);
        known.truncate(MAX_OFFERS);
        return known;
    }
    let mut rest: Vec<Offer> = people.iter().filter(|p| seen.insert(key(p))).map(|p| Offer { name: p.clone(), score: None }).collect();
    rest.sort_by_key(|o| key(&o.name));
    let mut ranked: Vec<(u8, usize, Offer)> = Vec::new();
    for (order, offer) in known.into_iter().chain(rest).enumerate() {
        if let Some(group) = rank(&offer.name, &typed) {
            ranked.push((group, order, offer));
        }
    }
    ranked.sort_by_key(|(group, order, _)| (*group, *order));
    let mut offers: Vec<Offer> = ranked.into_iter().map(|(_, _, o)| o).collect();
    last(&mut offers);
    offers.truncate(MAX_OFFERS);
    offers
}

/// What the box completes `typed` to: the first offered name that begins with it and is longer. The typed text is
/// matched as typed (not trimmed), so the characters it counts are the ones in the box.
pub fn completion<'a>(typed: &str, offers: &'a [Offer]) -> Option<&'a str> {
    if typed.is_empty() || typed.trim() != typed {
        return None;
    }
    let want = typed.to_lowercase();
    let n = typed.chars().count();
    offers.iter().map(|o| o.name.as_str()).find(|name| name.chars().count() > n && name.chars().take(n).collect::<String>().to_lowercase() == want)
}

/// The spelling to store for a typed name: the typed name, trimmed, unless it differs from an existing person only in
/// capitalization, whose spelling then wins. A face that already has that name (`current`) keeps what was typed, so
/// capitalization can still be corrected. Empty text stays empty: it removes the name.
pub fn stored_spelling(typed: &str, people: &[String], current: Option<&str>) -> String {
    let typed = typed.trim();
    let k = key(typed);
    if k.is_empty() || current.is_some_and(|c| key(c) == k) {
        return typed.to_string();
    }
    people.iter().find(|p| key(p) == k).map_or_else(|| typed.to_string(), |p| p.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ranked_name_shows_its_similarity_as_a_percentage_and_an_unranked_one_none() {
        let ranked = |score| label(&Offer { name: "Ann Lee".into(), score });
        assert_eq!(ranked(Some(0.874)), "Ann Lee   87%");
        assert_eq!(ranked(Some(1.0)), "Ann Lee   100%");
        assert_eq!(ranked(Some(-0.2)), "Ann Lee   0%", "never negative");
        assert_eq!(ranked(None), "Ann Lee");
    }

    fn names(offers: &[Offer]) -> Vec<&str> {
        offers.iter().map(|o| o.name.as_str()).collect()
    }
    fn people(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn names_that_begin_with_the_text_come_before_names_where_a_later_word_does() {
        let p = people(&["Maria Ann", "Anna", "Zed", "Ann Lee", "Joanna"]);
        let o = offered("ann", &[], &p);
        assert_eq!(names(&o), ["Ann Lee", "Anna", "Maria Ann"], "Joanna only contains it inside a word");
    }

    #[test]
    fn the_closest_people_come_first_within_each_group_then_alphabetical() {
        let p = people(&["Bob", "Bea", "Bo", "Abe Bell"]);
        let c = vec![("Bo".to_string(), 0.9), ("Abe Bell".to_string(), 0.8)];
        let o = offered("b", &c, &p);
        assert_eq!(names(&o), ["Bo", "Bea", "Bob", "Abe Bell"]);
        assert_eq!(o[0].score, Some(0.9));
        assert_eq!(o[1].score, None);
    }

    #[test]
    fn nothing_typed_offers_the_closest_people_and_at_most_seven_names_are_offered() {
        let c = vec![("Ann".to_string(), 0.7), ("Bob".to_string(), 0.5)];
        assert_eq!(names(&offered("  ", &c, &people(&["Zed"]))), ["Ann", "Bob"]);
        let many: Vec<String> = (0..20).map(|i| format!("Name {i:02}")).collect();
        assert_eq!(offered("name", &[], &many).len(), MAX_OFFERS);
    }

    #[test]
    fn the_names_a_face_rejected_are_offered_last() {
        let p = people(&["Ann", "Anna", "Annie", "Bob"]);
        let c = vec![("Ann".to_string(), 0.9), ("Anna".to_string(), 0.8), ("Bob".to_string(), 0.7)];
        let rejected = people(&[" ann "]);
        assert_eq!(names(&offered_for("an", &c, &p, &rejected)), ["Anna", "Annie", "Ann"], "last, though it looks most like them");
        assert_eq!(names(&offered_for("", &c, &p, &rejected)), ["Anna", "Bob", "Ann"], "also with nothing typed");
        assert_eq!(names(&offered_for("an", &c, &p, &[])), ["Ann", "Anna", "Annie"]);
        let many: Vec<String> = (0..20).map(|i| format!("Name {i:02}")).collect();
        let listed = offered_for("name", &[], &many, &people(&["Name 00"]));
        assert!(names(&listed).iter().all(|n| *n != "Name 00"), "a rejected name does not crowd out the others");
    }

    #[test]
    fn a_person_is_offered_once_whatever_the_capitalization() {
        let c = vec![("ann".to_string(), 0.7)];
        assert_eq!(names(&offered("a", &c, &people(&["Ann", "Al"]))), ["ann", "Al"]);
    }

    #[test]
    fn it_completes_to_the_first_name_that_begins_with_the_text() {
        let o = offered("an", &[], &people(&["Anna", "Ann", "Joan"]));
        assert_eq!(completion("an", &o), Some("Ann"));
        assert_eq!(completion("Ann", &o), Some("Anna"), "a longer name still completes");
        assert_eq!(completion("Anna", &o), None, "nothing left to complete");
        assert_eq!(completion("", &o), None);
        assert_eq!(completion("jo", &o), None, "a later-word match is listed but never completed");
    }

    #[test]
    fn completion_counts_characters_not_bytes() {
        let o = vec![Offer { name: "Ångström".into(), score: None }];
        assert_eq!(completion("ång", &o), Some("Ångström"));
        assert_eq!(completion("Å", &o), Some("Ångström"));
    }

    #[test]
    fn an_existing_persons_spelling_replaces_a_capitalization_only_difference() {
        let p = people(&["Ann Lee"]);
        assert_eq!(stored_spelling("  ann lee ", &p, None), "Ann Lee");
        assert_eq!(stored_spelling("Ann Leigh", &p, None), "Ann Leigh");
        assert_eq!(stored_spelling("", &p, Some("Ann Lee")), "", "empty removes the name");
    }

    #[test]
    fn a_face_that_already_has_the_name_keeps_the_capitalization_typed() {
        let p = people(&["Ann Lee"]);
        assert_eq!(stored_spelling("ann LEE", &p, Some("Ann Lee")), "ann LEE");
    }
}
