//! The left sidebar: activity header (Files / Git) and whichever tree is showing.
//!
//! Layout mirrors VS Code's left panel: a header row with two tabs, then the tree. Colours
//! carry the information — file-type badges, git status letters, the `+N −M` line counts,
//! and (in the explorer) the tint of a folder that holds a removal or an addition.
//!
//! Every row is built as spans and then padded by [`row`], which keeps each span's colour
//! and right-aligns a tail (the line counts). Without that the panel would be one colour
//! per row, since flattening the spans throws their styles away.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::sidebar::{Area, ChangeFlags, GitEntry, Sidebar, SidebarHit, SidebarTab};
use crate::state::EditorState;

const ACCENT: Color = Color::Rgb(97, 175, 239);
const DIM: Color = Color::Rgb(120, 124, 132);
/// A folder or file with a change in it: VS Code's `modifiedResourceForeground`.
const MODIFIED: Color = Color::Rgb(226, 192, 141);
/// Folders with nothing changed in them — neutral, so yellow can mean something.
const FOLDER: Color = Color::Rgb(180, 184, 192);
const TEXT: Color = Color::Rgb(200, 204, 212);
const ADDED: Color = Color::Rgb(152, 195, 121);
const DELETED: Color = Color::Rgb(224, 108, 117);
const NEW: Color = Color::Rgb(230, 160, 80);
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

/// `≡ Files   ⑂ Git 3        ⑂ main` — two views, the change count, and the branch.
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

// ── explorer ───────────────────────────────────────────────────────────────────

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
        let flags = flags_for(sidebar, &node.path);
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
                Style::default()
                    .fg(folder_color(flags))
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            let (badge, badge_color) = badge_for(&node.name);
            spans.push(Span::raw(format!("{indent}  ")));
            spans.push(Span::styled(
                format!("{badge} "),
                Style::default().fg(badge_color),
            ));
            spans.push(Span::styled(
                node.name.clone(),
                Style::default().fg(file_color(flags)),
            ));
        }
        lines.push(row(
            spans,
            Vec::new(),
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

/// The explorer's one rule, for folders and files alike: a removal is red, an addition
/// orange, a change yellow — and a path with nothing changed keeps the neutral colour.
/// (Red wins over orange, orange over yellow: the louder fact is the one you need.)
fn tint(flags: ChangeFlags) -> Option<Color> {
    if flags.deleted {
        Some(DELETED)
    } else if flags.added {
        Some(NEW)
    } else if flags.modified {
        Some(MODIFIED)
    } else {
        None
    }
}

fn folder_color(flags: ChangeFlags) -> Color {
    tint(flags).unwrap_or(FOLDER)
}

fn file_color(flags: ChangeFlags) -> Color {
    tint(flags).unwrap_or(TEXT)
}

/// What the git view knows about the path the explorer is drawing. Paths in the explorer
/// are absolute; the change list is relative to the panel's root.
fn flags_for(sidebar: &Sidebar, path: &std::path::Path) -> ChangeFlags {
    let rel = path.strip_prefix(&sidebar.root).unwrap_or(path);
    sidebar.git.flags_under(rel)
}

// ── source control ─────────────────────────────────────────────────────────────

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
        let mut tail: Vec<Span<'static>> = Vec::new();
        if let Some(group) = node.group {
            // "▾ Staged Changes 2   +12 −3" — the two sections VS Code shows.
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
            let (added, deleted) = git.totals_under(idx);
            tail = counts(added, deleted, false);
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
                Style::default().fg(MODIFIED).add_modifier(Modifier::BOLD),
            ));
            let (added, deleted) = git.totals_under(idx);
            // A folder's numbers are the sum of its files: present, but quieter.
            tail = counts(added, deleted, true);
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
                tail = counts(entry.added, entry.deleted, false);
            }
        }
        lines.push(row(
            spans,
            tail,
            width,
            idx == git.selected,
            sidebar.focused,
        ));
        hits.push(SidebarHit::Row(idx));
    }
    (lines, hits)
}

/// `+12 −3`, right-aligned. A binary change has no counts to show, so it gets nothing.
fn counts(added: usize, deleted: usize, dim: bool) -> Vec<Span<'static>> {
    if added == 0 && deleted == 0 {
        return Vec::new();
    }
    let style = |c: Color| Style::default().fg(if dim { DIM } else { c });
    vec![
        Span::styled(format!("+{}", thousands(added)), style(ADDED)),
        Span::raw(" "),
        Span::styled(format!("−{}", thousands(deleted)), style(DELETED)),
    ]
}

