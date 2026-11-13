//! VS Code-style left sidebar: a file explorer plus a git changes tree.
//!
//! The explorer reuses the same model Browse mode draws (`crate::browse::BrowseTree`) —
//! lazy folder reads, a `.gitignore`-aware background index — so the sidebar and the
//! full-screen Browser can never disagree about what is in the folder.
//!
//! The git side is the list VS Code's Source Control view shows: `git status --porcelain`
//! split into **Staged Changes** (what is in the index) and **Changes** (what is only in
//! the working tree), each grouped into a small tree of directories. A file that is both
//! staged and modified again appears in both, exactly as VS Code shows it. The list is
//! loaded on demand (first look at the tab, after a save, or `r`) rather than every frame:
//! each load is a `git` process.
//!
//! Like `browse`, this module holds no crossterm/ratatui types — colours and glyphs live
//! in `ui::render::sidebar`.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::browse::BrowseTree;

/// Panel width when the terminal is wide enough (columns).
pub const DEFAULT_WIDTH: u16 = 34;
/// Upper bound, so a very wide terminal does not turn the sidebar into the editor.
pub const MAX_WIDTH: u16 = 60;
/// Total terminal width below which the sidebar steps aside entirely.
pub const MIN_TOTAL_WIDTH: u16 = 70;

// ── screen geometry (mouse) ────────────────────────────────────────────────────

/// A screen rectangle in cells. Plain numbers on purpose: the state layer stays free of
/// ratatui types, and a click can be resolved without asking the renderer again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Area {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Area {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// First column past the right edge.
    pub fn right(&self) -> u16 {
        self.x.saturating_add(self.w)
    }

    /// First row past the bottom edge.
    pub fn bottom(&self) -> u16 {
        self.y.saturating_add(self.h)
    }

    pub fn contains(&self, col: u16, row: u16) -> bool {
        !self.is_empty()
            && col >= self.x
            && col < self.right()
            && row >= self.y
            && row < self.bottom()
    }
}

/// What a click on a panel body row means: a node of whichever tree is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarHit {
    Row(usize),
}

/// The screen as the last frame drew it — the map a mouse click is resolved against.
///
/// ⭐ This is the same idea as `desktop-cli`'s `Hit` in WA-Bot: the renderer records the
/// rectangles it just drew, and the mouse handler answers questions about them without
/// re-deriving the layout (which would drift the moment a row wraps or a folder opens).
#[derive(Debug, Clone, Default)]
pub struct UiHit {
    /// The whole frame, for clamping a drag-resize.
    pub screen: Area,
    /// The panel including its right border. Empty when it was not drawn.
    pub sidebar: Area,
    /// The column of the panel's right border — the resize handle.
    pub divider: Option<u16>,
    /// The header row and the clickable spans in it: `(tab, x_start, x_end)`.
    pub header: Area,
    pub tabs: Vec<(SidebarTab, u16, u16)>,
    /// The tree rows, top to bottom; index 0 sits at `body.y`.
    pub body: Area,
    pub rows: Vec<SidebarHit>,
    /// The text area and where the text itself starts inside it (past the gutter).
    pub text_area: Area,
    pub text_x: u16,
    pub text_width: u16,
}

impl UiHit {
    pub fn row_at(&self, col: u16, row: u16) -> Option<SidebarHit> {
        if !self.body.contains(col, row) {
            return None;
        }
        self.rows.get((row - self.body.y) as usize).copied()
    }

    pub fn tab_at(&self, col: u16, row: u16) -> Option<SidebarTab> {
        if !self.header.contains(col, row) {
            return None;
        }
        self.tabs
            .iter()
            .find(|(_, start, end)| col >= *start && col < *end)
            .map(|(tab, _, _)| *tab)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarTab {
    Files,
    Git,
}

impl SidebarTab {
    pub fn next(self) -> Self {
        match self {
            Self::Files => Self::Git,
            Self::Git => Self::Files,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Git => "Git",
        }
    }
}

// ── git status model ───────────────────────────────────────────────────────────

/// One `git status` entry: the two porcelain columns plus the path, relative to the
/// sidebar root (not the repository root — `GitChanges::load` strips the cwd prefix).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFile {
    pub rel: PathBuf,
    /// Index (staged) column of `git status --porcelain`.
    pub index: char,
    /// Work-tree (unstaged) column of `git status --porcelain`.
    pub worktree: char,
}

impl GitFile {
    /// The letter shown next to the row, from whichever side this row belongs to.
    pub fn badge(&self, staged: bool) -> char {
        let pick = if staged { self.index } else { self.worktree };
        match pick {
            '?' => 'U',
            '!' => '!',
            ' ' => '·',
            other => other,
        }
    }

