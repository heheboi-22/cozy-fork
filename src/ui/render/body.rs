use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::state::{EditorMode, EditorState};
use crate::utils::unicode::display_width_up_to;
use unicode_width::UnicodeWidthStr;

/// The band behind mouse-selected text. Dark theme: a deep blue that keeps the syntax
/// colours readable; light theme: VS Code's own light selection.
const SELECTION_DARK: Color = Color::Rgb(38, 79, 120);
const SELECTION_LIGHT: Color = Color::Rgb(173, 214, 255);
use crate::utils::wrap::{byte_at_visual_col, visual_row_count, wrap_chunks};

/// Clamp cursor position to stay within the terminal buffer bounds.
fn clamp_cursor(frame_area: Rect, x: u16, y: u16) -> (u16, u16) {
    let max_x = frame_area.right().saturating_sub(1);
    let max_y = frame_area.bottom().saturating_sub(1);
    (x.min(max_x), y.min(max_y))
}

pub fn render_text_buffer(editor: &mut EditorState, f: &mut Frame, outer: Rect) {
    let show_line_numbers = editor.line_numbers_visible();

    let (line_number_width, line_number_digits) = if show_line_numbers {
        let max_lines = editor.buffer.lines.len();
        let max_display = std::cmp::max(max_lines, editor.scroll_offset + outer.height as usize);
        let digits = max_display.to_string().len();
        let width = (digits + 2) as u16;
        (width, digits)
    } else {
        (0u16, 0usize)
    };

    let text_x_offset = if show_line_numbers {
        line_number_width + 1
    } else {
        0
    };

    // The bars take a column (and, with wrap off, a row) away from the text. They are
    // fitted against the width the text would have without them, so a bar appearing
    // cannot make the content stop overflowing — the decision cannot oscillate.
    editor.text_display_width = outer.width.saturating_sub(text_x_offset) as usize;
    let (area, bars) = fit_scrollbars(editor, outer, text_x_offset);
    let viewport_height = area.height as usize;
    let text_width = area.width.saturating_sub(text_x_offset) as usize;

    // Store text_display_width before adjust_scroll (which needs it for soft wrap)
    editor.text_display_width = text_width;
    // Recorded for the mouse: a click is resolved against the frame that was drawn.
    editor.ui_hit.text_area = crate::sidebar::Area::new(area.x, area.y, area.width, area.height);
    editor.ui_hit.text_x = area.x + text_x_offset;
    editor.ui_hit.text_width = text_width as u16;
    editor.ui_hit.editor_vbar = bars
        .vertical
        .map(|r| crate::sidebar::Area::new(r.x, r.y, r.width, r.height));
    editor.ui_hit.editor_hbar = bars
        .horizontal
        .map(|r| crate::sidebar::Area::new(r.x, r.y, r.width, r.height));
    editor.adjust_scroll(viewport_height);
    editor.adjust_hscroll();

    // Refresh syntax highlight spans for the visible window (no-op unless the
    // buffer is dirty or the window moved). Over-including with soft wrap is
    // fine — the extra lines just won't be drawn.
    let line_count = editor.buffer.lines.len();
    let vis_end = (editor.scroll_offset + viewport_height).min(line_count);
    let vis_start = editor.scroll_offset.min(vis_end);
    editor
        .highlighter
        .ensure(&editor.buffer.lines, vis_start..vis_end);

    let (bg_color, fg_color) = {
        let bg = match editor.config.line_number_bg.as_deref() {
            Some("blue") => Color::Blue,
            Some("black") => Color::Black,
            _ => Color::DarkGray,
        };
        let fg = match editor.config.line_number_fg.as_deref() {
            Some("yellow") => Color::Yellow,
            _ => Color::White,
        };
        (bg, fg)
    };

    let soft_wrap = editor.soft_wrap && text_width > 0;

    // ── render visual rows ────────────────────────────────────────────────────

    let mut visual_row = 0usize;
    let mut buf_y = editor.scroll_offset;

    while visual_row < viewport_height {
        let has_content = buf_y < editor.buffer.lines.len();

        if has_content && soft_wrap {
            let line = editor.buffer.lines[buf_y].clone();
            let chunks = wrap_chunks(&line, text_width);

            for (chunk_idx, &(cs, ce)) in chunks.iter().enumerate() {
                if visual_row >= viewport_height {
                    break;
                }
                let row_y = area.y + visual_row as u16;

                if show_line_numbers {
                    let num_area = Rect {
                        x: area.x,
                        y: row_y,
                        width: line_number_width,
                        height: 1,
                    };
                    let ta_x = area.x + line_number_width + 1;
                    let ta_w = area.width.saturating_sub(line_number_width + 1);
                    let ta = Rect {
                        x: ta_x,
                        y: row_y,
                        width: ta_w,
                        height: 1,
                    };

                    if chunk_idx == 0 {
                        let num_str = right_align(buf_y + 1, line_number_digits);
                        let is_current = buf_y == editor.cursor.y;
                        let line_fg = if is_current { fg_color } else { Color::Gray };
                        let mut style = Style::default().bg(bg_color).fg(line_fg);
                        if is_current {
                            style = style.add_modifier(Modifier::BOLD);
                        }
                        f.render_widget(Paragraph::new(num_str).style(style), num_area);
                    } else {
                        f.render_widget(
                            Paragraph::new("").style(Style::default().bg(bg_color)),
                            num_area,
                        );
                    }
                    f.render_widget(render_line_range(editor, buf_y, cs, ce), ta);
                } else {
                    let la = Rect {
                        x: area.x,
                        y: row_y,
                        width: area.width,
                        height: 1,
                    };
                    f.render_widget(render_line_range(editor, buf_y, cs, ce), la);
                }

                visual_row += 1;
            }
        } else {
            let row_y = area.y + visual_row as u16;

            if show_line_numbers {
                let num_area = Rect {
                    x: area.x,
                    y: row_y,
                    width: line_number_width,
                    height: 1,
                };
                let ta_x = area.x + line_number_width + 1;
                let ta_w = area.width.saturating_sub(line_number_width + 1);
                let ta = Rect {
                    x: ta_x,
                    y: row_y,
                    width: ta_w,
                    height: 1,
                };

                // Always show line number for every viewport row (including beyond buffer)
                let num_str = right_align(buf_y + 1, line_number_digits);
                let is_current = buf_y == editor.cursor.y;
                let line_fg = if is_current { fg_color } else { Color::Gray };
                let mut style = Style::default().bg(bg_color).fg(line_fg);
                if is_current {
                    style = style.add_modifier(Modifier::BOLD);
                }
                f.render_widget(Paragraph::new(num_str).style(style), num_area);

                if has_content {
                    f.render_widget(render_line(editor, buf_y), ta);
                } else {
                    f.render_widget(Paragraph::new(""), ta);
                }
            } else if has_content {
                let la = Rect {
                    x: area.x,
                    y: row_y,
                    width: area.width,
                    height: 1,
                };
                f.render_widget(render_line(editor, buf_y), la);
            } else {
                let la = Rect {
                    x: area.x,
                    y: row_y,
                    width: area.width,
                    height: 1,
                };
                f.render_widget(Paragraph::new(""), la);
            }

            visual_row += 1;
        }

        buf_y += 1;
    }

    // ── cursor positioning ────────────────────────────────────────────────────

    if editor.mode == EditorMode::Edit || editor.mode == EditorMode::Glide {
        let cursor_y = editor.cursor.y;
        let cursor_x = editor.cursor.x;
        let mut vrow = 0usize;
        let mut by = editor.scroll_offset;

        'find: while vrow < viewport_height {
            if by >= editor.buffer.lines.len() {
                break;
            }
            let line = &editor.buffer.lines[by];

            if by == cursor_y {
                if soft_wrap {
                    let chunks = wrap_chunks(line, text_width);
                    let n = chunks.len();
                    for (cidx, &(cs, _)) in chunks.iter().enumerate() {
                        if vrow >= viewport_height {
                            break;
                        }
                        let next_start = if cidx + 1 < n {
                            chunks[cidx + 1].0
                        } else {
                            line.len() + 1
                        };
                        if cursor_x >= cs && cursor_x < next_start {
                            let end = cursor_x.min(line.len());
                            let before = &line[cs..end];
                            let dcol = crate::utils::unicode::str_display_width(before) as u16;
                            let (cx, cy) = clamp_cursor(
                                f.area(),
                                area.x + text_x_offset + dcol,
                                area.y + vrow as u16,
                            );
                            f.set_cursor_position((cx, cy));
                            break 'find;
                        }
                        vrow += 1;
                    }
                } else {
                    let dcol = display_width_up_to(line, cursor_x);
                    let sx = editor.scroll_x;
                    // The cursor can be scrolled out of the window: leave it hidden then
                    // rather than draw it on the wrong character.
                    if dcol >= sx && (dcol - sx) < text_width {
                        let (cx, cy) = clamp_cursor(
                            f.area(),
                            area.x + text_x_offset + (dcol - sx) as u16,
                            area.y + vrow as u16,
                        );
                        f.set_cursor_position((cx, cy));
                    }
                }
                break;
            } else {
                vrow += if soft_wrap {
                    visual_row_count(line, text_width)
                } else {
                    1
                };
                by += 1;
            }
        }
    }

    // ── scrollbars ────────────────────────────────────────────────────────────
    //
    // Drawn last, over the cells the text did not use: the bar is a position hint, and
    // it must never be covered by whatever the last line put there.
    let hot = editor.scroll_drag;
    if let Some(bar) = bars.vertical {
        crate::ui::render::scrollbar::draw_vertical(
            f,
            bar,
            editor.scrollbar_total,
            editor.scrollbar_viewport,
            editor.scroll_offset,
            hot == Some(crate::state::ScrollDrag::EditorVertical),
        );
    }
    if let Some(bar) = bars.horizontal {
        let viewport = text_width;
        let total = viewport + editor.max_scroll_x();
        crate::ui::render::scrollbar::draw_horizontal(
            f,
            bar,
            total,
            viewport,
            editor.scroll_x,
            hot == Some(crate::state::ScrollDrag::EditorHorizontal),
        );
    }
}

