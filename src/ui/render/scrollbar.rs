//! The little scrollbars on the right of the sidebar and of the text, and along the
//! bottom of the text when soft wrap is off.
//!
//! Drawn by hand rather than with `ratatui::widgets::Scrollbar`: a terminal has one cell
//! per step, so the geometry is two lines of arithmetic — and owning it means the mouse
//! handler can map a click back to an offset with the same function (see [`thumb_span`]
//! and [`offset_at`]), instead of guessing what the widget did.
//!
//! The glyphs are deliberately quiet: a dim track and an accent thumb. A scrollbar is a
//! position hint, not content.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
};

/// The track: the whole travel of the bar.
const TRACK: Color = Color::Rgb(48, 51, 58);
/// The thumb: where the window is now.
const THUMB: Color = Color::Rgb(92, 99, 112);
/// The thumb while the pointer holds it.
const THUMB_HOT: Color = Color::Rgb(97, 175, 239);

const TRACK_V: &str = "│";
const THUMB_V: &str = "┃";
const TRACK_H: &str = "─";
const THUMB_H: &str = "━";

/// The cells of a `track`-cell bar covered by the thumb: `(start, length)`.
///
/// A bar with nothing to scroll has its thumb over the whole track, which is also what
/// makes the arithmetic total-safe: `total <= viewport` must not divide by zero.
pub fn thumb_span(track: usize, total: usize, viewport: usize, offset: usize) -> (usize, usize) {
    if track == 0 {
        return (0, 0);
    }
    if total <= viewport || viewport == 0 {
        return (0, track);
    }
    // At least one cell, so a huge file still shows a thumb rather than a bare track.
    let len = (track * viewport / total).clamp(1, track);
    let travel = track - len;
    let max_offset = total - viewport;
    let start = offset.min(max_offset) * travel / max_offset;
    (start, len)
}

/// The first content row that a click on `cell` of the bar should show.
///
/// The inverse of [`thumb_span`] for the thumb's start. Clicking the track above or below
/// the thumb therefore jumps (a page-ish move), and dragging it scrolls continuously —
/// both fall out of the same mapping.
pub fn offset_at(track: usize, total: usize, viewport: usize, cell: usize) -> usize {
    if track == 0 || total <= viewport || viewport == 0 {
        return 0;
    }
    let len = (track * viewport / total).clamp(1, track);
    // Centre the thumb on the pointer, so grabbing it does not snap it to the top edge.
    let travel = (track - len).max(1);
    let pos = cell.min(track - 1).saturating_sub(len / 2);
    pos.min(travel) * (total - viewport) / travel
}

/// Draw a vertical bar in `area` (one column wide). `hot` is true while it is dragged.
pub fn draw_vertical(
    f: &mut Frame,
    area: Rect,
    total: usize,
    viewport: usize,
    offset: usize,
    hot: bool,
) {
    if area.is_empty() || total <= viewport {
        return;
    }
    let track = area.height as usize;
    let (start, len) = thumb_span(track, total, viewport, offset);
    let thumb_style = Style::default().fg(if hot { THUMB_HOT } else { THUMB });
    let buf = f.buffer_mut();
    for i in 0..track {
        let inside = i >= start && i < start + len;
        let (sym, style) = if inside {
            (THUMB_V, thumb_style)
        } else {
            (TRACK_V, Style::default().fg(TRACK))
        };
        let cell = &mut buf[(area.x, area.y + i as u16)];
        cell.set_symbol(sym);
        cell.set_style(style);
    }
}

/// Draw a horizontal bar in `area` (one row tall).
pub fn draw_horizontal(
    f: &mut Frame,
    area: Rect,
    total: usize,
    viewport: usize,
    offset: usize,
    hot: bool,
) {
    if area.is_empty() || total <= viewport {
        return;
    }
    let track = area.width as usize;
    let (start, len) = thumb_span(track, total, viewport, offset);
    let thumb_style = Style::default().fg(if hot { THUMB_HOT } else { THUMB });
    let buf = f.buffer_mut();
    for i in 0..track {
        let inside = i >= start && i < start + len;
        let (sym, style) = if inside {
            (THUMB_H, thumb_style)
        } else {
            (TRACK_H, Style::default().fg(TRACK))
        };
        let cell = &mut buf[(area.x + i as u16, area.y)];
        cell.set_symbol(sym);
        cell.set_style(style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bar with nothing to scroll draws no thumb at all (the renderer skips it).
    #[test]
    fn a_fitting_content_fills_the_whole_track() {
        assert_eq!(thumb_span(20, 10, 20, 0), (0, 20));
        assert_eq!(thumb_span(20, 20, 20, 0), (0, 20));
        assert_eq!(thumb_span(0, 100, 20, 0), (0, 0));
    }

    /// At the top the thumb sits at the top; at the bottom it ends at the end.
    #[test]
    fn the_thumb_travels_between_both_ends() {
        let (start, len) = thumb_span(20, 100, 20, 0);
        assert_eq!((start, len), (0, 4));
        let (start, _) = thumb_span(20, 100, 20, 80);
        assert_eq!(start, 16, "16 + 4 = 20, the end of the track");
        // Halfway through the content is halfway down the track.
        let (start, _) = thumb_span(20, 100, 20, 40);
        assert_eq!(start, 8);
    }

    /// A gigantic file still shows a thumb, and an offset past the end cannot overflow.
    #[test]
    fn the_thumb_never_vanishes_or_leaves_the_track() {
        let (start, len) = thumb_span(20, 1_000_000, 20, 999_999);
        assert_eq!(len, 1);
        assert_eq!(start, 19);
    }

    /// Clicking a cell maps to the offset that puts that part of the content at the top.
    #[test]
    fn a_click_maps_back_to_an_offset() {
        assert_eq!(offset_at(20, 100, 20, 0), 0);
        assert_eq!(offset_at(20, 100, 20, 19), 80);
        // Clicking where the thumb already is must not move it (grabbing, not jumping).
        let (start, len) = thumb_span(20, 100, 20, 40);
        assert_eq!(offset_at(20, 100, 20, start + len / 2), 40);
    }
}
