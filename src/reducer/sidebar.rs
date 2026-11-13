//! Reducer for the left sidebar (explorer + source control).
//!
//! Everything here is a thin driver over `Sidebar`: the trees own their own cursor and
//! expansion state (exactly like Browse mode), and opening a file funnels through
//! `EditorState::open_file` so the sidebar cannot invent a second way to load a buffer.

use crate::reducer::EventResult;
use crate::sidebar::{Sidebar, SidebarTab};
use crate::state::EditorState;
use crate::state::editor::StatusKind;

/// Ensure the sidebar exists and put the keyboard in it.
///
/// Pressing the same key again while that tab is focused hides the panel — the same
/// toggle VS Code's explorer binding has, so one key both opens and closes it.
pub fn focus(editor: &mut EditorState, tab: SidebarTab) -> EventResult {
    if editor.sidebar.is_none() {
        editor.sidebar = Some(Sidebar::new(&editor._working_dir));
    }
    let Some(sidebar) = editor.sidebar.as_mut() else {
        return EventResult::Continue;
    };
    if sidebar.focused && sidebar.tab == tab {
        sidebar.focused = false;
        sidebar.visible = false;
        editor.set_status_message("Sidebar: hidden".to_string(), StatusKind::Info, false);
        return EventResult::Continue;
    }
    sidebar.visible = true;
    sidebar.focused = true;
    if sidebar.tab != tab {
        sidebar.tab = tab;
        // Leaving the git tab is the moment its list can have gone stale (a build, a
        // formatter, another window); re-reading here keeps it honest without a tick.
        if tab == SidebarTab::Git {
            sidebar.refresh_git();
        }
    }
    editor.set_status_message(
        format!("Sidebar: {} (Esc to edit)", tab.name()),
        StatusKind::Info,
        false,
    );
    EventResult::Continue
}

pub fn unfocus(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        sidebar.focused = false;
    }
    editor.status_message = None;
    EventResult::Continue
}

pub fn move_up(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.move_up(),
            SidebarTab::Git => sidebar.git.move_up(),
        }
    }
    EventResult::Continue
}

pub fn move_down(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.move_down(),
            SidebarTab::Git => sidebar.git.move_down(),
        }
    }
    EventResult::Continue
}

pub fn goto_top(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.goto_top(),
            SidebarTab::Git => sidebar.git.goto_top(),
        }
    }
    EventResult::Continue
}

pub fn goto_bottom(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.goto_bottom(),
            SidebarTab::Git => sidebar.git.goto_bottom(),
        }
    }
    EventResult::Continue
}

/// Enter / `l` / →: open the selected file (or toggle the selected folder).
pub fn open(editor: &mut EditorState) -> EventResult {
    let target = {
        let Some(sidebar) = editor.sidebar.as_mut() else {
            return EventResult::Continue;
        };
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.expand_or_open(),
            SidebarTab::Git => {
                // A deleted file still has a row — that is how you learn it is gone — but
                // there is nothing to open. Say so instead of failing to read it.
                if sidebar.git.selected_file().is_some_and(|f| f.deleted()) {
                    let name = sidebar
                        .git
                        .selected_file()
                        .map(|f| f.rel.display().to_string())
                        .unwrap_or_default();
                    editor.set_status_message(
                        format!("Deleted file: {name}"),
                        StatusKind::Info,
                        false,
                    );
                    return EventResult::Continue;
                }
                sidebar.git.expand_or_open()
            }
        }
    };

    let Some(path) = target else {
        return EventResult::Continue;
    };
    let path_str = path.to_string_lossy().to_string();
    let result = editor.open_file(&path_str);
    // Opening a file hands the keyboard back to the text, like VS Code: the panel stays
    // open so the next file is one click away.
    editor.enter_mode(editor.home_mode());
    if let Some(sidebar) = editor.sidebar.as_mut() {
        sidebar.focused = false;
    }
    match result {
        Ok(_) => crate::reducer::status::set_success(editor, "Opened", &path_str),
        Err(e) => crate::reducer::status::set_error(editor, &e.to_string()),
    }
    EventResult::Continue
}