/// `1234` → `1,234` — the reference panel reads counts at a glance, and 5 digits in a
/// 34-column sidebar is where that stops being possible.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Lay out one row: left spans (colour kept), an optional right-aligned tail, then the
/// selection background across the full width.
fn row(
    spans: Vec<Span<'static>>,
    tail: Vec<Span<'static>>,
    width: usize,
    selected: bool,
    focused: bool,
) -> Line<'static> {
    // One column of breathing room before the panel's border: a count jammed against the
    // border reads as part of it.
    let usable = width.saturating_sub(1);
    let tail_w: usize = tail
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let gap = usize::from(!tail.is_empty());
    let budget = usable.saturating_sub(tail_w + gap);
    let mut all = truncate_spans(spans, budget);
    let used: usize = all
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let pad = usable.saturating_sub(used + tail_w);
    if pad > 0 {
        all.push(Span::raw(" ".repeat(pad)));
    }
    all.extend(tail);
    let total: usize = all
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    if total < width {
        all.push(Span::raw(" ".repeat(width - total)));
    }
    if selected {
        let bg = if focused {
            SELECTED_BG
        } else {
            SELECTED_BG_IDLE
        };
        for span in &mut all {
            span.style = span.style.bg(bg);
        }
    }
    Line::from(all)
}