    pub fn untracked(&self) -> bool {
        self.index == '?' && self.worktree == '?'
    }

    /// Changed in the index (staged), i.e. it belongs in the "Staged Changes" group.
    pub fn staged(&self) -> bool {
        self.index != ' ' && self.index != '?'
    }

    /// Changed in the working tree, i.e. it belongs in the "Changes" group.
    pub fn unstaged(&self) -> bool {
        self.worktree != ' ' || self.index == '?'
    }
}

/// One row of the change list: a path on the staged side or the unstaged side. A file
/// that is staged *and* modified again produces two entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitEntry {
    pub file: GitFile,
    pub staged: bool,
    /// Lines added on this side (`git diff --numstat`).
    pub added: usize,
    /// Lines removed on this side.
    pub deleted: usize,
}

impl GitEntry {
    pub fn badge(&self) -> char {
        self.file.badge(self.staged)
    }

    pub fn untracked(&self) -> bool {
        !self.staged && self.file.untracked()
    }
}

/// What lives under a path, for tinting the *explorer* — which only knows files on disk,
/// not git. The tint is ranked: a removal outranks an addition, which outranks a change.
/// A path with nothing changed has all three flags false and is drawn in the neutral
/// colour (a folder is *not* yellow just for existing).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChangeFlags {
    /// Something under this path was deleted — the loudest fact, and the one that wins.
    pub deleted: bool,
    /// Something new appeared (untracked or added to the index).
    pub added: bool,
    /// Something that already existed was changed.
    pub modified: bool,
}

/// The two groups, in the order they are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitGroup {
    Staged,
    Unstaged,
}

impl GitGroup {
    pub fn label(self) -> &'static str {
        match self {
            Self::Staged => "Staged Changes",
            Self::Unstaged => "Changes",
        }
    }

    pub fn staged(self) -> bool {
        matches!(self, Self::Staged)
    }
}

/// A row in the git tree: a section header, a directory that holds changed files, or a
/// changed file.
#[derive(Debug, Clone)]
pub struct GitNode {
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Index into [`GitChanges::entries`] for file rows.
    pub file: Option<usize>,
    /// `Some` for the two section headers ("Staged Changes" / "Changes").
    pub group: Option<GitGroup>,
}

/// The Source Control view: branch, the two groups, and a tree over their paths.
#[derive(Debug, Clone, Default)]
pub struct GitChanges {
    /// `false` when the root is not inside a git work tree (no error, just empty).
    pub repo: bool,
    pub branch: Option<String>,
    pub entries: Vec<GitEntry>,
    pub nodes: Vec<GitNode>,
    pub selected: usize,
    pub expanded: HashSet<usize>,
    pub error: Option<String>,
}

impl GitChanges {
    /// Read `git status` under `root`. Never fails: a missing git binary, a folder that
    /// is not a repository, or a broken repo all end up as a flag or an `error` string,
    /// so the view can say what happened instead of going blank.
    pub fn load(root: &Path) -> Self {
        let mut changes = Self::default();
        match git_status(root) {
            Ok((text, prefix)) => {
                changes.repo = true;
                // The counts come from `git diff --numstat`, one call per side: the
                // porcelain status says *that* a file changed, never how much.
                let staged = numstat(root, &prefix, true);
                let unstaged = numstat(root, &prefix, false);
                changes.absorb(root, &text, &prefix, &staged, &unstaged);
            }
            Err(err) if err.contains("not a git repository") => {}
            Err(err) => changes.error = Some(err),
        }
        changes
    }

    /// Total rows (staged + unstaged) — the badge next to the tab.
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    pub fn staged_count(&self) -> usize {
        self.entries.iter().filter(|e| e.staged).count()
    }

