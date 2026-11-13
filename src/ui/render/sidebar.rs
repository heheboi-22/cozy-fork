//! The left sidebar: activity header (Files / Git) and whichever tree is showing.
//!
//! Layout mirrors VS Code's left panel closely enough to feel familiar: a header row
//! with two tabs, then the tree. Colours carry the file-type information (a small
//! two-letter badge instead of an icon font — no Nerd Font required), and the git
//! rows carry the porcelain status letter.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::sidebar::{Area, GitEntry, Sidebar, SidebarHit, SidebarTab};
use crate::state::EditorState;

const ACCENT: Color = Color::Rgb(97, 175, 239);
const DIM: Color = Color::Rgb(120, 124, 132);
const FOLDER: Color = Color::Rgb(220, 200, 120);
const TEXT: Color = Color::Rgb(200, 204, 212);
const SELECTED_BG: Color = Color::Rgb(45, 50, 60);
const SELECTED_BG_IDLE: Color = Color::Rgb(32, 34, 38);

/// Draws the panel and records where it landed in `editor.ui_hit`, so a click can be
/// answered without re-deriving the layout (see `SidebarHit` / `UiHit`).
pub fn render_sidebar(editor: &mut EditorState, f: &mut Frame, area: Rect) {
    let Some(sidebar) = editor.sidebar.as_ref() else {
        return;
    };
    if !sidebar.visible || area.width < 12 || area.height < 3 {
        return;
    }

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(Color::Rgb(60, 62, 68)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 8 || inner.height < 2 {
        return;
    }

    let header = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    let body = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(2),
    };

    // Tab spans come back with the draw: the header knows where it put each word, so a
    // click does not have to measure text again.
    let tabs = render_header(sidebar, f, header);
    let rows = render_body(sidebar, f, body);

    // The borrow of `sidebar` ends here; record the geometry for the mouse handler.
    editor.ui_hit.sidebar = Area::new(area.x, area.y, area.width, area.height);
    // The right border column is the resize handle (the block draws it inside `area`).
    editor.ui_hit.divider = Some(area.x + area.width.saturating_sub(1));
    editor.ui_hit.header = Area::new(header.x, header.y, header.width, header.height);
    editor.ui_hit.tabs = tabs;
    editor.ui_hit.body = Area::new(body.x, body.y, body.width, body.height);
    editor.ui_hit.rows = rows;
}

/// `▸ Files   ▸ Git 3        ⑂ main` — two views, the change count, and the branch.
///
/// Returns the clickable span of each tab as `(tab, x_start, x_end)`.
fn render_header(sidebar: &Sidebar, f: &mut Frame, area: Rect) -> Vec<(SidebarTab, u16, u16)> {
    let mut spans: Vec<Span> = Vec::new();
    let mut hit: Vec<(SidebarTab, u16, u16)> = Vec::new();
    let mut x = area.x;
    for (i, tab) in [SidebarTab::Files, SidebarTab::Git].into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
            x += 2;
        }
        let style = if sidebar.tab == tab {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(DIM)
        };
        let lead = format!("{} ", marker(tab));
        let start = x;
        x += UnicodeWidthStr::width(lead.as_str()) as u16;
        spans.push(Span::styled(lead, style));
        spans.push(Span::styled(tab.name(), style));
        x += UnicodeWidthStr::width(tab.name()) as u16;
        if tab == SidebarTab::Git && sidebar.git.has_changes() {
            let count = format!(" {}", sidebar.git.count());
            x += UnicodeWidthStr::width(count.as_str()) as u16;
            spans.push(Span::styled(count, Style::default().fg(ACCENT)));
        }
        hit.push((tab, start, x));
    }

    if sidebar.tab == SidebarTab::Git
        && let Some(branch) = sidebar.git.branch.as_deref()
    {
        let used: usize = spans.iter().map(|s| s.content.len()).sum();
        let room = (area.width as usize).saturating_sub(used + 4);
        if room >= 6 {
            let label: String = format!("⑂ {branch}").chars().take(room).collect();
            spans.push(Span::raw(
                " ".repeat((area.width as usize).saturating_sub(used + label.len())),
            ));
            spans.push(Span::styled(label, Style::default().fg(DIM)));
        }
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
    hit
}

fn marker(tab: SidebarTab) -> &'static str {
    match tab {
        SidebarTab::Files => "≡",
        SidebarTab::Git => "⑂",
    }
}

