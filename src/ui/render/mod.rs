pub mod body;
pub mod browse;
pub mod diff;
pub mod filediff;
pub mod footer;
pub mod markdown;
pub mod screen;
pub mod scrollbar;
pub mod sidebar;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::Paragraph,
};
use std::rc::Rc;

use crate::state::{EditorMode, EditorState};

// Re-export public functions
pub use body::render_text_buffer;
pub use screen::{render_help, render_welcome};

pub struct Renderer;

impl Renderer {
    pub fn editor_layout(area: Rect, editor: &EditorState) -> Rc<[Rect]> {
        let is_narrow = area.width < 50;
        let is_low_height = area.height < 18;
        let status_bar_h: u16 = if matches!(editor.mode, EditorMode::Welcome) || area.height < 2 {
            0
        } else {
            1
        };
        let requested_shortcut_h = if !editor.footer_visible_runtime {
            hidden_shortcut_rows(editor.mode)
        } else if is_low_height {
            compact_shortcut_rows(is_narrow, editor.mode)
        } else {
            normal_shortcut_rows(is_narrow, editor.mode)
        };
        let available = area.height.saturating_sub(1 + status_bar_h);
        let actual_shortcut_h = requested_shortcut_h.min(available);
        // 📌 配列は**値で渡す**（このリポの他の 4 箇所と同じ綴り）。`.as_ref()` 付きの
        // 旧い綴りはここ 1 箇所だけで、`imageview` の依存（image / fast_image_resize）が
        // 乗った途端に AsRef の行き先が一意でなくなり E0283 で落ちた（2026-10-03 実測 ——
        // feature を切ると通る）。値渡しなら `I = [Constraint; 3]` で曖昧さが無い。
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(actual_shortcut_h),
                Constraint::Length(status_bar_h),
            ])
            .split(area);

        // The sidebar shares the body band with the text: full editor height, minus the
        // footer and status bar — VS Code's shape. It steps aside when the terminal is
        // too narrow to keep a useful editing column beside it, and never appears over
        // the full-screen surfaces (Browse/Help/preview) that own the whole area.
        // ⭐ One pane or two? Wide terminals put the panel beside the text; narrow ones
        // (phones, split terminals) give each the whole band in turn — WA-Bot's
        // list/detail shape: the list on screen, tap a row, the file takes the screen,
        // `‹` or Esc comes back.
        let sidebar_w = Self::sidebar_width(editor, area);
        let blank = Rect {
            x: rows[0].x,
            y: rows[0].y,
            width: 0,
            height: 0,
        };
        let (body, panel) = if editor.shows_list(area.width) {
            (blank, rows[0])
        } else if sidebar_w > 0 {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(sidebar_w), Constraint::Min(1)])
                .split(rows[0]);
            (cols[1], cols[0])
        } else {
            (rows[0], blank)
        };

        // 📌 並びは `[body, shortcuts, status, sidebar]` —— 既存の呼び手（テストを含む）は
        // 先頭 3 つを今までどおり読む。サイドバーは幅 0 のときに描かれない。
        vec![body, rows[1], rows[2], panel].into()
    }

    /// Rows the panel occupies: 0 when it is hidden, off, or the terminal is narrow.
    fn sidebar_width(editor: &EditorState, area: Rect) -> u16 {
        let Some(sidebar) = editor.sidebar.as_ref() else {
            return 0;
        };
        if !editor.sidebar_on_screen() || area.width < crate::sidebar::MIN_TOTAL_WIDTH {
            return 0;
        }
        sidebar.width_for(area.width)
    }

    pub fn render_body(editor: &mut EditorState, f: &mut Frame, area: Rect) {
        editor.page_size = area.height as usize;
        // A picture: the body *is* the viewer. No text geometry is recorded, so a click
        // in the image cannot move a cursor that is not there.
        #[cfg(feature = "imageview")]
        if editor.mode == EditorMode::Image {
            editor.ui_hit.text_area = crate::sidebar::Area::default();
            let pending = editor
                .image_pending
                .as_ref()
                .map(|p| (p.label.clone(), p.started.elapsed()));
            match (editor.image_view.as_mut(), pending) {
                (Some(viewer), _) => viewer.draw_in(f, area),
                (None, Some((label, elapsed))) => body::render_loading(f, area, &label, elapsed),
                (None, None) => {}
            }
            return;
        }
        if editor.mode == EditorMode::Welcome {
            // ⚠️ 出すのは **error のときだけ**。Welcome に status bar は無いので、
            // ここで拾わない限り理由は消える。逆に info/success まで出すと起動画面が騒がしい。
            let notice = editor
                .status_message
                .as_deref()
                .filter(|_| editor.status_kind == crate::state::StatusKind::Error);
            render_welcome(f, area, notice);
            return;
        }
        if editor.mode == EditorMode::Help {
            render_help(editor, f, area);
            return;
        }
        if editor.mode == EditorMode::Browse {
            browse::render_browse(editor, f, area);
            return;
        }
        if editor.mode == EditorMode::Markdown {
            markdown::render_markdown(editor, f, area);
            return;
        }
        if editor.mode == EditorMode::FileDiff {
            filediff::render_file_diff(editor, f, area);
            return;
        }
        if editor.mode == EditorMode::DiffReview || editor.mode == EditorMode::DiffCommitMsg {
            // Keep the diff visible while the commit message is typed.
            diff::render_diff_review(editor, f, area);
            return;
        }
        render_text_buffer(editor, f, area);
    }

    pub fn render_shortcuts(editor: &EditorState, f: &mut Frame, area: Rect) {
        footer::render_shortcuts(editor, f, area);
    }

    pub fn render_status_bar(editor: &mut EditorState, f: &mut Frame, area: Rect) {
        // ⭐ The pane switch lives in the corner of the status row, and its columns are
        // reserved before the line is composed so the message never runs underneath:
        //   narrow, file on screen → `‹ Files` (back to the list)
        //   wide, panel folded    → `» Files` (bring it back — 🚨 the panel used to be
        //                            unreachable once `«` hid it, if the terminal did not
        //                            deliver Alt+B)
        editor.ui_hit.pane_toggle = None;
        let label = if editor.sidebar_folded() {
            "» Files "
        } else {
            "‹ Files "
        };
        let width = label.chars().count() as u16;
        let show =
            (editor.sidebar_folded() || editor.shows_detail(area.width)) && area.width > width + 9;
        // One blank column between the message and the chip.
        let reserve = if show { width + 1 } else { 0 };
        footer::render_status_bar(editor, f, area, reserve);
        if reserve > 0 {
            let rect = Rect {
                x: area.right() - width,
                y: area.y,
                width,
                height: 1,
            };
            f.render_widget(
                Paragraph::new(label)
                    .style(Style::default().fg(Color::White).bg(Color::Rgb(45, 50, 60))),
                rect,
            );
            editor.ui_hit.pane_toggle = Some(crate::sidebar::Area::from(rect));
        }
    }

    /// The left panel. A zero-width area (see `editor_layout`) draws nothing.
    pub fn render_sidebar(editor: &mut EditorState, f: &mut Frame, area: Rect) {
        sidebar::render_sidebar(editor, f, area);
    }
}