    pub fn unstaged_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.staged).count()
    }

    pub fn has_changes(&self) -> bool {
        !self.entries.is_empty()
    }

    fn absorb(
        &mut self,
        root: &Path,
        text: &str,
        prefix: &Path,
        staged_stats: &HashMap<PathBuf, (usize, usize)>,
        unstaged_stats: &HashMap<PathBuf, (usize, usize)>,
    ) {
        let mut files: Vec<GitFile> = Vec::new();
        self.branch = None;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("## ") {
                self.branch = parse_branch(rest);
                continue;
            }
            if line.len() < 4 {
                continue;
            }
            let bytes = line.as_bytes();
            let index = bytes[0] as char;
            let worktree = bytes[1] as char;
            // `XY <path>`; renames read `R  old -> new`, and the new name is the one
            // that exists on disk now.
            let mut raw = &line[3..];
            if let Some(pos) = raw.find(" -> ") {
                raw = &raw[pos + 4..];
            }
            let raw = unquote(raw);
            let rel = if prefix.as_os_str().is_empty() {
                PathBuf::from(&raw)
            } else {
                match Path::new(&raw).strip_prefix(prefix) {
                    Ok(stripped) => stripped.to_path_buf(),
                    // Outside the folder the sidebar is rooted at — the editor is not
                    // showing that folder, so it has no row to attach to.
                    Err(_) => continue,
                }
            };
            files.push(GitFile {
                rel,
                index,
                worktree,
            });
        }

        // One entry per side, in the order the groups are drawn.
        self.entries.clear();
        for staged in [true, false] {
            for file in &files {
                let belongs = if staged {
                    file.staged()
                } else {
                    file.unstaged()
                };
                if belongs {
                    let stats = if staged { staged_stats } else { unstaged_stats };
                    let (added, deleted) = match stats.get(&file.rel) {
                        Some(&(a, d)) => (a, d),
                        // An untracked file has no diff to count: every line is new.
                        None if file.untracked() => (count_lines(&root.join(&file.rel)), 0),
                        None => (0, 0),
                    };
                    self.entries.push(GitEntry {
                        file: file.clone(),
                        staged,
                        added,
                        deleted,
                    });
                }
            }
        }
        self.rebuild();
    }

    /// Rebuild the tree: node 0 and node 1 are the two group headers, everything else
    /// nests under the one it belongs to. Sorted directories-first, like the explorer.
    fn rebuild(&mut self) {
        self.nodes.clear();
        self.expanded.clear();
        for group in [GitGroup::Staged, GitGroup::Unstaged] {
            let idx = self.nodes.len();
            self.nodes.push(GitNode {
                name: group.label().to_string(),
                is_dir: true,
                depth: 0,
                parent: None,
                children: Vec::new(),
                file: None,
                group: Some(group),
            });
            self.expanded.insert(idx);
        }
        let staged_root = 0usize;
        let unstaged_root = 1usize;

        let mut index_of: HashMap<(usize, PathBuf), usize> = HashMap::new();
        let entries: Vec<(usize, PathBuf)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.file.rel.clone()))
            .collect();

        for (entry_idx, rel) in entries {
            let staged = self.entries[entry_idx].staged;
            let root = if staged { staged_root } else { unstaged_root };
            let parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();
            if parts.is_empty() {
                continue;
            }
            let mut cur = root;
            let mut acc = PathBuf::new();
            for (i, part) in parts.iter().enumerate() {
                acc.push(part);
                let last = i + 1 == parts.len();
                let key = (root, acc.clone());
                if let Some(&existing) = index_of.get(&key) {
                    cur = existing;
                    continue;
                }
                let idx = self.nodes.len();
                let depth = self.nodes[cur].depth + 1;
                self.nodes.push(GitNode {
                    name: part.clone(),
                    is_dir: !last,
                    depth,
                    parent: Some(cur),
                    children: Vec::new(),
                    file: if last { Some(entry_idx) } else { None },
                    group: None,
                });
                self.nodes[cur].children.push(idx);
                // Directories start open: a change list is read at a glance.
                if !last {
                    self.expanded.insert(idx);
                }
                index_of.insert(key, idx);
                cur = idx;
            }
        }

        let keys: Vec<(bool, String)> = self
            .nodes
            .iter()
            .map(|n| (!n.is_dir, n.name.to_lowercase()))
            .collect();
        for node in &mut self.nodes {
            let mut children = std::mem::take(&mut node.children);
            children.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
            node.children = children;
        }
        self.clamp_selection();
    }

    /// Rows to draw: the two group headers (when they have anything in them), their open
    /// directories, and the files inside, depth-first.
    pub fn visible(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.nodes.len());
        for (idx, node) in self.nodes.iter().enumerate() {
            let Some(group) = node.group else { continue };
            let count = if group.staged() {
                self.staged_count()
            } else {
                self.unstaged_count()
            };
            if count == 0 {
                continue;
            }
            out.push(idx);
            if self.expanded.contains(&idx) {
                for &child in &node.children {
                    self.collect_visible(child, &mut out);
                }
            }
        }
        out
    }

    fn collect_visible(&self, idx: usize, out: &mut Vec<usize>) {
        out.push(idx);
        if !self.expanded.contains(&idx) {
            return;
        }
        for &child in &self.nodes[idx].children {
            self.collect_visible(child, out);
        }
    }

    /// Changed files under `idx` (the count beside a group or directory row).
    pub fn count_under(&self, idx: usize) -> usize {
        if let Some(group) = self.nodes[idx].group {
            return if group.staged() {
                self.staged_count()
            } else {
                self.unstaged_count()
            };
        }
        let mut n = 0;
        for &child in &self.nodes[idx].children {
            if self.nodes[child].is_dir {
                n += self.count_under(child);
            } else {
                n += 1;
            }
        }
        n
    }

    /// `(added, deleted)` lines under a group or directory node.
    pub fn totals_under(&self, idx: usize) -> (usize, usize) {
        let mut added = 0;
        let mut deleted = 0;
        let mut stack = vec![idx];
        while let Some(node) = stack.pop() {
            if let Some(entry) = self.nodes[node].file.and_then(|i| self.entries.get(i)) {
                added += entry.added;
                deleted += entry.deleted;
            }
            stack.extend(self.nodes[node].children.iter().copied());
        }
        (added, deleted)
    }

    /// What kind of change sits under `rel` — the explorer's tint. Works for a file too
    /// (the answer is then about that one file).
    pub fn flags_under(&self, rel: &Path) -> ChangeFlags {
        let mut flags = ChangeFlags::default();
        for entry in &self.entries {
            if !entry.file.rel.starts_with(rel) {
                continue;
            }
            if entry.file.index == 'D' || entry.file.worktree == 'D' {
                flags.deleted = true;
                continue;
            }
            let badge = entry.badge();
            if badge == 'U' || badge == 'A' {
                flags.added = true;
            } else {
                flags.modified = true;
            }
        }
        flags
    }

    fn clamp_selection(&mut self) {
        let visible = self.visible();
        if !visible.contains(&self.selected) {
            self.selected = visible.first().copied().unwrap_or(0);
        }
    }

    pub fn move_down(&mut self) {
        let visible = self.visible();
        if let Some(pos) = visible.iter().position(|&i| i == self.selected)
            && pos + 1 < visible.len()
        {
            self.selected = visible[pos + 1];
        }
    }

    pub fn move_up(&mut self) {
        let visible = self.visible();
        if let Some(pos) = visible.iter().position(|&i| i == self.selected)
            && pos > 0
        {
            self.selected = visible[pos - 1];
        }
    }

    pub fn goto_top(&mut self) {
        if let Some(&first) = self.visible().first() {
            self.selected = first;
        }
    }

    pub fn goto_bottom(&mut self) {
        if let Some(&last) = self.visible().last() {
            self.selected = last;
        }
    }

    /// Enter / `l`: open a group or directory, or hand back the entry to diff.
    pub fn expand_or_open(&mut self) -> Option<GitEntry> {
        let idx = self.selected;
        let node = self.nodes.get(idx)?;
        if node.is_dir {
            if !self.expanded.remove(&idx) {
                self.expanded.insert(idx);
            }
            None
        } else {
            node.file.and_then(|i| self.entries.get(i)).cloned()
        }
    }

    /// `h` / ←: close an open directory, otherwise step to the parent.
    pub fn collapse_or_parent(&mut self) {
        let idx = self.selected;
        if idx >= self.nodes.len() {
            return;
        }
        if self.nodes[idx].is_dir && self.expanded.remove(&idx) {
            return;
        }
        if let Some(parent) = self.nodes[idx].parent {
            self.selected = parent;
        }
    }

    /// The selected row's entry, if the selection is on a file.
    pub fn selected_entry(&self) -> Option<&GitEntry> {
        let node = self.nodes.get(self.selected)?;
        self.entries.get(node.file?)
    }

    /// Select the row for `(rel, staged)` if it is still there (used across a reload).
    pub fn select_entry(&mut self, rel: &Path, staged: bool) {
        let found = self
            .entries
            .iter()
            .position(|e| e.staged == staged && e.file.rel == rel);
        if let Some(entry) = found
            && let Some(node) = self
                .nodes
                .iter()
                .position(|n| n.file == Some(entry) && !n.is_dir)
        {
            self.selected = node;
            // Reveal it: open the group and every directory above it.
            let mut cur = self.nodes[node].parent;
            while let Some(idx) = cur {
                self.expanded.insert(idx);
                cur = self.nodes[idx].parent;
            }
        }
    }
}