/// Which cells the text got, and where the two bars went (if they were needed).
#[derive(Debug, Clone, Copy, Default)]
struct Bars {
    vertical: Option<Rect>,
    horizontal: Option<Rect>,
}

/// Split `outer` into the text rectangle and the two scrollbars.
///
/// The order matters: the horizontal bar costs a row (which the vertical decision must
/// see), and the vertical bar costs a column (which the horizontal decision must not
/// see — otherwise a bar appearing could stop the content overflowing and the next frame
/// would hide it again). Both are computed from the *narrower* candidate width for the
/// same reason: narrowing only ever adds wrapped rows.
fn fit_scrollbars(editor: &mut EditorState, outer: Rect, gutter: u16) -> (Rect, Bars) {
    let mut text = outer;
    let mut bars = Bars::default();
    let width_no_v = outer.width.saturating_sub(gutter) as usize;
    if width_no_v < 4 || outer.height < 2 {
        editor.scrollbar_total = editor.buffer.lines.len();
        editor.scrollbar_viewport = outer.height as usize;
        return (text, bars);
    }

    // Horizontal: only when soft wrap is off (a wrapped line has nothing to the right).
    let widest = widest_visible(editor);
    if !editor.soft_wrap && widest > width_no_v && outer.height >= 3 {
        text.height = text.height.saturating_sub(1);
        bars.horizontal = Some(Rect {
            x: outer.x + gutter,
            y: text.bottom(),
            width: outer.width.saturating_sub(gutter),
            height: 1,
        });
    }

    let (total, viewport) = content_rows(editor, text.height as usize);
    editor.scrollbar_total = total;
    editor.scrollbar_viewport = viewport;
    if total > viewport && text.width >= 3 {
        bars.vertical = Some(Rect {
            x: text.right() - 1,
            y: text.y,
            width: 1,
            height: text.height,
        });
        text.width = text.width.saturating_sub(1);
    }
    (text, bars)
}

