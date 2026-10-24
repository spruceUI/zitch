//! The on-screen keyboard, for typing a search with a controller.

use egui::{Align2, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::actions::{Action, Direction, wrap_step};
use crate::ui::{self, Metrics};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Space,
    Delete,
    Done,
}

impl Key {
    fn label(self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Space => "Space".to_string(),
            Key::Delete => "Delete".to_string(),
            Key::Done => "Search".to_string(),
        }
    }
}

/// How many columns wide every row is.
const COLUMNS: usize = 10;

/// The keys row by row, each with the columns it spans.
fn layout() -> Vec<Vec<(Key, usize)>> {
    let chars = |text: &str| -> Vec<(Key, usize)> {
        text.chars().map(|c| (Key::Char(c), 1)).collect()
    };
    let mut rows = vec![
        chars("1234567890"),
        chars("qwertyuiop"),
        chars("asdfghjkl-"),
        chars("zxcvbnm"),
        vec![(Key::Space, 6), (Key::Done, 4)],
    ];
    rows[3].push((Key::Delete, 3));
    rows
}

/// The column in the middle of a row's key, for moving up and down.
fn middle(row: &[(Key, usize)], key: usize) -> usize {
    let start: usize = row[..key].iter().map(|(_, span)| span).sum();
    start + row[key].1 / 2
}

/// The key of a row that covers `column`.
fn key_at(row: &[(Key, usize)], column: usize) -> usize {
    let mut end = 0;
    for (index, (_, span)) in row.iter().enumerate() {
        end += span;
        if column < end {
            return index;
        }
    }
    row.len() - 1
}

/// Which key has focus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Keyboard {
    row: usize,
    key: usize,
}

impl Keyboard {
    pub fn focused(&self) -> Key {
        layout()[self.row][self.key].0
    }

    pub fn focus(&mut self, row: usize, key: usize) {
        if layout().get(row).is_some_and(|keys| key < keys.len()) {
            self.row = row;
            self.key = key;
        }
    }

    pub fn step(&mut self, direction: Direction) {
        let rows = layout();
        let column = middle(&rows[self.row], self.key);
        let row = match direction {
            Direction::Left | Direction::Right => {
                self.key = wrap_step(self.key, rows[self.row].len(), direction);
                return;
            }
            Direction::Home => {
                self.key = 0;
                return;
            }
            Direction::End => {
                self.key = rows[self.row].len() - 1;
                return;
            }
            Direction::Up | Direction::Down => wrap_step(self.row, rows.len(), direction),
            Direction::PageUp | Direction::Top => 0,
            Direction::PageDown | Direction::Bottom => rows.len() - 1,
        };
        self.row = row;
        self.key = key_at(&rows[row], column);
    }
}

/// The query being typed over the keys. A click focuses a key and
/// presses it.
pub fn draw(
    ui: &mut Ui,
    m: &Metrics,
    keyboard: &Keyboard,
    query: &str,
    actions: &mut Vec<Action>,
) {
    let rows = layout();
    let area = ui.available_rect_before_wrap();
    let gap = m.space(6.0);
    let width = area.width().min(m.space(560.0));
    let left = area.center().x - width / 2.0;
    let field_height = m.space(34.0);
    let count = rows.len() as f32;
    let key_height = ((area.height() - field_height - gap * (count + 2.0)) / count)
        .clamp(m.space(16.0), m.space(34.0));
    let key_width = (width - gap * (COLUMNS as f32 - 1.0)) / COLUMNS as f32;
    let border = m.space(1.25).max(1.0);
    let font = ui::bold(m.button);

    let field = Rect::from_min_size(pos2(left, area.top() + gap), vec2(width, field_height));
    ui::fill_squircle(
        ui,
        field,
        [m.space(12.0); 4],
        ui::SURFACE,
        Stroke::new(border, ui::BORDER_HOVER),
    );
    let inner = field.shrink2(vec2(m.space(14.0), 0.0));
    let (text, color) = if query.is_empty() {
        ("Search itch.io", ui::DIM)
    } else {
        (query, ui::TEXT)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(m.button), color);
    // The caret sits before the hint, or after what was typed.
    let (typed, offset) = if query.is_empty() {
        (0.0, m.space(6.0))
    } else {
        (galley.size().x, 0.0)
    };
    // A long query shows its end, where the typing is.
    let x = inner.left().min(inner.right() - typed - m.space(4.0));
    let painter = ui.painter().with_clip_rect(inner);
    painter.galley(
        pos2(x + offset, field.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    let caret = Rect::from_center_size(
        pos2(x + typed + m.space(2.0), field.center().y),
        vec2(m.space(2.0), field_height * 0.5),
    );
    painter.rect_filled(caret, 0.0, ui::ACCENT);

    let pointer_moved = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    let radii = [m.space(8.0); 4];
    let mut y = field.bottom() + gap * 2.0;
    for (row, keys) in rows.iter().enumerate() {
        let mut x = left;
        for (index, (key, span)) in keys.iter().enumerate() {
            let span = *span as f32;
            let rect = Rect::from_min_size(
                pos2(x, y),
                vec2(key_width * span + gap * (span - 1.0), key_height),
            );
            x = rect.right() + gap;
            let focused = keyboard.row == row && keyboard.key == index;
            let response = ui.interact(rect, ui.id().with(("key", row, index)), Sense::click());
            if response.hovered() && pointer_moved && !focused {
                actions.push(Action::FocusKey { row, key: index });
            }
            if response.clicked() {
                actions.push(Action::FocusKey { row, key: index });
                actions.push(Action::Activate);
            }
            let (fill, edge) = match (*key == Key::Done, focused) {
                (true, true) => (ui::ACCENT, ui::ACCENT),
                (true, false) => (ui::FILTER_BORDER, ui::FILTER_BORDER),
                (false, true) => (ui::SURFACE_HOVER, ui::BORDER_HOVER),
                (false, false) => (ui::SURFACE, ui::BORDER),
            };
            ui::fill_squircle(ui, rect, radii, fill, Stroke::new(border, edge));
            if focused {
                ui::focus_ring(ui, rect, radii, m);
            }
            let color = if focused || *key == Key::Done {
                ui::TEXT
            } else {
                ui::DIM
            };
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                key.label(),
                font.clone(),
                color,
            );
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        y += key_height + gap;
    }
    ui.allocate_rect(
        Rect::from_min_max(area.min, pos2(area.max.x, y)),
        Sense::hover(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_fill_the_columns() {
        for keys in layout() {
            assert_eq!(keys.iter().map(|(_, span)| span).sum::<usize>(), COLUMNS);
        }
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let mut keyboard = Keyboard::default();
        keyboard.focus(3, 7);
        assert_eq!(keyboard.focused(), Key::Delete);
        keyboard.step(Direction::Down);
        assert_eq!(keyboard.focused(), Key::Done);
        keyboard.step(Direction::Down);
        assert_eq!(keyboard.focused(), Key::Char('9'));
        keyboard.step(Direction::Up);
        assert_eq!(keyboard.focused(), Key::Done);
        keyboard.step(Direction::Left);
        assert_eq!(keyboard.focused(), Key::Space);
    }
}
