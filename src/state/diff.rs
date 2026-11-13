//! Session diff review model.
//!
//! A read-only review surface for *your own changes before you commit them*:
//! `git diff` is parsed into hunks, the user reads the `+`/`-` lines hunk by
//! hunk, approves the ones to keep, and stages them — comfortable `git add -p`.
//! **Standalone, that is the whole feature** (self-review + partial staging, no
//! AI required).
//!
//! The exact same mechanism is the human approval gate of argo-tty's agentic
//! loop: because the diff is **author-blind**, "review changes before commit"
//! works whether you wrote them or an AI did — standalone self-review and the AI
//! "read == approve" gate are one tool with two narratives. Data source is plain
//! `git diff` (uncommitted working-tree changes); approving then staging records
//! the selected hunks into git's index, so a partial selection stages exactly
//! those hunks. The structured approval + index handoff is the seam the AI loop
//! later consumes (a Flux signal / reading the index) without coupling here now.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

/// One `@@ ... @@` hunk, tagged with the file it belongs to. Approval is the
/// unit the user reads at: the `+`/`-` lines inside are the meat, the file
/// header (`+++`/`---`) is just the divider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub file: String,
    pub header: String,
    pub lines: Vec<DiffLine>,
    pub approved: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffReviewState {
    pub hunks: Vec<DiffHunk>,
    pub current: usize,
    pub scroll: usize,
}

impl DiffReviewState {
    pub fn approved_count(&self) -> usize {
        self.hunks.iter().filter(|h| h.approved).count()
    }
}

/// Run `git diff` in `dir` and parse the unified output into hunks.
pub fn load_git_diff(dir: &Path) -> Result<Vec<DiffHunk>, String> {
    let output = Command::new("git")
        .arg("diff")
        .current_dir(dir)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_unified_diff(&text))
}

/// Parse a unified diff into hunks. File headers (`+++ b/path` / `--- a/path`)
/// set the current file; `@@` opens a hunk; `+`/`-`/space lines are content.
pub fn parse_unified_diff(diff: &str) -> Vec<DiffHunk> {
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut current_file = String::new();
    let mut cur: Option<DiffHunk> = None;

    for line in diff.lines() {
        // File header (triple +++ / ---). Check before the single +/- content
        // test so an added line `+foo` is never mistaken for a header.
        if let Some(rest) = line.strip_prefix("+++ ") {
            current_file = rest
                .strip_prefix("b/")
                .unwrap_or(rest)
                .trim_end()
                .to_string();
            continue;
        }
        if line.starts_with("--- ") {
            continue;
        }
        // git metadata lines that are not part of any hunk body.
        if line.starts_with("diff --git")
            || line.starts_with("index ")
            || line.starts_with("new file")
            || line.starts_with("deleted file")
            || line.starts_with("rename ")
            || line.starts_with("copy ")
            || line.starts_with("similarity ")
            || line.starts_with("dissimilarity ")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
            || line.starts_with("Binary files")
            || line.starts_with("\\ No newline")
        {
            continue;
        }
        if line.starts_with("@@") {
            if let Some(h) = cur.take() {
                hunks.push(h);
            }
            cur = Some(DiffHunk {
                file: current_file.clone(),
                header: line.to_string(),
                lines: Vec::new(),
                approved: false,
            });
            continue;
        }
        if let Some(h) = cur.as_mut() {
            let (kind, text) = if let Some(t) = line.strip_prefix('+') {
                (DiffLineKind::Add, t.to_string())
            } else if let Some(t) = line.strip_prefix('-') {
                (DiffLineKind::Del, t.to_string())
            } else {
                let t = line.strip_prefix(' ').unwrap_or(line);
                (DiffLineKind::Context, t.to_string())
            };
            h.lines.push(DiffLine { kind, text });
        }
    }
    if let Some(h) = cur.take() {
        hunks.push(h);
    }
    hunks
}

/// Rebuild unified-diff text from parsed hunks — the inverse of
/// [`parse_unified_diff`] over a chosen subset. One `diff --git` + `---`/`+++`
/// header per file, then each hunk's `@@` header and its lines with the leading
/// sign restored. Hunks of the same file must be consecutive (they always are,
/// since review keeps git's diff order).
fn reconstruct_patch(hunks: &[&DiffHunk]) -> String {
    let mut out = String::new();
    let mut last_file: Option<&str> = None;
    for h in hunks {
        if last_file != Some(h.file.as_str()) {
            out.push_str(&format!("diff --git a/{f} b/{f}\n", f = h.file));
            out.push_str(&format!("--- a/{}\n", h.file));
            out.push_str(&format!("+++ b/{}\n", h.file));
            last_file = Some(h.file.as_str());
        }
        out.push_str(&h.header);
        out.push('\n');
        for l in &h.lines {
            let sign = match l.kind {
                DiffLineKind::Context => ' ',
                DiffLineKind::Add => '+',
                DiffLineKind::Del => '-',
            };
            out.push(sign);
            out.push_str(&l.text);
            out.push('\n');
        }
    }
    out
}

