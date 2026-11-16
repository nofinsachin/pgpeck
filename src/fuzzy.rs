//! A small fuzzy matcher for pickers. Every character of the query must
//! appear in the text, in order. Pure logic.

const MATCH: i64 = 10;
const CONSECUTIVE: i64 = 25;
const TEXT_START: i64 = 20;
const WORD_START: i64 = 15;
const CAMEL_HUMP: i64 = 10;
const NONE: i64 = i64::MIN / 2;

/// Higher is better. `None` when the query does not match.
///
/// Finds the best way to place the query characters in the text. Matches
/// score higher when they are consecutive, start a word, or start the text,
/// and shorter texts win ties.
pub fn score(query: &str, text: &str) -> Option<i64> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.chars().collect();
    if text.len() < query.len() {
        return None;
    }
    let lower: Vec<char> = text
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let position_bonus = |i: usize| match i.checked_sub(1).map(|p| text[p]) {
        None => TEXT_START,
        Some(p) if !p.is_alphanumeric() => WORD_START,
        Some(p) if p.is_lowercase() && text[i].is_uppercase() => CAMEL_HUMP,
        _ => 0,
    };

    // best[i] = best score with the current query character matched at i.
    let mut previous: Vec<i64> = vec![NONE; text.len()];
    for (j, wanted) in query.iter().enumerate() {
        let mut current = vec![NONE; text.len()];
        // Best score of the previous query character at any position before i - 1.
        let mut earlier = NONE;
        for i in 0..text.len() {
            if j > 0 && i >= 2 {
                earlier = earlier.max(previous[i - 2]);
            }
            if lower[i] != *wanted {
                continue;
            }
            let base = if j == 0 {
                0
            } else {
                let adjacent = if i >= 1 && previous[i - 1] > NONE {
                    previous[i - 1] + CONSECUTIVE
                } else {
                    NONE
                };
                earlier.max(adjacent)
            };
            if base > NONE {
                current[i] = base + MATCH + position_bonus(i);
            }
        }
        previous = current;
    }
    let best = previous.into_iter().max().filter(|s| *s > NONE)?;
    Some(best * 100 - i64::try_from(text.len()).unwrap_or(99).min(99))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_subsequence_ignoring_case() {
        assert!(score("prd", "Production").is_some());
        assert!(score("PRD", "production").is_some());
        assert!(score("pdn", "production").is_some());
    }

    #[test]
    fn rejects_when_characters_are_missing_or_out_of_order() {
        assert_eq!(score("xyz", "production"), None);
        assert_eq!(score("dp", "production"), None);
        assert_eq!(
            score("pp", "pro"),
            None,
            "each query character needs its own match"
        );
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert_eq!(score("", "anything"), Some(0));
        assert_eq!(score("  ", ""), Some(0));
    }

    #[test]
    fn consecutive_and_word_start_matches_rank_higher() {
        let best = score("prod", "prod-db").expect("match");
        let scattered = score("prod", "p-r-o-d").expect("match");
        assert!(best > scattered);
        let word_start = score("db", "prod-db").expect("match");
        let inside = score("db", "proddbx").expect("match");
        assert!(word_start > inside);
    }

    #[test]
    fn shorter_text_wins_a_tie() {
        assert!(score("db", "db").expect("m") > score("db", "db-long-name").expect("m"));
    }

    #[test]
    fn works_with_non_ascii_text() {
        assert!(score("cafe", "café-db").is_none(), "no accent folding");
        assert!(score("caf", "café-db").is_some());
    }
}