/// The content height the bar speaks about.
///
/// It counts **buffer lines**, not visual rows, because that is the unit the view scrolls
/// in: `scroll_offset` (the wheel, `Ctrl+D`, a scrollbar drag) is a line, and a bar
/// measured in wrapped rows would promise a scroll to a row the view cannot start on.
/// The ends still land exactly — first line at the top, last line at the bottom.
fn content_rows(editor: &EditorState, height: usize) -> (usize, usize) {
    (editor.buffer.lines.len().max(height), height)
}

/// The widest line in the window — the content width the horizontal bar speaks about.
fn widest_visible(editor: &EditorState) -> usize {
    let lines = &editor.buffer.lines;
    let start = editor.scroll_offset.min(lines.len());
    let end = (start + editor.page_size.max(1)).min(lines.len());
    lines[start..end]
        .iter()
        .map(|line| crate::utils::unicode::str_display_width(line))
        .max()
        .unwrap_or(0)
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// A picture is being prepared on a worker thread: one animated line in the middle of
/// the body, so the screen says "working" instead of looking stuck.
pub fn render_loading(f: &mut Frame, area: Rect, label: &str, elapsed: std::time::Duration) {
    if area.width < 8 || area.height == 0 {
        return;
    }
    let text = format!(
        "{} {label}  {}",
        loading_frame(elapsed),
        loading_note(elapsed)
    );
    let width = UnicodeWidthStr::width(text.as_str()) as u16;
    let line = Line::from(Span::styled(
        text,
        Style::default().fg(Color::Rgb(200, 204, 212)),
    ));
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height / 2,
        width: width.min(area.width),
        height: 1,
    };
    f.render_widget(Paragraph::new(line), rect);
}

