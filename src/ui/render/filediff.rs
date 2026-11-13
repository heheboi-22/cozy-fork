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
    let lines: Vec<Line<'static>> = (scroll..view.rows.len())
        .take(height)
        .map(|i| {
            let spans = view.spans.get(i).map(Vec::as_slice).unwrap_or(&[]);
            row_line(&view.rows[i], width, spans)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), body);
}

fn row_line(row: &FileDiffRow, width: usize, spans: &[(usize, usize, Style)]) -> Line<'static> {
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
            let mut parts = vec![
                Span::styled(format!("{old} {new} "), Style::default().fg(gutter_bg)),
                Span::styled(sign.to_string(), Style::default().fg(color)),
            ];
            // ⭐ The code keeps its syntax colours here too, over the add/remove band.
            // The band (and the sign) say *what* changed; the colours say *what it is* —
            // one-colour code made a long hunk unreadable.
            parts.extend(coloured_text(&row.text, spans));
            if let Some(bg) = bg {
                parts = fill_row(parts, width, bg);
            }
            Line::from(parts)
        }
    }
}

/// The row's code, split at the syntax spans. `spans` are byte ranges into `text`; the
/// leading space the row draws after the sign is added here, so the offsets line up.
fn coloured_text(text: &str, spans: &[(usize, usize, Style)]) -> Vec<Span<'static>> {
    let drawn = format!(" {text}");
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut pos = 0usize;
    for (start, end, style) in spans {
        // +1 for the space the sign is separated by.
        let start = boundary(&drawn, start + 1);
        let end = boundary(&drawn, end + 1);
        if start > pos {
            out.push(Span::styled(
                drawn[pos..start].to_string(),
                Style::default().fg(TEXT),
            ));
        }
        if end > start && start >= pos {
            out.push(Span::styled(drawn[start..end].to_string(), *style));
            pos = end;
        }
    }
    if pos < drawn.len() {
        out.push(Span::styled(
            drawn[pos..].to_string(),
            Style::default().fg(TEXT),
        ));
    }
    if out.is_empty() {
        out.push(Span::styled(drawn, Style::default().fg(TEXT)));
    }
    out
}

/// Clamp a byte offset into `text` to a char boundary (tree-sitter offsets are already
/// boundaries; this only guards the arithmetic).
fn boundary(text: &str, i: usize) -> usize {
    let mut i = i.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
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
        let line = row_line(&added(), 40, &[]);
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
        let line = row_line(&row, 30, &[]);
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
        let line = row_line(&row, 30, &[]);
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

        // …and the syntax colours survive the band: `added` has no grammar here, so use a
        // real one — `let` in Rust is magenta on top of the green.
        let rows = vec![FileDiffRow {
            kind: FileDiffKind::Add,
            old: None,
            new: Some(1),
            text: "let x = 1;".to_string(),
        }];
        editor.file_diff = Some(FileDiffView::build(
            std::path::Path::new("a.rs"),
            false,
            rows,
        ));
        terminal
            .draw(|f| render_file_diff(&mut editor, f, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let keyword = (0..60u16)
            .map(|x| &buffer[(x, 1)])
            .find(|c| c.symbol() == "l")
            .expect("huruf pertama `let`");
        assert_eq!(
            keyword.fg,
            Color::Magenta,
            "kata kunci tetap magenta di layar"
        );
        assert_eq!(keyword.bg, ADD_BG, "dan latarnya band hijau");
    }

    /// ⭐ The diff keeps the file's syntax colours: the band says *what changed*, the
    /// colours say *what it is*. Built through `FileDiffView::build`, so the language is
    /// picked from the path exactly as the editor does it.
    #[test]
    fn a_changed_line_keeps_its_syntax_colours() {
        let rows = vec![
            FileDiffRow {
                kind: FileDiffKind::Hunk,
                old: None,
                new: None,
                text: "@@ -1,1 +1,2 @@".to_string(),
            },
            FileDiffRow {
                kind: FileDiffKind::Add,
                old: None,
                new: Some(1),
                text: "let x = 1;".to_string(),
            },
        ];
        let view = crate::state::diff::FileDiffView::build(
            std::path::Path::new("src/main.rs"),
            false,
            rows,
        );
        let spans = &view.spans[1];
        assert!(
            spans
                .iter()
                .any(|(_, _, style)| style.fg == Some(Color::Magenta)),
            "kata kunci `let` harus magenta: {spans:?}"
        );

        let line = row_line(&view.rows[1], 60, spans);
        let keyword = line
            .spans
            .iter()
            .find(|s| s.content.as_ref().contains("let"))
            .expect("span kata kunci");
        assert_eq!(
            keyword.style.fg,
            Some(Color::Magenta),
            "warna dipertahankan"
        );
        assert_eq!(keyword.style.bg, Some(ADD_BG), "tetap di atas band hijau");
    }

    /// A file with no grammar still paints its rows (just without syntax colours).
    #[test]
    fn a_plain_text_diff_still_has_its_band() {
        let rows = vec![FileDiffRow {
            kind: FileDiffKind::Add,
            old: None,
            new: Some(1),
            text: "hello".to_string(),
        }];
        let view =
            crate::state::diff::FileDiffView::build(std::path::Path::new("notes.xyz"), false, rows);
        assert!(view.spans[0].is_empty(), "tidak ada grammar untuk .xyz");
        let line = row_line(&view.rows[0], 40, &view.spans[0]);
        assert_eq!(line.spans[1].style.bg, Some(ADD_BG));
    }

    #[test]
    fn a_long_diff_line_is_not_cut_by_the_tint() {
        let row = FileDiffRow {
            kind: FileDiffKind::Add,
            old: None,
            new: Some(1),
            text: "x".repeat(200),
        };
        let line = row_line(&row, 40, &[]);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            UnicodeWidthStr::width(text.as_str()) >= 40,
            "teks panjang tidak dipotong, hanya melebar"
        );
    }
}