/// Stage the approved hunks into git's index via `git apply --cached`. Approved
/// hunks are emitted as one patch (grouped per file, in original coordinates),
/// so a partial selection stages exactly those hunks — `git add -p` territory.
/// `--recount` lets git recompute the `@@` line counts from the body, which
/// keeps us robust even if a reconstructed header is slightly off. Returns the
/// number of hunks staged; `Ok(0)` (no-op) when nothing is approved.
pub fn stage_approved(dir: &Path, hunks: &[DiffHunk]) -> Result<usize, String> {
    let approved: Vec<&DiffHunk> = hunks.iter().filter(|h| h.approved).collect();
    if approved.is_empty() {
        return Ok(0);
    }
    let patch = reconstruct_patch(&approved);
    let mut child = Command::new("git")
        .args(["apply", "--cached", "--recount", "-"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run git: {e}"))?;
    // Drop stdin after writing so git sees EOF and proceeds.
    child
        .stdin
        .take()
        .ok_or_else(|| "git stdin unavailable".to_string())?
        .write_all(patch.as_bytes())
        .map_err(|e| format!("failed to send patch to git: {e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("git apply failed: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_string());
    }
    Ok(approved.len())
}

/// `git commit -m <msg>` in `dir`. Commits whatever is in the index (the caller
/// stages the approved hunks first). Returns the new short hash on success, or
/// git's stderr (e.g. "nothing to commit") on failure.
pub fn commit_index(dir: &Path, message: &str) -> Result<String, String> {
    let status = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        let stdout = String::from_utf8_lossy(&status.stdout);
        // git puts "nothing to commit" on stdout; surface whichever is non-empty.
        let msg = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(msg.to_string());
    }
    let head = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    Ok(String::from_utf8_lossy(&head.stdout).trim().to_string())
}

/// `git push` in `dir` — pushes the current branch to its upstream. Explicit and
/// separate from commit (an outward-facing action). Returns git's summary line
/// on success, or stderr (no upstream / rejected / auth / offline) on failure.
pub fn push_current(dir: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("push")
        .current_dir(dir)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    // git push writes its progress/summary to stderr even on success.
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(stderr.trim().to_string());
    }
    let summary = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("pushed")
        .trim()
        .to_string();
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/main.rs b/src/main.rs\n\
index 1234567..89abcde 100644\n\
--- a/src/main.rs\n\
+++ b/src/main.rs\n\
@@ -1,3 +1,3 @@\n\
 fn main() {\n\
-    old();\n\
+    new();\n\
 }\n\
diff --git a/README.md b/README.md\n\
--- a/README.md\n\
+++ b/README.md\n\
@@ -10,2 +10,3 @@\n\
 line\n\
+added line\n\
 tail\n";

    #[test]
    fn parses_two_hunks_across_two_files() {
        let hunks = parse_unified_diff(SAMPLE);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].file, "src/main.rs");
        assert_eq!(hunks[1].file, "README.md");
    }

    #[test]
    fn classifies_add_del_context_lines() {
        let hunks = parse_unified_diff(SAMPLE);
        let kinds: Vec<_> = hunks[0].lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DiffLineKind::Context,
                DiffLineKind::Del,
                DiffLineKind::Add,
                DiffLineKind::Context,
            ]
        );
        // The leading sign is stripped from the stored text.
        assert_eq!(hunks[0].lines[2].text, "    new();");
    }

    #[test]
    fn file_header_is_not_mistaken_for_added_line() {
        let hunks = parse_unified_diff(SAMPLE);
        // No hunk line should carry the "++ b/..." header text.
        assert!(
            !hunks
                .iter()
                .flat_map(|h| &h.lines)
                .any(|l| l.text.starts_with("++ ") || l.text.contains("b/src/main.rs"))
        );
    }

    #[test]
    fn empty_diff_yields_no_hunks() {
        assert!(parse_unified_diff("").is_empty());
    }

    #[test]
    fn reconstruct_round_trips_through_parser() {
        // hunks -> patch text -> hunks must preserve file + line content, so the
        // patch we feed `git apply` faithfully encodes what was approved.
        let hunks = parse_unified_diff(SAMPLE);
        let refs: Vec<&DiffHunk> = hunks.iter().collect();
        let reparsed = parse_unified_diff(&reconstruct_patch(&refs));
        assert_eq!(reparsed.len(), hunks.len());
        for (got, want) in reparsed.iter().zip(hunks.iter()) {
            assert_eq!(got.file, want.file);
            assert_eq!(got.header, want.header);
            assert_eq!(got.lines, want.lines);
        }
    }

    #[test]
    fn reconstruct_keeps_only_the_chosen_subset() {
        // Approving just the second hunk must emit a patch with only that file.
        let hunks = parse_unified_diff(SAMPLE);
        let only_second: Vec<&DiffHunk> = hunks.iter().skip(1).collect();
        let patch = reconstruct_patch(&only_second);
        assert!(patch.contains("README.md"));
        assert!(!patch.contains("src/main.rs"));
        let reparsed = parse_unified_diff(&patch);
        assert_eq!(reparsed.len(), 1);
        assert_eq!(reparsed[0].file, "README.md");
    }
}

