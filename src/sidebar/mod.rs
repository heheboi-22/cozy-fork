//! VS Code-style left sidebar: a file explorer plus a git changes tree.
//!
//! The explorer reuses the same model Browse mode draws (`crate::browse::BrowseTree`) —
//! lazy folder reads, a `.gitignore`-aware background index — so the sidebar and the
//! full-screen Browser can never disagree about what is in the folder.
//!
//! The git side is a small tree of `git status --porcelain` entries: the list VS Code's
//! Source Control view shows, grouped by directory. It is loaded on demand (first look at
//! the tab, after a save, or `r`) rather than every frame: each load is a `git` process.
//!
//! Like `browse`, this module holds no crossterm/ratatui types — colours and glyphs live
//! in `ui::render::sidebar`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::browse::BrowseTree;

/// Panel width when the terminal is wide enough (columns).
pub const DEFAULT_WIDTH: u16 = 34;
/// Upper bound, so a very wide terminal does not turn the sidebar into the editor.
pub const MAX_WIDTH: u16 = 60;
/// Total terminal width below which the sidebar steps aside entirely.
pub const MIN_TOTAL_WIDTH: u16 = 70;

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
    /// The single letter shown next to the row, VS Code style.
    pub fn badge(&self) -> char {
        let pick = if self.index != ' ' && self.index != '?' {
            self.index
        } else {
            self.worktree
        };
        match pick {
            '?' => 'U',
            '!' => '!',
            ' ' => '·',
            other => other,
        }
    }

    pub fn staged(&self) -> bool {
        self.index != ' ' && self.index != '?'
    }

    pub fn deleted(&self) -> bool {
        self.index == 'D' || self.worktree == 'D'
    }
}

/// A row in the git tree: a directory that holds changed files, or a changed file.
#[derive(Debug, Clone)]
pub struct GitNode {
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Index into [`GitChanges::files`] for file rows.
    pub file: Option<usize>,
}