/// Draws the rows and returns what each one was, top to bottom.
fn render_body(sidebar: &Sidebar, f: &mut Frame, area: Rect) -> Vec<SidebarHit> {
    if area.height == 0 {
        return Vec::new();
    }
    let (lines, hits) = match sidebar.tab {
        SidebarTab::Files => explorer_lines(sidebar, area),
        SidebarTab::Git => git_lines(sidebar, area),
    };
    f.render_widget(Paragraph::new(lines), area);
    hits
}

fn explorer_lines(sidebar: &Sidebar, area: Rect) -> (Vec<Line<'static>>, Vec<SidebarHit>) {
    let tree = &sidebar.explorer;
    let width = area.width as usize;
    let height = area.height as usize;
    let visible = tree.visible_nodes();
    let sel_pos = visible
        .iter()
        .position(|&i| i == tree.selected)
        .unwrap_or(0);
    let scroll = if sel_pos < height {
        0
    } else {
        sel_pos - height + 1
    };

    let mut lines = Vec::with_capacity(height);
    let mut hits = Vec::with_capacity(height);
    for &idx in visible.iter().skip(scroll).take(height) {
        let node = &tree.nodes[idx];
        let indent = "  ".repeat(node.depth);
        let mut spans: Vec<Span<'static>> = Vec::new();
        if node.is_dir {
            let arrow = if tree.is_expanded(idx) {
                "▾ "
            } else {
                "▸ "
            };
            spans.push(Span::raw(format!("{indent}{arrow}")));
            spans.push(Span::styled(
                node.name.clone(),
                Style::default().fg(FOLDER).add_modifier(Modifier::BOLD),
            ));
        } else {
            let (badge, color) = badge_for(&node.name);
            spans.push(Span::raw(format!("{indent}  ")));
            spans.push(Span::styled(
                format!("{badge} "),
                Style::default().fg(color),
            ));
            spans.push(Span::styled(node.name.clone(), Style::default().fg(TEXT)));
        }
        lines.push(finish_row(
            spans,
            width,
            idx == tree.selected,
            sidebar.focused,
        ));
        hits.push(SidebarHit::Row(idx));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (empty folder)",
            Style::default().fg(DIM),
        )));
    }
    (lines, hits)
}

fn git_lines(sidebar: &Sidebar, area: Rect) -> (Vec<Line<'static>>, Vec<SidebarHit>) {
    let git = &sidebar.git;
    let width = area.width as usize;
    let height = area.height as usize;

    if !git.repo {
        let msg = match &git.error {
            Some(err) => format!("  {err}"),
            None => "  not a git repository".to_string(),
        };
        return (
            vec![Line::from(Span::styled(msg, Style::default().fg(DIM)))],
            Vec::new(),
        );
    }
    if git.entries.is_empty() {
        return (
            vec![Line::from(Span::styled(
                "  Working tree clean",
                Style::default().fg(DIM),
            ))],
            Vec::new(),
        );
    }

    let visible = git.visible();
    let sel_pos = visible.iter().position(|&i| i == git.selected).unwrap_or(0);
    let scroll = if sel_pos < height {
        0
    } else {
        sel_pos - height + 1
    };

    let mut lines = Vec::with_capacity(height);
    let mut hits = Vec::with_capacity(height);
    for &idx in visible.iter().skip(scroll).take(height) {
        let node = &git.nodes[idx];
        let mut spans: Vec<Span<'static>> = Vec::new();
        if let Some(group) = node.group {
            // "▾ Staged Changes 2" — the two sections VS Code shows.
            let arrow = if git.expanded.contains(&idx) {
                "▾ "
            } else {
                "▸ "
            };
            let count = git.count_under(idx);
            spans.push(Span::styled(
                format!("{arrow}{}", group.label()),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!(" {count}"),
                Style::default().fg(ACCENT),
            ));
        } else if node.is_dir {
            let indent = "  ".repeat(node.depth);
            let arrow = if git.expanded.contains(&idx) {
                "▾ "
            } else {
                "▸ "
            };
            spans.push(Span::raw(format!("{indent}{arrow}")));
            spans.push(Span::styled(
                node.name.clone(),
                Style::default().fg(FOLDER).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!(" {}", git.count_under(idx)),
                Style::default().fg(DIM),
            ));
        } else {
            let indent = "  ".repeat(node.depth);
            spans.push(Span::raw(indent));
            if let Some(entry) = node.file.and_then(|i| git.entries.get(i)) {
                spans.push(Span::styled(
                    format!("{} ", entry.badge()),
                    Style::default()
                        .fg(status_color(entry))
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(node.name.clone(), Style::default().fg(TEXT)));
            }
        }
        lines.push(finish_row(
            spans,
            width,
            idx == git.selected,
            sidebar.focused,
        ));
        hits.push(SidebarHit::Row(idx));
    }
    (lines, hits)
}

/// Truncate to the panel width and paint the selection row edge to edge.
fn finish_row(
    spans: Vec<Span<'static>>,
    width: usize,
    selected: bool,
    focused: bool,
) -> Line<'static> {
    let mut text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    if UnicodeWidthStr::width(text.as_str()) > width {
        text = truncate_to(&text, width);
    }
    let w = UnicodeWidthStr::width(text.as_str());
    let style = if !selected {
        Style::default()
    } else if focused {
        Style::default().bg(SELECTED_BG)
    } else {
        Style::default().bg(SELECTED_BG_IDLE)
    };
    let mut line = Line::from(Span::styled(text, style));
    if width > w {
        line.spans.push(Span::styled(" ".repeat(width - w), style));
    }
    line
}

