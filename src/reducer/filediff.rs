//! The single-file diff view: what a Source Control row opens into.
//!
//! A read-only surface, one file at a time, with the same feel as `less`: `j`/`k` scroll,
//! `n`/`N` jump between hunks, `q`/`Esc` goes back. It is **not** `DiffReview` — that one
//! is the session-wide approval bench (`git add -p` territory). Reading one file and
//! approving a session are different jobs, and mixing them is how "I only looked" turns
//! into "I staged it".

use crate::reducer::EventResult;
use crate::state::EditorState;
use crate::state::diff::{FileDiffView, load_file_diff};

use crate::sidebar::GitEntry;

/// Open `entry`'s diff. The panel stays up, so the next change is one click away.
pub fn open(editor: &mut EditorState, entry: &GitEntry) -> EventResult {
    let root = editor
        .sidebar
        .as_ref()
        .map(|s| s.root.clone())
        .unwrap_or_else(|| editor._working_dir.clone());
    let untracked = entry.untracked();
    match load_file_diff(&root, &entry.file.rel, entry.staged, untracked) {
        Ok(rows) => {
            let view = FileDiffView::build(&entry.file.rel, entry.staged, rows);
            let title = view.title.clone();
            editor.file_diff = Some(view);
            editor.enter_mode(crate::state::EditorMode::FileDiff);
            // The view's own header and the status bar already name the file, so a
            // message would only repeat it.
            let _ = title;
        }
        Err(e) => crate::reducer::status::set_error(editor, &format!("git diff failed: {e}")),
    }
    EventResult::Continue
}

/// Scroll the viewport. The hunk counter follows the viewport, so `n`/`N` continue from
/// what is on screen instead of from where the last jump left off.
pub fn scroll(editor: &mut EditorState, delta: isize) -> EventResult {
    let Some(view) = editor.file_diff.as_mut() else {
        return EventResult::Continue;
    };
    let next = if delta.is_negative() {
        view.scroll.saturating_sub(delta.unsigned_abs())
    } else {
        view.scroll.saturating_add(delta as usize)
    };
    view.scroll = next.min(view.max_scroll());
    sync_current(view);
    EventResult::Continue
}

pub fn goto_top(editor: &mut EditorState) -> EventResult {
    if let Some(view) = editor.file_diff.as_mut() {
        view.scroll = 0;
        view.current = 0;
    }
    EventResult::Continue
}

pub fn goto_bottom(editor: &mut EditorState) -> EventResult {
    if let Some(view) = editor.file_diff.as_mut() {
        view.scroll = view.max_scroll();
        sync_current(view);
    }
    EventResult::Continue
}

/// `n` / `N`: put the next (or previous) `@@` header at the top of the screen.
pub fn next_hunk(editor: &mut EditorState, forward: bool) -> EventResult {
    let Some(view) = editor.file_diff.as_mut() else {
        return EventResult::Continue;
    };
    if view.hunks.is_empty() {
        return EventResult::Continue;
    }
    let last = view.hunks.len() - 1;
    // Where we are: the hunk at or above the top of the screen.
    let at = view
        .hunks
        .iter()
        .rposition(|&row| row <= view.scroll)
        .unwrap_or(0);
    let next = if forward {
        (at + 1).min(last)
    } else {
        at.saturating_sub(1)
    };
    view.current = next;
    let row = view.hunks[next];
    view.scroll = row.min(view.max_scroll());
    EventResult::Continue
}

/// `q` / `Esc`: back to the buffer, with the keyboard back in the panel — the next change
/// is one `j`/click away.
pub fn close(editor: &mut EditorState) -> EventResult {
    editor.file_diff = None;
    editor.enter_mode(editor.home_mode());
    if let Some(sidebar) = editor.sidebar.as_mut()
        && sidebar.visible
    {
        sidebar.focused = true;
    }
    editor.status_message = None;
    EventResult::Continue
}

/// Keep the counter pointing at the hunk the viewport is showing.
fn sync_current(view: &mut FileDiffView) {
    if view.hunks.is_empty() {
        view.current = 0;
        return;
    }
    view.current = view
        .hunks
        .iter()
        .rposition(|&row| row <= view.scroll)
        .unwrap_or(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::diff::{FileDiffKind, FileDiffRow};

    fn rows(hunks: usize) -> Vec<FileDiffRow> {
        let mut out = Vec::new();
        for h in 0..hunks {
            out.push(FileDiffRow {
                kind: FileDiffKind::Hunk,
                old: None,
                new: None,
                text: format!("@@ -{h},1 +{h},1 @@"),
            });
            for _ in 0..5 {
                out.push(FileDiffRow {
                    kind: FileDiffKind::Add,
                    old: None,
                    new: None,
                    text: "line".to_string(),
                });
            }
        }
        out
    }

    fn editor_with_view() -> EditorState {
        let mut editor = EditorState::new(None);
        let mut view = FileDiffView::build(std::path::Path::new("a.rs"), false, rows(3));
        view.view_height = 10;
        editor.file_diff = Some(view);
        editor.enter_mode(crate::state::EditorMode::FileDiff);
        editor
    }

    #[test]
    fn the_next_hunk_lands_at_the_top_of_the_screen() {
        let mut editor = editor_with_view();
        continued(next_hunk(&mut editor, true));
        let view = editor.file_diff.as_ref().unwrap();
        assert_eq!(view.current, 1);
        assert_eq!(view.scroll, view.hunks[1]);
    }

    #[test]
    fn the_last_hunk_stops_at_the_bottom_of_the_list() {
        let mut editor = editor_with_view();
        for _ in 0..10 {
            continued(next_hunk(&mut editor, true));
        }
        let view = editor.file_diff.as_ref().unwrap();
        assert_eq!(view.current, view.hunks.len() - 1);
        assert!(view.scroll <= view.max_scroll());
    }

    #[test]
    fn the_first_hunk_stops_at_the_top() {
        let mut editor = editor_with_view();
        continued(next_hunk(&mut editor, false));
        let view = editor.file_diff.as_ref().unwrap();
        assert_eq!(view.current, 0);
        assert_eq!(view.scroll, 0);
    }

    #[test]
    fn scrolling_updates_which_hunk_is_current() {
        let mut editor = editor_with_view();
        continued(scroll(&mut editor, 12)); // past the second hunk header
        let view = editor.file_diff.as_ref().unwrap();
        assert_eq!(view.scroll, view.max_scroll(), "digeser sampai ujung");
        assert_eq!(view.current, 1, "hunk kedua yang sedang terlihat");

        // From the top, the first hunk is the current one.
        let _ = goto_top(&mut editor);
        assert_eq!(editor.file_diff.as_ref().unwrap().current, 0);
    }

    #[test]
    fn closing_hands_the_keyboard_back_to_the_panel() {
        let mut editor = editor_with_view();
        continued(close(&mut editor));
        assert!(editor.file_diff.is_none());
        assert_eq!(editor.mode, crate::state::EditorMode::Edit);
    }

    fn continued(result: EventResult) {
        assert!(matches!(result, EventResult::Continue));
    }
}