/// The spinner: nine frames, a new one every 80 ms.
fn loading_frame(elapsed: std::time::Duration) -> char {
    const FRAMES: [char; 9] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇'];
    FRAMES[(elapsed.as_millis() / 80) as usize % FRAMES.len()]
}

/// After a moment, say what is taking the time — "rendering" for a PDF page is expected,
/// a stuck spinner is not.
fn loading_note(elapsed: std::time::Duration) -> &'static str {
    if elapsed.as_millis() > 1500 {
        "still loading…"
    } else {
        "loading…"
    }
}

fn right_align(num: usize, digits: usize) -> String {
    format!(" {:>width$} ", num, width = digits)
}

fn render_line(editor: &EditorState, y: usize) -> Line<'static> {
    let line = &editor.buffer.lines[y];
    // With soft wrap off the window can start mid-line: skip the columns scrolled away
    // rather than cut the text at the right edge.
    let start = if editor.soft_wrap || editor.scroll_x == 0 {
        0
    } else {
        crate::utils::wrap::byte_at_visual_col(line, 0, line.len(), editor.scroll_x, true)
    };
    render_line_range(editor, y, start, line.len())
}

fn render_line_range(
    editor: &EditorState,
    y: usize,
    byte_start: usize,
    byte_end: usize,
) -> Line<'static> {
    let line_text = &editor.buffer.lines[y];
    let mut spans: Vec<Span<'static>> = Vec::new();

    let is_light = editor.config.theme.as_deref() == Some("light");
    let default_style = if is_light {
        Style::default().fg(Color::Black).bg(Color::White)
    } else {
        Style::default()
    };
    let selection_color = if is_light {
        SELECTION_LIGHT
    } else {
        SELECTION_DARK
    };

    // The span of *this* line the mouse selected, if any. Wrapped chunks draw different
    // byte ranges of the same line; the `in_range` test below keeps them apart, so only
    // the bytes are needed here.
    let selected = editor.selection.and_then(|selection| {
        if selection.is_empty() {
            return None;
        }
        let ((sy, sx), (ey, ex)) = selection.range();
        if y < sy || y > ey {
            return None;
        }
        let from = if y == sy { sx.min(line_text.len()) } else { 0 };
        let to = if y == ey {
            ex.min(line_text.len())
        } else {
            line_text.len()
        };
        Some((from, to))
    });

    let line_matches: Vec<(usize, usize)> = editor
        .search_matches
        .iter()
        .filter(|&&(my, _, _)| my == y)
        .map(|&(_, s, e)| (s, e))
        .collect();
    let current_match = editor
        .search_matches
        .get(editor.search_current)
        .filter(|&&(my, _, _)| my == y)
        .map(|&(_, s, e)| (s, e));

    let mut has_content = false;
    // ⚠️ **描く範囲の外も桁を進める。** TAB の幅は論理行の先頭から数えるので、
    // 折り返した 2 行目だけを描くときも、そこまでに何桁使ったかを知っている必要がある。
    let mut col = 0usize;

    for (byte_pos, ch) in line_text.char_indices() {
        let in_range = if byte_end > byte_start {
            byte_pos >= byte_start && byte_pos < byte_end
        } else {
            true
        };

        if in_range {
            // Base style comes from the precomputed highlight cache (tree-sitter
            // or regex fallback), patched onto the theme default so modifier-only
            // highlights (e.g. Markdown bold/italic, which set no fg) keep the
            // theme's foreground/background; the search/yank overlays go on top.
            let mut final_style =
                default_style.patch(editor.highlighter.style_at(y, byte_pos).unwrap_or_default());
            // The mouse selection is a band behind the characters — under the search and
            // yank highlights, which mean "look here" and must stay legible.
            if let Some((from, to)) = selected
                && byte_pos >= from
                && byte_pos < to
            {
                final_style = final_style.bg(selection_color);
            }
            // Yank flash (green): shown until the next keypress so you can see
            // what was just copied. A live search match still wins over it.
            if let Some(hl) = &editor.yank_highlight
                && hl.contains(y, byte_pos)
            {
                final_style = final_style.bg(Color::Green).fg(Color::Black);
            }
            if let Some(&(ms, me)) = line_matches
                .iter()
                .find(|&&(s, e)| byte_pos >= s && byte_pos < e)
            {
                if current_match == Some((ms, me)) {
                    final_style = final_style
                        .bg(Color::Yellow)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD);
                } else {
                    final_style = final_style.bg(Color::Rgb(100, 80, 0)).fg(Color::White);
                }
            }
            // 🚨 **他人のバイトを端末へ渡す関門。** 制御文字はここで `^X` になる ——
            // 素のまま渡すと、ファイルの中身が端末制御を奪う（`ROADMAP.md`
            // 「Stop handing the terminal whatever the file says」）。
            // ⭐ 幅を数える `char_display_width` と**同じ関数**に訊いているので、
            // カーソルの列と描かれた字がずれない。
            spans.push(Span::styled(
                crate::utils::unicode::visible_char_at(ch, col),
                final_style,
            ));
            has_content = true;
        }
        col += crate::utils::unicode::char_display_width_at(ch, col);
    }

    if !has_content {
        // An empty line inside the selection still shows the band, one cell wide.
        let style = if selected.is_some() {
            default_style.bg(selection_color)
        } else {
            default_style
        };
        spans.push(Span::styled(" ", style));
    }

    Line::from(spans)
}