/// `h` / ←: close the selected folder, otherwise step to its parent.
pub fn collapse(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.collapse_or_parent(),
            SidebarTab::Git => sidebar.git.collapse_or_parent(),
        }
    }
    EventResult::Continue
}

/// Tab: switch between the explorer and the source-control view.
pub fn next_tab(editor: &mut EditorState) -> EventResult {
    let Some(sidebar) = editor.sidebar.as_mut() else {
        return EventResult::Continue;
    };
    sidebar.tab = sidebar.tab.next();
    if sidebar.tab == SidebarTab::Git {
        sidebar.refresh_git();
    }
    let tab = sidebar.tab;
    editor.set_status_message(format!("Sidebar: {}", tab.name()), StatusKind::Info, false);
    EventResult::Continue
}

/// `r`: re-read `git status` for the folder.
pub fn refresh(editor: &mut EditorState) -> EventResult {
    let Some(sidebar) = editor.sidebar.as_mut() else {
        return EventResult::Continue;
    };
    sidebar.refresh_git();
    let (repo, count) = (sidebar.git.repo, sidebar.git.count());
    let msg = if !repo {
        "Not a git repository".to_string()
    } else if count == 0 {
        "Working tree clean".to_string()
    } else {
        format!("{count} change(s)")
    };
    crate::reducer::status::set_info(editor, &msg);
    EventResult::Continue
}

/// After a save: refresh the change list if the sidebar is showing it, so the file's
/// row goes from modified to clean without the user asking.
pub fn after_save(editor: &mut EditorState) {
    let showing_git = editor
        .sidebar
        .as_ref()
        .is_some_and(|s| s.visible && s.tab == SidebarTab::Git);
    if showing_git && let Some(sidebar) = editor.sidebar.as_mut() {
        sidebar.refresh_git();
    }
}

// ── mouse ─────────────────────────────────────────────────────────────────────
//
// Everything below answers questions about `editor.ui_hit` — the rectangles the last
// frame drew — instead of recomputing the layout. That is the same shape WA-Bot's
// `desktop-cli` uses (`Hit` + `on_mouse`): one source of truth for "what is at (col,row)",
// so wrapping, scrolling and a half-drawn tree can never make a click land elsewhere.

use crate::action::MouseButton;
use crate::sidebar::SidebarHit;

/// A press. `col`/`row` are screen cells, 0-based.
pub fn mouse_down(
    editor: &mut EditorState,
    button: MouseButton,
    col: u16,
    row: u16,
) -> EventResult {
    if button != MouseButton::Left {
        return EventResult::Continue;
    }
    let hit = editor.ui_hit.clone();

    // 1. The divider: grabbing it starts a resize that follows the pointer until release.
    if let Some(divider) = hit.divider
        && col == divider
        && row >= hit.sidebar.y
        && row < hit.sidebar.bottom()
    {
        editor.sidebar_drag = true;
        return EventResult::Continue;
    }

    // 2. The panel: header tabs, then rows.
    if hit.sidebar.contains(col, row) {
        if let Some(tab) = hit.tab_at(col, row) {
            return select_tab(editor, tab);
        }
        if let Some(SidebarHit::Row(node)) = hit.row_at(col, row) {
            return click_row(editor, node);
        }
        return EventResult::Continue;
    }

    // 3. The buffer: put the cursor where the character was drawn.
    if hit.text_area.contains(col, row)
        && matches!(
            editor.mode,
            crate::state::EditorMode::Edit | crate::state::EditorMode::Glide
        )
        && let Some((line, byte)) = crate::ui::render::body::click_to_cursor(editor, col, row)
    {
        editor.cursor.place(line, byte);
        if let Some(sidebar) = editor.sidebar.as_mut() {
            sidebar.focused = false;
        }
        editor.status_message = None;
    }
    EventResult::Continue
}

