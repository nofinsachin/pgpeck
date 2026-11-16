//! Completion sources and ranking. Pure logic.
//!
//! Each source answers for the word being typed. Catalog-backed sources
//! (relations, columns, functions, types) plug in through the same trait.

use crate::sql;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Keyword,
}

impl Kind {
    /// Short tag shown next to a suggestion.
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Keyword => "kw",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The text to insert.
    pub text: String,
    pub kind: Kind,
}

/// What the user has typed so far.
pub struct Request<'a> {
    pub prefix: &'a str,
}

pub trait CompletionSource {
    fn candidates(&self, request: &Request) -> Vec<Candidate>;
}

/// SQL keywords. The suggestion follows the case of what was typed:
/// capitals give `SELECT`, anything else gives `select`.
pub struct KeywordSource;

impl CompletionSource for KeywordSource {
    fn candidates(&self, request: &Request) -> Vec<Candidate> {
        let prefix = request.prefix;
        let upper =
            prefix.chars().any(char::is_uppercase) && !prefix.chars().any(char::is_lowercase);
        sql::keywords()
            .iter()
            .filter(|k| k.len() >= prefix.len() && k[..prefix.len()].eq_ignore_ascii_case(prefix))
            .map(|k| Candidate {
                text: if upper {
                    (*k).to_string()
                } else {
                    k.to_ascii_lowercase()
                },
                kind: Kind::Keyword,
            })
            .collect()
    }
}

/// Maximum number of suggestions kept.
pub const LIMIT: usize = 50;

/// Suggestions for `prefix` from all sources, best first. A suggestion equal
/// to what was typed is dropped, since accepting it changes nothing.
pub fn complete(sources: &[&dyn CompletionSource], prefix: &str) -> Vec<Candidate> {
    let request = Request { prefix };
    let mut found: Vec<Candidate> = sources
        .iter()
        .flat_map(|s| s.candidates(&request))
        .filter(|c| !c.text.eq_ignore_ascii_case(prefix))
        .collect();
    // Shorter first, so the closest completion is on top. The sort is stable,
    // so equal lengths keep alphabetical order.
    found.sort_by_key(|c| c.text.len());
    found.truncate(LIMIT);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(prefix: &str) -> Vec<String> {
        complete(&[&KeywordSource], prefix)
            .into_iter()
            .map(|c| c.text)
            .collect()
    }

    #[test]
    fn matches_keywords_by_prefix_ignoring_case() {
        assert_eq!(texts("sel"), ["select"]);
        assert_eq!(texts("SeL"), ["select"]);
        assert!(texts("zzz").is_empty());
    }

    #[test]
    fn follows_the_case_of_the_prefix() {
        assert_eq!(texts("SEL"), ["SELECT"]);
        assert_eq!(texts("Sel"), ["select"]);
        assert_eq!(texts("sel"), ["select"]);
    }

    #[test]
    fn shorter_keywords_rank_first_then_alphabetical() {
        assert_eq!(texts("de"), ["desc", "delete", "declare", "default"]);
    }

    #[test]
    fn an_exact_match_is_not_suggested() {
        assert!(texts("select").is_empty());
        assert!(texts("SELECT").is_empty());
        // A longer keyword that starts with it still is.
        assert_eq!(texts("order"), Vec::<String>::new());
        assert_eq!(texts("tem"), ["temp", "temporary"]);
    }

    #[test]
    fn results_are_limited() {
        struct Many;
        impl CompletionSource for Many {
            fn candidates(&self, _: &Request) -> Vec<Candidate> {
                (0..500)
                    .map(|i| Candidate {
                        text: format!("x{i}"),
                        kind: Kind::Keyword,
                    })
                    .collect()
            }
        }
        assert_eq!(complete(&[&Many], "x").len(), LIMIT);
    }

    #[test]
    fn several_sources_are_merged() {
        struct Extra;
        impl CompletionSource for Extra {
            fn candidates(&self, _: &Request) -> Vec<Candidate> {
                vec![Candidate {
                    text: "selected_rows".to_string(),
                    kind: Kind::Keyword,
                }]
            }
        }
        assert_eq!(
            complete(&[&KeywordSource, &Extra], "sel"),
            vec![
                Candidate {
                    text: "select".to_string(),
                    kind: Kind::Keyword
                },
                Candidate {
                    text: "selected_rows".to_string(),
                    kind: Kind::Keyword
                },
            ]
        );
    }
}
