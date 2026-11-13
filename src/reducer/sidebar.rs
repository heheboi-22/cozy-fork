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
    // ⭐ On a narrow screen there is nothing to hide: the panel and the text take turns,
    // so this key means "show the list" and never folds it away — the same button that
    // collapses the panel on a wide screen goes back to the list here.
    let narrow = editor.narrow_screen();
    let Some(sidebar) = editor.sidebar.as_mut() else {
        return EventResult::Continue;
    };
    if !narrow && sidebar.focused && sidebar.tab == tab {
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

/// Put the list on screen and give it the keyboard. On a wide terminal this is just a
/// focus move; on a narrow one it is the "back" of the one-pane layout.
pub fn show_list(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        sidebar.visible = true;
        sidebar.focused = true;
    }
    editor.status_message = None;
    EventResult::Continue
}

/// Hide the panel on a wide terminal; on a narrow one, hand the screen back to the file
/// (the list stays one key away).
pub fn fold_panel(editor: &mut EditorState) -> EventResult {
    let narrow = editor.narrow_screen();
    let Some(sidebar) = editor.sidebar.as_mut() else {
        return EventResult::Continue;
    };
    sidebar.focused = false;
    if !narrow {
        sidebar.visible = false;
        editor.set_status_message("Sidebar: hidden".to_string(), StatusKind::Info, false);
    }
    EventResult::Continue
}

pub fn move_up(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        sidebar.scroll_parked = false;
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.move_up(),
            SidebarTab::Git => sidebar.git.move_up(),
        }
    }
    EventResult::Continue
}

pub fn move_down(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        sidebar.scroll_parked = false;
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.move_down(),
            SidebarTab::Git => sidebar.git.move_down(),
        }
    }
    EventResult::Continue
}

pub fn goto_top(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        sidebar.scroll_parked = false;
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.goto_top(),
            SidebarTab::Git => sidebar.git.goto_top(),
        }
    }
    EventResult::Continue
}

pub fn goto_bottom(editor: &mut EditorState) -> EventResult {
    if let Some(sidebar) = editor.sidebar.as_mut() {
        sidebar.scroll_parked = false;
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.goto_bottom(),
            SidebarTab::Git => sidebar.git.goto_bottom(),
        }
    }
    EventResult::Continue
}

/// Enter / `l` / →: open the selected thing. In the explorer that is a file (into the
/// buffer); in Source Control it is the change's diff — clicking a change must show the
/// hunks, not the whole file.
pub fn open(editor: &mut EditorState) -> EventResult {
    enum Target {
        File(std::path::PathBuf),
        Diff(crate::sidebar::GitEntry),
    }

    let target = {
        let Some(sidebar) = editor.sidebar.as_mut() else {
            return EventResult::Continue;
        };
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.expand_or_open().map(Target::File),
            SidebarTab::Git => sidebar.git.expand_or_open().map(Target::Diff),
        }
    };

    match target {
        Some(Target::File(path)) => open_path(editor, &path),
        Some(Target::Diff(entry)) => crate::reducer::filediff::open(editor, &entry),
        None => EventResult::Continue,
    }
}

