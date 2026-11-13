use crate::host_backend::HostBackend;
use crate::input::{self, EventSource, InputEvent};
use crate::reducer::{EventResult, reduce};
use crate::state::{EditorMode, EditorState};
use crate::ui::Renderer;
use ratatui::Terminal;
use std::io::{self, Write};
use std::time::Duration;

/// Alternate scroll (`1007`): the terminal turns the wheel into ↑/↓ while the
/// alternate screen is up, and the reading views scroll on ↑/↓ (`keymap`).
///
/// 🚨 **Not mouse reporting (`1000`/`1006`).** That takes the mouse away from the
/// terminal: drag-select to copy stops working unless Shift is held, and nothing
/// says so. `1007` leaves the mouse with the terminal, so selecting text just works.
const WHEEL_ON: &[u8] = b"\x1b[?1007h";
pub(crate) const WHEEL_OFF: &[u8] = b"\x1b[?1007l";

/// Mouse reporting, taken only while the left panel is on screen: `1000` clicks,
/// `1002` drags, `1006` coordinates as `ESC [ < … M`.
///
/// ⚠️ **This is the trade the panel makes.** While it is up, the terminal hands the mouse
/// to cozy — so drag-to-select belongs to the terminal again only with Shift held. Hiding
/// the panel (`Alt+B` twice) gives plain drag-select back, and the pager (`czv`) and the
/// reading views never ask for it at all.
/// `1003` is what makes hover possible: without any-event tracking the terminal only
/// reports motion while a button is held, so a row could never light up under the pointer.
const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h";
pub(crate) const MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

/// While a hover is fading in, wake this often so the fade is smooth — then fall back to
/// the idle cadence (a fade must not keep the CPU warm afterwards).
const ANIM_WAIT: Duration = Duration::from_millis(16);

/// How long the loop waits for a key. Normally a second (the idle swap write and
/// the cursor blink run on it); while Browse is still indexing in the background,
/// a fifth of one, so the `indexing N` count and the newly found entries show up
/// without a key press (`#25`). Back to a second once the index is absorbed.
const IDLE_WAIT: Duration = Duration::from_millis(1000);
const INDEXING_WAIT: Duration = Duration::from_millis(200);

/// Markdown プレビューで、`i` で頼んだ URL の画像がまだ取れていないか。取得中も Browse の
/// 索引と同じく `INDEXING_WAIT` で起き、取れた物をキーを待たずに描く。
fn images_are_arriving(editor: &EditorState) -> bool {
    #[cfg(feature = "remote-images")]
    {
        editor.mode == EditorMode::Markdown && crate::imageview::fetch::any_pending()
    }
    #[cfg(not(feature = "remote-images"))]
    {
        let _ = editor;
        false
    }
}

/// 取得が 1 つでも終わったら進む数（feature 無しでは常に 0）。
fn fetch_generation() -> u64 {
    #[cfg(feature = "remote-images")]
    {
        crate::imageview::fetch::generation()
    }
    #[cfg(not(feature = "remote-images"))]
    {
        0
    }
}

/// Whether a folder tree is still filling in from its background index. Browse owns the
/// whole screen while it does; the sidebar keeps indexing under the buffer, and its rows
/// have to appear without a keypress just the same.
/// A hover highlight is still fading in.
fn hover_is_animating(editor: &EditorState) -> bool {
    editor.sidebar.as_ref().is_some_and(|s| s.hover_animating())
}

fn browse_is_indexing(editor: &EditorState) -> bool {
    let browsing = editor.mode == EditorMode::Browse
        && editor
            .browse_tree
            .as_ref()
            .is_some_and(|tree| tree.is_indexing());
    let sidebar = editor
        .sidebar
        .as_ref()
        .is_some_and(|s| s.visible && s.explorer.is_indexing());
    browsing || sidebar
}

/// The modes a wheel scrolls (↑/↓ → `Action::Wheel`). Elsewhere the terminal
/// keeps the wheel for its own scrollback. The mouse stays with the terminal
/// everywhere, so text can always be selected.
fn wants_wheel(mode: &EditorMode) -> bool {
    matches!(
        mode,
        EditorMode::Markdown | EditorMode::Help | EditorMode::View
    )
}

