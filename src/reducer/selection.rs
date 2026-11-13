//! Mouse selection and the clipboard copy that follows it.
//!
//! The terminal hands cozy the mouse while the panel is up, which means the terminal's
//! own drag-select is gone (Shift gets it back). Rather than ask the user to hold a
//! modifier, cozy does the selection itself: a press anchors, motion extends, release
//! copies. `Ctrl+C` copies again (and is the way to copy after a double-click, where no
//! drag happened).
//!
//! The selection is a pair of buffer positions, so it survives wrapping and horizontal
//! scrolling: only the *drawing* has to translate it into screen cells.

use crate::reducer::EventResult;
use crate::state::{EditorMode, EditorState, Selection, StatusKind};

/// How close two presses have to be, in time and place, to count as a double-click.
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

/// A press in the text: put the cursor there and start a selection. A second press in
/// the same place a moment later selects the word instead — the same gesture as VS Code,
/// and the only way to grab a word without pixel-precise dragging.
pub fn press(editor: &mut EditorState, line: usize, byte: usize) -> EventResult {
    let now = std::time::Instant::now();
    let double = editor.last_press.is_some_and(|(at, pos)| {
        now.duration_since(at) < DOUBLE_CLICK && pos.0 == line && pos.1.abs_diff(byte) <= 1
    });
    editor.last_press = Some((now, (line, byte)));
    editor.cursor.place(line, byte);
    editor.selection = Some(if double {
        let (start, end) = word_at(editor, line, byte);
        Selection {
            anchor: (line, start),
            head: (line, end),
        }
    } else {
        Selection::at((line, byte))
    });
    editor.select_drag = true;
    EventResult::Continue
}

/// The pointer moved with the button held. Moving outside the text keeps the last head —
/// the gesture is about the text, and the sidebar is not part of it.
pub fn drag(editor: &mut EditorState, col: u16, row: u16) -> EventResult {
    if !editor.select_drag || !matches!(editor.mode, EditorMode::Edit | EditorMode::Glide) {
        return EventResult::Continue;
    }
    if let Some((line, byte)) = crate::ui::render::body::click_to_cursor(editor, col, row)
        && let Some(selection) = editor.selection.as_mut()
    {
        selection.head = (line, byte);
        editor.cursor.place(line, byte);
    }
    EventResult::Continue
}

/// The button came up: a drag that selected something is copied; a plain click is not
/// (an empty selection is just the cursor moving).
pub fn finish(editor: &mut EditorState) -> EventResult {
    if !editor.select_drag {
        return EventResult::Continue;
    }
    editor.select_drag = false;
    if editor
        .selection
        .is_some_and(|selection| selection.is_empty())
    {
        editor.selection = None;
        return EventResult::Continue;
    }
    copy(editor)
}

/// `Ctrl+C` (and the end of a drag): put the selection on the system clipboard.
pub fn copy(editor: &mut EditorState) -> EventResult {
    let Some((text, lines)) = editor.selected_text() else {
        return EventResult::Continue;
    };
    let chars = text.chars().count();
    let message = if crate::clipboard_io::set_text_ok(&text) {
        editor.set_status_message(
            format!("Copied {lines} line(s), {chars} char(s)"),
            StatusKind::Success,
            false,
        );
        return EventResult::Continue;
    } else {
        "Copy failed: no system clipboard".to_string()
    };
    editor.set_status_message(message, StatusKind::Error, false);
    EventResult::Continue
}

/// The word around `byte`, or the single character under it when it is punctuation — a
/// double-click on `,` selecting the whole line would be a surprise.
fn word_at(editor: &EditorState, line: usize, byte: usize) -> (usize, usize) {
    let Some(text) = editor.buffer.lines.get(line) else {
        return (byte, byte);
    };
    // The character under the pointer. A press at the very end of a line has no
    // character to its right, so it grabs the last one instead of nothing.
    let mut pos = byte.min(text.len());
    while pos > 0 && !text.is_char_boundary(pos) {
        pos -= 1;
    }
    let (start, ch) = match text[pos..].chars().next() {
        Some(ch) => (pos, ch),
        None => match text[..pos].char_indices().next_back() {
            Some((i, ch)) => (i, ch),
            None => return (0, 0),
        },
    };
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    if !is_word(ch) {
        return (start, start + ch.len_utf8());
    }
    let mut from = start;
    for (i, c) in text[..start].char_indices().rev() {
        if is_word(c) {
            from = i;
        } else {
            break;
        }
    }
    let base = start + ch.len_utf8();
    let mut to = base;
    for (i, c) in text[base..].char_indices() {
        if !is_word(c) {
            break;
        }
        to = base + i + c.len_utf8();
    }
    (from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TextBuffer;

    fn editor_with(lines: &[&str]) -> EditorState {
        let mut editor = EditorState::new(None);
        editor.buffer = TextBuffer::from_lines(lines.iter().map(|s| s.to_string()).collect());
        editor.page_size = 10;
        editor.enter_mode(EditorMode::Edit);
        editor
    }

    /// A double press on a word selects it — the gesture that makes "copy this
    /// identifier" possible without dragging.
    #[test]
    fn a_double_press_selects_the_word() {
        let mut editor = editor_with(&["let value = compute_count(2);"]);
        press(&mut editor, 0, 5);
        press(&mut editor, 0, 5);
        let (text, lines) = editor.selected_text().expect("selected");
        assert_eq!(text, "value");
        assert_eq!(lines, 1);
    }

    /// ... and punctuation selects just the character, not the line.
    #[test]
    fn a_double_press_on_punctuation_selects_one_character() {
        let mut editor = editor_with(&["a, b"]);
        press(&mut editor, 0, 1);
        press(&mut editor, 0, 1);
        assert_eq!(editor.selected_text().unwrap().0, ",");
    }

    /// Dragging across lines copies them newline-joined, which is what a paste expects.
    #[test]
    fn a_multi_line_selection_joins_with_newlines() {
        let mut editor = editor_with(&["one", "two", "three"]);
        editor.selection = Some(Selection {
            anchor: (0, 1),
            head: (2, 2),
        });
        assert_eq!(
            editor.selected_text().unwrap(),
            ("ne\ntwo\nth".to_string(), 3)
        );
    }

    /// A plain click is not a copy: the release clears the empty selection.
    #[test]
    fn a_click_without_a_drag_copies_nothing() {
        let mut editor = editor_with(&["abc"]);
        press(&mut editor, 0, 1);
        finish(&mut editor);
        assert!(editor.selection.is_none());
        assert!(editor.selected_text().is_none());
    }

    /// Dragging backwards selects the same text as dragging forwards.
    #[test]
    fn a_backwards_drag_selects_the_same_span() {
        let mut editor = editor_with(&["abcdef"]);
        editor.selection = Some(Selection {
            anchor: (0, 4),
            head: (0, 1),
        });
        assert_eq!(editor.selected_text().unwrap().0, "bcd");
    }
}