/// The Source Control view: branch, status entries, and a tree over their paths.
#[derive(Debug, Clone, Default)]
pub struct GitChanges {
    pub root: PathBuf,
    /// `false` when the root is not inside a git work tree (no error, just empty).
    pub repo: bool,
    pub branch: Option<String>,
    pub files: Vec<GitFile>,
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
        let mut changes = Self {
            root: root.to_path_buf(),
            ..Default::default()
        };
        match git_status(root) {
            Ok((text, prefix)) => {
                changes.repo = true;
                changes.absorb(&text, &prefix);
            }
            Err(err) if err.contains("not a git repository") => {}
            Err(err) => changes.error = Some(err),
        }
        changes
    }

    /// Total changed files (the badge next to the tab).
    pub fn count(&self) -> usize {
        self.files.len()
    }

    pub fn has_changes(&self) -> bool {
        !self.files.is_empty()
    }

    fn absorb(&mut self, text: &str, prefix: &Path) {
        self.files.clear();
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
            self.files.push(GitFile {
                rel,
                index,
                worktree,
            });
        }
        self.rebuild();
    }

    /// Rebuild the tree from `files`. Parents are created on the way down, and the
    /// result is sorted directories-first like the explorer.
    fn rebuild(&mut self) {
        self.nodes.clear();
        self.expanded.clear();
        let root_name = self
            .root
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| self.root.to_string_lossy().to_string());
        self.nodes.push(GitNode {
            name: root_name,
            is_dir: true,
            depth: 0,
            parent: None,
            children: Vec::new(),
            file: None,
        });
        self.expanded.insert(0);

        let mut index_of: HashMap<PathBuf, usize> = HashMap::new();
        index_of.insert(PathBuf::new(), 0);

        let entries: Vec<(PathBuf, usize)> = self
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.rel.clone(), i))
            .collect();

        for (rel, file_idx) in entries {
            let parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();
            if parts.is_empty() {
                continue;
            }
            let mut cur = 0usize;
            let mut acc = PathBuf::new();
            for (i, part) in parts.iter().enumerate() {
                acc.push(part);
                let last = i + 1 == parts.len();
                if let Some(&existing) = index_of.get(&acc) {
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
                    file: if last { Some(file_idx) } else { None },
                });
                self.nodes[cur].children.push(idx);
                // Directories start open: a change list is read at a glance.
                if !last {
                    self.expanded.insert(idx);
                }
                index_of.insert(acc.clone(), idx);
                cur = idx;
            }
        }

        // Sort with a key captured up front: the closure cannot borrow `self.nodes`
        // while the same vector is being mutated in place.
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

    /// Rows to draw: expanded directories and the files inside them, depth-first.
    pub fn visible(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.nodes.len());
        self.collect_visible(0, &mut out);
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

    /// Changed files under `idx` (used for the count on directory rows).
    pub fn count_under(&self, idx: usize) -> usize {
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

    /// Enter / `l`: open a directory, or hand back the file to open.
    pub fn expand_or_open(&mut self) -> Option<PathBuf> {
        let idx = self.selected;
        if idx < self.nodes.len() && self.nodes[idx].is_dir {
            if !self.expanded.remove(&idx) {
                self.expanded.insert(idx);
            }
            None
        } else {
            let file = self.nodes.get(idx).and_then(|n| n.file)?;
            self.files.get(file).map(|f| self.root.join(&f.rel))
        }
    }

    /// `h` / ←: close an open directory, otherwise step to the parent.
    pub fn collapse_or_parent(&mut self) {
        let idx = self.selected;
        if idx == 0 || idx >= self.nodes.len() {
            return;
        }
        if self.nodes[idx].is_dir && self.expanded.remove(&idx) {
            return;
        }
        if let Some(parent) = self.nodes[idx].parent {
            self.selected = parent;
        }
    }

    /// The selected row's status entry, if the selection is on a file.
    pub fn selected_file(&self) -> Option<&GitFile> {
        let node = self.nodes.get(self.selected)?;
        self.files.get(node.file?)
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
            explorer: BrowseTree::build_lazy(root, false),
            git,
        }
    }

    pub fn refresh_git(&mut self) {
        self.git = GitChanges::load(&self.root);
    }

    /// Panel width for a terminal `total` columns wide.
    pub fn width_for(&self, total: u16) -> u16 {
        self.width.clamp(20, MAX_WIDTH).min(total / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(text: &str, root: &Path, prefix: &str) -> GitChanges {
        let mut changes = GitChanges {
            root: root.to_path_buf(),
            repo: true,
            ..Default::default()
        };
        changes.absorb(text, Path::new(prefix));
        changes
    }

    #[test]
    fn parses_modified_untracked_and_deleted() {
        let root = Path::new("/work/app");
        let text = "## main...origin/main\n M src/main.rs\n?? notes/todo.md\n D old.txt\n";
        let changes = status(text, root, "");
        assert_eq!(changes.branch.as_deref(), Some("main"));
        assert_eq!(changes.files.len(), 3);
        let badges: Vec<char> = changes.files.iter().map(|f| f.badge()).collect();
        assert_eq!(badges, vec!['M', 'U', 'D']);
        assert_eq!(changes.files[1].badge(), 'U', "untracked");
        assert!(changes.files[2].deleted());
        assert!(!changes.files[0].staged(), " M is a work-tree change");
    }

    #[test]
    fn staged_status_wins_and_badge_is_a_single_letter() {
        let root = Path::new("/work/app");
        let changes = status("## main\nM  src/lib.rs\nMM both.rs\n", root, "");
        assert!(changes.files[0].staged());
        assert_eq!(changes.files[0].badge(), 'M');
        assert_eq!(changes.files[1].badge(), 'M', "staged M and work-tree M");
    }

    #[test]
    fn git_paths_are_re_rooted_at_the_sidebar_folder() {
        let root = Path::new("/work/app");
        let text = "## main\n M src/main.rs\n M elsewhere/other.rs\n";
        let changes = status(text, root, "src/");
        assert_eq!(changes.files.len(), 1, "only files under src/");
        assert_eq!(changes.files[0].rel, Path::new("main.rs"));
    }

    #[test]
    fn tree_groups_by_directory_and_opens_every_folder() {
        let root = Path::new("/work/app");
        let text = "## main\n M src/ui/a.rs\n M src/ui/b.rs\n M src/lib.rs\n?? top.md\n";
        let changes = status(text, root, "");
        let visible: Vec<&str> = changes
            .visible()
            .iter()
            .map(|&i| changes.nodes[i].name.as_str())
            .collect();
        assert_eq!(
            visible,
            vec!["app", "src", "ui", "a.rs", "b.rs", "lib.rs", "top.md"]
        );
        let ui = changes.nodes.iter().position(|n| n.name == "ui").unwrap();
        assert_eq!(changes.count_under(ui), 2);
    }

    #[test]
    fn enter_on_a_file_hands_back_the_absolute_path() {
        let root = Path::new("/work/app");
        let changes = status("## main\n M src/main.rs\n", root, "");
        let mut changes = changes;
        let file = changes
            .nodes
            .iter()
            .position(|n| n.name == "main.rs")
            .unwrap();
        changes.selected = file;
        assert_eq!(
            changes.expand_or_open(),
            Some(PathBuf::from("/work/app/src/main.rs"))
        );
    }

    #[test]
    fn collapsing_a_folder_hides_its_rows() {
        let root = Path::new("/work/app");
        let mut changes = status("## main\n M src/a.rs\n", root, "");
        let src = changes.nodes.iter().position(|n| n.name == "src").unwrap();
        changes.selected = src;
        changes.collapse_or_parent();
        let names: Vec<&str> = changes
            .visible()
            .iter()
            .map(|&i| changes.nodes[i].name.as_str())
            .collect();
        assert_eq!(names, vec!["app", "src"]);
    }

    #[test]
    fn renames_show_the_new_name() {
        let root = Path::new("/work/app");
        let changes = status("## main\nR  old.rs -> new.rs\n", root, "");
        assert_eq!(changes.files[0].rel, Path::new("new.rs"));
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
        assert_eq!(changes.files.len(), 1);
    }
}
