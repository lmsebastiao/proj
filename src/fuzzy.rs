//! Small subsequence fuzzy matcher with word-boundary and adjacency bonuses.

/// Scores `query` against `text`. Returns the score and the byte offsets of the
/// matched characters in `text`, or `None` if `query` is not a subsequence.
pub fn score(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|&(_, c)| c.to_lowercase().next().unwrap_or(c))
        .collect();

    // Try every start position of the first query char and keep the best greedy run.
    let mut best: Option<(i32, Vec<usize>)> = None;
    for start in (0..lower.len()).filter(|&i| lower[i] == query[0]) {
        let Some(positions) = greedy(&query, &lower, start) else {
            break; // later starts can't match either
        };
        let score = score_positions(&positions, &chars, lower.len());
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, positions.iter().map(|&i| chars[i].0).collect()));
        }
    }
    best
}

fn greedy(query: &[char], lower: &[char], start: usize) -> Option<Vec<usize>> {
    let mut positions = Vec::with_capacity(query.len());
    let mut i = start;
    for &q in query {
        while i < lower.len() && lower[i] != q {
            i += 1;
        }
        if i == lower.len() {
            return None;
        }
        positions.push(i);
        i += 1;
    }
    Some(positions)
}

fn score_positions(positions: &[usize], chars: &[(usize, char)], len: usize) -> i32 {
    let mut score = 0;
    let mut prev: Option<usize> = None;
    for &i in positions {
        score += 16;
        if i == 0 {
            score += 24;
        } else if is_boundary(chars[i - 1].1, chars[i].1) {
            score += 12;
        }
        match prev {
            Some(p) if p + 1 == i => score += 10,
            Some(p) => score -= ((i - p - 1) as i32).min(8),
            None => score -= (i as i32).min(12),
        }
        prev = Some(i);
    }
    score - (len as i32) / 4
}

fn is_boundary(prev: char, cur: char) -> bool {
    matches!(prev, '-' | '_' | ' ' | '.' | '/' | '\\')
        || (prev.is_lowercase() && cur.is_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_subsequence() {
        assert!(score("adv", "advertising-v2").is_some());
        assert!(score("xyz", "advertising").is_none());
        assert_eq!(score("", "abc").unwrap().0, 0);
    }

    #[test]
    fn prefers_prefix_and_boundaries() {
        let (a, _) = score("sd", "shared-sdk").unwrap();
        let (b, _) = score("sd", "services-dashboard").unwrap();
        let (c, _) = score("sd", "backoffice-sd").unwrap();
        assert!(a > c && b > c);
        let (exact, _) = score("api", "api-v2").unwrap();
        let (inner, _) = score("api", "rapid").unwrap();
        assert!(exact > inner);
    }

    #[test]
    fn picks_best_start() {
        // Greedy from the first 'v' would match "advertising"; the boundary 'v2' is better.
        let (_, positions) = score("v2", "advertising-v2").unwrap();
        assert_eq!(positions, vec![12, 13]);
    }
}
