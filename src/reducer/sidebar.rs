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
