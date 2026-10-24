//! The Search tab: a query typed on the on-screen keyboard or in the
//! search box, and the games itch.io finds for it.

use std::sync::Arc;

use crate::actions::Action;
use crate::glyphs::Glyph;
use crate::model::{Game, Loadable};
use crate::osk::{Key, Keyboard};
use crate::ui;

/// The longest query the keyboard types.
const MAX_QUERY: usize = 64;

#[derive(Default)]
pub struct Search {
    pub query: String,
    /// The on-screen keyboard, while it is open.
    pub keyboard: Option<Keyboard>,
    /// The query the results are for.
    searched: String,
    /// The games found, by id, in itch.io's order.
    pub results: Loadable<Vec<i64>>,
    pub rows: ui::Rows,
    /// The one playable type to show, from `playable::playable_types`.
    pub type_filter: Option<String>,
}

/// What a press came to while the keyboard is open.
pub enum Typed {
    Taken,
    /// Run the search for the query.
    Run,
    /// Not the keyboard's; it goes where it would without one.
    Pass(Action),
}

impl Search {
    pub fn open_keyboard(&mut self) {
        if self.keyboard.is_none() {
            self.keyboard = Some(Keyboard::default());
        }
    }

    /// A on a key types it, B deletes, closing the keyboard once there is
    /// nothing left to delete, Y types a space and Start searches.
    pub fn type_key(&mut self, action: Action) -> Typed {
        let Some(keyboard) = self.keyboard.as_mut() else {
            return Typed::Pass(action);
        };
        match action {
            Action::MoveFocus(direction) => keyboard.step(direction),
            Action::FocusKey { row, key } => keyboard.focus(row, key),
            Action::Activate => match keyboard.focused() {
                Key::Char(c) => push(&mut self.query, c),
                Key::Space => push(&mut self.query, ' '),
                Key::Delete => {
                    self.query.pop();
                }
                Key::Done => return Typed::Run,
            },
            Action::Secondary => push(&mut self.query, ' '),
            Action::Back => {
                if self.query.pop().is_none() {
                    self.keyboard = None;
                }
            }
            Action::Menu => return Typed::Run,
            other => return Typed::Pass(other),
        }
        Typed::Taken
    }

    /// Closes the keyboard and clears the old results for a search of the
    /// query, returning it; none when there is nothing to search for.
    /// `browse` stands in for an empty query: the chosen type's name.
    pub fn start(&mut self, browse: Option<String>) -> Option<String> {
        let typed = self.query.trim().to_string();
        let query = if typed.is_empty() { browse? } else { typed.clone() };
        self.keyboard = None;
        self.query = typed;
        self.searched = query.clone();
        self.results = Loadable::Loading;
        self.rows = ui::Rows::default();
        Some(query)
    }

    /// The games found for `query`; the caller lays out the ones its
    /// filters pass with [`Search::show`]. An answer to an earlier search
    /// is dropped, and false.
    pub fn found(&mut self, query: &str, games: &[Arc<Game>]) -> bool {
        if query != self.searched {
            return false;
        }
        let mut seen = std::collections::HashSet::new();
        let ids: Vec<i64> = games
            .iter()
            .map(|game| game.id)
            .filter(|id| seen.insert(*id))
            .collect();
        self.results = Loadable::Loaded(ids);
        true
    }

    /// Every game found, before filters.
    pub fn found_ids(&self) -> &[i64] {
        self.results.get().map(Vec::as_slice).unwrap_or(&[])
    }

    /// Lays out the results that passed the filters.
    pub fn show(&mut self, shown: Vec<i64>) {
        let sections = if shown.is_empty() {
            Vec::new()
        } else {
            vec![ui::Section {
                title: format!("Results for “{}” · {}", self.searched, shown.len()),
                games: shown,
                note: None,
                more: false,
                collection: None,
            }]
        };
        self.rows.set_sections(sections);
    }

    pub fn failed(&mut self, query: &str, error: String) {
        if query == self.searched {
            self.results = Loadable::Failed(error);
        }
    }

    /// What the tab says in place of results.
    pub fn placeholder(&self) -> String {
        match &self.results {
            Loadable::Loaded(ids) if !ids.is_empty() => format!(
                "None of the {} games found for “{}” run here with these filters",
                ids.len(),
                self.searched
            ),
            Loadable::Loaded(_) => format!("No games found for “{}”", self.searched),
            Loadable::Failed(error) => error.clone(),
            Loadable::NotLoaded | Loadable::Loading => "Search itch.io for games".to_string(),
        }
    }

    /// The footer while the keyboard is open.
    pub fn keyboard_hints(&self) -> Option<Vec<(Vec<Glyph>, String)>> {
        let keyboard = self.keyboard.as_ref()?;
        let confirm = match keyboard.focused() {
            Key::Char(_) | Key::Space => "Type",
            Key::Delete => "Delete",
            Key::Done => "Search",
        };
        let back = if self.query.is_empty() {
            "Close"
        } else {
            "Delete"
        };
        Some(vec![
            (vec![Glyph::Navigate], "Keys".to_string()),
            (vec![Glyph::Confirm], confirm.to_string()),
            (vec![Glyph::Secondary], "Space".to_string()),
            (vec![Glyph::Back], back.to_string()),
            (vec![Glyph::Menu], "Search".to_string()),
        ])
    }
}

fn push(query: &mut String, c: char) {
    if query.chars().count() < MAX_QUERY {
        query.push(c);
    }
}