pub fn run<W: Write>(
    terminal: &mut Terminal<HostBackend<W>>,
    editor: &mut EditorState,
    event_src: &mut dyn EventSource,
    report_wheel: bool,
) -> io::Result<()> {
    let mut wheel_on = false;
    let mut mouse_on = false;
    let result = run_loop(
        terminal,
        editor,
        event_src,
        report_wheel,
        &mut wheel_on,
        &mut mouse_on,
    );
    let backend = terminal.backend_mut();
    if wheel_on {
        let _ = backend.write_all(WHEEL_OFF);
    }
    if mouse_on {
        let _ = backend.write_all(MOUSE_OFF);
    }
    let _ = Write::flush(backend);
    result
}

fn run_loop<W: Write>(
    terminal: &mut Terminal<HostBackend<W>>,
    editor: &mut EditorState,
    event_src: &mut dyn EventSource,
    report_wheel: bool,
    wheel_on: &mut bool,
    mouse_on: &mut bool,
) -> io::Result<()> {
    let mut needs_redraw = true;
    let mut seen_fetches = fetch_generation();
    loop {
        let want = report_wheel && wants_wheel(&editor.mode);
        if want != *wheel_on {
            let backend = terminal.backend_mut();
            backend.write_all(if want { WHEEL_ON } else { WHEEL_OFF })?;
            Write::flush(backend)?;
            *wheel_on = want;
        }
        // Clicks belong to the panel while the panel is up; nothing else asks for them.
        let want_mouse = report_wheel && editor.sidebar_on_screen();
        if want_mouse != *mouse_on {
            let backend = terminal.backend_mut();
            backend.write_all(if want_mouse { MOUSE_ON } else { MOUSE_OFF })?;
            Write::flush(backend)?;
            *mouse_on = want_mouse;
        }
        if needs_redraw {
            terminal.draw(|f| {
                // Geometry for the mouse: rebuilt from scratch each frame, so a click can
                // never be answered with last mode's rectangles.
                editor.ui_hit = crate::sidebar::UiHit {
                    screen: crate::sidebar::Area::from(f.area()),
                    ..Default::default()
                };
                let chunks = Renderer::editor_layout(f.area(), editor);
                Renderer::render_body(editor, f, chunks[0]);
                Renderer::render_shortcuts(editor, f, chunks[1]);
                Renderer::render_status_bar(editor, f, chunks[2]);
                Renderer::render_sidebar(editor, f, chunks[3]);
            })?;
            needs_redraw = false;
        }

        // Poll with longer timeout to avoid clearing IME composition overlays.
        // crossterm receives no events during IME composition (macOS intercepts
        // keystrokes at the Cocoa layer), so longer polling means the IME inline
        // display persists undisturbed until the user confirms the composition.
        // ⚠️ Shorter only while Browse indexes (`INDEXING_WAIT`): each wake redraws,
        // which is what clears a composing IME — acceptable for the few seconds a
        // large folder takes, not as the normal cadence.
        let indexing = browse_is_indexing(editor);
        let arriving = images_are_arriving(editor);
        let animating = hover_is_animating(editor);
        let wait = if animating {
            ANIM_WAIT
        } else if indexing || arriving {
            INDEXING_WAIT
        } else {
            IDLE_WAIT
        };
        if event_src.poll(wait)? {
            match input::map_event(editor, event_src.read()?) {
                InputEvent::Action(action) => {
                    // A pointer moving across the panel must not repaint the world: only a
                    // change of hovered row is worth a frame.
                    let hover_before = editor.sidebar.as_ref().and_then(|s| s.hover);
                    let is_move = matches!(action, crate::action::Action::MouseMove(..));
                    if let EventResult::Exit = reduce(editor, action) {
                        // A deliberate exit: whatever the user wanted to keep is
                        // saved, and whatever they discarded they discarded on
                        // purpose. The swap exists for the exits nobody chose.
                        crate::swap::remove(editor);
                        break;
                    }
                    if editor.modified {
                        editor.swap_dirty = true;
                        editor.swap_edits += 1;
                        // Do not let a long uninterrupted burst of typing outrun
                        // the journal — the idle tick below never fires while the
                        // keys keep coming.
                        if editor.swap_edits >= crate::swap::EDITS_PER_WRITE {
                            crate::swap::write(editor);
                        }
                    }
                    editor.cursor_blink = true;
                    // ⚠️ **No blanket `mark_dirty()` here.** A reparse is O(document)
                    // (`join` + a full tree-sitter parse), and this loop runs for every
                    // wheel notch and pointer move — that is what made scrolling slow
                    // down as the file grew. The edits mark it themselves, at the one
                    // place every mutation passes through (`reducer::helper::mark_modified`),
                    // plus undo/redo and opening a file.
                    needs_redraw = if is_move {
                        editor.sidebar.as_ref().and_then(|s| s.hover) != hover_before
                    } else {
                        true
                    };
                }
                InputEvent::Resize(cols, rows) => {
                    // The size first: `resize` asks the backend for it while clearing,
                    // and an embedding host's backend answers with what it was told.
                    terminal.backend_mut().note_size(cols, rows);
                    let _ = terminal.resize(ratatui::layout::Rect::new(0, 0, cols, rows));
                    needs_redraw = true;
                }
                InputEvent::Flush => {
                    // The one second of typing the idle tick has not written yet
                    // is exactly what a kill would take. Write it while we still
                    // hold the thread.
                    if editor.swap_dirty {
                        crate::swap::write(editor);
                    }
                }
                InputEvent::Ignore => {}
            }
        } else {
            // No key for a second: the user stopped typing, so write the swap
            // now. This is the whole cadence — an iOS kill is announced to
            // nobody, so "on the way out" is not a moment we are given.
            if editor.swap_dirty {
                crate::swap::write(editor);
            }

            // An image asked for with `i` arrived: draw it without waiting for a key.
            let fetches = fetch_generation();
            if fetches != seen_fetches {
                seen_fetches = fetches;
                needs_redraw = true;
            }
            if indexing {
                // Take in what the background index found and show the new count —
                // including the last time, which removes `indexing` from the footer.
                // The blink waits: it would toggle five times a second here.
                if let Some(tree) = editor.browse_tree.as_mut() {
                    tree.tick();
                }
                if let Some(sidebar) = editor.sidebar.as_mut()
                    && sidebar.visible
                {
                    sidebar.explorer.tick();
                }
                needs_redraw = true;
            } else if editor.config.cursor_blink.unwrap_or(false) {
                // Timeout: toggle cursor blink only if enabled
                editor.cursor_blink = !editor.cursor_blink;
                needs_redraw = true;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::Event;
    use ratatui::layout::Rect;
    use ratatui::{TerminalOptions, Viewport};

    /// 1 回目の poll で失敗する入力 —— ループは途中で `Err` で抜ける。
    /// ⭐ 正常終了ではなく失敗の道で測る: `?` で抜けても解除が出ることも同時に見られる。
    struct Fails;
    impl EventSource for Fails {
        fn poll(&mut self, _: Duration) -> io::Result<bool> {
            Err(io::Error::other("stop"))
        }
        fn read(&mut self) -> io::Result<Event> {
            unreachable!()
        }
    }

    fn bytes_written(mode: EditorMode, report_wheel: bool) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let backend = HostBackend::new(&mut out, Some((40, 10)));
            let mut terminal = Terminal::with_options(
                backend,
                TerminalOptions {
                    viewport: Viewport::Fixed(Rect::new(0, 0, 40, 10)),
                },
            )
            .unwrap();
            let mut editor = EditorState::new(None);
            editor.enter_mode(mode);
            let _ = run(&mut terminal, &mut editor, &mut Fails, report_wheel);
        }
        out
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn a_reading_mode_asks_for_the_wheel_and_gives_it_back() {
        let out = bytes_written(EditorMode::Help, true);
        let on = find(&out, WHEEL_ON).expect("読むモードでホイールを頼んでいない");
        let off = find(&out, WHEEL_OFF).expect("抜けるときに解除していない");
        assert!(on < off);
    }

    /// 読むモードでもマウスは端末に残す（ドラッグで選んでコピーできるように）——
    /// マウス報告（`1000`/`1002`/`1003`/`1006`）はどのモードでも頼まない。
    #[test]
    fn no_mode_takes_the_mouse_from_the_terminal() {
        for mode in [
            EditorMode::Help,
            EditorMode::Markdown,
            EditorMode::View,
            EditorMode::Edit,
        ] {
            let out = bytes_written(mode, true);
            for report in [
                &b"\x1b[?1000h"[..],
                b"\x1b[?1002h",
                b"\x1b[?1003h",
                b"\x1b[?1006h",
            ] {
                assert_eq!(
                    find(&out, report),
                    None,
                    "{mode:?} がマウス報告 {:?} を頼んだ",
                    String::from_utf8_lossy(report)
                );
            }
        }
    }

    /// 対照: 編集のモードでは頼まない（マウスでの文字の選択を端末に残す）。
    #[test]
    fn an_editing_mode_leaves_the_mouse_to_the_terminal() {
        let out = bytes_written(EditorMode::Edit, true);
        assert_eq!(find(&out, WHEEL_ON), None);
    }

    /// Records how long each poll was asked to wait; the second poll ends the loop.
    struct Waits(Vec<Duration>);
    impl EventSource for Waits {
        fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            self.0.push(timeout);
            if self.0.len() >= 2 {
                return Err(io::Error::other("stop"));
            }
            Ok(false)
        }
        fn read(&mut self) -> io::Result<Event> {
            unreachable!()
        }
    }

    fn waits_with_a_stalled_index(
        mode: EditorMode,
        finished: bool,
    ) -> (Vec<Duration>, EditorState) {
        let mut out = Vec::new();
        let backend = HostBackend::new(&mut out, Some((40, 10)));
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 40, 10)),
            },
        )
        .unwrap();
        let mut editor = EditorState::new(None);
        let tree =
            crate::browse::BrowseTree::stalled(std::path::Path::new("/nonexistent-cozy-root"));
        if finished {
            tree.finish_stalled_index();
        }
        editor.browse_tree = Some(tree);
        editor.mode = mode;
        let mut src = Waits(Vec::new());
        let _ = run(&mut terminal, &mut editor, &mut src, false);
        (src.0, editor)
    }

    /// `#25`: Browse の索引中は 200 ms で起きる（キーを待たずに件数を描き直す）。
    #[test]
    fn browse_wakes_often_while_it_indexes() {
        let (waits, editor) = waits_with_a_stalled_index(EditorMode::Browse, false);
        assert_eq!(waits, vec![INDEXING_WAIT, INDEXING_WAIT]);
        assert!(editor.browse_tree.as_ref().unwrap().is_indexing());
    }

    /// 索引が完成したら、取り込んだ回を最後に 1 秒へ戻る（待機中の CPU を増やさない）。
    #[test]
    fn browse_goes_back_to_idle_once_the_index_is_absorbed() {
        let (waits, editor) = waits_with_a_stalled_index(EditorMode::Browse, true);
        assert_eq!(waits, vec![INDEXING_WAIT, IDLE_WAIT]);
        assert!(!editor.browse_tree.as_ref().unwrap().is_indexing());
    }

    /// 対照: Browse 以外では、索引が途中でも 1 秒のまま。
    #[test]
    fn other_modes_keep_the_idle_wait() {
        let (waits, _) = waits_with_a_stalled_index(EditorMode::Edit, false);
        assert_eq!(waits, vec![IDLE_WAIT, IDLE_WAIT]);
    }

    /// 対照: 埋め込み（`report_wheel = false`）では読むモードでも頼まない。
    #[test]
    fn an_embedding_host_is_never_asked() {
        let out = bytes_written(EditorMode::Help, false);
        assert_eq!(find(&out, WHEEL_ON), None);
    }
}
