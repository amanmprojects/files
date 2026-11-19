//! The grid view's selection and keyboard movement. The list view keeps its selection in the
//! table's own state; the grid keeps the same things (selected names, the keyboard cursor and
//! the anchor Shift extends from) here.

use std::collections::HashSet;

use rune::Key;

#[derive(Default, Clone, Debug)]
pub struct GridSel {
    pub selected: HashSet<String>,
    pub cursor: Option<String>,
    anchor: Option<String>,
}

impl GridSel {
    pub fn is_selected(&self, id: &str) -> bool {
        self.selected.contains(id)
    }

    /// Selects `id` alone and puts the cursor on it.
    pub fn select(&mut self, id: impl Into<String>) {
        let id = id.into();
        self.selected.clear();
        self.selected.insert(id.clone());
        self.cursor = Some(id.clone());
        self.anchor = Some(id);
    }

    pub fn clear(&mut self) {
        self.selected.clear();
    }

    /// A selection carried over from the list: its names and cursor.
    pub fn from_parts(selected: impl IntoIterator<Item = String>, cursor: Option<String>) -> GridSel {
        let selected: HashSet<String> = selected.into_iter().collect();
        let cursor = cursor.or_else(|| selected.iter().next().cloned());
        GridSel { anchor: cursor.clone(), cursor, selected }
    }

    pub fn select_all(&mut self, names: &[&str]) {
        self.selected = names.iter().map(|n| (*n).to_owned()).collect();
    }

    pub fn cursor_index(&self, names: &[&str]) -> Option<usize> {
        let c = self.cursor.as_deref()?;
        names.iter().position(|n| *n == c)
    }

    /// A click on `id`: alone, Ctrl toggles it, Shift selects from the anchor to it.
    pub fn click(&mut self, id: &str, names: &[&str], ctrl: bool, shift: bool) {
        if shift && let Some(i) = names.iter().position(|n| *n == id) {
            return self.move_to(i, names, true);
        }
        if ctrl {
            if !self.selected.remove(id) {
                self.selected.insert(id.to_owned());
            }
            self.cursor = Some(id.to_owned());
            self.anchor = Some(id.to_owned());
            return;
        }
        self.select(id);
    }

    /// Moves the cursor to item `i`, selecting it alone or (`extend`) everything from the
    /// anchor to it.
    pub fn move_to(&mut self, i: usize, names: &[&str], extend: bool) {
        let Some(id) = names.get(i) else { return };
        if !extend {
            return self.select(*id);
        }
        let anchor = self.anchor.as_deref().and_then(|a| names.iter().position(|n| *n == a)).unwrap_or(i);
        let (a, b) = (anchor.min(i), anchor.max(i));
        self.selected = names[a..=b].iter().map(|n| (*n).to_owned()).collect();
        self.cursor = Some((*id).to_owned());
        if self.anchor.is_none() {
            self.anchor = Some((*id).to_owned());
        }
    }
}

/// Where an arrow, Home, End or Page key takes the cursor from item `cur` of `len` laid out
/// `cols` to a row, `page` rows to a screen. None: not a movement key.
pub fn target(key: &Key, cur: Option<usize>, len: usize, cols: usize, page: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let cols = cols.max(1);
    let last = len - 1;
    let Some(c) = cur else {
        return matches!(key, Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End | Key::PageUp | Key::PageDown)
            .then(|| if *key == Key::End { last } else { 0 });
    };
    Some(match key {
        Key::Left => c.saturating_sub(1),
        Key::Right => (c + 1).min(last),
        Key::Up => c.checked_sub(cols).unwrap_or(c),
        // Down from a row above the last lands in the last row, at its end if it's short.
        Key::Down if c / cols < last / cols => (c + cols).min(last),
        Key::Down => c,
        Key::Home => 0,
        Key::End => last,
        Key::PageUp => c.saturating_sub(cols * page.max(1)),
        Key::PageDown => (c + cols * page.max(1)).min(last),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves() {
        // 10 items, 4 to a row: 0-3, 4-7, 8-9.
        assert_eq!(target(&Key::Down, Some(1), 10, 4, 2), Some(5));
        assert_eq!(target(&Key::Down, Some(6), 10, 4, 2), Some(9));
        assert_eq!(target(&Key::Down, Some(9), 10, 4, 2), Some(9));
        assert_eq!(target(&Key::Up, Some(2), 10, 4, 2), Some(2));
        assert_eq!(target(&Key::Up, Some(9), 10, 4, 2), Some(5));
        assert_eq!(target(&Key::Left, Some(0), 10, 4, 2), Some(0));
        assert_eq!(target(&Key::Right, Some(9), 10, 4, 2), Some(9));
        assert_eq!(target(&Key::PageDown, Some(0), 10, 4, 2), Some(8));
        assert_eq!(target(&Key::End, None, 10, 4, 2), Some(9));
        assert_eq!(target(&Key::Enter, Some(0), 10, 4, 2), None);
    }

    #[test]
    fn selects() {
        let names = ["a", "b", "c", "d", "e"];
        let mut s = GridSel::default();
        s.click("b", &names, false, false);
        s.click("d", &names, false, true);
        assert_eq!(s.selected.len(), 3);
        s.click("c", &names, true, false);
        assert!(!s.is_selected("c") && s.is_selected("b") && s.is_selected("d"));
        s.move_to(4, &names, true);
        // Shift extends from the last Ctrl-clicked item.
        assert_eq!(s.selected, ["c", "d", "e"].iter().map(|n| n.to_string()).collect());
        s.move_to(0, &names, false);
        assert_eq!(s.selected.len(), 1);
        assert_eq!(s.cursor.as_deref(), Some("a"));
    }
}
