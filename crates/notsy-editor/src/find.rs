//! Finding text in a note, ignoring case.

/// Every place `query` occurs in `text`, ignoring case, as byte ranges in
/// order and not overlapping. Characters whose lowercase is more than one
/// character (a rare few) compare as they are.
pub fn find_all(text: &str, query: &str) -> Vec<(usize, usize)> {
    let fold = |c: char| {
        let mut l = c.to_lowercase();
        match (l.next(), l.next()) {
            (Some(a), None) => a,
            _ => c,
        }
    };
    let q: Vec<char> = query.chars().map(fold).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> = text.char_indices().map(|(i, c)| (i, fold(c))).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + q.len() <= chars.len() {
        if chars[i..i + q.len()].iter().zip(&q).all(|((_, c), d)| c == d) {
            let start = chars[i].0;
            let end = chars.get(i + q.len()).map_or(text.len(), |(j, _)| *j);
            out.push((start, end));
            i += q.len();
        } else {
            i += 1;
        }
    }
    out
}

/// The match to start from for the caret at `at`: the first that ends
/// after it, or the first of all when it is past the last.
pub fn nearest(matches: &[(usize, usize)], at: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(matches.iter().position(|&(_, end)| end > at).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_every_occurrence_ignoring_case() {
        let text = "Plan the plan. PLAN!";
        assert_eq!(find_all(text, "plan"), vec![(0, 4), (9, 13), (15, 19)]);
        assert_eq!(find_all(text, ""), vec![]);
        assert_eq!(find_all("aaaa", "aa"), vec![(0, 2), (2, 4)], "no overlaps");
        assert_eq!(find_all("Ünïcode ünï", "ÜNÏ"), vec![(0, 5), (10, 15)]);
        let arabic = "ملاحظة عن الملاحظة";
        let second = arabic.rfind("ملاحظة").unwrap();
        assert_eq!(find_all(arabic, "ملاحظة"), vec![(0, 12), (second, second + 12)]);
    }

    #[test]
    fn starts_from_the_caret() {
        let m = [(0, 4), (9, 13), (15, 19)];
        assert_eq!(nearest(&m, 0), Some(0));
        assert_eq!(nearest(&m, 5), Some(1));
        assert_eq!(nearest(&m, 30), Some(0), "wraps around");
        assert_eq!(nearest(&[], 3), None);
    }
}
