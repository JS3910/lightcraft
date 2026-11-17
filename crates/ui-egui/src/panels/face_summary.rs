//! The loupe toolbar's line that says who is in the photo: the names of the named faces, then how many
//! are not named. Read from the photo's face regions on every frame, so it follows every edit at once.

use lightcraft_meta::{Region, RegionKind};

/// Who the face regions of a photo are: the distinct names, left to right, and how many faces have none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Who {
    pub names: Vec<String>,
    pub unnamed: usize,
}

/// Count the face regions (other regions are not faces). A name is trimmed; two spellings that differ only in
/// case are one person (as the People view groups them) and the first, left to right, is shown.
pub fn who(regions: &[Region]) -> Who {
    let mut faces: Vec<(f64, &Region)> = regions
        .iter()
        .filter(|r| r.kind == RegionKind::Face)
        .map(|r| {
            let centre = (r.rect.x0 + r.rect.x1) / 2.0;
            (if centre.is_finite() { centre } else { 0.0 }, r)
        })
        .collect();
    // stable: faces at the same place keep their order in the file
    faces.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Who::default();
    let mut seen: Vec<String> = Vec::new();
    for (_, region) in faces {
        match region.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            Some(name) => {
                let key = name.to_lowercase();
                if !seen.contains(&key) {
                    seen.push(key);
                    out.names.push(name.to_string());
                }
            }
            None => out.unnamed += 1,
        }
    }
    out
}

/// The text for `who`, in the UI language; `None` when the photo has no face boxes.
pub fn text(who: &Who) -> Option<String> {
    let n = who.unnamed;
    if who.names.is_empty() {
        return match n {
            0 => None,
            1 => Some(crate::i18n::tr_format!("{n} unnamed person", n = n)),
            _ => Some(crate::i18n::tr_format!("{n} unnamed people", n = n)),
        };
    }
    let names = who.names.join(", ");
    Some(if n == 0 { names } else { crate::i18n::tr_format!("{names}, {n} unnamed", names = names, n = n) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_geom::Rect;

    fn face(x: f64, name: Option<&str>) -> Region {
        Region {
            rect: Rect { x0: x, y0: 0.2, x1: x + 0.1, y1: 0.4 },
            kind: RegionKind::Face,
            name: name.map(str::to_string),
            description: None,
            rejected: Vec::new(),
        }
    }

    fn say(regions: &[Region]) -> Option<String> {
        text(&who(regions))
    }

    #[test]
    fn names_are_listed_left_to_right_each_once() {
        let r = [face(0.7, Some("Sasha")), face(0.1, Some("Johan")), face(0.4, Some("johan ")), face(0.5, Some("Sasha"))];
        assert_eq!(say(&r).as_deref(), Some("Johan, Sasha"));
    }

    #[test]
    fn only_unnamed_faces_are_counted() {
        assert_eq!(say(&[face(0.1, None)]).as_deref(), Some("1 unnamed person"));
        assert_eq!(say(&[face(0.1, None), face(0.3, Some("  ")), face(0.5, None)]).as_deref(), Some("3 unnamed people"));
    }

    #[test]
    fn names_are_followed_by_the_count_of_the_rest() {
        let r = [face(0.1, Some("Johan")), face(0.3, None), face(0.5, Some("Sasha"))];
        assert_eq!(say(&r).as_deref(), Some("Johan, Sasha, 1 unnamed"));
    }

    #[test]
    fn no_faces_say_nothing_and_other_regions_are_not_faces() {
        assert_eq!(say(&[]), None);
        let mut pet = face(0.1, Some("Rex"));
        pet.kind = RegionKind::Pet;
        assert_eq!(say(&[pet]), None);
    }

    #[test]
    fn a_region_with_a_broken_rect_does_not_panic() {
        let mut r = face(f64::NAN, None);
        r.rect.x1 = f64::INFINITY;
        assert_eq!(say(&[r]).as_deref(), Some("1 unnamed person"));
    }
}