/// `git diff --numstat` per side, keyed by path relative to the sidebar root.
///
/// `--numstat` prints `<added>\t<deleted>\t<path>`; a binary file has `-` for both
/// counts, which maps to `(0, 0)` — the row still shows its status letter.
fn numstat(dir: &Path, prefix: &Path, cached: bool) -> HashMap<PathBuf, (usize, usize)> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(["diff", "--numstat"]);
    if cached {
        cmd.arg("--cached");
    }
    let Ok(output) = cmd.output() else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut map = HashMap::new();
    for line in text.lines() {
        let mut cols = line.split('\t');
        let (Some(added), Some(deleted), Some(path)) = (cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        let parse = |s: &str| s.parse::<usize>().unwrap_or(0);
        let rel = match Path::new(&unquote(path)).strip_prefix(prefix) {
            Ok(stripped) => stripped.to_path_buf(),
            Err(_) if prefix.as_os_str().is_empty() => PathBuf::from(unquote(path)),
            Err(_) => continue,
        };
        map.insert(rel, (parse(added), parse(deleted)));
    }
    map
}

/// Lines in a file, for an untracked one (git has nothing to diff it against). Reads at
/// most [`MAX_COUNT_BYTES`] so a stray 5 GB artefact cannot stall the refresh.
fn count_lines(path: &Path) -> usize {
    const MAX_COUNT_BYTES: u64 = 4 * 1024 * 1024;
    let Ok(file) = std::fs::File::open(path) else {
        return 0;
    };
    let mut buf = Vec::new();
    if file.take(MAX_COUNT_BYTES).read_to_end(&mut buf).is_err() {
        return 0;
    }
    let newlines = buf.iter().filter(|&&b| b == b'\n').count();
    // A last line without a trailing newline still counts.
    if !buf.is_empty() && !buf.ends_with(b"\n") {
        newlines + 1
    } else {
        newlines
    }
}