#[cfg(all(test, feature = "treesitter"))]
mod render_color_tests {
    use super::*;
    use crate::state::EditorState;
    use crate::state::buffer::TextBuffer;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn function_name_gets_color_through_render() {
        let mut editor = EditorState::new(Some("probe.rs".to_string()));
        editor.buffer = TextBuffer::from_lines(vec!["fn foo() {}".to_string()]);
        editor
            .highlighter
            .set_file(Some(std::path::Path::new("probe.rs")));
        editor.highlighter.mark_dirty();
        editor.mode = EditorMode::Edit;

        let backend = TestBackend::new(40, 4);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_text_buffer(&mut editor, f, Rect::new(0, 0, 40, 4)))
            .unwrap();
        let buf = term.backend().buffer();
        let mut fgs = std::collections::HashSet::new();
        for x in 0..40u16 {
            if let Some(cell) = buf.cell((x, 0)) {
                fgs.insert(cell.fg);
            }
        }
        assert!(
            fgs.contains(&Color::Magenta),
            "fn keyword should be magenta"
        );
        assert!(
            fgs.contains(&Color::LightBlue),
            "function name should be LightBlue; got {:?}",
            fgs
        );
    }
}

/// 🚨 **描いた字そのものを固定する網。**
///
/// ⚠️ 幅の側（`utils::unicode`）だけを網で押さえていた間、**描画を元に戻しても
/// テストは全部緑のまま**だった（2026-08-28 に陰性対照で発覚）。
/// ⭐ 幅と描画は**別々に壊れる**ので、**別々に固定する**必要がある ——
/// 片方だけ直すと「画面と内部が別々に正しい」状態になり、それがまさに `#8` の症状。
/// Where the cursor goes when the text is clicked at screen cell `(col, row)`.
///
/// Uses the same arithmetic as the frame that was just drawn — the gutter width, the
/// soft-wrap chunks, the scroll offset — so a click lands on the character under the
/// pointer instead of on a guess. `None` when the cell is not over the text area.
///
/// ⭐ The geometry comes from `editor.ui_hit` (recorded by [`render_text_buffer`]) rather
/// than from the arguments, so this stays correct for a frame drawn with a sidebar and
/// for one drawn without.
pub fn click_to_cursor(editor: &EditorState, col: u16, row: u16) -> Option<(usize, usize)> {
    let hit = &editor.ui_hit;
    if !hit.text_area.contains(col, row) || hit.text_width == 0 {
        return None;
    }
    let text_width = hit.text_width as usize;
    let gutter = col < hit.text_x;
    let vcol = (col.saturating_sub(hit.text_x) as usize).min(text_width.saturating_sub(1));
    let target_row = (row - hit.text_area.y) as usize;
    let soft_wrap = editor.soft_wrap && text_width > 0;
    let line_count = editor.buffer.lines.len();

    let mut visual = 0usize;
    let mut buf_y = editor.scroll_offset;
    while visual <= target_row {
        if buf_y >= line_count {
            // Past the last line: park at the end of the buffer rather than do nothing.
            let last = line_count.saturating_sub(1);
            return Some((last, editor.buffer.lines[last].len()));
        }
        let line = &editor.buffer.lines[buf_y];
        if soft_wrap {
            let chunks = wrap_chunks(line, text_width);
            let last_chunk = chunks.len().saturating_sub(1);
            for (i, &(cs, ce)) in chunks.iter().enumerate() {
                if visual + i == target_row {
                    if gutter {
                        return Some((buf_y, cs));
                    }
                    return Some((
                        buf_y,
                        byte_at_visual_col(line, cs, ce, vcol, i == last_chunk),
                    ));
                }
            }
            visual += chunks.len();
        } else {
            if visual == target_row {
                let byte = if gutter {
                    0
                } else {
                    // The window may be scrolled sideways; a click lands on the column the
                    // user sees, not on the column of an unscrolled line.
                    let col = vcol + editor.scroll_x;
                    byte_at_visual_col(line, 0, line.len(), col, true)
                };
                return Some((buf_y, byte));
            }
            visual += 1;
        }
        buf_y += 1;
    }
    None
}