impl From<Rect> for crate::sidebar::Area {
    fn from(r: Rect) -> Self {
        Self::new(r.x, r.y, r.width, r.height)
    }
}

fn normal_shortcut_rows(is_narrow: bool, mode: EditorMode) -> u16 {
    match (is_narrow, mode) {
        (_, EditorMode::Welcome) => 0,
        (_, EditorMode::Command) => 10,
        (true, _) => 4,
        (false, _) => 2,
    }
}

fn compact_shortcut_rows(is_narrow: bool, mode: EditorMode) -> u16 {
    match mode {
        EditorMode::Welcome => 0,
        EditorMode::Command => {
            if is_narrow {
                3
            } else {
                4
            }
        }
        EditorMode::Save | EditorMode::Open | EditorMode::DiffCommitMsg => 2,
        EditorMode::Quit | EditorMode::Replace => 3,
        EditorMode::Search | EditorMode::Goto => 2,
        EditorMode::Edit
        | EditorMode::Glide
        | EditorMode::Browse
        | EditorMode::Markdown
        | EditorMode::View
        | EditorMode::FileDiff
        | EditorMode::DiffReview
        | EditorMode::Image
        | EditorMode::Help => 1,
    }
}

fn hidden_shortcut_rows(mode: EditorMode) -> u16 {
    match mode {
        EditorMode::Welcome => 0,
        EditorMode::Save | EditorMode::Open | EditorMode::Replace | EditorMode::Command => 2,
        EditorMode::Quit | EditorMode::DiffCommitMsg => 2,
        EditorMode::Search | EditorMode::Goto => 1,
        EditorMode::Edit
        | EditorMode::Glide
        | EditorMode::Browse
        | EditorMode::Markdown
        | EditorMode::View
        | EditorMode::FileDiff
        | EditorMode::DiffReview
        | EditorMode::Image
        | EditorMode::Help => 0,
    }
}
