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

use crate::state::{EditorState, FileDiffKind, FileDiffRow};

const TEXT: Color = Color::Rgb(200, 204, 212);
const DIM: Color = Color::Rgb(110, 114, 122);
const GUTTER: Color = Color::Rgb(80, 84, 92);
const ADD: Color = Color::Rgb(152, 195, 121);
const DEL: Color = Color::Rgb(224, 108, 117);
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
    let scroll = view.scroll;

    // ── header ────────────────────────────────────────────────────────────────
    // The title already says which side this is (`src/main.rs  ·  staged`).
    let mut spans = vec![Span::styled(
        format!(" {title}"),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )];
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
    let lines: Vec<Line<'static>> = view
        .rows
        .iter()
        .skip(scroll)
        .take(height)
        .map(row_line)
        .collect();
    f.render_widget(Paragraph::new(lines), body);
}

fn row_line(row: &FileDiffRow) -> Line<'static> {
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
            let (sign, color) = match kind {
                FileDiffKind::Add => ("+", ADD),
                FileDiffKind::Del => ("-", DEL),
                _ => (" ", TEXT),
            };
            Line::from(vec![
                Span::styled(format!("{old} {new} "), Style::default().fg(GUTTER)),
                Span::styled(sign.to_string(), Style::default().fg(color)),
                Span::styled(format!(" {}", row.text), Style::default().fg(color)),
            ])
        }
    }
}