#[cfg(test)]
mod control_char_rendering_tests {
    use super::*;
    use crate::state::EditorState;
    use crate::state::buffer::TextBuffer;

    /// 1 行を描いて、画面に出る文字列を組み立てる。
    fn rendered(line: &str) -> String {
        let mut editor = EditorState::new(None);
        editor.buffer = TextBuffer::from_lines(vec![line.to_string()]);
        render_line(&editor, 0)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    /// 🚨 **本丸** —— ESC が端末へ渡らないこと。生で渡ると、ファイルの中身が
    /// cozy の画面（行番号・フッタ・ステータス行）を書き換えられる。
    #[test]
    fn escape_never_reaches_the_terminal() {
        assert_eq!(rendered("a\u{1b}[2Jb"), "a^[[2Jb");
        assert_eq!(rendered("a\u{1b}]0;t\u{7}b"), "a^[]0;t^Gb");
    }

    #[test]
    fn control_chars_are_drawn_as_caret_notation() {
        assert_eq!(rendered("a\rb\rc"), "a^Mb^Mc");
        assert_eq!(rendered("a\u{8}b"), "a^Hb");
        assert_eq!(rendered("a\u{7f}b"), "a^?b");
    }

    /// 陽性対照 —— 普通の行は**1 文字も変わらない**。
    /// ⭐ これが無いと「全部 `^X` にする」実装が緑で通る。
    #[test]
    fn ordinary_lines_are_drawn_unchanged() {
        assert_eq!(rendered("abc"), "abc");
        assert_eq!(rendered("あい"), "あい");
        // ⚠️ TAB だけは「変わらない」の例外 —— **cozy が空白へ展開する**。
        // 🚨 素通しすると端末が自分のタブストップで桁を進め、cozy の計算とずれる。
        assert_eq!(
            rendered("a\tb"),
            "a       b",
            "TAB は次のタブストップまでの空白に展開される（`a` が 1 桁 → 7 つ）"
        );
    }

    /// ⭐ 描いた桁数と、幅の計算が一致すること。**この 2 つを繋ぐ網**が無いと、
    /// 片方だけ直した状態が緑で通る（実際に一度そうなった）。
    #[test]
    fn drawn_columns_match_the_counted_width() {
        for line in [
            "a\rb",
            "a\u{1b}[31mX",
            "abc",
            "あい",
            "a\tb",
            "\tx",
            "12345678\ty",
        ] {
            let drawn = rendered(line);
            // ⭐ 描いた結果は TAB を含まない（空白に展開済み）ので、単純に足せる。
            let drawn_cols: usize = drawn
                .chars()
                .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
                .sum();
            assert!(!drawn.contains('\t'), "端末に TAB を渡している: {drawn:?}");
            assert_eq!(
                drawn_cols,
                crate::utils::unicode::str_display_width(line),
                "{line:?} を描くと {drawn:?}（{drawn_cols} 桁）だが、幅は別の値と数えている"
            );
        }
    }
}

#[cfg(test)]
mod horizontal_scroll_tests {
    use super::*;
    use crate::state::EditorState;
    use crate::state::buffer::TextBuffer;