// ── single-file diff (the Source Control view's reader) ───────────────────────

/// How many rows a single-file diff may hold. A 100k-line diff is not a reading surface;
/// past this the view says it stopped instead of holding the whole thing in memory.
const MAX_FILE_DIFF_ROWS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDiffKind {
    /// A note that is not content: "new file", "binary", a truncation marker.
    Meta,
    /// An `@@ … @@` header.
    Hunk,
    Add,
    Del,
    Context,
}

/// One row of the single-file diff view, with the line numbers the `@@` header promised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiffRow {
    pub kind: FileDiffKind,
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
}

/// One row's syntax colours, as byte ranges into [`FileDiffRow::text`].
///
/// ⭐ Computed once when the diff opens, not per frame: a diff is a still picture, and the
/// parse is the expensive part. It is plain data (`Style` is `Copy`), so the view keeps
/// its `Clone`/`PartialEq` derives.
pub type RowSpans = Vec<(usize, usize, ratatui::style::Style)>;

/// An open single-file diff: what the Source Control row opens into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiffView {
    /// `"src/main.rs"` plus which side it came from.
    pub title: String,
    pub rel: std::path::PathBuf,
    /// `true` when this is the index side (`git diff --cached`).
    pub staged: bool,
    pub rows: Vec<FileDiffRow>,
    /// Syntax colours per row, same indices as `rows`.
    pub spans: Vec<RowSpans>,
    pub scroll: usize,
    /// Set by the renderer — the reducer needs it to clamp scrolling.
    pub view_height: usize,
    /// Row indices of the `@@` headers, for `n`/`N`.
    pub hunks: Vec<usize>,
    pub current: usize,
}

impl FileDiffView {
    pub fn build(rel: &std::path::Path, staged: bool, rows: Vec<FileDiffRow>) -> Self {
        let hunks: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == FileDiffKind::Hunk)
            .map(|(i, _)| i)
            .collect();
        let side = if staged { "staged" } else { "working tree" };
        let spans = highlight_rows(rel, &rows);
        Self {
            title: format!("{}  ·  {side}", rel.display()),
            rel: rel.to_path_buf(),
            staged,
            rows,
            spans,
            scroll: 0,
            view_height: 0,
            hunks,
            current: 0,
        }
    }

    pub fn max_scroll(&self) -> usize {
        self.rows.len().saturating_sub(self.view_height.max(1))
    }

    pub fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll());
    }
}

/// Syntax-colour every row, once, by parsing the whole diff as one document.
///
/// ⚠️ A hunk is a fragment of a file, so a parse of the fragments is approximate — the
/// colours can differ from the editor's at a hunk boundary (an unclosed block is the usual
/// case). That is what every diff viewer does with syntax highlighting, and it is still
/// far more readable than a wall of one colour.
fn highlight_rows(rel: &Path, rows: &[FileDiffRow]) -> Vec<RowSpans> {
    // Row `i` must stay row `i`: the renderer indexes both with the same number.
    let lines: Vec<String> = rows.iter().map(|row| row.text.clone()).collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut highlighter = crate::utils::highlight::Highlighter::new();
    highlighter.set_file(Some(rel));
    highlighter.ensure(&lines, 0..lines.len());
    (0..lines.len())
        .map(|i| highlighter.line_spans(i).to_vec())
        .collect()
}

