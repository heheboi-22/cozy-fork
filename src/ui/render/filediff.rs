//! The single-file diff view — the reader a Source Control row opens into.
//!
//! Laid out like a unified diff: two line-number gutters, the `+`/`-` sign, then the text.
//! `@@` headers are cyan, additions green, deletions red, and notes ("new file mode 1024",
//! "binary file") dim. The viewport is driven by the reducer; this only draws.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use unicode_width::UnicodeWidthStr;

use crate::state::{EditorState, FileDiffKind, FileDiffRow};

const TEXT: Color = Color::Rgb(200, 204, 212);
const DIM: Color = Color::Rgb(110, 114, 122);
const GUTTER: Color = Color::Rgb(80, 84, 92);
const ADD: Color = Color::Rgb(152, 195, 121);
const DEL: Color = Color::Rgb(224, 108, 117);
/// Whole-row tints: the sign says *what* changed, the background says *where the row is*,
/// which is what makes a long diff scannable (and what the reference shows).
const ADD_BG: Color = Color::Rgb(24, 46, 29);
const DEL_BG: Color = Color::Rgb(52, 28, 32);
const ADD_GUTTER_BG: Color = Color::Rgb(31, 58, 37);
const DEL_GUTTER_BG: Color = Color::Rgb(64, 33, 38);
const HUNK: Color = Color::Rgb(86, 182, 194);

pub fn render_file_diff(editor: &mut EditorState, f: &mut Frame, area: Rect) {
    let Some(view) = editor.file_diff.as_mut() else {
        return;
    };
    // The reducer needs the viewport height to clamp scrolling; it can only learn it here.
    let height = area.height.saturating_sub(1) as usize;
    view.view_height = height;
    view.clamp_scroll();

    let (title, cur, total) = (view.title.clone(), view.current + 1, view.hunks.len());
    // Lines changed in this file, counted from the rows we already have.
    let added = view
        .rows
        .iter()
        .filter(|r| r.kind == FileDiffKind::Add)
        .count();
    let deleted = view
        .rows
        .iter()
        .filter(|r| r.kind == FileDiffKind::Del)
        .count();
    let scroll = view.scroll;

    // ── header ────────────────────────────────────────────────────────────────
    // The title already says which side this is (`src/main.rs  ·  staged`).
    let mut spans = vec![Span::styled(
        format!(" {title}"),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )];
    if added > 0 {
        spans.push(Span::styled(
            format!("   +{added}"),
            Style::default().fg(ADD),
        ));
    }
    if deleted > 0 {
        spans.push(Span::styled(
            format!(" −{deleted}"),
            Style::default().fg(DEL),
        ));
    }
    if total > 0 {
        spans.push(Span::styled(
            format!("   hunk {cur}/{total}"),
            Style::default().fg(DIM),
        ));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );

    // ── rows ──────────────────────────────────────────────────────────────────
    let body = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    let width = area.width as usize;
    let lines: Vec<Line<'static>> = view
        .rows
        .iter()
        .skip(scroll)
        .take(height)
        .map(|row| row_line(row, width))
        .collect();
    f.render_widget(Paragraph::new(lines), body);
}

fn row_line(row: &FileDiffRow, width: usize) -> Line<'static> {
    match row.kind {
        FileDiffKind::Hunk => Line::from(Span::styled(
            format!(" {}", row.text),
            Style::default().fg(HUNK).add_modifier(Modifier::BOLD),
        )),
        FileDiffKind::Meta => Line::from(Span::styled(
            format!(" {}", row.text),
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        )),
        kind => {
            let old = row
                .old
                .map(|n| format!("{n:>4}"))
                .unwrap_or_else(|| "    ".to_string());
            let new = row
                .new
                .map(|n| format!("{n:>4}"))
                .unwrap_or_else(|| "    ".to_string());
            let (sign, color, bg, gutter_bg) = match kind {
                FileDiffKind::Add => ("+", ADD, Some(ADD_BG), ADD_GUTTER_BG),
                FileDiffKind::Del => ("-", DEL, Some(DEL_BG), DEL_GUTTER_BG),
                _ => (" ", TEXT, None, GUTTER),
            };
            let mut spans = vec![
                Span::styled(format!("{old} {new} "), Style::default().fg(gutter_bg)),
                Span::styled(sign.to_string(), Style::default().fg(color)),
                Span::styled(format!(" {}", row.text), Style::default().fg(color)),
            ];
            if let Some(bg) = bg {
                spans = fill_row(spans, width, bg);
            }
            Line::from(spans)
        }
    }
}