    fn rendered_at(line: &str, scroll_x: usize) -> String {
        let mut editor = EditorState::new(None);
        editor.buffer = TextBuffer::from_lines(vec![line.to_string()]);
        editor.soft_wrap = false;
        editor.scroll_x = scroll_x;
        editor.text_display_width = 40;
        render_line(&editor, 0)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    /// The window starts mid-line instead of cutting the text at the right edge.
    #[test]
    fn a_scrolled_line_starts_at_the_offset_column() {
        assert_eq!(rendered_at("0123456789abcdef", 0), "0123456789abcdef");
        let scrolled = rendered_at("0123456789abcdef", 5);
        assert!(
            scrolled.starts_with("56789abcdef"),
            "mulai dari kolom 5: {scrolled:?}"
        );
    }

    /// A click lands on the character the user sees, not on the unscrolled one.
    #[test]
    fn a_click_accounts_for_the_horizontal_offset() {
        let mut editor = EditorState::new(None);
        editor.buffer = TextBuffer::from_lines(vec!["0123456789abcdef".to_string()]);
        editor.soft_wrap = false;
        editor.scroll_x = 5;
        editor.ui_hit.text_area = crate::sidebar::Area::new(0, 0, 40, 5);
        editor.ui_hit.text_x = 0;
        editor.ui_hit.text_width = 40;
        // Column 2 of the window is column 7 of the line.
        let (line, byte) = click_to_cursor(&editor, 2, 0).expect("hit");
        assert_eq!(line, 0);
        assert_eq!(byte, 7);
    }
}

#[cfg(test)]
mod scrollbar_layout_tests {
    use super::*;
    use crate::state::EditorState;
    use crate::state::buffer::TextBuffer;

    fn editor_with(lines: Vec<String>) -> EditorState {
        let mut editor = EditorState::new(None);
        editor.buffer = TextBuffer::from_lines(lines);
        editor.page_size = 10;
        editor.enter_mode(crate::state::EditorMode::Edit);
        editor
    }

    /// A buffer that fits gets the whole rectangle and no bar at all — an empty track
    /// would just eat a column.
    #[test]
    fn a_short_buffer_has_no_scrollbar() {
        let mut editor = editor_with((0..5).map(|i| format!("line {i}")).collect());
        let (text, bars) = fit_scrollbars(&mut editor, Rect::new(0, 0, 40, 10), 0);
        assert_eq!(text, Rect::new(0, 0, 40, 10));
        assert!(bars.vertical.is_none() && bars.horizontal.is_none());
    }

    /// A long one loses its last column to the bar, and the bar covers the window with
    /// a thumb of the right size.
    #[test]
    fn a_long_buffer_reserves_the_last_column() {
        let mut editor = editor_with((0..100).map(|i| format!("line {i}")).collect());
        let (text, bars) = fit_scrollbars(&mut editor, Rect::new(0, 0, 40, 10), 0);
        assert_eq!(text.width, 39, "satu kolom untuk scrollbar");
        let bar = bars.vertical.expect("bar");
        assert_eq!((bar.x, bar.y, bar.width, bar.height), (39, 0, 1, 10));
        assert_eq!(editor.scrollbar_total, 100);
        assert_eq!(editor.scrollbar_viewport, 10);
    }

    /// With soft wrap off and a line wider than the window, the last row goes to the
    /// horizontal bar — the text keeps its last column, since nothing is there to read.
    #[test]
    fn a_wide_line_takes_the_bottom_row() {
        let mut editor = editor_with(vec!["x".repeat(200), "second".to_string()]);
        editor.soft_wrap = false;
        editor.text_display_width = 40;
        let (text, bars) = fit_scrollbars(&mut editor, Rect::new(0, 0, 40, 10), 0);
        let bar = bars.horizontal.expect("bar horizontal");
        assert_eq!((bar.x, bar.y, bar.width), (0, 9, 40));
        assert_eq!(text.height, 9);
        assert!(
            bars.vertical.is_none(),
            "dua baris tidak butuh bar vertikal"
        );
    }