/// Read one path's diff: `staged` picks the index side, and an untracked file has no diff
/// at all, so it is shown as wholly added (that is also what `git diff --no-index` would
/// say, without the process).
pub fn load_file_diff(
    dir: &Path,
    rel: &Path,
    staged: bool,
    untracked: bool,
) -> Result<Vec<FileDiffRow>, String> {
    if untracked {
        let path = dir.join(rel);
        let bytes = std::fs::read(&path).map_err(|e| format!("could not read file: {e}"))?;
        if bytes.contains(&0) {
            return Ok(vec![FileDiffRow {
                kind: FileDiffKind::Meta,
                old: None,
                new: None,
                text: "binary file — no text diff".to_string(),
            }]);
        }
        let text = String::from_utf8_lossy(&bytes);
        return Ok(rows_from_added(&text));
    }

    let mut cmd = Command::new("git");
    cmd.arg("diff").arg("--no-color").arg("-U3");
    if staged {
        cmd.arg("--cached");
    }
    cmd.arg("--").arg(rel);
    cmd.current_dir(dir);
    let output = cmd
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    if text.trim().is_empty() {
        // Nothing between the two sides (a stat-only change, or a file that was staged
        // and then reverted). Say so rather than show an empty page.
        return Ok(vec![FileDiffRow {
            kind: FileDiffKind::Meta,
            old: None,
            new: None,
            text: "no textual differences".to_string(),
        }]);
    }
    Ok(parse_file_diff(&text))
}

/// A whole file rendered as additions (untracked files, which git has nothing to compare
/// against).
fn rows_from_added(text: &str) -> Vec<FileDiffRow> {
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if rows.len() >= MAX_FILE_DIFF_ROWS {
            rows.push(FileDiffRow {
                kind: FileDiffKind::Meta,
                old: None,
                new: None,
                text: format!("… {} more lines not shown", text.lines().count() - i),
            });
            break;
        }
        rows.push(FileDiffRow {
            kind: FileDiffKind::Add,
            old: None,
            new: Some(i + 1),
            text: line.to_string(),
        });
    }
    rows
}

/// Parse one file's unified diff into display rows. File headers are dropped (the title
/// already says which file this is); notes like "new file mode" are kept.
pub fn parse_file_diff(diff: &str) -> Vec<FileDiffRow> {
    let mut rows = Vec::with_capacity(diff.lines().count().min(MAX_FILE_DIFF_ROWS));
    let mut old_no = 0usize;
    let mut new_no = 0usize;

    for line in diff.lines() {
        if rows.len() >= MAX_FILE_DIFF_ROWS {
            rows.push(FileDiffRow {
                kind: FileDiffKind::Meta,
                old: None,
                new: None,
                text: "… diff truncated".to_string(),
            });
            break;
        }
        if line.starts_with("@@") {
            if let Some((old, new)) = hunk_numbers(line) {
                old_no = old;
                new_no = new;
            }
            rows.push(FileDiffRow {
                kind: FileDiffKind::Hunk,
                old: None,
                new: None,
                text: line.to_string(),
            });
            continue;
        }
        // The per-file plumbing `git diff` emits; the view already knows the file.
        if line.starts_with("diff --git")
            || line.starts_with("index ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
        {
            continue;
        }
        if line.starts_with("new file")
            || line.starts_with("deleted file")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
            || line.starts_with("rename ")
            || line.starts_with("copy ")
            || line.starts_with("similarity ")
            || line.starts_with("dissimilarity ")
            || line.starts_with("Binary files")
            || line.starts_with("\\ No newline")
        {
            rows.push(FileDiffRow {
                kind: FileDiffKind::Meta,
                old: None,
                new: None,
                text: line.trim_start_matches("\\ ").to_string(),
            });
            continue;
        }
        if let Some(text) = line.strip_prefix('+') {
            rows.push(FileDiffRow {
                kind: FileDiffKind::Add,
                old: None,
                new: Some(new_no),
                text: text.to_string(),
            });
            new_no += 1;
        } else if let Some(text) = line.strip_prefix('-') {
            rows.push(FileDiffRow {
                kind: FileDiffKind::Del,
                old: Some(old_no),
                new: None,
                text: text.to_string(),
            });
            old_no += 1;
        } else {
            let text = line.strip_prefix(' ').unwrap_or(line);
            rows.push(FileDiffRow {
                kind: FileDiffKind::Context,
                old: Some(old_no),
                new: Some(new_no),
                text: text.to_string(),
            });
            old_no += 1;
            new_no += 1;
        }
    }
    rows
}

/// `@@ -12,7 +12,9 @@` → `(12, 12)` (the first line of each side).
fn hunk_numbers(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old_part, rest) = rest.split_once(' ')?;
    let rest = rest.strip_prefix('+')?;
    let new_part = rest.split(' ').next()?;
    let old = old_part.split(',').next()?.parse().ok()?;
    let new = new_part.split(',').next()?.parse().ok()?;
    Some((old, new))
}