/// Cut spans to `budget` columns, trimming the span that overflows and dropping the rest.
/// The tail is laid out first, so a long path loses its own name — not the numbers.
fn truncate_spans(spans: Vec<Span<'static>>, budget: usize) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let w = UnicodeWidthStr::width(span.content.as_ref());
        if used + w <= budget {
            used += w;
            out.push(span);
            continue;
        }
        let room = budget.saturating_sub(used);
        if room > 0 {
            let text = truncate_to(span.content.as_ref(), room);
            out.push(Span::styled(text, span.style));
        }
        break;
    }
    out
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
        'A' | 'U' => ADDED,
        'D' | '!' => DELETED,
        'R' | 'C' => Color::Rgb(86, 182, 194),
        'M' => Color::Rgb(229, 192, 123),
        _ => {
            if entry.staged {
                ADDED
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
        return ("gi", DELETED);
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
        "sh" | "bash" | "zsh" | "fish" => ("sh", ADDED),
        "sql" => ("sq", Color::Rgb(198, 120, 221)),
        "css" | "scss" | "sass" | "less" => ("cs", Color::Rgb(86, 182, 194)),
        "html" | "htm" => ("ht", DELETED),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" => {
            ("im", Color::Rgb(198, 120, 221))
        }
        "lock" => ("lk", DIM),
        "log" => ("lg", ADDED),
        "txt" | "text" => ("tx", DIM),
        "pdf" => ("pd", DELETED),
        "c" | "h" | "cc" | "cpp" | "hpp" => ("c", ACCENT),
        "java" => ("jv", DELETED),
        "rb" => ("rb", DELETED),
        "php" => ("ph", Color::Rgb(198, 120, 221)),
        "swift" => ("sw", Color::Rgb(229, 192, 123)),
        "kt" | "kts" => ("kt", Color::Rgb(198, 120, 221)),
        _ => ("··", Color::Rgb(80, 84, 92)),
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    #[test]
    fn a_row_keeps_each_spans_colour() {
        let spans = vec![
            Span::styled("a", Style::default().fg(ADDED)),
            Span::styled("b", Style::default().fg(DELETED)),
        ];
        let line = row(spans, Vec::new(), 10, false, false);
        assert_eq!(line.spans[0].style.fg, Some(ADDED));
        assert_eq!(line.spans[1].style.fg, Some(DELETED));
        let width: usize = line
            .spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        assert_eq!(width, 10, "baris selalu penuh selebar panel");
    }

    #[test]
    fn the_tail_is_right_aligned_and_the_name_gives_way() {
        let spans = vec![Span::raw("a-very-long-file-name.rs")];
        let line = row(spans, counts(190, 2, false), 20, false, false);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        // One column of margin before the border, so a count never touches it.
        assert!(text.ends_with("+190 −2 "), "angka tetap di kanan: {text:?}");
        assert!(
            UnicodeWidthStr::width(text.as_str()) == 20,
            "lebar tetap: {text:?}"
        );
        assert!(!text.contains("name.rs"), "nama dipotong");
    }

    #[test]
    fn a_binary_change_shows_no_counts() {
        assert!(counts(0, 0, false).is_empty());
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(94), "94");
        assert_eq!(thousands(3989), "3,989");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    /// The explorer itself: a folder holding a removal is drawn red, one holding an
    /// addition orange, and a plain folder keeps the folder colour.
    #[test]
    fn explorer_folders_carry_the_change_tint() {
        use crate::sidebar::{GitChanges, GitEntry, GitFile};
        use std::fs;

        let tmp = tempfile::Builder::new()
            .prefix("cozy-tint-")
            .tempdir()
            .unwrap();
        for dir in ["removed", "added", "plain", "quiet"] {
            fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        let mut sidebar = Sidebar::new(tmp.path());
        let entry = |rel: &str, index: char, worktree: char| GitEntry {
            file: GitFile {
                rel: std::path::PathBuf::from(rel),
                index,
                worktree,
            },
            staged: false,
            added: 0,
            deleted: 0,
        };
        sidebar.git = GitChanges {
            repo: true,
            entries: vec![
                entry("removed/gone.rs", ' ', 'D'),
                entry("added/new.rs", '?', '?'),
                entry("plain/edited.rs", ' ', 'M'),
            ],
            ..Default::default()
        };

        let area = ratatui::layout::Rect::new(0, 0, 40, 10);
        let (lines, _) = explorer_lines(&sidebar, area);
        let color_of = |name: &str| -> Option<Color> {
            lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .find(|s| s.content == name)
                .and_then(|s| s.style.fg)
        };
        assert_eq!(color_of("removed"), Some(DELETED), "folder dengan hapusan");
        assert_eq!(color_of("added"), Some(NEW), "folder dengan tambahan");
        assert_eq!(color_of("plain"), Some(MODIFIED), "folder dengan perubahan");
        // A folder with nothing changed in it stays neutral — that was the complaint.
        assert_eq!(
            color_of("quiet"),
            Some(FOLDER),
            "folder tanpa perubahan tidak boleh kuning"
        );
        // And so does the root when nothing under it changed... but here things did, so the
        // root is tinted by what is below it.
        assert_eq!(color_of(&sidebar.explorer.nodes[0].name), Some(DELETED));
    }

    /// End to end: the tint must survive the actual draw, cell by cell.
    #[test]
    fn a_rendered_explorer_carries_the_colours_to_the_screen() {
        use crate::sidebar::{GitChanges, GitEntry, GitFile};
        use crate::state::EditorState;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use std::fs;

        let tmp = tempfile::Builder::new()
            .prefix("cozy-tint-frame-")
            .tempdir()
            .unwrap();
        for dir in ["removed", "plain", "quiet"] {
            fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        let entry = |rel: &str, index: char, worktree: char| GitEntry {
            file: GitFile {
                rel: std::path::PathBuf::from(rel),
                index,
                worktree,
            },
            staged: false,
            added: 0,
            deleted: 0,
        };

        let mut editor = EditorState::new(None);
        let mut sidebar = Sidebar::new(tmp.path());
        sidebar.git = GitChanges {
            repo: true,
            entries: vec![
                entry("removed/gone.rs", ' ', 'D'),
                entry("plain/edited.rs", ' ', 'M'),
            ],
            ..Default::default()
        };
        editor.sidebar = Some(sidebar);

        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|f| render_sidebar(&mut editor, f, f.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let color_of_word = |word: &str| -> Option<Color> {
            for y in 0..buffer.area.height {
                let row: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect();
                if let Some(byte) = row.find(word) {
                    let x = row[..byte].chars().count() as u16;
                    return buffer[(x, y)].fg.into();
                }
            }
            None
        };
        assert_eq!(color_of_word("removed"), Some(DELETED), "folder hapusan");
        assert_eq!(color_of_word("plain"), Some(MODIFIED), "folder perubahan");
        assert_eq!(
            color_of_word("quiet"),
            Some(FOLDER),
            "folder tanpa perubahan"
        );
    }

    #[test]
    fn the_tint_ranks_removal_over_addition_over_change() {
        let flags = |deleted: bool, added: bool, modified: bool| ChangeFlags {
            deleted,
            added,
            modified,
        };
        assert_eq!(
            folder_color(flags(true, true, true)),
            DELETED,
            "yang terhapus menang"
        );
        assert_eq!(folder_color(flags(false, true, true)), NEW);
        assert_eq!(folder_color(flags(false, false, true)), MODIFIED);
        assert_eq!(
            folder_color(flags(false, false, false)),
            FOLDER,
            "folder tanpa perubahan tidak berwarna kuning"
        );
        assert_eq!(
            file_color(flags(false, false, true)),
            MODIFIED,
            "berkas diubah"
        );
        assert_eq!(file_color(flags(false, false, false)), TEXT);
    }
}