/// Paint the row edge to edge: a changed line reads as a band, not as a few coloured
/// words. The gutter keeps a slightly stronger shade so the numbers stay legible.
fn fill_row(mut spans: Vec<Span<'static>>, width: usize, bg: Color) -> Vec<Span<'static>> {
    let used: usize = spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    if used < width {
        spans.push(Span::raw(" ".repeat(width - used)));
    }
    for (i, span) in spans.iter_mut().enumerate() {
        // The first span is the gutter: give it its own shade.
        let shade = if i == 0 {
            match bg {
                ADD_BG => ADD_GUTTER_BG,
                DEL_BG => DEL_GUTTER_BG,
                other => other,
            }
        } else {
            bg
        };
        span.style = span.style.bg(shade);
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn added() -> FileDiffRow {
        FileDiffRow {
            kind: FileDiffKind::Add,
            old: None,
            new: Some(7),
            text: "let x = 1;".to_string(),
        }
    }

    #[test]
    fn an_added_line_is_tinted_edge_to_edge() {
        let line = row_line(&added(), 40);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(
            UnicodeWidthStr::width(text.as_str()),
            40,
            "penuh selebar area"
        );
        assert!(
            line.spans.iter().all(|s| s.style.bg.is_some()),
            "semua span (termasuk pengisi) berlatar"
        );
        assert_eq!(line.spans[1].style.bg, Some(ADD_BG), "teks berlatar hijau");
        assert_eq!(
            line.spans[0].style.bg,
            Some(ADD_GUTTER_BG),
            "gutter punya nuansa sendiri"
        );
    }

    #[test]
    fn a_deleted_line_is_tinted_red() {
        let row = FileDiffRow {
            kind: FileDiffKind::Del,
            old: Some(3),
            new: None,
            text: "gone".to_string(),
        };
        let line = row_line(&row, 30);
        assert_eq!(line.spans[2].style.bg, Some(DEL_BG));
    }

    #[test]
    fn a_context_line_is_not_tinted() {
        let row = FileDiffRow {
            kind: FileDiffKind::Context,
            old: Some(1),
            new: Some(1),
            text: "same".to_string(),
        };
        let line = row_line(&row, 30);
        assert!(line.spans.iter().all(|s| s.style.bg.is_none()));
    }

    /// End to end through a real frame: the tint must survive `Paragraph`, not just exist
    /// in the spans.
    #[test]
    fn a_rendered_frame_paints_the_row_background() {
        use crate::state::diff::FileDiffView;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let rows = vec![
            FileDiffRow {
                kind: FileDiffKind::Hunk,
                old: None,
                new: None,
                text: "@@ -1 +1 @@".to_string(),
            },
            FileDiffRow {
                kind: FileDiffKind::Add,
                old: None,
                new: Some(1),
                text: "added".to_string(),
            },
        ];
        let mut editor = EditorState::new(None);
        editor.file_diff = Some(FileDiffView::build(
            std::path::Path::new("a.rs"),
            false,
            rows,
        ));
        editor.enter_mode(crate::state::EditorMode::FileDiff);

        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
        terminal
            .draw(|f| render_file_diff(&mut editor, f, f.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row = 2; // header at 0, hunk at 1, the added line at 2
        let cell = &buffer[(0, row)];
        assert_eq!(cell.bg, ADD_GUTTER_BG, "gutter ikut berwarna");
        let cell = &buffer[(20, row)];
        assert_eq!(cell.bg, ADD_BG, "sel kosong di kanan pun ikut berwarna");
        assert!(
            buffer[(0, 1)].bg != ADD_GUTTER_BG,
            "baris hunk tidak ikut diwarnai"
        );
    }

    #[test]
    fn a_long_diff_line_is_not_cut_by_the_tint() {
        let row = FileDiffRow {
            kind: FileDiffKind::Add,
            old: None,
            new: Some(1),
            text: "x".repeat(200),
        };
        let line = row_line(&row, 40);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            UnicodeWidthStr::width(text.as_str()) >= 40,
            "teks panjang tidak dipotong, hanya melebar"
        );
    }
}