    /// The gutter stays put: the bars belong to the text, not to the line numbers.
    #[test]
    fn the_gutter_is_not_scrolled_by_the_bars() {
        let mut editor = editor_with((0..100).map(|i| format!("line {i}")).collect());
        let (text, bars) = fit_scrollbars(&mut editor, Rect::new(0, 0, 40, 10), 4);
        assert_eq!(text.x, 0);
        assert_eq!(text.width, 39, "40 kolom: bar memakai satu, gutter 4 tetap");
        assert_eq!(text.width - 4, 35, "yang tersisa untuk teks");
        assert_eq!(bars.vertical.unwrap().x, 39);
    }
}

#[cfg(test)]
mod selection_render_tests {
    use super::*;
    use crate::state::EditorState;
    use crate::state::buffer::TextBuffer;

    fn line_with_selection(from: usize, to: usize) -> Vec<(String, Option<Color>)> {
        let mut editor = EditorState::new(None);
        editor.buffer = TextBuffer::from_lines(vec!["abcdef".to_string()]);
        editor.enter_mode(EditorMode::Edit);
        editor.selection = Some(crate::state::Selection {
            anchor: (0, from),
            head: (0, to),
        });
        render_line(&editor, 0)
            .spans
            .iter()
            .map(|s| (s.content.to_string(), s.style.bg))
            .collect()
    }

    /// The band covers exactly the selected characters — edge to edge, not just the text
    /// colour of a `+`/`-` sign (the same rule the diff rows follow).
    #[test]
    fn the_selection_band_covers_the_selected_bytes() {
        let spans = line_with_selection(2, 5);
        for (i, (content, bg)) in spans.iter().enumerate() {
            let inside = (2..5).contains(&i);
            assert_eq!(
                *bg == Some(SELECTION_DARK),
                inside,
                "span {i} ({content:?}) band={bg:?}"
            );
        }
    }

    /// Dragging backwards paints the same band as dragging forwards.
    #[test]
    fn a_backwards_selection_paints_the_same_band() {
        assert_eq!(line_with_selection(5, 2), line_with_selection(2, 5));
    }

    /// An empty line inside the selection shows a one-cell band, so a copied blank line
    /// is visible rather than invisible.
    #[test]
    fn a_selected_empty_line_shows_a_band() {
        let mut editor = EditorState::new(None);
        editor.buffer = TextBuffer::from_lines(vec!["".to_string()]);
        editor.enter_mode(EditorMode::Edit);
        editor.selection = Some(crate::state::Selection {
            anchor: (0, 0),
            head: (0, 0),
        });
        // A click (empty selection) draws nothing.
        assert_eq!(render_line(&editor, 0).spans[0].style.bg, None);
        // A real range over the empty line does.
        editor.selection = Some(crate::state::Selection {
            anchor: (0, 0),
            head: (1, 0),
        });
        editor.buffer.lines.push("x".to_string());
        assert_eq!(
            render_line(&editor, 0).spans[0].style.bg,
            Some(SELECTION_DARK)
        );
    }
}

#[cfg(test)]
mod loading_tests {
    use super::*;

    /// The spinner turns: nine frames, one every 80 ms, and it comes back around.
    #[test]
    fn the_spinner_animates_and_loops() {
        let at = |ms| loading_frame(std::time::Duration::from_millis(ms));
        assert_ne!(at(0), at(80), "frame berikutnya berbeda");
        assert_eq!(at(0), at(80 * 9), "sembilan frame lalu kembali");
        assert_eq!(at(0), at(80 * 90), "berputar terus");
        // Every frame is one cell wide, so the line does not jump while it spins.
        for ms in (0..800).step_by(80) {
            assert_eq!(
                UnicodeWidthStr::width(at(ms).to_string().as_str()),
                1,
                "frame {ms} ms selebar satu sel"
            );
        }
    }

    /// A slow load says so instead of looking stuck at the same word.
    #[test]
    fn a_slow_load_says_it_is_still_working() {
        assert_eq!(
            loading_note(std::time::Duration::from_millis(200)),
            "loading…"
        );
        assert_eq!(
            loading_note(std::time::Duration::from_millis(2000)),
            "still loading…"
        );
    }
}