/// `git status --porcelain=v1 -b -uall`, plus the path from the repository root to
/// `root` (empty at the top). Porcelain paths are always repository-relative, so the
/// prefix is what turns them into rows the sidebar can nest.
fn git_status(root: &Path) -> Result<(String, PathBuf), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain=v1", "-b", "-uall"])
        .output()
        .map_err(|e| format!("git not available: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let prefix = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-prefix"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim_end()))
        .unwrap_or_default();
    Ok((String::from_utf8_lossy(&out.stdout).into_owned(), prefix))
}

fn parse_branch(rest: &str) -> Option<String> {
    let rest = rest.trim();
    if let Some(name) = rest.strip_prefix("No commits yet on ") {
        return Some(name.trim().to_string());
    }
    let head = rest.split("...").next()?.trim();
    if head.is_empty() {
        None
    } else {
        Some(head.to_string())
    }
}

/// git quotes paths that contain spaces/quotes/unicode: `"a b.rs"`.
fn unquote(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    trimmed.to_string()
}

// ── the panel ──────────────────────────────────────────────────────────────────

/// The whole left panel: which tab is showing, where the keyboard is, and both trees.
#[derive(Debug)]
pub struct Sidebar {
    /// Folder the explorer and the git view are rooted at (the editor's working dir).
    pub root: PathBuf,
    pub visible: bool,
    /// Keyboard focus is in the sidebar, so `j`/`k`/Enter drive the tree, not the buffer.
    pub focused: bool,
    pub tab: SidebarTab,
    pub width: u16,
    pub explorer: BrowseTree,
    pub git: GitChanges,
}