/// A click on a header tab shows that tab and gives it the keyboard. Unlike `Alt+B`/`Alt+G`
/// it never hides the panel — a click on a tab that is already open must not close it.
pub fn select_tab(editor: &mut EditorState, tab: SidebarTab) -> EventResult {
    let Some(sidebar) = editor.sidebar.as_mut() else {
        return EventResult::Continue;
    };
    sidebar.visible = true;
    sidebar.focused = true;
    if sidebar.tab == tab {
        return EventResult::Continue;
    }
    sidebar.tab = tab;
    if tab == SidebarTab::Git {
        sidebar.refresh_git();
    }
    EventResult::Continue
}

/// A click on a tree row: select it, then act — a folder opens or closes, a file opens.
/// One click, because a terminal has no hover and no second click to waste.
fn click_row(editor: &mut EditorState, node: usize) -> EventResult {
    {
        let Some(sidebar) = editor.sidebar.as_mut() else {
            return EventResult::Continue;
        };
        if !sidebar.visible {
            return EventResult::Continue;
        }
        match sidebar.tab {
            SidebarTab::Files => {
                if node >= sidebar.explorer.nodes.len() {
                    return EventResult::Continue;
                }
                sidebar.explorer.selected = node;
            }
            SidebarTab::Git => {
                if node >= sidebar.git.nodes.len() {
                    return EventResult::Continue;
                }
                sidebar.git.selected = node;
            }
        }
        // Selection is also a focus move: the keyboard should follow the eye.
        sidebar.focused = true;
    }
    open(editor)
}

/// Dragging the divider. The panel keeps a sane range no matter where the pointer is.
pub fn mouse_drag(editor: &mut EditorState, col: u16, row: u16) -> EventResult {
    if !editor.sidebar_drag {
        return EventResult::Continue;
    }
    let _ = row;
    let left = editor.ui_hit.sidebar.x;
    let total = editor.ui_hit.screen.w;
    // Keep at least half the screen for the text, like `width_for` does.
    let ceiling = (total / 2).clamp(20, crate::sidebar::MAX_WIDTH);
    let want = col.saturating_sub(left).clamp(20, ceiling);
    if let Some(sidebar) = editor.sidebar.as_mut() {
        sidebar.width = want;
    }
    EventResult::Continue
}

pub fn mouse_up(editor: &mut EditorState) -> EventResult {
    editor.sidebar_drag = false;
    EventResult::Continue
}

