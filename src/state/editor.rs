use crate::state::Cursor;
use crate::state::FileFormat;
use crate::state::TextBuffer;
use serde::Deserialize;
use std::io;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// 🚨 **`Option` でも `serde(default)` でもなかった。** ∴ この行を書いていない
    /// `config.toml` は**丸ごと**パースに失敗し、`[keys]` も配色も**全部落ちた** ——
    /// しかも落ちたことは `Using default configuration` という 1 行でしか出ない
    /// （`#2` を測っている最中に踏んだ）。
    /// ⭐ 設定は**書いた分だけ効く**べきなので、既定値を持たせる。
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    pub theme: Option<String>,
    pub show_line_numbers: Option<bool>,
    pub status_duration: Option<u64>,
    pub keys: Option<std::collections::HashMap<String, String>>,
    pub line_number_bg: Option<String>,
    pub line_number_fg: Option<String>,
    pub footer_bg: Option<String>,
    pub footer_key_fg: Option<String>,
    pub footer_fg: Option<String>,
    pub status_bar_bg: Option<String>,
    pub status_bar_fg: Option<String>,
    pub cursor_blink: Option<bool>,
    /// Which mode you rest in. "edit" (default) = type like nano. "glide" =
    /// navigate like vim. Affects every action's return target, not just startup.
    pub default_mode: Option<String>,

    /// 設定を読むときに出た苦情。**画面に出すために持ち歩く**。
    ///
    /// 🚨 以前は `eprintln!` で吐いていた。⚠️ cozy は**その端末で TUI を描いている**ので、
    /// 標準エラーはエディタの絵の上に混ざる —— 実際に 1 行目が
    /// `ihelloailed to parse` のような読めない状態になった（`#2` を測っている最中に踏んだ）。
    /// ⭐ `#8` と同じ家族（画面に勝手なものが出る）。
    #[serde(skip)]
    pub load_warnings: Vec<String>,
}

fn default_page_size() -> usize {
    20
}

impl Config {
    pub fn load() -> Self {
        crate::config_io::load()
    }

    /// Load config, optionally overriding the search directory (for iOS sandbox etc.).
    pub fn load_from(config_dir: Option<&PathBuf>) -> Self {
        crate::config_io::load_from(config_dir)
    }

    pub fn user_config_path(config_dir: Option<&PathBuf>) -> Option<PathBuf> {
        crate::config_io::user_config_path(config_dir)
    }

    pub fn load_from_path(path: &std::path::Path) -> io::Result<Self> {
        crate::config_io::load_from_path(path)
    }

    pub fn ensure_default_config_file(config_dir: Option<&PathBuf>) -> io::Result<PathBuf> {
        crate::config_io::ensure_default_config_file(config_dir)
    }