impl Sidebar {
    /// A sidebar rooted at `root`. The explorer reads the top level only; the rest of
    /// the tree arrives from its background index (see `BrowseTree::tick`).
    pub fn new(root: &Path) -> Self {
        // Only pay for `git status` where there is a repository to ask about.
        let git = if root.join(".git").exists() {
            GitChanges::load(root)
        } else {
            GitChanges::default()
        };
        Self {
            root: root.to_path_buf(),
            visible: true,
            focused: false,
            tab: SidebarTab::Files,
            width: DEFAULT_WIDTH,
            // `skip_hidden = false`: a VS Code-like explorer shows `.vscode`, `.env`.
            explorer: BrowseTree::build_lazy(root, false),
            git,
        }
    }

    /// Re-read `git status`, keeping the cursor on the same change when it is still there.
    pub fn refresh_git(&mut self) {
        let keep = self
            .git
            .selected_entry()
            .map(|e| (e.file.rel.clone(), e.staged));
        self.git = GitChanges::load(&self.root);
        if let Some((rel, staged)) = keep {
            self.git.select_entry(&rel, staged);
        }
    }

    /// Panel width for a terminal `total` columns wide.
    pub fn width_for(&self, total: u16) -> u16 {
        self.width.clamp(20, MAX_WIDTH).min(total / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse porcelain output against a fake root (nothing here touches the disk).
    fn status(text: &str, root: &Path, prefix: &str) -> GitChanges {
        let mut changes = GitChanges {
            repo: true,
            ..Default::default()
        };
        changes.absorb(
            root,
            text,
            Path::new(prefix),
            &HashMap::new(),
            &HashMap::new(),
        );
        changes
    }

    fn names(changes: &GitChanges) -> Vec<&str> {
        changes
            .visible()
            .iter()
            .map(|&i| changes.nodes[i].name.as_str())
            .collect()
    }

    #[test]
    fn a_rect_answers_its_own_edges() {
        let r = Area::new(10, 4, 5, 3);
        assert!(r.contains(10, 4), "left/top corner is inside");
        assert!(r.contains(14, 6), "right/bottom corner is inside");
        assert!(!r.contains(15, 6), "right edge is exclusive");
        assert!(!r.contains(14, 7), "bottom edge is exclusive");
        assert!(
            !Area::default().contains(0, 0),
            "an empty rect holds nothing"
        );
    }

    fn ui_hit() -> UiHit {
        UiHit {
            screen: Area::new(0, 0, 100, 30),
            sidebar: Area::new(0, 0, 34, 24),
            divider: Some(33),
            header: Area::new(1, 0, 32, 1),
            tabs: vec![(SidebarTab::Files, 1, 8), (SidebarTab::Git, 10, 19)],
            body: Area::new(1, 2, 32, 22),
            rows: vec![SidebarHit::Row(0), SidebarHit::Row(3), SidebarHit::Row(7)],
            ..Default::default()
        }
    }

    #[test]
    fn a_click_on_a_body_row_resolves_to_its_node() {
        let hit = ui_hit();
        assert_eq!(hit.row_at(5, 2), Some(SidebarHit::Row(0)));
        assert_eq!(hit.row_at(5, 4), Some(SidebarHit::Row(7)));
        assert_eq!(hit.row_at(5, 5), None, "past the drawn rows");
        assert_eq!(hit.row_at(40, 2), None, "outside the panel");
    }

    #[test]
    fn a_click_on_the_header_resolves_to_its_tab() {
        let hit = ui_hit();
        assert_eq!(hit.tab_at(3, 0), Some(SidebarTab::Files));
        assert_eq!(hit.tab_at(14, 0), Some(SidebarTab::Git));
        assert_eq!(hit.tab_at(9, 0), None, "the gap between tabs is not a tab");
        assert_eq!(hit.tab_at(3, 1), None, "row 1 is not the header");
    }

    #[test]
    fn parses_modified_untracked_and_deleted() {
        let root = Path::new("/work/app");
        let text = "## main...origin/main\n M src/main.rs\n?? notes/todo.md\n D old.txt\n";
        let changes = status(text, root, "");
        assert_eq!(changes.branch.as_deref(), Some("main"));
        assert_eq!(changes.count(), 3);
        assert_eq!(changes.staged_count(), 0, "semua perubahan work-tree");
        assert_eq!(changes.unstaged_count(), 3);
        let badges: Vec<char> = changes.entries.iter().map(|e| e.badge()).collect();
        assert_eq!(badges, vec!['M', 'U', 'D']);
        assert!(changes.entries[1].untracked());
        assert_eq!(changes.entries[2].badge(), 'D', "berkas terhapus");
    }

    #[test]
    fn staged_and_unstaged_are_two_groups() {
        let root = Path::new("/work/app");
        // `M ` staged only, ` M` work-tree only, `MM` both.
        let text = "## main\nM  a.rs\n M b.rs\nMM c.rs\n?? d.rs\n";
        let changes = status(text, root, "");
        assert_eq!(changes.staged_count(), 2, "a.rs + c.rs");
        assert_eq!(changes.unstaged_count(), 3, "b.rs + c.rs + d.rs");
        assert_eq!(
            names(&changes),
            vec![
                "Staged Changes",
                "a.rs",
                "c.rs",
                "Changes",
                "b.rs",
                "c.rs",
                "d.rs",
            ]
        );
    }

    #[test]
    fn a_staged_file_keeps_its_own_badge() {
        let root = Path::new("/work/app");
        let changes = status("## main\nMM c.rs\n", root, "");
        let staged = changes
            .entries
            .iter()
            .find(|e| e.staged)
            .expect("baris staged");
        let unstaged = changes
            .entries
            .iter()
            .find(|e| !e.staged)
            .expect("baris unstaged");
        assert_eq!(staged.badge(), 'M', "sisi index");
        assert_eq!(unstaged.badge(), 'M', "sisi work-tree");
        assert!(staged.file.rel == unstaged.file.rel, "jalur yang sama");
    }

    #[test]
    fn an_empty_group_is_not_drawn() {
        let root = Path::new("/work/app");
        let changes = status("## main\n M only-unstaged.rs\n", root, "");
        assert_eq!(names(&changes), vec!["Changes", "only-unstaged.rs"]);
    }

    #[test]
    fn git_paths_are_re_rooted_at_the_sidebar_folder() {
        let root = Path::new("/work/app");
        let text = "## main\n M src/main.rs\n M elsewhere/other.rs\n";
        let changes = status(text, root, "src/");
        assert_eq!(changes.count(), 1, "only files under src/");
        assert_eq!(changes.entries[0].file.rel, Path::new("main.rs"));
    }

    #[test]
    fn tree_groups_by_directory_and_opens_every_folder() {
        let root = Path::new("/work/app");
        let text = "## main\n M src/ui/a.rs\n M src/ui/b.rs\n M src/lib.rs\n?? top.md\n";
        let changes = status(text, root, "");
        assert_eq!(
            names(&changes),
            vec!["Changes", "src", "ui", "a.rs", "b.rs", "lib.rs", "top.md",]
        );
        let ui = changes.nodes.iter().position(|n| n.name == "ui").unwrap();
        assert_eq!(changes.count_under(ui), 2);
        assert_eq!(changes.count_under(0), 0, "Staged kosong");
        assert_eq!(changes.count_under(1), 4, "Changes berisi 4");
    }

    #[test]
    fn enter_on_a_file_hands_back_its_entry() {
        let root = Path::new("/work/app");
        let mut changes = status("## main\n M src/main.rs\n", root, "");
        let file = changes
            .nodes
            .iter()
            .position(|n| n.name == "main.rs")
            .unwrap();
        changes.selected = file;
        let entry = changes.expand_or_open().expect("entry");
        assert_eq!(entry.file.rel, Path::new("src/main.rs"));
        assert!(!entry.staged);
    }

    #[test]
    fn collapsing_a_folder_hides_its_rows() {
        let root = Path::new("/work/app");
        let mut changes = status("## main\n M src/a.rs\n", root, "");
        let src = changes.nodes.iter().position(|n| n.name == "src").unwrap();
        changes.selected = src;
        changes.collapse_or_parent();
        assert_eq!(names(&changes), vec!["Changes", "src"]);
    }

    #[test]
    fn a_group_header_collapses_too() {
        let root = Path::new("/work/app");
        let mut changes = status("## main\n M a.rs\n", root, "");
        let group = changes
            .visible()
            .into_iter()
            .find(|&i| changes.nodes[i].group == Some(GitGroup::Unstaged))
            .unwrap();
        changes.selected = group;
        changes.collapse_or_parent();
        assert_eq!(names(&changes), vec!["Changes"]);
    }

    #[test]
    fn renames_show_the_new_name() {
        let root = Path::new("/work/app");
        let changes = status("## main\nR  old.rs -> new.rs\n", root, "");
        assert_eq!(changes.entries[0].file.rel, Path::new("new.rs"));
        assert!(changes.entries[0].staged, "R di kolom index = staged");
    }

    #[test]
    fn quoted_paths_are_unquoted() {
        assert_eq!(unquote("\"a b/c.rs\""), "a b/c.rs");
        assert_eq!(unquote("\"a\\\"b.rs\""), "a\"b.rs");
    }

    #[test]
    fn a_branchless_repo_still_parses() {
        let root = Path::new("/work/app");
        let changes = status("## No commits yet on main\n?? a.txt\n", root, "");
        assert_eq!(changes.branch.as_deref(), Some("main"));
        assert_eq!(changes.count(), 1);
    }

    #[test]
    fn totals_add_up_per_group_and_per_folder() {
        let root = Path::new("/work/app");
        let mut changes = status(
            "## main\n M src/a.rs\n M src/ui/b.rs\n?? top.md\n",
            root,
            "",
        );
        for (entry, (added, deleted)) in
            changes
                .entries
                .iter_mut()
                .zip([(2usize, 1usize), (10, 0), (5, 0)])
        {
            entry.added = added;
            entry.deleted = deleted;
        }
        let src = changes.nodes.iter().position(|n| n.name == "src").unwrap();
        assert_eq!(changes.totals_under(src), (12, 1));
        let group = changes
            .nodes
            .iter()
            .position(|n| n.group == Some(GitGroup::Unstaged))
            .unwrap();
        assert_eq!(changes.totals_under(group), (17, 1), "grup menjumlah semua");
    }

    #[test]
    fn flags_tell_the_explorer_what_lives_under_a_folder() {
        let root = Path::new("/work/app");
        let changes = status(
            "## main\n D src/gone.rs\n?? src/new.rs\n M src/old.rs\n",
            root,
            "",
        );
        let src = changes.flags_under(Path::new("src"));
        assert!(src.deleted, "ada berkas terhapus");
        assert!(src.added, "ada berkas baru");
        assert!(changes.flags_under(Path::new("src/gone.rs")).deleted);
        let untouched = changes.flags_under(Path::new("src/old.rs"));
        assert!(!untouched.deleted && !untouched.added);
        let other = changes.flags_under(Path::new("other"));
        assert!(!other.deleted && !other.added);
    }

    /// The counts come from `git diff --numstat` — measured on a real repository.
    #[test]
    fn loads_line_counts_from_a_real_repository() {
        use std::process::Command as Cmd;
        let tmp = tempfile::Builder::new()
            .prefix("cozy-numstat-")
            .tempdir()
            .unwrap();
        let dir = tmp.path();
        let run = |args: &[&str]| {
            let out = Cmd::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} gagal");
        };
        run(&["init", "-q", "."]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        run(&["add", "-A"]);
        run(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ]);
        // one line replaced, one added
        std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\nfour\n").unwrap();
        std::fs::write(dir.join("new.txt"), "alpha\nbeta\n").unwrap();

        let changes = GitChanges::load(dir);
        let a = changes
            .entries
            .iter()
            .find(|e| e.file.rel == Path::new("a.txt"))
            .expect("a.txt");
        assert_eq!(
            (a.added, a.deleted),
            (2, 1),
            "satu baris ganti + satu tambah"
        );
        let new = changes
            .entries
            .iter()
            .find(|e| e.file.rel == Path::new("new.txt"))
            .expect("new.txt");
        assert_eq!(
            (new.added, new.deleted),
            (2, 0),
            "untracked: semua baris baru"
        );
        assert_eq!(changes.entries.len(), 2);
    }

    #[test]
    fn select_entry_finds_the_row_again_after_a_reload() {
        let root = Path::new("/work/app");
        let mut changes = status("## main\n M a.rs\n?? b.rs\n", root, "");
        changes.select_entry(Path::new("b.rs"), false);
        assert_eq!(
            changes.selected_entry().unwrap().file.rel,
            Path::new("b.rs")
        );
    }
}
