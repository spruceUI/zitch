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
    /// The query the results are for, or the type browsed.
    searched: String,
    /// What was sent: the query, or the browse path.
    asked: String,
    /// The games found, by id, in itch.io's order.
    pub results: Loadable<Vec<i64>>,
    /// The browse page to fetch next, while there are more.
    next_page: Option<u32>,
    loading_more: bool,
    pub rows: ui::Rows,
    /// The one playable type to show, from `playable::playable_types`.
    pub type_filter: Option<String>,
}

/// What to ask itch.io for.
pub enum Asked {
    Query(String),
    /// A browse listing's path, like `tag-nes`.
    Browse(String),
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
    /// query; none when there is nothing to search for. `browse` stands in
    /// for an empty query: the chosen type's name and browse path.
    pub fn start(&mut self, browse: Option<(String, String)>) -> Option<Asked> {
        let typed = self.query.trim().to_string();
        let (searched, asked) = if typed.is_empty() {
            let (label, path) = browse?;
            (label, Asked::Browse(path))
        } else {
            (typed.clone(), Asked::Query(typed.clone()))
        };
        self.keyboard = None;
        self.query = typed;
        self.searched = searched;
        self.asked = match &asked {
            Asked::Query(query) | Asked::Browse(query) => query.clone(),
        };
        self.results = Loadable::Loading;
        self.next_page = None;
        self.loading_more = false;
        self.rows = ui::Rows::default();
        Some(asked)
    }

    /// The browse path and page to fetch next, once per page.
    pub fn more(&mut self) -> Option<(String, u32)> {
        if self.loading_more || self.results.get().is_none() {
            return None;
        }
        let page = self.next_page?;
        self.loading_more = true;
        Some((self.asked.clone(), page))
    }

    /// The games found for `query`, after those already found when they
    /// are a further browse page; the caller lays out the ones its
    /// filters pass with [`Search::show`]. An answer to an earlier search
    /// is dropped, and false.
    pub fn found(&mut self, query: &str, games: &[Arc<Game>], next_page: Option<u32>) -> bool {
        if query != self.asked {
            return false;
        }
        let mut ids = match std::mem::take(&mut self.results) {
            Loadable::Loaded(ids) if self.loading_more => ids,
            _ => Vec::new(),
        };
        for game in games {
            if !ids.contains(&game.id) {
                ids.push(game.id);
            }
        }
        self.results = Loadable::Loaded(ids);
        self.next_page = next_page;
        self.loading_more = false;
        true
    }

    /// Every game found, before filters.
    pub fn found_ids(&self) -> &[i64] {
        self.results.get().map(Vec::as_slice).unwrap_or(&[])
    }

    /// Lays out the results that passed the filters.
    pub fn show(&mut self, shown: Vec<i64>) {
        let sections = if shown.is_empty() && self.next_page.is_none() {
            Vec::new()
        } else {
            vec![ui::Section {
                title: format!("Results for “{}” · {}", self.searched, shown.len()),
                games: shown,
                note: None,
                more: self.next_page.is_some(),
                collection: None,
            }]
        };
        self.rows.set_sections(sections);
    }

    /// The search for `query` failed. A further browse page that failed
    /// keeps what was found and gives back the error to show.
    pub fn failed(&mut self, query: &str, error: String) -> Option<String> {
        if query != self.asked {
            return None;
        }
        if self.loading_more {
            self.loading_more = false;
            self.next_page = None;
            return Some(error);
        }
        self.results = Loadable::Failed(error);
        None
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