/// The wheel. Over the panel it walks the tree; anywhere else it means exactly what it
/// always meant (scroll the reading view, or move the cursor while editing).
pub fn mouse_scroll(editor: &mut EditorState, delta: isize, col: u16, row: u16) -> EventResult {
    let over_panel = editor.sidebar_on_screen() && editor.ui_hit.sidebar.contains(col, row);
    if over_panel {
        let Some(sidebar) = editor.sidebar.as_mut() else {
            return EventResult::Continue;
        };
        let steps = delta.unsigned_abs();
        for _ in 0..steps {
            if delta.is_negative() {
                match sidebar.tab {
                    SidebarTab::Files => sidebar.explorer.move_up(),
                    SidebarTab::Git => sidebar.git.move_up(),
                }
            } else {
                match sidebar.tab {
                    SidebarTab::Files => sidebar.explorer.move_down(),
                    SidebarTab::Git => sidebar.git.move_down(),
                }
            }
        }
        return EventResult::Continue;
    }
    crate::reducer::reduce(editor, crate::action::Action::Wheel(delta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::MouseButton;
    use crate::sidebar::{Area, SidebarHit, UiHit};
    use std::fs;

    /// An editor with a real (tiny) folder behind the panel.
    fn editor_with_panel() -> (tempfile::TempDir, EditorState) {
        let tmp = tempfile::Builder::new()
            .prefix("cozy-sidebar-mouse-")
            .tempdir()
            .unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::write(tmp.path().join("README.md"), "hello\n").unwrap();
        fs::write(tmp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let mut editor = EditorState::new(None);
        editor.enter_mode(crate::state::EditorMode::Edit);
        editor._working_dir = tmp.path().to_path_buf();
        editor.sidebar = Some(Sidebar::new(tmp.path()));
        // Geometry as a frame would have recorded it: header on row 0, rows from row 2.
        editor.ui_hit = UiHit {
            screen: Area::new(0, 0, 100, 30),
            sidebar: Area::new(0, 0, 34, 24),
            divider: Some(33),
            header: Area::new(1, 0, 32, 1),
            tabs: vec![(SidebarTab::Files, 1, 8), (SidebarTab::Git, 10, 19)],
            body: Area::new(1, 2, 32, 22),
            rows: Vec::new(), // filled below from the tree's own visible order
            ..Default::default()
        };
        let rows = {
            let tree = &editor.sidebar.as_ref().unwrap().explorer;
            tree.visible_nodes()
                .into_iter()
                .map(SidebarHit::Row)
                .collect()
        };
        editor.ui_hit.rows = rows;
        (tmp, editor)
    }

    #[test]
    fn clicking_a_row_selects_it_and_opens_a_file() {
        let (_tmp, mut editor) = editor_with_panel();
        // Which node sits at row 2 of the demo tree — the click must land on that one.
        let row2 = {
            let tree = &editor.sidebar.as_ref().unwrap().explorer;
            tree.visible_nodes()[2]
        };
        assert!(
            !editor.sidebar.as_ref().unwrap().explorer.nodes[row2].is_dir,
            "row 2 is a file in this fixture"
        );
        let row_y = editor.ui_hit.body.y + 2;
        let _ = mouse_down(&mut editor, MouseButton::Left, 5, row_y);
        assert_eq!(
            editor.sidebar.as_ref().unwrap().explorer.selected,
            row2,
            "the click moved the selection to the row under the pointer"
        );
        assert!(editor.filename.is_some(), "clicking a file row opens it");
    }

    #[test]
    fn clicking_the_git_tab_switches_without_hiding() {
        let (_tmp, mut editor) = editor_with_panel();
        let _ = mouse_down(&mut editor, MouseButton::Left, 14, 0);
        let sidebar = editor.sidebar.as_ref().unwrap();
        assert_eq!(sidebar.tab, SidebarTab::Git);
        assert!(sidebar.visible, "a click on a tab never hides the panel");
        assert!(sidebar.focused);

        // And clicking Files again comes back, still visible.
        let _ = mouse_down(&mut editor, MouseButton::Left, 3, 0);
        assert_eq!(editor.sidebar.as_ref().unwrap().tab, SidebarTab::Files);
        assert!(editor.sidebar.as_ref().unwrap().visible);
    }

    #[test]
    fn dragging_the_divider_resizes_within_bounds() {
        let (_tmp, mut editor) = editor_with_panel();
        let _ = mouse_down(&mut editor, MouseButton::Left, 33, 4);
        assert!(editor.sidebar_drag);
        let _ = mouse_drag(&mut editor, 46, 4);
        assert_eq!(editor.sidebar.as_ref().unwrap().width, 46);
        // Never more than half the screen — the text keeps its side of the deal.
        let _ = mouse_drag(&mut editor, 90, 4);
        assert_eq!(editor.sidebar.as_ref().unwrap().width, 50);
        let _ = mouse_up(&mut editor);
        assert!(!editor.sidebar_drag);
    }

    #[test]
    fn the_wheel_over_the_panel_walks_the_tree() {
        let (_tmp, mut editor) = editor_with_panel();
        let before = editor.sidebar.as_ref().unwrap().explorer.selected;
        let _ = mouse_scroll(&mut editor, 1, 5, 4);
        assert_ne!(editor.sidebar.as_ref().unwrap().explorer.selected, before);
    }

    /// Outside the panel the wheel is the wheel it has always been (cursor motion here).
    #[test]
    fn the_wheel_outside_the_panel_keeps_its_old_meaning() {
        let (_tmp, mut editor) = editor_with_panel();
        let before = editor.sidebar.as_ref().unwrap().explorer.selected;
        let _ = mouse_scroll(&mut editor, 1, 60, 4);
        assert_eq!(editor.sidebar.as_ref().unwrap().explorer.selected, before);
    }
}
