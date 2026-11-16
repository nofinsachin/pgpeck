//! The connection picker (R1.3): saved profiles with fuzzy search, plus a
//! connection URI typed straight in (R1.5). Pure logic.

use crate::fuzzy;
use crate::profile::{Profile, Profiles};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picker {
    pub input: String,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// The text typed so far, used as a connection URI.
    Uri(String),
    /// A saved profile, by name.
    Profile(String),
}

pub fn looks_like_uri(text: &str) -> bool {
    text.starts_with("postgres://") || text.starts_with("postgresql://")
}

/// What the profile can be found by.
fn search_fields(p: &Profile) -> [&str; 5] {
    [
        p.display_name(),
        &p.name,
        &p.host,
        p.database.as_deref().unwrap_or(""),
        p.user.as_deref().unwrap_or(""),
    ]
}

impl Picker {
    pub fn push(&mut self, c: char) {
        self.input.push(c);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.input.pop();
        self.selected = 0;
    }

    pub fn clear(&mut self) {
        self.input.clear();
        self.selected = 0;
    }

    /// Items in display order. Without a query, profiles come most recently
    /// used first. With one, best match first.
    pub fn items(&self, profiles: &Profiles) -> Vec<Item> {
        let mut items = Vec::new();
        if looks_like_uri(&self.input) {
            items.push(Item::Uri(self.input.clone()));
            return items;
        }
        let query = self.input.trim();
        let mut scored: Vec<(i64, &Profile)> = profiles
            .for_picker()
            .into_iter()
            .filter_map(|p| {
                let best = search_fields(p)
                    .iter()
                    .filter_map(|f| fuzzy::score(query, f))
                    .max()?;
                Some((best, p))
            })
            .collect();
        if !query.is_empty() {
            // Stable sort keeps the recent-first order for equal scores.
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        }
        items.extend(
            scored
                .into_iter()
                .map(|(_, p)| Item::Profile(p.name.clone())),
        );
        items
    }

    pub fn select_next(&mut self, count: usize) {
        if count > 0 {
            self.selected = (self.selected + 1) % count;
        }
    }

    pub fn select_previous(&mut self, count: usize) {
        if count > 0 {
            self.selected = (self.selected + count - 1) % count;
        }
    }

    /// The item Enter would act on.
    pub fn current(&self, profiles: &Profiles) -> Option<Item> {
        self.items(profiles).into_iter().nth(self.selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profiles() -> Profiles {
        let mut profiles = Profiles::default();
        for (name, host, db) in [
            ("production", "db.example.com", "app"),
            ("staging", "stg.example.com", "app"),
            ("local-dev", "localhost", "dev"),
        ] {
            let mut p = Profile::new(name);
            p.host = host.to_string();
            p.database = Some(db.to_string());
            profiles.add(p).expect("add");
        }
        profiles
    }

    fn names(items: &[Item]) -> Vec<&str> {
        items
            .iter()
            .map(|i| match i {
                Item::Profile(n) => n.as_str(),
                Item::Uri(_) => "<uri>",
            })
            .collect()
    }

    fn typed(text: &str) -> Picker {
        let mut p = Picker::default();
        text.chars().for_each(|c| p.push(c));
        p
    }

    #[test]
    fn without_a_query_all_profiles_are_listed_recent_first() {
        let mut profiles = profiles();
        profiles.touch("staging", 100);
        let items = Picker::default().items(&profiles);
        assert_eq!(names(&items), ["staging", "local-dev", "production"]);
    }

    #[test]
    fn the_query_filters_and_ranks() {
        let profiles = profiles();
        assert_eq!(names(&typed("prod").items(&profiles)), ["production"]);
        assert_eq!(names(&typed("stg").items(&profiles)), ["staging"]);
        assert_eq!(names(&typed("zzz").items(&profiles)), Vec::<&str>::new());
    }

    #[test]
    fn profiles_are_found_by_host_database_and_user_too() {
        let mut profiles = profiles();
        let mut p = Profile::new("misc");
        p.user = Some("reporting".to_string());
        p.host = "reports.internal".to_string();
        profiles.add(p).expect("add");
        assert_eq!(names(&typed("reporting").items(&profiles)), ["misc"]);
        assert_eq!(names(&typed("localhost").items(&profiles)), ["local-dev"]);
        assert!(names(&typed("example").items(&profiles)).contains(&"production"));
    }

    #[test]
    fn a_label_is_searchable() {
        let mut profiles = Profiles::default();
        let mut p = Profile::new("p1");
        p.label = Some("Billing replica".to_string());
        profiles.add(p).expect("add");
        assert_eq!(names(&typed("billing").items(&profiles)), ["p1"]);
    }

    #[test]
    fn a_typed_uri_is_the_only_item() {
        let picker = typed("postgres://u@h/d");
        assert_eq!(
            picker.items(&profiles()),
            [Item::Uri("postgres://u@h/d".to_string())]
        );
        assert!(looks_like_uri("postgresql://x"));
        assert!(!looks_like_uri("postgres"));
    }

    #[test]
    fn selection_wraps_and_resets_when_the_query_changes() {
        let mut picker = Picker::default();
        picker.select_next(3);
        picker.select_next(3);
        assert_eq!(picker.selected, 2);
        picker.select_next(3);
        assert_eq!(picker.selected, 0);
        picker.select_previous(3);
        assert_eq!(picker.selected, 2);
        picker.push('a');
        assert_eq!(picker.selected, 0);
        picker.select_next(0);
        picker.select_previous(0);
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn editing_the_query() {
        let mut picker = typed("abc");
        picker.backspace();
        assert_eq!(picker.input, "ab");
        picker.clear();
        assert_eq!(picker.input, "");
        picker.backspace();
    }

    #[test]
    fn current_follows_the_selection() {
        let profiles = profiles();
        let mut picker = Picker::default();
        picker.select_next(3);
        assert_eq!(
            picker.current(&profiles),
            Some(Item::Profile("production".to_string()))
        );
        assert_eq!(typed("zzz").current(&profiles), None);
    }
}