fn truncate_to(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut w = 0usize;
    for c in text.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out
}

fn status_color(entry: &GitEntry) -> Color {
    match entry.badge() {
        'A' | 'U' => Color::Rgb(152, 195, 121),
        'D' | '!' => Color::Rgb(224, 108, 117),
        'R' | 'C' => Color::Rgb(86, 182, 194),
        'M' => Color::Rgb(229, 192, 123),
        _ => {
            if entry.staged {
                Color::Rgb(152, 195, 121)
            } else {
                Color::Rgb(229, 192, 123)
            }
        }
    }
}

/// Two-letter type badge and its colour — the "icons" of this tree, sized so they line
/// up in one column without a patched font.
fn badge_for(name: &str) -> (&'static str, Color) {
    let lower = name.to_lowercase();
    if lower == ".gitignore" || lower == ".gitattributes" {
        return ("gi", Color::Rgb(224, 108, 117));
    }
    if lower.starts_with("dockerfile") {
        return ("dk", ACCENT);
    }
    if lower == "makefile" {
        return ("mk", Color::Rgb(198, 120, 221));
    }
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "rs" => ("rs", Color::Rgb(222, 165, 132)),
        "md" | "markdown" | "mdx" => ("md", ACCENT),
        "js" | "mjs" | "cjs" | "jsx" => ("js", Color::Rgb(229, 192, 123)),
        "ts" | "tsx" => ("ts", Color::Rgb(86, 182, 194)),
        "json" | "jsonc" => ("{}", Color::Rgb(229, 192, 123)),
        "toml" | "ini" | "cfg" | "conf" | "env" => ("tm", Color::Rgb(198, 120, 221)),
        "yaml" | "yml" => ("ym", Color::Rgb(198, 120, 221)),
        "go" => ("go", Color::Rgb(86, 182, 194)),
        "py" => ("py", ACCENT),
        "sh" | "bash" | "zsh" | "fish" => ("sh", Color::Rgb(152, 195, 121)),
        "sql" => ("sq", Color::Rgb(198, 120, 221)),
        "css" | "scss" | "sass" | "less" => ("cs", Color::Rgb(86, 182, 194)),
        "html" | "htm" => ("ht", Color::Rgb(224, 108, 117)),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" => {
            ("im", Color::Rgb(198, 120, 221))
        }
        "lock" => ("lk", DIM),
        "log" => ("lg", Color::Rgb(152, 195, 121)),
        "txt" | "text" => ("tx", DIM),
        "pdf" => ("pd", Color::Rgb(224, 108, 117)),
        "c" | "h" | "cc" | "cpp" | "hpp" => ("c", ACCENT),
        "java" => ("jv", Color::Rgb(224, 108, 117)),
        "rb" => ("rb", Color::Rgb(224, 108, 117)),
        "php" => ("ph", Color::Rgb(198, 120, 221)),
        "swift" => ("sw", Color::Rgb(229, 192, 123)),
        "kt" | "kts" => ("kt", Color::Rgb(198, 120, 221)),
        _ => ("··", Color::Rgb(80, 84, 92)),
    }
}