#[cfg(test)]
mod file_diff_tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/main.rs b/src/main.rs
index 1111111..2222222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
+    println!(\"more\");
 }
";

    #[test]
    fn parses_hunks_with_both_line_numbers() {
        let rows = parse_file_diff(DIFF);
        let kinds: Vec<FileDiffKind> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                FileDiffKind::Hunk,
                FileDiffKind::Context,
                FileDiffKind::Del,
                FileDiffKind::Add,
                FileDiffKind::Add,
                FileDiffKind::Context,
            ]
        );
        assert_eq!(rows[1].old, Some(1), "context: baris sisi lama");
        assert_eq!(rows[1].new, Some(1), "context: baris sisi baru");
        assert_eq!(rows[2].old, Some(2), "hapus: hanya nomor lama");
        assert_eq!(rows[2].new, None);
        assert_eq!(rows[3].new, Some(2), "tambah: hanya nomor baru");
        assert_eq!(rows[5].old, Some(3));
        assert_eq!(rows[5].new, Some(4), "baris baru menggeser nomor");
    }

    #[test]
    fn the_file_header_is_not_a_row() {
        let rows = parse_file_diff(DIFF);
        assert!(
            !rows.iter().any(|r| r.text.contains("diff --git")),
            "judul sudah menyebut berkasnya"
        );
    }

    #[test]
    fn notes_like_new_file_are_kept_as_meta() {
        let rows =
            parse_file_diff("diff --git a/x b/x\nnew file mode 100644\n@@ -0,0 +1,1 @@\n+hi\n");
        assert_eq!(rows[0].kind, FileDiffKind::Meta);
        assert_eq!(rows[0].text, "new file mode 100644");
        assert_eq!(rows[1].kind, FileDiffKind::Hunk);
    }

    #[test]
    fn an_untracked_file_is_all_additions() {
        let rows = rows_from_added("one\ntwo\n");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.kind == FileDiffKind::Add));
        assert_eq!(rows[1].new, Some(2));
    }

    #[test]
    fn the_view_knows_where_its_hunks_are() {
        let rows = parse_file_diff(DIFF);
        let view = FileDiffView::build(std::path::Path::new("src/main.rs"), false, rows);
        assert_eq!(view.hunks, vec![0]);
        assert_eq!(view.title, "src/main.rs  ·  working tree");
    }

    /// The real thing: a temporary repository with a staged and an unstaged edit.
    #[test]
    fn loads_the_two_sides_of_a_real_repository() {
        let tmp = tempfile::Builder::new()
            .prefix("cozy-file-diff-")
            .tempdir()
            .unwrap();
        let dir = tmp.path();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} gagal");
        };
        run(&["init", "-q", "."]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
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
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();

        let unstaged = load_file_diff(dir, Path::new("a.txt"), false, false).unwrap();
        assert!(
            unstaged
                .iter()
                .any(|r| r.kind == FileDiffKind::Add && r.text == "two"),
            "sisi work-tree memuat baris baru"
        );
        assert!(
            !unstaged
                .iter()
                .any(|r| r.kind == FileDiffKind::Add && r.text == "one"),
            "sisi work-tree tidak memuat baris yang sudah di-commit"
        );

        run(&["add", "-A"]);
        let staged = load_file_diff(dir, Path::new("a.txt"), true, false).unwrap();
        assert!(
            staged
                .iter()
                .any(|r| r.kind == FileDiffKind::Add && r.text == "two")
        );
    }

    #[test]
    fn an_untracked_file_loads_as_additions() {
        let tmp = tempfile::Builder::new()
            .prefix("cozy-file-diff-new-")
            .tempdir()
            .unwrap();
        std::fs::write(tmp.path().join("new.txt"), "alpha\nbeta\n").unwrap();
        let rows = load_file_diff(tmp.path(), Path::new("new.txt"), false, true).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.kind == FileDiffKind::Add));
    }
}