/// Load a file into the buffer and hand the keyboard back to the text, like VS Code: the
/// panel stays open so the next file is one click away.
fn open_path(editor: &mut EditorState, path: &std::path::Path) -> EventResult {
    let path_str = path.to_string_lossy().to_string();
    let result = editor.open_file(&path_str);
    // A picture opens its own view; forcing the home mode would drop it immediately.
    if editor.mode != crate::state::EditorMode::Image {
        editor.enter_mode(editor.home_mode());
    }
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
    let staged = sidebar.git.staged_count();
    let unstaged = sidebar.git.unstaged_count();
    let repos = sidebar.git.repos.iter().filter(|r| r.count > 0).count();
    let msg = if !repo {
        "Not a git repository".to_string()
    } else if count == 0 {
        "Working tree clean".to_string()
    } else if repos > 1 {
        format!("{count} change(s) in {repos} repositories ({staged} staged, {unstaged} unstaged)")
    } else {
        format!("{count} change(s) ({staged} staged, {unstaged} unstaged)")
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

    // 0. The pane switch (`«` in the panel's header, `‹ / »` in the status bar): folding
    //    the panel away, or — on a narrow screen — walking between the list and the file.
    if let Some(rect) = hit.pane_toggle
        && rect.contains(col, row)
    {
        let narrow = editor.narrow_screen();
        let showing_list = editor.shows_list(hit.screen.w);
        let tab = editor
            .sidebar
            .as_ref()
            .map(|sidebar| sidebar.tab)
            .unwrap_or(SidebarTab::Files);
        return if narrow && showing_list {
            // `» Edit`: back to the file.
            unfocus(editor)
        } else if narrow {
            // `‹ Files`: back to the list.
            show_list(editor)
        } else if editor.sidebar_folded() {
            // `» Files` on the status row: the panel was folded away and this is the way
            // back (the same key works, but a terminal without Alt must have a button).
            focus(editor, tab)
        } else {
            // `«` in the panel's own corner.
            fold_panel(editor)
        };
    }

    // 1. The divider: grabbing it starts a resize that follows the pointer until release.
    if let Some(divider) = hit.divider
        && col == divider
        && row >= hit.sidebar.y
        && row < hit.sidebar.bottom()
    {
        editor.sidebar_drag = true;
        return EventResult::Continue;
    }

    // 2. The panel's scrollbar: grabbing it moves the viewport without selecting
    //    anything, exactly like the wheel it replaces.
    if let Some(bar) = hit.sidebar_vbar
        && bar.contains(col, row)
    {
        editor.scroll_drag = Some(crate::state::ScrollDrag::Sidebar);
        scroll_panel_to(editor, row);
        return EventResult::Continue;
    }

    // 3. The panel: header tabs, then rows.
    if hit.sidebar.contains(col, row) {
        if let Some(tab) = hit.tab_at(col, row) {
            return select_tab(editor, tab);
        }
        if let Some(SidebarHit::Row(node)) = hit.row_at(col, row) {
            return click_row(editor, node);
        }
        return EventResult::Continue;
    }

    // 4. The text's scrollbars: vertical parks the view on the row, horizontal on the
    //    column. Neither moves the cursor, which is what a scrollbar is for.
    if let Some(bar) = hit.editor_vbar
        && bar.contains(col, row)
        && matches!(
            editor.mode,
            crate::state::EditorMode::Edit | crate::state::EditorMode::Glide
        )
    {
        editor.scroll_drag = Some(crate::state::ScrollDrag::EditorVertical);
        editor.scroll_parked = true;
        scroll_text_to_row(editor, row);
        return EventResult::Continue;
    }
    if let Some(bar) = hit.editor_hbar
        && bar.contains(col, row)
        && matches!(
            editor.mode,
            crate::state::EditorMode::Edit | crate::state::EditorMode::Glide
        )
    {
        editor.scroll_drag = Some(crate::state::ScrollDrag::EditorHorizontal);
        editor.scroll_parked = true;
        scroll_text_to_col(editor, col);
        return EventResult::Continue;
    }

    // 5. The buffer: put the cursor where the character was drawn and start a selection.
    //    The terminal's own drag-select is unavailable here (cozy has the mouse), so this
    //    is what makes plain drag-to-copy work.
    if hit.text_area.contains(col, row)
        && matches!(
            editor.mode,
            crate::state::EditorMode::Edit | crate::state::EditorMode::Glide
        )
        && let Some((line, byte)) = crate::ui::render::body::click_to_cursor(editor, col, row)
    {
        if let Some(sidebar) = editor.sidebar.as_mut() {
            sidebar.focused = false;
        }
        editor.status_message = None;
        return crate::reducer::selection::press(editor, line, byte);
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
        sidebar.scroll_parked = false;
    }
    open(editor)
}

/// Dragging a scrollbar. The pointer row (or column) maps through the same geometry the
/// bar was drawn with, so the thumb stays under the hand.
fn scroll_panel_to(editor: &mut EditorState, row: u16) {
    let Some(bar) = editor.ui_hit.sidebar_vbar else {
        return;
    };
    let cell = row.saturating_sub(bar.y) as usize;
    let track = bar.h as usize;
    let Some(sidebar) = editor.sidebar.as_mut() else {
        return;
    };
    let total = match sidebar.tab {
        SidebarTab::Files => sidebar.explorer.visible_nodes().len(),
        SidebarTab::Git => sidebar.git.visible().len(),
    };
    let offset = crate::ui::render::scrollbar::offset_at(track, total, track, cell);
    match sidebar.tab {
        SidebarTab::Files => sidebar.explorer.scroll = offset,
        SidebarTab::Git => sidebar.git.scroll = offset,
    }
    sidebar.scroll_parked = true;
}

fn scroll_text_to_row(editor: &mut EditorState, row: u16) {
    let Some(bar) = editor.ui_hit.editor_vbar else {
        return;
    };
    let cell = row.saturating_sub(bar.y) as usize;
    editor.scroll_offset = crate::ui::render::scrollbar::offset_at(
        bar.h as usize,
        editor.scrollbar_total,
        editor.scrollbar_viewport,
        cell,
    )
    .min(editor.max_scroll_offset());
}

fn scroll_text_to_col(editor: &mut EditorState, col: u16) {
    let Some(bar) = editor.ui_hit.editor_hbar else {
        return;
    };
    let cell = col.saturating_sub(bar.x) as usize;
    let viewport = editor.text_display_width;
    let total = viewport + editor.max_scroll_x();
    editor.scroll_x =
        crate::ui::render::scrollbar::offset_at(bar.w as usize, total, viewport, cell);
}

/// Dragging the divider. The panel keeps a sane range no matter where the pointer is.
pub fn mouse_drag(editor: &mut EditorState, col: u16, row: u16) -> EventResult {
    // A selection drag owns the motion until the button comes up.
    if editor.select_drag {
        return crate::reducer::selection::drag(editor, col, row);
    }
    match editor.scroll_drag {
        Some(crate::state::ScrollDrag::Sidebar) => {
            scroll_panel_to(editor, row);
            return EventResult::Continue;
        }
        Some(crate::state::ScrollDrag::EditorVertical) => {
            scroll_text_to_row(editor, row);
            return EventResult::Continue;
        }
        Some(crate::state::ScrollDrag::EditorHorizontal) => {
            scroll_text_to_col(editor, col);
            return EventResult::Continue;
        }
        None => {}
    }
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
    editor.scroll_drag = None;
    // The end of a drag: copy what it selected. A click (no drag) copies nothing.
    crate::reducer::selection::finish(editor)
}

/// The pointer moved. Hover is its own state: it never moves the selection and never
/// starts a drag.
pub fn mouse_move(editor: &mut EditorState, col: u16, row: u16) -> EventResult {
    let hit = editor.ui_hit.clone();
    let Some(sidebar) = editor.sidebar.as_mut() else {
        return EventResult::Continue;
    };
    let node = hit.row_at(col, row).map(|SidebarHit::Row(node)| node);
    let tab = hit.tab_at(col, row);
    if sidebar.hover != node || sidebar.hover_tab != tab {
        sidebar.hover = node;
        sidebar.hover_tab = tab;
        sidebar.hover_at = Some(std::time::Instant::now());
    }
    EventResult::Continue
}

/// The wheel. Over the panel it moves the *viewport* — the selection stays where it was,
/// so scrolling to read does not decide anything. Anywhere else it means what it always
/// meant (scroll the reading view, or move the viewport while editing).
pub fn mouse_scroll(editor: &mut EditorState, delta: isize, col: u16, row: u16) -> EventResult {
    let over_panel = editor.sidebar_on_screen() && editor.ui_hit.sidebar.contains(col, row);
    if over_panel {
        let Some(sidebar) = editor.sidebar.as_mut() else {
            return EventResult::Continue;
        };
        match sidebar.tab {
            SidebarTab::Files => sidebar.explorer.scroll_by(delta),
            SidebarTab::Git => sidebar.git.scroll_by(delta),
        }
        // The pointer owns the window until the keyboard asks for the selection back.
        sidebar.scroll_parked = true;
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

    /// The panel's scrollbar: a grab moves the viewport and leaves the selection where
    /// it was — the same contract as the wheel.
    #[test]
    fn grabbing_the_panel_scrollbar_scrolls_without_selecting() {
        let (_tmp, mut editor) = editor_with_panel();
        // As the renderer would have recorded it: one column, two rows of travel.
        editor.ui_hit.sidebar_vbar = Some(Area::new(32, 2, 1, 2));
        let before = editor.sidebar.as_ref().unwrap().explorer.selected;
        let _ = mouse_down(&mut editor, MouseButton::Left, 32, 3);
        assert_eq!(
            editor.scroll_drag,
            Some(crate::state::ScrollDrag::Sidebar),
            "grab tercatat"
        );
        {
            let sidebar = editor.sidebar.as_ref().unwrap();
            assert!(sidebar.explorer.scroll > 0, "viewport bergeser");
            assert_eq!(sidebar.explorer.selected, before, "seleksi tidak ikut");
        }
        // Dragging back to the top returns the view to the top.
        let _ = mouse_drag(&mut editor, 32, 2);
        assert_eq!(editor.sidebar.as_ref().unwrap().explorer.scroll, 0);
        let _ = mouse_up(&mut editor);
        assert_eq!(editor.scroll_drag, None, "lepas tombol mengakhiri grab");
    }

    /// The keyboard takes the window back: `j` after a wheel may not leave the selection
    /// off screen.
    #[test]
    fn a_key_moves_the_panel_back_to_its_selection() {
        let (_tmp, mut editor) = editor_with_panel();
        editor.ui_hit.sidebar_vbar = Some(Area::new(32, 2, 1, 2));
        let _ = mouse_down(&mut editor, MouseButton::Left, 32, 3);
        assert!(editor.sidebar.as_ref().unwrap().scroll_parked);
        move_down(&mut editor);
        assert!(!editor.sidebar.as_ref().unwrap().scroll_parked);
    }

    /// The editor's bars: the vertical one parks the window, the horizontal one moves it
    /// sideways, and neither touches the cursor.
    #[test]
    fn the_editor_scrollbars_move_the_window_not_the_cursor() {
        let mut editor = EditorState::new(None);
        editor.buffer =
            crate::state::TextBuffer::from_lines((0..100).map(|i| format!("line {i}")).collect());
        editor.enter_mode(crate::state::EditorMode::Edit);
        editor.page_size = 10;
        editor.scrollbar_total = 100;
        editor.scrollbar_viewport = 10;
        editor.ui_hit.editor_vbar = Some(Area::new(80, 0, 1, 10));
        let _ = mouse_down(&mut editor, MouseButton::Left, 80, 9);
        assert!(editor.scroll_offset > 0, "jendela bergeser");
        assert!(editor.scroll_parked);
        assert_eq!(editor.cursor.y, 0, "kursor tidak ikut");

        editor.soft_wrap = false;
        editor.text_display_width = 40;
        editor.buffer =
            crate::state::TextBuffer::from_lines(vec!["x".repeat(200), "short".to_string()]);
        editor.scroll_offset = 0;
        editor.ui_hit.editor_hbar = Some(Area::new(0, 9, 40, 1));
        let _ = mouse_down(&mut editor, MouseButton::Left, 39, 9);
        assert_eq!(editor.scroll_x, 160, "sampai ujung kanan");
        assert_eq!(
            editor.scroll_drag,
            Some(crate::state::ScrollDrag::EditorHorizontal)
        );
    }

    /// Press-and-drag over the text: the terminal cannot do it (cozy holds the mouse),
    /// so cozy does — the band, and the copy when the button comes up.
    #[test]
    fn dragging_over_the_text_selects_and_copies_it() {
        let (_tmp, mut editor) = editor_with_panel();
        editor.buffer = crate::state::TextBuffer::from_lines(vec![
            "hello world".to_string(),
            "second line".to_string(),
        ]);
        // The text sits to the right of the panel, where a real frame puts it.
        editor.ui_hit.text_area = Area::new(40, 0, 40, 10);
        editor.ui_hit.text_x = 40;
        editor.ui_hit.text_width = 40;
        editor.page_size = 10;

        // Press at column 2 of line 0, drag to column 5 of line 1, release.
        let _ = mouse_down(&mut editor, MouseButton::Left, 42, 0);
        assert!(editor.select_drag);
        let _ = mouse_drag(&mut editor, 45, 1);
        let _ = mouse_up(&mut editor);

        assert!(!editor.select_drag, "lepas tombol mengakhiri drag");
        assert_eq!(
            editor.selected_text(),
            Some(("llo world\nsecon".to_string(), 2))
        );
        assert_eq!(editor.cursor.y, 1, "kursor ikut ujung drag");
        assert!(
            editor
                .status_message
                .as_deref()
                .is_some_and(|m| m.starts_with("Copied") || m.starts_with("Copy failed")),
            "pesan copy: {:?}",
            editor.status_message
        );
    }

    /// A click (no drag) is not a copy — it only moves the cursor.
    #[test]
    fn a_plain_click_clears_the_selection_again() {
        let (_tmp, mut editor) = editor_with_panel();
        editor.buffer = crate::state::TextBuffer::from_lines(vec!["hello".to_string()]);
        editor.ui_hit.text_area = Area::new(40, 0, 40, 10);
        editor.ui_hit.text_x = 40;
        editor.ui_hit.text_width = 40;
        let _ = mouse_down(&mut editor, MouseButton::Left, 43, 0);
        let _ = mouse_up(&mut editor);
        assert!(editor.selection.is_none());
        assert_eq!(editor.cursor.x, 3);
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

    /// ⭐ Over the panel the wheel moves the viewport; the selection stays put, so
    /// scrolling to read never changes what Enter would open.
    #[test]
    fn the_wheel_over_the_panel_scrolls_the_view_not_the_selection() {
        let (_tmp, mut editor) = editor_with_panel();
        let (before_sel, before_scroll) = {
            let tree = &editor.sidebar.as_ref().unwrap().explorer;
            (tree.selected, tree.scroll)
        };
        let _ = mouse_scroll(&mut editor, 1, 5, 4);
        let tree = &editor.sidebar.as_ref().unwrap().explorer;
        assert_eq!(tree.selected, before_sel, "seleksi tidak bergerak");
        assert_eq!(tree.scroll, before_scroll + 1, "viewport bergerak");
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

#[cfg(test)]
mod rendered_frame_tests {
    use super::{focus, fold_panel, mouse_down, unfocus};
    use crate::action::MouseButton;
    use crate::sidebar::{SidebarTab, UiHit};
    use crate::state::{EditorMode, EditorState};
    use crate::ui::Renderer;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use std::fs;

    /// Draw one real frame and hand back the editor with its recorded geometry.
    fn drawn(cols: u16, rows: u16) -> (tempfile::TempDir, EditorState) {
        let tmp = tempfile::Builder::new()
            .prefix("cozy-probe-")
            .tempdir()
            .unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::write(tmp.path().join("a.rs"), "fn a() {}\n").unwrap();

        let mut editor = EditorState::new(None);
        editor._working_dir = tmp.path().to_path_buf();
        editor.sidebar = Some(crate::sidebar::Sidebar::new(tmp.path()));
        editor.enter_mode(EditorMode::Edit);

        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        terminal
            .draw(|f| {
                editor.ui_hit = UiHit {
                    screen: crate::sidebar::Area::from(f.area()),
                    ..Default::default()
                };
                let chunks = Renderer::editor_layout(f.area(), &editor);
                Renderer::render_body(&mut editor, f, chunks[0]);
                Renderer::render_shortcuts(&editor, f, chunks[1]);
                Renderer::render_status_bar(&mut editor, f, chunks[2]);
                Renderer::render_sidebar(&mut editor, f, chunks[3]);
            })
            .unwrap();
        (tmp, editor)
    }

    // ── one pane on a narrow screen (WA-Bot's list/detail shape) ────────────────

    /// Below `MIN_TOTAL_WIDTH` the panel and the text take turns instead of sharing.
    /// With nothing open, the list owns the screen.
    #[test]
    fn a_narrow_window_gives_the_whole_screen_to_the_list() {
        let (_tmp, editor) = drawn(60, 20);
        let chunks = Renderer::editor_layout(Rect::new(0, 0, 60, 20), &editor);
        assert_eq!(chunks[0].width, 0, "tidak ada kolom teks");
        assert_eq!(chunks[3].width, 60, "list selebar layar");
        assert_eq!(editor.ui_hit.divider, None, "tidak ada yang bisa di-resize");
    }

    /// Opening a file from the list turns it into the detail pane, full width.
    #[test]
    fn opening_a_file_in_a_narrow_window_fills_the_screen_with_it() {
        let (tmp, mut editor) = drawn(60, 20);
        let path = tmp.path().join("a.rs");
        editor.open_file(path.to_str().unwrap()).unwrap();
        editor.enter_mode(EditorMode::Edit);
        editor.sidebar.as_mut().unwrap().focused = false;

        let chunks = Renderer::editor_layout(Rect::new(0, 0, 60, 20), &editor);
        assert_eq!(chunks[0].width, 60, "berkas selebar layar");
        assert_eq!(chunks[3].width, 0, "list tidak menemani");
    }

    /// `Esc` in the file goes back to the list; `Esc` again returns to the file.
    #[test]
    fn esc_walks_between_the_list_and_the_file() {
        let (tmp, mut editor) = drawn(60, 20);
        let path = tmp.path().join("a.rs");
        editor.open_file(path.to_str().unwrap()).unwrap();
        editor.enter_mode(EditorMode::Edit);
        editor.sidebar.as_mut().unwrap().focused = false;

        let _ = crate::reducer::reduce(&mut editor, crate::action::Action::Cancel);
        assert!(editor.sidebar.as_ref().unwrap().focused, "Esc membuka list");
        // In the list the keymap turns Esc into `SidebarUnfocus` (see `ui::keymap`), which
        // is the step back to the file.
        unfocus(&mut editor);
        assert!(
            !editor.sidebar.as_ref().unwrap().focused,
            "Esc di list kembali ke berkas"
        );
        assert!(
            editor.sidebar.as_ref().unwrap().visible,
            "panel tidak pernah disembunyikan di layar sempit"
        );
    }

    /// The pane switch: `‹ Files` on the file, `» Edit` on the list — both clickable.
    #[test]
    fn the_pane_switch_chip_goes_both_ways() {
        let (tmp, mut editor) = drawn(60, 20);
        let path = tmp.path().join("a.rs");
        editor.open_file(path.to_str().unwrap()).unwrap();
        editor.enter_mode(EditorMode::Edit);
        editor.sidebar.as_mut().unwrap().focused = false;

        // Draw again: the chip lives on the status row now that the file has the screen.
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        let draw = |editor: &mut EditorState, terminal: &mut Terminal<TestBackend>| {
            terminal
                .draw(|f| {
                    editor.ui_hit = UiHit {
                        screen: crate::sidebar::Area::from(f.area()),
                        ..Default::default()
                    };
                    let chunks = Renderer::editor_layout(f.area(), editor);
                    Renderer::render_body(editor, f, chunks[0]);
                    Renderer::render_shortcuts(editor, f, chunks[1]);
                    Renderer::render_status_bar(editor, f, chunks[2]);
                    Renderer::render_sidebar(editor, f, chunks[3]);
                })
                .unwrap();
        };
        draw(&mut editor, &mut terminal);
        let chip = editor.ui_hit.pane_toggle.expect("‹ Files di baris status");
        let _ = mouse_down(&mut editor, MouseButton::Left, chip.x + 1, chip.y);
        assert!(editor.sidebar.as_ref().unwrap().focused, "chip → list");

        draw(&mut editor, &mut terminal);
        let chip = editor.ui_hit.pane_toggle.expect("» Edit di header list");
        let _ = mouse_down(&mut editor, MouseButton::Left, chip.x + 1, chip.y);
        assert!(!editor.sidebar.as_ref().unwrap().focused, "chip → berkas");
    }

    /// 🚨 The panel used to be unreachable once `«` folded it away on a wide terminal:
    /// the only way back was Alt+B, which some terminals never deliver. The status row now
    /// carries `» Files`, and a click brings the panel back.
    #[test]
    fn a_folded_panel_comes_back_from_the_status_row() {
        let (_tmp, mut editor) = drawn(110, 20);
        editor.sidebar.as_mut().unwrap().focused = true;
        fold_panel(&mut editor);
        assert!(!editor.sidebar.as_ref().unwrap().visible);
        assert!(
            editor.sidebar_folded(),
            "status row harus menawarkan jalan balik"
        );

        let mut terminal = Terminal::new(TestBackend::new(110, 20)).unwrap();
        let draw = |editor: &mut EditorState, terminal: &mut Terminal<TestBackend>| {
            terminal
                .draw(|f| {
                    editor.ui_hit = UiHit {
                        screen: crate::sidebar::Area::from(f.area()),
                        ..Default::default()
                    };
                    let chunks = Renderer::editor_layout(f.area(), editor);
                    Renderer::render_body(editor, f, chunks[0]);
                    Renderer::render_shortcuts(editor, f, chunks[1]);
                    Renderer::render_status_bar(editor, f, chunks[2]);
                    Renderer::render_sidebar(editor, f, chunks[3]);
                })
                .unwrap();
        };
        draw(&mut editor, &mut terminal);
        let chip = editor
            .ui_hit
            .pane_toggle
            .expect("chip `» Files` di baris status");
        let _ = mouse_down(&mut editor, MouseButton::Left, chip.x + 1, chip.y);
        assert!(
            editor.sidebar.as_ref().unwrap().visible,
            "klik chip membuka panel lagi"
        );
        assert!(editor.sidebar.as_ref().unwrap().focused);
        assert!(!editor.sidebar_folded(), "chip hilang setelah panel tampil");
    }

    /// No chip when there is nowhere to put the panel (a full-screen surface owns the
    /// screen) — a button that does nothing is worse than no button.
    #[test]
    fn a_full_screen_mode_offers_no_panel_chip() {
        let (_tmp, mut editor) = drawn(110, 20);
        fold_panel(&mut editor);
        editor.enter_mode(EditorMode::Markdown);
        assert!(!editor.sidebar_folded(), "Markdown memakai seluruh layar");
    }

    /// 🚨 With the panel folded, cozy used to release the mouse — so a click went to the
    /// terminal (which selected its own block over the line numbers) and the editor's
    /// cursor never moved. The editor now keeps the pointer, and its own selection is
    /// text-only.
    #[test]
    fn the_editor_keeps_the_mouse_when_the_panel_is_folded() {
        let (_tmp, mut editor) = drawn(110, 20);
        editor.sidebar.as_mut().unwrap().focused = true;
        fold_panel(&mut editor);
        assert!(!editor.sidebar.as_ref().unwrap().visible, "panel terlipat");
        assert!(editor.wants_mouse(), "editor tetap memegang mouse");

        // A click that cozy receives moves the cursor (before the fix it never arrived).
        editor.buffer =
            crate::state::TextBuffer::from_lines((1..=20).map(|n| format!("line {n}")).collect());
        // Geometry as the *next* frame records it (the panel gone, the text full width).
        let mut terminal = Terminal::new(TestBackend::new(110, 20)).unwrap();
        terminal
            .draw(|f| {
                editor.ui_hit = UiHit {
                    screen: crate::sidebar::Area::from(f.area()),
                    ..Default::default()
                };
                let chunks = Renderer::editor_layout(f.area(), &editor);
                Renderer::render_body(&mut editor, f, chunks[0]);
                Renderer::render_shortcuts(&editor, f, chunks[1]);
                Renderer::render_status_bar(&mut editor, f, chunks[2]);
                Renderer::render_sidebar(&mut editor, f, chunks[3]);
            })
            .unwrap();
        let _ = mouse_down(&mut editor, MouseButton::Left, 20, 5);
        assert_eq!(editor.cursor.y, 5, "klik mendarat di baris yang diklik");
    }

    /// …but not where cozy has nothing to do with the pointer: the terminal keeps its own
    /// drag-select there (preview, help, `czv`, the full-screen Browser).
    #[test]
    fn reading_views_leave_the_mouse_to_the_terminal() {
        let (_tmp, mut editor) = drawn(110, 20);
        for mode in [
            EditorMode::Markdown,
            EditorMode::Help,
            EditorMode::View,
            EditorMode::Browse,
        ] {
            editor.enter_mode(mode);
            assert!(
                !editor.wants_mouse(),
                "{mode:?} tidak boleh mengambil mouse"
            );
        }
        editor.enter_mode(EditorMode::Edit);
        assert!(editor.wants_mouse());
    }

    /// A wide window keeps the old behaviour: the panel folds away, and the same key
    /// brings it back.
    #[test]
    fn a_wide_window_folds_the_panel_away() {
        let (_tmp, mut editor) = drawn(100, 20);
        editor.sidebar.as_mut().unwrap().focused = true;
        fold_panel(&mut editor);
        assert!(
            !editor.sidebar.as_ref().unwrap().visible,
            "panel disembunyikan"
        );
        focus(&mut editor, SidebarTab::Files);
        assert!(
            editor.sidebar.as_ref().unwrap().visible,
            "Alt+B membukanya lagi"
        );
        // …and pressing it again hides it, as before.
        focus(&mut editor, SidebarTab::Files);
        assert!(!editor.sidebar.as_ref().unwrap().visible);
    }

    /// In a narrow window the same key must never hide the panel: it is the list.
    #[test]
    fn a_narrow_window_never_hides_the_panel() {
        let (_tmp, mut editor) = drawn(60, 20);
        editor.sidebar.as_mut().unwrap().focused = true;
        focus(&mut editor, SidebarTab::Files);
        focus(&mut editor, SidebarTab::Files);
        assert!(editor.sidebar.as_ref().unwrap().visible);
    }

    #[test]
    fn the_panel_and_its_rows_are_recorded() {
        let (_tmp, editor) = drawn(100, 24);
        let hit = &editor.ui_hit;
        assert!(!hit.sidebar.is_empty(), "panel rect");
        assert_eq!(hit.divider, Some(33), "border column at width 34");
        assert_eq!(hit.tabs.len(), 2, "dua tab");
        assert!(!hit.rows.is_empty(), "baris tree tercatat");
        assert_eq!(hit.tabs[1].0, SidebarTab::Git);
        assert!(hit.body.contains(5, 3), "baris tree di dalam body");
    }

    #[test]
    fn a_click_on_the_recorded_rect_switches_tabs() {
        let (_tmp, mut editor) = drawn(100, 24);
        let (_, start, end) = editor
            .ui_hit
            .tabs
            .iter()
            .find(|(tab, _, _)| *tab == SidebarTab::Git)
            .copied()
            .expect("tab Git");
        let x = (start + end) / 2;
        let y = editor.ui_hit.header.y;
        let _ = super::mouse_down(&mut editor, crate::action::MouseButton::Left, x, y);
        assert_eq!(editor.sidebar.as_ref().unwrap().tab, SidebarTab::Git);
    }
}
