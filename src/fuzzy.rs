//! Fuzzy matching for the searches, with [nucleo](https://github.com/helix-editor/nucleo):
//! fzf's algorithm, which finds the best way the query's letters fit the text
//! (word starts, letters together) rather than the first. Each word of the
//! query matches on its own, in any order.

use std::{cell::RefCell, cmp::Ordering};

use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

thread_local! {
    /// Matchers allocate a fair amount, so each thread keeps one.
    static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new(Config::DEFAULT));
}

/// What's searched, for the bonuses that suit it.
#[derive(Clone, Copy)]
pub enum Text {
    /// A name, typed from its start: matches nearer it rank a little higher.
    Name,
    /// A path: the letter after `/` (or `\` on Windows) starts a word.
    Path,
}

impl Text {
    fn config(self) -> Config {
        match self {
            Text::Name => {
                let mut config = Config::DEFAULT;
                config.prefer_prefix = true;
                config
            }
            Text::Path => Config::DEFAULT.match_paths(),
        }
    }
}

/// What was typed, ready to match many texts with.
pub struct Query(Pattern);

impl Query {
    /// Each word (split at spaces) matches on its own; case is ignored and
    /// accents don't matter.
    pub fn new(query: &str) -> Self {
        Self(Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        ))
    }

    /// Scores `text`: `None` if a word doesn't match. With `positions`, the
    /// byte offsets of the matched characters too (slower: for what shows).
    /// An empty query matches everything, scoring 0.
    pub fn score(&self, text: &str, kind: Text, positions: bool) -> Option<(i32, Vec<usize>)> {
        if self.0.atoms.is_empty() {
            return Some((0, Vec::new()));
        }
        let chars: Vec<char>;
        let haystack = if text.is_ascii() {
            Utf32Str::Ascii(text.as_bytes())
        } else {
            // One per char (not per grapheme), so the indices map back to bytes.
            chars = text.chars().collect();
            Utf32Str::Unicode(&chars)
        };
        let mut indices = Vec::new();
        let score = MATCHER.with_borrow_mut(|matcher| {
            matcher.config = kind.config();
            if positions {
                self.0.indices(haystack, matcher, &mut indices)
            } else {
                self.0.score(haystack, matcher)
            }
        })?;
        indices.sort_unstable();
        indices.dedup();
        let offsets = if text.is_ascii() {
            indices.into_iter().map(|i| i as usize).collect()
        } else {
            let starts: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
            indices.into_iter().map(|i| starts[i as usize]).collect()
        };
        Some((score as i32, offsets))
    }
}

/// A list item that matched: its score and the matched byte offsets in its
/// title, or else in its subtitle.
pub struct ItemMatch {
    pub score: i32,
    pub title_hl: Vec<usize>,
    pub subtitle_hl: Vec<usize>,
}

/// Scores an item by its title, else its subtitle. Title matches always rank
/// above subtitle-only ones; `boost` breaks ties between title matches.
pub fn score_item(query: &str, title: &str, subtitle: &str, boost: i32) -> Option<ItemMatch> {
    let query = Query::new(query);
    if let Some((score, hl)) = query.score(title, Text::Name, true) {
        return Some(ItemMatch {
            score: score + 1000 + boost,
            title_hl: hl,
            subtitle_hl: Vec::new(),
        });
    }
    query
        .score(subtitle, Text::Path, true)
        .map(|(score, hl)| ItemMatch {
            score,
            title_hl: Vec::new(),
            subtitle_hl: hl,
        })
}

/// Where a matched entry goes in a list ranked by match and by use: by how
/// well it matched plus a bonus for use, then, between equals, the shorter.
#[derive(Clone, Copy, Debug)]
pub struct Rank {
    score: f32,
    length: usize,
}

impl PartialEq for Rank {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Rank {}

impl PartialOrd for Rank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rank {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then(other.length.cmp(&self.length))
    }
}

/// `score_item` for a list ranked by use as well: `boost` (how much and how
/// recently the entry was used) decides between matches equally good, and
/// only then the shorter name: "examp" puts the "example-v2" used minutes
/// ago above the "example" used an hour ago.
pub fn rank_item(
    query: &str,
    title: &str,
    subtitle: &str,
    boost: f32,
) -> Option<(Rank, ItemMatch)> {
    let m = score_item(query, title, subtitle, 0)?;
    let matched = if m.title_hl.is_empty() && !m.subtitle_hl.is_empty() {
        subtitle
    } else {
        title
    };
    let rank = Rank {
        score: m.score as f32 + boost,
        length: matched.chars().count(),
    };
    Some((rank, m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
        Query::new(query).score(text, Text::Name, true)
    }

    #[test]
    fn matches_subsequence() {
        assert!(score("rev", "reviews-v2").is_some());
        assert!(score("xyz", "reviews").is_none());
        assert_eq!(score("", "abc").unwrap().0, 0);
    }

    #[test]
    fn prefers_prefix_and_boundaries() {
        let (a, _) = score("sd", "sample-sdk").unwrap();
        let (b, _) = score("sd", "services-dashboard").unwrap();
        let (c, _) = score("sd", "backoffice-sd").unwrap();
        assert!(a > c && b > c);
        let (exact, _) = score("api", "api-v2").unwrap();
        let (inner, _) = score("api", "rapid").unwrap();
        assert!(exact > inner);
    }

    #[test]
    fn picks_best_start() {
        // Greedy from the first 'v' would match "reviews"; the boundary 'v2' is better.
        let (_, positions) = score("v2", "reviews-v2").unwrap();
        assert_eq!(positions, vec![8, 9]);
        // A word together, not its first letter at the start and the rest later.
        let (_, positions) = Query::new("palette")
            .score("proj/src/palette/", Text::Path, true)
            .unwrap();
        assert_eq!(positions, (9..16).collect::<Vec<_>>());
    }

    #[test]
    fn words_match_on_their_own() {
        assert!(score("sdk shared", "interactive-v2 + shared-sdk").is_some());
        assert!(score("sdk nope", "interactive-v2 + shared-sdk").is_none());
    }

    #[test]
    fn offsets_are_bytes_past_accents() {
        // "é" is two bytes: the last "c" is char 5 but byte 6.
        let (_, positions) = score("cafec", "café-c").unwrap();
        assert_eq!(positions, vec![0, 1, 2, 3, 6]);
        assert!(score("cafe", "Café").is_some());
    }

    #[test]
    fn length_only_breaks_ties() {
        let rank = |title: &str, boost: f32| rank_item("examp", title, "", boost).unwrap().0;
        // Equal matches, no use: the shorter.
        assert!(rank("example", 0.) > rank("example-v2", 0.));
        // A little use outweighs a longer name.
        assert!(rank("example-v2", 0.5) > rank("example", 0.));
        // But not a much better match: a prefix over a scattered one.
        assert!(rank("example", 0.) > rank("eqxqaqmqp", 37.));
        // Title matches stay above path-only ones.
        let (by_path, _) = rank_item("examp", "app", "~/example", 37.).unwrap();
        assert!(rank("counterexample", 0.) > by_path);
    }
}