    pub(crate) fn default_values() -> Self {
        Self {
            page_size: 20,
            theme: Some("dark".to_string()),
            show_line_numbers: Some(true),
            status_duration: Some(3),
            line_number_bg: Some("darkgray".to_string()),
            line_number_fg: Some("white".to_string()),
            footer_bg: Some("#222226".to_string()),
            footer_key_fg: Some("cyan".to_string()),
            footer_fg: Some("gray".to_string()),
            status_bar_bg: Some("darkgray".to_string()),
            status_bar_fg: Some("white".to_string()),
            cursor_blink: Some(true),
            keys: None,
            default_mode: None,
            load_warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]

pub enum EditorMode {
    Edit,
    Glide,
    Search,
    Replace,
    Save,
    Open,
    Help,
    Quit,
    Welcome,
    Goto,
    Browse,
    Markdown,
    /// 閲覧だけの面（`czv`）。⭐ **ファイル本文を、`less` と同じ持ち物で読む** ——
    /// スクロール offset だけを持ち、カーソルは無い（`body.rs` の門番が描かない）。
    ///
    /// 🚨 **`Help` / `Markdown` と鍵の綴りは同じだが、座標系が違う。** あの 2 つは
    /// 専用の中身を描くので専用の offset（`help_scroll_offset` 等）を持つが、
    /// こちらは**バッファそのもの**を描くので `editor.scroll_offset` を使う。
    /// ∴ `ReadView` の機械には載せず、`reducer::view_scroll` が面を直接送る。
    ///
    /// ⚠️ 編集アクションは**腕が産まない**ので、`read_only` の検査ではなく
    /// **構造で**止まっている（`#14`）。
    View,
    DiffReview,
    DiffCommitMsg,
    /// Satu berkas, hunk demi hunk — yang dibuka dari baris Source Control.
    ///
    /// ⚠️ **`DiffReview` とは別の面。** あちらは**セッション全体**を承認して stage する
    /// 作業台、こちらは**1 本の差分を読む**だけの閲覧面。混ぜると「承認したつもりで
    /// 読んでいた」が起きる。
    FileDiff,
    Command,
    /// A picture, drawn in place of the text (the file was an image, not a document).
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StatusKind {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchMode {
    MatchCase,
    Regex,
    ByWord,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReplaceFocus {
    Query,
    Replace,
}

pub struct EditorState {
    pub buffer: TextBuffer,
    pub cursor: Cursor,
    pub highlighter: crate::utils::highlight::Highlighter, // syntax highlight cache
    pub filename: Option<PathBuf>,                         // PathBufに変更
    pub _working_dir: PathBuf,                             // 新規: カレントディレクトリ
    pub modified: bool,
    /// 開いているファイルが書き込みを禁じられているか。
    ///
    /// ⭐ **開いた時点で名乗るために持つ。** 保存を押すまで黙っていると、
    /// `Ctrl+S` の失敗が唯一の知らせになり、しかもそのメッセージは狭い端末で切られる
    /// （`#7`）。∴ フッタに常時出す。⚠️ ここは**表示のための事実**で、
    /// 保存を止めているのは `file_io::write_buffer` 側の検査（そちらが正本）。
    pub read_only: bool, // 新規: 変更フラグ
    /// この席が **閲覧専用の席**（`czv`）かどうか。
    ///
    /// 🚨 **`mode` では代用できない。** モードは検索やヘルプで一時的に離れるので、
    /// 「いまどこに居るか」しか言わない。⭐ 必要なのは **「戻る先はどこか」** で、
    /// それを持つのがこの欄（`home_mode` が見る）。
    ///
    /// 📏 **実機で踏んだ**（2026-09-12）: ここが無かったとき、
    /// `czv` → `/` → `Esc` で **編集面に降りた**（帯が `Ctrl+S Save` に変わった）。
    /// ⚠️ `read_only` が保存を止めるので**ファイルは無事**だが、
    /// 「誤操作で汚さない」という約束は**画面の上で破れていた**。
    pub view_session: bool,
    pub mode: EditorMode, // 新規: モード管理
    pub diff_review: Option<crate::state::diff::DiffReviewState>, // 新規: セッション diff レビュー状態
    pub commit_msg_buffer: String, // 新規: diff レビューからのコミットメッセージ入力
    pub save_filename_buffer: String, // 新規: 保存時の入力バッファ
    pub open_filename_buffer: String, // 新規: 読み込み時の入力バッファ
    pub filename_cursor: usize,    // 新規: ファイル名入力カーソル位置 (byte index)
    pub status_message: Option<String>, // Optionに変更
    pub status_timestamp: Option<Instant>, // 名前変更 & Option化
    pub status_persistent: bool,   // 新規: 常時表示フラグ
    pub status_kind: StatusKind,   // 新規: ステータス種別
    pub scroll_offset: usize,      // 新規: スクロール位置
    pub _startup_args: Vec<String>, // 新規: 将来用
    pub _startup_time: Instant,    // 新規: 将来用
    pub shortcut_map: std::collections::HashMap<
        (crate::state::key::KeyCode, crate::state::key::KeyModifiers),
        crate::shortcuts::EditorAction,
    >, // 新規: ショートカットマップ
    pub search_buffer: String,     // 新規: 検索用バッファ
    pub replace_buffer: String,    // 新規: 置換用バッファ
    pub search_cursor: usize,      // 検索/置換入力欄のカーソル位置 (byte index)
    pub replace_focus: ReplaceFocus, // 新規: 置換モードのフォーカス
    pub page_size: usize,          // 新規: ページサイズ
    pub config: Config,            // 新規: 設定
    pub cursor_blink: bool,        // 新規: カーソル点滅フラグ
    pub search_mode: SearchMode,   // 新規: 検索モード
    pub undo_stack: Vec<(TextBuffer, Cursor)>, // 新規: Undoスタック
    pub redo_stack: Vec<(TextBuffer, Cursor)>, // 新規: Redoスタック
    pub last_saved_id: usize,      // 新規: 保存時のスナップショットID（dirty判定用）
    pub help_scroll_offset: usize, // ヘルプ画面のスクロール位置
    pub help_cursor_line: usize,   // ヘルプ画面の現在行（Markdown preview と共通モデル）
    pub help_view_height: usize,   // ヘルプ画面の表示行数
    pub help_rendered_line_count: usize, // ヘルプ画面の折り返し後の行数
    pub markdown_scroll_offset: usize, // Markdown preview のスクロール位置
    pub markdown_cursor_line: usize, // Markdown preview の現在行
    pub markdown_view_height: usize, // Markdown preview の表示行数
    pub markdown_rendered_line_count: usize, // Markdown preview のレンダリング後の行数
    /// プレビューに出ている URL の画像（描くたびに数え直す・`i` が取りに行く相手）。
    /// 端末が画像を描けないとき・feature `remote-images` が無いときは空。
    pub markdown_remote_images: Vec<String>,
    pub show_line_numbers_runtime: Option<bool>, // 新規: ランタイム行番号表示フラグ (Noneならconfigに従う)
    pub goto_line_buffer: String,                // 行ジャンプ入力バッファ
    /// All search match positions: (line_y, byte_start, byte_end)
    pub search_matches: Vec<(usize, usize, usize)>,
    /// Index of the currently focused match in search_matches
    pub search_current: usize,
    /// Whether soft-wrap is active (long lines wrap within the viewport)
    pub soft_wrap: bool,
    /// Runtime visibility for shortcut footers. Prompt input rows remain visible.
    pub footer_visible_runtime: bool,
    /// Text display width (columns after subtracting line-number gutter); set by renderer
    pub text_display_width: usize,
    /// Pending prefix key in Glide mode (e.g. Some('g') while waiting for second 'g')
    pub glide_prefix: Option<char>,
    /// Digit accumulation buffer for Glide count prefix (e.g. "53" for 53j)
    pub glide_count: String,
    /// Unnamed register holding the last delete/yank, for `p`/`P`.
    pub register: Register,
    /// Operator awaiting a motion in Glide mode (`d`/`c`/`y` pressed).
    pub pending_operator: Option<crate::glide::Operator>,
    /// Set while waiting for the target char of a find motion (e.g. after `df`,
    /// `dt`, `dF`, `dT`); the variant records which find family is pending.
    pub glide_find_pending: Option<crate::glide::FindKind>,
    /// The span flashed after a yank, shown until the next keypress.
    pub yank_highlight: Option<YankHighlight>,
    /// The last to-char motion (`>`/`<`/`t`/`T`), replayed by `.` (forward) and
    /// `,` (backward).
    pub last_find: Option<(crate::glide::FindKind, char)>,
    /// The folder tree shown in Browse mode (built on entry, `None` otherwise).
    pub browse_tree: Option<crate::browse::BrowseTree>,
    /// Command palette filter text.
    pub command_query: String,
    /// Selected row within the filtered command palette results.
    pub command_selected: usize,
    /// Where swap files live for this session (`None` = no writable state dir,
    /// so recovery is simply off rather than half-working).
    pub swap_dir: Option<PathBuf>,
    /// The swap file for the open document. Follows Save As.
    pub swap_path: Option<PathBuf>,
    /// Edits since the swap was last written (see `swap::EDITS_PER_WRITE`).
    pub swap_edits: usize,
    /// The buffer has moved on since the swap was written.
    pub swap_dirty: bool,
    /// A swap found at startup, waiting on the user's one-line answer. While
    /// this is set, input is answering the prompt, not editing the buffer.
    pub recovery: Option<crate::swap::Recovery>,
    /// A save that stopped because the target's parent directory does not
    /// exist, waiting on the same kind of one-line answer as `recovery`.
    pub create_dir: Option<crate::file_io::CreateDirOffer>,
    /// The left panel (explorer + source control). `None` for hosts that have no room
    /// for it (compact/mobile) and in unit tests, which opt in by building one.
    pub sidebar: Option<crate::sidebar::Sidebar>,
    /// Where last frame's surfaces were drawn — the map mouse clicks are resolved against.
    /// Rebuilt (and cleared) on every draw.
    pub ui_hit: crate::sidebar::UiHit,
    /// A drag on the panel's right border is in progress (the width follows the pointer).
    pub sidebar_drag: bool,
    /// The single-file diff the Source Control view opened, if any.
    pub file_diff: Option<crate::state::diff::FileDiffView>,
    /// The wheel moved the viewport away from the cursor. While this is set,
    /// `adjust_scroll` leaves the offset alone — the view stays where the wheel put it
    /// until the next keypress asks for the cursor back.
    pub scroll_parked: bool,
    /// Columns scrolled off to the left. Only meaningful with soft wrap **off**: a wrapped
    /// line has nothing to scroll horizontally, so the offset is held at 0 there.
    pub scroll_x: usize,
    /// The content height the scrollbar drew last frame, in rows (soft wrap counted), and
    /// the window it was compared against. The drag handler maps a pointer row back to an
    /// offset with these, so it sees exactly the bar the user grabbed.
    pub scrollbar_total: usize,
    pub scrollbar_viewport: usize,
    /// Which scrollbar the pointer is holding, if any.
    pub scroll_drag: Option<ScrollDrag>,
    /// The picture on screen, when the opened file was an image rather than a document.
    #[cfg(feature = "imageview")]
    pub image_view: Option<crate::imageview::Viewer>,
    /// A picture being decoded / a PDF page being rasterized on a worker thread. The
    /// screen shows a spinner meanwhile — that is the difference between "opening" and
    /// "frozen".
    #[cfg(feature = "imageview")]
    pub image_pending: Option<ImagePending>,
    /// A picture to open once the terminal's protocol is known (startup only).
    #[cfg(feature = "imageview")]
    pub(crate) pending_image: Option<PathBuf>,
    /// The text picked with the mouse, if any. Drawn as a band behind the characters and
    /// copied on release (and by `Ctrl+C`).
    pub selection: Option<Selection>,
    /// The pointer is mid-drag on the text: motion extends the selection instead of
    /// being ignored.
    pub select_drag: bool,
    /// Last press, for the double-click (word) test: when and where it happened.
    pub last_press: Option<(std::time::Instant, (usize, usize))>,
}

/// A scrollbar the pointer is dragging. The sidebar and the text have separate bars, and
/// the text has two axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDrag {
    Sidebar,
    EditorVertical,
    EditorHorizontal,
}

pub(crate) struct EditorStateInit {
    pub filename: Option<String>,
    pub config_dir: Option<PathBuf>,
    pub working_dir: PathBuf,
    pub startup_args: Vec<String>,
    pub startup_time: Instant,
    /// `czv <folder>`: keep the full-screen Browser. Everything else opens a folder
    /// argument in the sidebar, which is the same tree with the buffer beside it.
    pub browse_folders: bool,
    /// A pager session (`czv`, or a host asking for `view`). Pictures are that side's
    /// business — `czv` has its own full-screen viewer, and without image caps it
    /// deliberately opens them as text (`#22`'s negative control). So the editor's
    /// image mode is only armed for the editing session.
    pub pager: bool,
}

impl EditorStateInit {
    pub(crate) fn from_runtime(filename: Option<String>, config_dir: Option<PathBuf>) -> Self {
        Self {
            filename,
            config_dir,
            working_dir: crate::runtime_env::current_working_dir(),
            startup_args: crate::runtime_env::startup_args(),
            startup_time: Instant::now(),
            browse_folders: false,
            pager: false,
        }
    }
}

/// A picture that is still being prepared by a worker thread.
#[cfg(feature = "imageview")]
pub struct ImagePending {
    /// The file name, shown next to the spinner.
    pub label: String,
    /// What is being prepared — a PDF turns pages, a picture steps through its folder, and
    /// the footer says so even before the first frame is ready.
    pub kind: crate::imageview::LoadKind,
    pub started: std::time::Instant,
    pub(crate) rx: std::sync::mpsc::Receiver<Result<crate::imageview::Loaded, String>>,
}

/// A span of text picked with the mouse: two buffer positions (line, byte), kept in the
/// order the pointer crossed them.
///
/// 🚨 This is why the terminal can keep its mouse capture while the panel is up: cozy
/// owns the drag and copies it itself, so plain (non-Shift) drag-select still works —
/// it is cozy's selection, not the terminal's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Where the press landed — the end that stays put while the pointer moves.
    pub anchor: (usize, usize),
    /// Where the pointer is now.
    pub head: (usize, usize),
}

impl Selection {
    pub fn at(pos: (usize, usize)) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    /// `(start, end)` in reading order, start inclusive / end exclusive.
    pub fn range(&self) -> ((usize, usize), (usize, usize)) {
        if self.head < self.anchor {
            (self.head, self.anchor)
        } else {
            (self.anchor, self.head)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// The unnamed register: holds cut/yanked text. `linewise` means the content is
/// whole lines (`dd`, `yy`) rather than an inline span (`D`, `dw`).
#[derive(Debug, Clone, Default)]
pub struct Register {
    pub text: String,
    pub linewise: bool,
}

/// The span just yanked, flashed in the buffer so you can see what was copied
/// (yank has no other visible effect). Cleared on the next keypress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct YankHighlight {
    /// Inclusive start `(line, byte)`.
    pub start: (usize, usize),
    /// Exclusive end `(line, byte)` for charwise; for linewise only `end.0` matters.
    pub end: (usize, usize),
    pub linewise: bool,
}

impl YankHighlight {
    /// Whether the char at `(y, byte)` falls inside this highlight.
    pub fn contains(&self, y: usize, byte: usize) -> bool {
        if self.linewise {
            y >= self.start.0 && y <= self.end.0
        } else {
            (y, byte) >= self.start && (y, byte) < self.end
        }
    }
}

/// Pick a default save name for a new buffer that doesn't clobber an existing
/// file in `dir`: `untitled.txt`, then `untitled (1).txt`, `untitled (2).txt`, …
fn default_save_name(dir: &std::path::Path) -> String {
    let first = "untitled.txt".to_string();
    if !dir.join(&first).exists() {
        return first;
    }
    for n in 1..10_000 {
        let candidate = format!("untitled ({}).txt", n);
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    first
}

impl EditorState {
    #[cfg(test)]
    pub fn new(filename: Option<String>) -> Self {
        Self::new_with_config_dir(filename, None)
    }

    #[cfg(test)]
    pub fn new_with_config_dir(filename: Option<String>, config_dir: Option<&PathBuf>) -> Self {
        let mut state =
            Self::from_init(EditorStateInit::from_runtime(filename, config_dir.cloned()));
        // No sidebar by default in tests: building one spawns a background index over
        // the crate directory for every `EditorState::new`. Tests that exercise the
        // panel build it themselves (`sidebar::tests`, `reducer::sidebar`).
        state.sidebar = None;
        state
    }

    /// Effective line-number visibility. Precedence: an explicit runtime toggle
    /// (Ctrl+L) wins; otherwise the config default, but a compact/low-spec host
    /// (`COZY_COMPACT`) hides them by default to shrink the per-frame gutter.
    /// Single source of truth so the renderer and the toggle reducer agree
    /// (otherwise the first toggle on a compact host would be a no-op).
    /// Whether the left panel is on screen right now: visible, built, and in a mode that
    /// keeps the buffer (the full-screen surfaces — Browser, Help, preview, `czv`'s View —
    /// own the whole area and draw no panel).
    pub fn sidebar_on_screen(&self) -> bool {
        let visible = self.sidebar.as_ref().is_some_and(|s| s.visible);
        visible
            && !matches!(
                self.mode,
                EditorMode::Browse
                    | EditorMode::Help
                    | EditorMode::Markdown
                    | EditorMode::View
                    | EditorMode::DiffReview
                    | EditorMode::DiffCommitMsg
            )
    }

    /// The terminal is too narrow for a panel beside the text, so the two take turns
    /// (`Sidebar` list on screen, then the file, then the list again).
    pub fn narrow_screen(&self) -> bool {
        let width = self.ui_hit.screen.w;
        width > 0 && width < crate::sidebar::MIN_TOTAL_WIDTH
    }

    /// Something worth calling the detail pane: a named file, an unnamed buffer with
    /// edits in it, or a picture being looked at.
    pub fn has_document(&self) -> bool {
        #[cfg(feature = "imageview")]
        if self.image_view.is_some() {
            return true;
        }
        self.filename.is_some() || self.modified
    }

    /// ⭐ The width decides the shape, exactly like WA-Bot's one-pane window:
    ///
    /// - wide: the panel and the text share the screen;
    /// - narrow: one pane at a time. The **list** shows when the user asked for it or when
    ///   there is nothing else to look at, otherwise the **file** does. Clicking a row
    ///   opens that file, which walks the same rule from list to detail; `Esc` and the
    ///   `‹ / »` chip walk back.
    pub fn shows_list(&self, width: u16) -> bool {
        width > 0
            && width < crate::sidebar::MIN_TOTAL_WIDTH
            && self.sidebar_on_screen()
            && self
                .sidebar
                .as_ref()
                .is_some_and(|sidebar| sidebar.focused || !self.has_document())
    }

    /// The narrow layout is showing the file, with the list one step away. (False when
    /// the terminal is wide: there the panel is beside the text, not behind it.)
    pub fn shows_detail(&self, width: u16) -> bool {
        width > 0
            && width < crate::sidebar::MIN_TOTAL_WIDTH
            && self.sidebar_on_screen()
            && !self.shows_list(width)
    }

    pub(crate) fn line_numbers_visible(&self) -> bool {
        self.show_line_numbers_runtime.unwrap_or(
            self.config.show_line_numbers.unwrap_or(true) && !crate::runtime_env::compact(),
        )
    }

    pub(crate) fn from_init(init: EditorStateInit) -> Self {
        let config = Config::load_from(init.config_dir.as_ref());
        let config_warning = config.load_warnings.first().cloned();
        let startup_document =
            crate::file_io::load_startup_document(init.filename.as_deref(), !init.pager);
        // A folder argument re-roots the session: the sidebar, `git status` and relative
        // saves all speak about that folder from now on.
        let working_dir = match &startup_document {
            crate::file_io::StartupDocument::Directory { path } if !init.browse_folders => {
                path.clone()
            }
            _ => init.working_dir.clone(),
        };
        // A picture is opened by the viewer after the terminal exists (it needs the
        // measured protocol); until then the mode says what is on screen.
        #[cfg_attr(not(feature = "imageview"), allow(unused_mut))]
        let mut pending_image: Option<PathBuf> = None;
        let (lines, format, path_buf, initial_mode, browse_tree, startup_error, read_only) =
            match startup_document {
                // A picture or a PDF, not a document: no buffer, no save target.
                crate::file_io::StartupDocument::Image { path }
                | crate::file_io::StartupDocument::Pdf { path } => {
                    pending_image = Some(path);
                    (
                        vec![String::new()],
                        FileFormat::default(),
                        None,
                        EditorMode::Image,
                        None,
                        None,
                        false,
                    )
                }
                crate::file_io::StartupDocument::Empty => (
                    vec![String::new()],
                    FileFormat::default(),
                    None,
                    EditorMode::Welcome,
                    None,
                    None,
                    false,
                ),
                crate::file_io::StartupDocument::Stdin {
                    lines,
                    format,
                    truncated,
                } => (
                    lines,
                    format,
                    Some(PathBuf::from("<stdin>")),
                    EditorMode::View,
                    None,
                    if truncated {
                        Some("Input truncated at 64MB".to_string())
                    } else {
                        None
                    },
                    true,
                ),
                crate::file_io::StartupDocument::File {
                    path,
                    lines,
                    format,
                    read_only,
                } => (
                    lines,
                    format,
                    Some(path),
                    Self::resolve_home(&config),
                    None,
                    None,
                    read_only,
                ),
                // `cozy <folder>` lands in the editor with the folder in the sidebar —
                // the same tree, with the buffer beside it (and `Ctrl+B` still gives the
                // full-screen Browser). `czv <folder>` is a pager: it keeps the full screen.
                crate::file_io::StartupDocument::Directory { path } => {
                    if init.browse_folders {
                        (
                            vec![String::new()],
                            FileFormat::default(),
                            None,
                            EditorMode::Browse,
                            Some(crate::browse::BrowseTree::build(
                                &std::fs::canonicalize(&path).unwrap_or(path.clone()),
                            )),
                            None,
                            false,
                        )
                    } else {
                        (
                            vec![String::new()],
                            FileFormat::default(),
                            None,
                            Self::resolve_home(&config),
                            None,
                            None,
                            false,
                        )
                    }
                }
                // 🚨 **ファイル名を引き受けない**のが肝。空バッファ自体は無害だが、そこに
                // 読めなかったファイルの名前が結びつくと `Ctrl+S` が破壊になる。
                crate::file_io::StartupDocument::Unreadable { message } => (
                    vec![String::new()],
                    FileFormat::default(),
                    None,
                    EditorMode::Welcome,
                    None,
                    Some(message),
                    false,
                ),
            };

        // The left panel is built here rather than on first use, so the folder is on
        // screen from the first frame. Hosts that declared themselves low-spec keep the
        // full width for text (`COZY_COMPACT`), and `COZY_NO_SIDEBAR=1` opts out.
        // A `czv <folder>` session is a full-screen pager: it draws no panel, so it does
        // not pay for a tree walk either.
        let sidebar = (!crate::runtime_env::compact()
            && !crate::runtime_env::sidebar_disabled()
            && !init.browse_folders)
            .then(|| crate::sidebar::Sidebar::new(&working_dir));

        Self {
            buffer: TextBuffer::from_lines_with_format(lines, format),
            cursor: Cursor::default(),
            highlighter: {
                let mut h = crate::utils::highlight::Highlighter::new();
                h.set_file(path_buf.as_deref());
                h
            },
            filename: path_buf,
            _working_dir: working_dir,
            modified: false,
            read_only,
            view_session: false,
            mode: initial_mode,
            diff_review: None,
            commit_msg_buffer: String::new(),
            save_filename_buffer: String::new(),
            open_filename_buffer: String::new(),
            filename_cursor: 0,
            status_message: None,
            status_timestamp: None,
            status_persistent: true, // デフォルトは常時表示
            status_kind: StatusKind::Info,
            scroll_offset: 0,
            _startup_args: init.startup_args,
            _startup_time: init.startup_time,
            shortcut_map: crate::shortcuts::build_shortcut_map(config.keys.as_ref()),
            search_buffer: String::new(),
            replace_buffer: String::new(),
            search_cursor: 0,
            replace_focus: ReplaceFocus::Query,
            page_size: config.page_size,
            config,
            cursor_blink: true,
            search_mode: SearchMode::ByWord,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_saved_id: 0,
            help_scroll_offset: 0,
            help_cursor_line: 0,
            help_view_height: 0,
            help_rendered_line_count: 1,
            markdown_scroll_offset: 0,
            markdown_cursor_line: 0,
            markdown_view_height: 0,
            markdown_rendered_line_count: 1,
            markdown_remote_images: Vec::new(),
            show_line_numbers_runtime: None,
            goto_line_buffer: String::new(),
            search_matches: Vec::new(),
            search_current: 0,
            soft_wrap: true,
            footer_visible_runtime: true,
            text_display_width: 0,
            glide_prefix: None,
            glide_count: String::new(),
            register: Register::default(),
            pending_operator: None,
            glide_find_pending: None,
            yank_highlight: None,
            last_find: None,
            browse_tree,
            command_query: String::new(),
            command_selected: 0,
            swap_dir: crate::swap::swap_dir(init.config_dir.as_ref()),
            swap_path: None,
            swap_edits: 0,
            swap_dirty: false,
            recovery: None,
            create_dir: None,
            sidebar,
            ui_hit: crate::sidebar::UiHit::default(),
            sidebar_drag: false,
            file_diff: None,
            scroll_parked: false,
            scroll_x: 0,
            scrollbar_total: 0,
            scrollbar_viewport: 0,
            scroll_drag: None,
            #[cfg(feature = "imageview")]
            image_view: None,
            #[cfg(feature = "imageview")]
            image_pending: None,
            #[cfg(feature = "imageview")]
            pending_image,
            selection: None,
            select_drag: false,
            last_press: None,
        }
        .with_recovery_offer()
        // ⭐ 設定の苦情も**画面へ**（`eprintln!` は TUI の絵を壊す・`#2`）。
        // ⚠️ 開けなかったファイルの理由があるならそちらが先 —— そちらの方が急ぎ。
        .with_startup_error(startup_error.or(config_warning))
    }

    /// 起動引数のファイルを開けなかったことを、最初の画面で言う。
    ///
    /// ⚠️ **プロセスは落とさない。** cozy は argo に TUI プロバイダとして埋め込まれるのが
    /// 本体で、そこでの `exit` は「エディタが開かない」ではなく**ホストごと消える**に化ける。
    /// ∴ 空の Welcome に留まり、理由だけ出す（名前は持っていないので保存先も無い）。
    fn with_startup_error(mut self, message: Option<String>) -> Self {
        if let Some(message) = message {
            self.set_status_message(message, StatusKind::Error, true);
        }
        self
    }

    /// Look for a swap left behind by a session that did not get to finish, and
    /// hold it out as an offer. Nothing is restored without an answer: the buffer
    /// still shows the file, so a wrong swap costs the user one keystroke, not
    /// their document.
    fn with_recovery_offer(mut self) -> Self {
        if let Some(dir) = &self.swap_dir {
            crate::swap::prune(dir);
        }
        self.swap_path = crate::swap::path_for(&self);
        if let (Some(swap), Some(target)) = (&self.swap_path, &self.filename) {
            let target = self.resolve_in_working_dir(target);
            self.recovery = crate::swap::load(swap, &target);
        }
        self
    }

    pub fn save_snapshot(&mut self) {
        self.undo_stack.push((self.buffer.clone(), self.cursor));
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) {
        if let Some((buffer, cursor)) = self.undo_stack.pop() {
            self.redo_stack.push((self.buffer.clone(), self.cursor));
            self.buffer = buffer;
            self.cursor = cursor;
            // The text under the highlighter is gone: reparse before drawing again.
            self.highlighter.mark_dirty();
        }
    }

    pub fn redo(&mut self) {
        if let Some((buffer, cursor)) = self.redo_stack.pop() {
            self.undo_stack.push((self.buffer.clone(), self.cursor));
            self.buffer = buffer;
            self.cursor = cursor;
            self.highlighter.mark_dirty();
        }
    }

    pub fn set_status_message(&mut self, msg: String, kind: StatusKind, persistent: bool) {
        self.status_message = Some(msg);
        self.status_timestamp = Some(Instant::now());
        self.status_kind = kind;
        self.status_persistent = persistent;
    }

    pub fn should_show_status(&self) -> bool {
        if self.status_persistent {
            return true;
        }
        if let Some(timestamp) = self.status_timestamp {
            let duration = self.config.status_duration.unwrap_or(3);
            if timestamp.elapsed() < std::time::Duration::from_secs(duration) {
                return true;
            }
        }
        false
    }

    /// The resting/return mode the user lands in after an action completes,
    /// resolved from `config.default_mode`. Unknown/missing → Edit (the safe
    /// default that keeps zero hidden state for newcomers).
    pub fn home_mode(&self) -> EditorMode {
        // ⭐ **閲覧の席は `default_mode` より強い。** `czv` で開いた人は、
        // 検索やヘルプから戻ったときに**閲覧へ帰る**のであって、
        // 設定ファイルに書いた編集面へ降りるのではない。
        if self.view_session {
            return EditorMode::View;
        }
        Self::resolve_home(&self.config)
    }

    /// 開いているファイルが Markdown か（拡張子で判定・大小無視）。
    /// ⭐ **ページャ（`czv`）でプレビュー切替を出すのはこれが真のときだけ**
    /// （決定 2026-10-03・🧑「生が既定・プレビュー対象は md でいい」）。czv は何でも開く
    /// ので、整形は md のときだけ意味を持つ（`.rs` を Markdown として描くのは無意味）。
    /// 名前が無いとき（新規バッファ・stdin）は false。Edit モードの切替はこれを見ない
    /// （従来どおり任意のバッファを整形できる）。
    pub fn is_markdown_file(&self) -> bool {
        self.filename
            .as_deref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
    }

    /// Same resolution as [`home_mode`], usable before `EditorState` exists
    /// (e.g. when picking the startup mode in `new_with_config_dir`).
    pub fn resolve_home(config: &Config) -> EditorMode {
        match config.default_mode.as_deref() {
            Some("glide") => EditorMode::Glide,
            _ => EditorMode::Edit,
        }
    }

    pub fn enter_mode(&mut self, mode: EditorMode) {
        self.mode = mode;
        match mode {
            EditorMode::Save | EditorMode::Quit => {
                // A new, unnamed buffer gets a default name so the user can just
                // hit Enter to drop a memo into the current folder. The default
                // dodges existing files by counting up: untitled (1).txt, etc.
                let dir = self._working_dir.clone();
                // ⚠️ **縮めるのは種を仕込むここ**（描画時ではない）。この文字列は
                // そのまま編集される buffer なので、表示だけ別物にすると
                // `filename_cursor` が指す位置と食い違う。
                // 保存時は `file_io` の `expand_tilde` が戻すので往復する。
                self.save_filename_buffer = self
                    .filename
                    .as_ref()
                    .map(|p| crate::file_io::shorten_home(&p.to_string_lossy()))
                    .unwrap_or_else(|| default_save_name(&dir));
                self.filename_cursor = self.save_filename_buffer.len();
                self.status_message = None;
            }
            EditorMode::Open => {
                // 🚨 **ここは Save と逆。空で始める。**
                //
                // 以前は Save と同じ形で現在のファイル名を敷き、カーソルを末尾に置いていた。
                // ∴ 利用者が欄を空だと思って打つと **2 つが連結される** ——
                // `ok.txt` を開いている状態で `sjis.txt` と打つと `ok.txtsjis.txt` になり、
                // cozy は `File not found` と正しく答える。⚠️ **打ったのは `sjis.txt` だけ**
                // なので、利用者はタイプミスを疑い、打ち直して同じことを言われる。
                //
                // ⭐ Save に名前が入っているのは「**同じ名前に**保存する」が既定だから。
                // Open は**別のファイルを開く**操作なので、既定で名前が入っている理由が無い
                // （nano の `Ctrl+R` も空で始まる）。近くのファイルを選ぶ道は `Ctrl+B`。
                self.open_filename_buffer.clear();
                self.filename_cursor = 0;
                self.status_message = None;
            }
            EditorMode::Search => {
                self.search_buffer.clear();
                self.search_matches.clear();
                self.search_current = 0;
                self.search_mode = crate::state::SearchMode::ByWord;
                self.search_cursor = 0;
                self.status_message = None;
            }
            EditorMode::Replace => {
                self.search_buffer.clear();
                self.replace_buffer.clear();
                self.search_matches.clear();
                self.search_current = 0;
                self.replace_focus = crate::state::ReplaceFocus::Query;
                self.search_cursor = 0;
                self.status_message = None;
            }
            EditorMode::Goto => {
                self.goto_line_buffer.clear();
                self.status_message = None;
            }
            EditorMode::Glide => {
                self.glide_prefix = None;
                self.glide_count.clear();
                self.status_message = None;
            }
            EditorMode::Browse => {
                self.browse_tree = Some(crate::file_io::build_browse_tree(
                    self.filename.as_ref(),
                    &self._working_dir,
                ));
                self.status_message = None;
            }
            EditorMode::Image => {
                // The picture owns the body; there is nothing to scroll or highlight.
                self.status_message = None;
            }
            EditorMode::Markdown => {
                self.markdown_scroll_offset = 0;
                self.markdown_cursor_line = 0;
                self.markdown_view_height = 0;
                self.markdown_rendered_line_count = 1;
                self.markdown_remote_images.clear();
                self.glide_prefix = None;
                self.glide_count.clear();
                self.status_message = None;
            }
            EditorMode::Help => {
                self.help_scroll_offset = 0;
                self.help_cursor_line = 0;
                self.help_view_height = 0;
                self.help_rendered_line_count = 1;
                self.glide_prefix = None;
                self.glide_count.clear();
                self.status_message = None;
            }
            EditorMode::Command => {
                self.command_query.clear();
                self.command_selected = 0;
                self.status_message = None;
            }
            EditorMode::DiffCommitMsg => {
                self.commit_msg_buffer.clear();
                self.status_message = None;
            }
            _ => {
                self.status_message = None;
            }
        }
    }
}

// --- 保存系メソッド ---
impl EditorState {
    /// Resolve a possibly-relative save target against the folder cozy is anchored
    /// to (`_working_dir`). This keeps the actual write in the same folder the
    /// default-name collision check looked at — and the same folder regardless of
    /// the process cwd, which matters once Browse can span multiple repos.
    pub(crate) fn resolve_in_working_dir(&self, path: &std::path::Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self._working_dir.join(path)
        }
    }

    /// 通常保存（現在のファイル名に保存）
    pub fn save(&mut self) -> io::Result<()> {
        crate::file_io::save(self)
    }

    /// 名前をつけて保存（新しいファイル名を指定して保存し、記憶する）
    pub fn save_as(&mut self, path: &str) -> io::Result<()> {
        crate::file_io::save_as(self, path)
    }

    /// ファイルを開く
    pub fn open_file(&mut self, path: &str) -> io::Result<()> {
        // ⭐ A picture is decided by its bytes (`#20`): decoding a PNG as text fills the
        // buffer with mojibake and — worse — names it, so `Ctrl+S` would write the
        // garbage back over the image. The image view keeps `filename` untouched.
        #[cfg(feature = "imageview")]
        if self.try_open_image(std::path::Path::new(path))? {
            return Ok(());
        }
        let result = crate::file_io::open_file(self, path);
        if result.is_ok() {
            self.highlighter.set_file(self.filename.as_deref());
            // A new document starts at column 0 — the old offset means nothing here.
            self.scroll_x = 0;
        }
        result
    }

    /// Open `path` in the image view when it is a picture. `Ok(false)` means "not an
    /// image — go ahead and load it as text". `Ok(true)` means the viewer is on screen.
    #[cfg(feature = "imageview")]
    pub(crate) fn try_open_image(&mut self, path: &std::path::Path) -> io::Result<bool> {
        let kind = if crate::imageview::sniff_path(path).is_some() {
            crate::imageview::LoadKind::Image
        } else if crate::pdfview::sniff_path(path) {
            crate::imageview::LoadKind::Pdf
        } else {
            return Ok(false);
        };
        // ⭐ The decode (or `pdftocairo`) runs on a worker thread; the screen shows a
        // spinner and stays alive. See `poll_image_load`.
        let label = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        self.image_view = None;
        self.image_pending = Some(ImagePending {
            label,
            kind,
            started: std::time::Instant::now(),
            rx: crate::imageview::spawn_load(path.to_path_buf(), kind),
        });
        self.enter_mode(EditorMode::Image);
        Ok(true)
    }

    /// Collect a finished background load. Returns whether anything changed (so the loop
    /// knows to draw). Cheap enough to call every iteration: `try_recv` on an empty
    /// channel is a load and a branch.
    #[cfg(feature = "imageview")]
    pub fn poll_image_load(&mut self) -> bool {
        let Some(pending) = self.image_pending.as_ref() else {
            return false;
        };
        let result = match pending.rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("the loader stopped without an answer".to_string())
            }
        };
        self.image_pending = None;
        match result {
            Ok(loaded) => self.image_view = Some(loaded.into_viewer()),
            // A broken file is not a reason to open it as text (the bytes are not text).
            Err(e) => self.set_status_message(format!("Cannot show: {e}"), StatusKind::Error, true),
        }
        true
    }

    /// A picture is still being prepared — the loop keeps animating while this is true.
    #[cfg(feature = "imageview")]
    pub fn image_loading(&self) -> bool {
        self.image_pending.is_some()
    }

    /// A picture named on the command line opens here — after the terminal exists, since
    /// the viewer needs the protocol measured from it.
    #[cfg(feature = "imageview")]
    pub(crate) fn load_pending_image(&mut self) -> io::Result<()> {
        match self.pending_image.take() {
            Some(path) => self.try_open_image(&path).map(|_| ()),
            None => Ok(()),
        }
    }

    /// Keep the cursor's column on screen when the view is scrolled horizontally. Called
    /// by the renderer after [`Self::adjust_scroll`] (only it knows the text width).
    ///
    /// With soft wrap on there is no horizontal overflow, so this only clears the offset.
    pub fn adjust_hscroll(&mut self) {
        if self.soft_wrap || self.text_display_width == 0 {
            self.scroll_x = 0;
            return;
        }
        if self.scroll_parked {
            return;
        }
        let Some(line) = self.buffer.lines.get(self.cursor.y) else {
            self.scroll_x = 0;
            return;
        };
        let width = self.text_display_width;
        let col = crate::utils::unicode::str_display_width(&line[..self.cursor.x.min(line.len())]);
        let line_width = crate::utils::unicode::str_display_width(line);
        let max = line_width.saturating_sub(width);
        if col < self.scroll_x {
            self.scroll_x = col;
        } else if col >= self.scroll_x + width {
            self.scroll_x = col + 1 - width;
        }
        self.scroll_x = self.scroll_x.min(max);
    }

    /// The text under the mouse selection, newline-joined, and how many buffer lines it
    /// spans. `None` when nothing is selected (a plain click is not a copy).
    pub fn selected_text(&self) -> Option<(String, usize)> {
        let selection = self.selection?;
        if selection.is_empty() {
            return None;
        }
        let ((sy, sx), (ey, ex)) = selection.range();
        let mut out = String::new();
        let mut lines = 0usize;
        for y in sy..=ey {
            let Some(line) = self.buffer.lines.get(y) else {
                break;
            };
            let from = if y == sy { clamp_boundary(line, sx) } else { 0 };
            let to = if y == ey {
                clamp_boundary(line, ex)
            } else {
                line.len()
            };
            if from < to {
                out.push_str(&line[from..to]);
            }
            if y != ey {
                out.push('\n');
            }
            lines += 1;
        }
        (!out.is_empty()).then_some((out, lines))
    }

    /// The furthest row the window can start on. Scrollbar dragging clamps to this so a
    /// grab cannot park the view past the end of the buffer.
    pub fn max_scroll_offset(&self) -> usize {
        self.buffer
            .lines
            .len()
            .saturating_sub(self.page_size.max(1))
    }

    /// The furthest column this window can be scrolled to (the widest visible line).
    pub fn max_scroll_x(&self) -> usize {
        if self.soft_wrap || self.text_display_width == 0 {
            return 0;
        }
        let end = (self.scroll_offset + self.page_size.max(1)).min(self.buffer.lines.len());
        self.buffer.lines[self.scroll_offset.min(end)..end]
            .iter()
            .map(|l| crate::utils::unicode::str_display_width(l))
            .max()
            .unwrap_or(0)
            .saturating_sub(self.text_display_width)
    }

    pub fn adjust_scroll(&mut self, viewport_height: usize) {
        if viewport_height == 0 {
            return;
        }
        // The wheel owns the viewport until something else moves the cursor: pulling the
        // view back here would undo the scroll on the very next frame.
        if self.scroll_parked {
            return;
        }

        // Always scroll up if cursor is above viewport
        if self.cursor.y < self.scroll_offset {
            self.scroll_offset = self.cursor.y;
            return;
        }

        if !self.soft_wrap || self.text_display_width == 0 {
            if self.cursor.y >= self.scroll_offset + viewport_height {
                self.scroll_offset = self.cursor.y - viewport_height + 1;
            }
            return;
        }

        // Soft wrap: check if cursor's visual sub-row is already visible
        let tw = self.text_display_width;
        let mut vrows = 0usize;
        let mut cursor_visible = false;

        for by in self.scroll_offset..=self.cursor.y {
            if by >= self.buffer.lines.len() {
                break;
            }
            let line = &self.buffer.lines[by];
            let lrows = crate::utils::wrap::visual_row_count(line, tw);
            if by == self.cursor.y {
                let (sub, _) = crate::utils::wrap::cursor_visual_pos(line, self.cursor.x, tw);
                cursor_visible = vrows + sub < viewport_height;
                break;
            }
            vrows += lrows;
            if vrows >= viewport_height {
                break;
            }
        }

        if cursor_visible {
            return;
        }

        // Cursor is below viewport: find a scroll_offset that shows cursor near bottom
        let tw = self.text_display_width;
        let cursor_line = &self.buffer.lines[self.cursor.y];
        let (cursor_sub, _) = crate::utils::wrap::cursor_visual_pos(cursor_line, self.cursor.x, tw);

        if cursor_sub >= viewport_height {
            // cursor's sub-row alone fills the viewport; show from cursor.y
            self.scroll_offset = self.cursor.y;
            return;
        }

        // Walk backward from cursor.y, accumulating rows, until viewport is full
        let mut rows_used = cursor_sub + 1;
        let mut new_offset = self.cursor.y;
        let mut y = self.cursor.y;
        while y > 0 && rows_used < viewport_height {
            y -= 1;
            let r = crate::utils::wrap::visual_row_count(&self.buffer.lines[y], tw);
            if rows_used + r <= viewport_height {
                rows_used += r;
                new_offset = y;
            } else {
                break;
            }
        }
        self.scroll_offset = new_offset;
    }
}

/// Clamp `i` down to the nearest char boundary of `s` (mouse positions come from display
/// columns, and a wide char can put one past the start of a character).
fn clamp_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
