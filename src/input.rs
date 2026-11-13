use crate::action::{Action, MouseButton};
use crate::state::EditorState;
use crate::state::key::{KeyCode, KeyModifiers};
use crate::ui::Keymap;
use crossterm::event::{Event, MouseEventKind};
use std::io;
use std::time::Duration;

/// Abstracts the event source so the loop can run with crossterm or an IPC queue.
pub trait EventSource {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<Event>;
}

/// Default implementation that reads directly from crossterm (CLI use).
pub struct CrosstermEventSource;

impl EventSource for CrosstermEventSource {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        crossterm::event::poll(timeout)
    }

    fn read(&mut self) -> io::Result<Event> {
        crossterm::event::read()
    }
}

pub enum InputEvent {
    Action(Action),
    Resize(u16, u16),
    /// Write the swap now, without waiting for the idle tick.
    ///
    /// Losing focus is the last moment a host can still hand us: on iOS the app
    /// is told it is leaving the screen, and is killed later without a word. A
    /// terminal that reports focus gives us the same moment for free.
    Flush,
    Ignore,
}

pub fn map_event(editor: &EditorState, event: Event) -> InputEvent {
    match event {
        Event::Key(key_event) => match ct_code(key_event.code) {
            Some(code) => {
                let mods = ct_mods(key_event.modifiers);
                Keymap::map_key_to_action(editor, code, mods)
                    .map(InputEvent::Action)
                    .unwrap_or(InputEvent::Ignore)
            }
            None => InputEvent::Ignore,
        },
        Event::Paste(data) => InputEvent::Action(Action::InsertString(data)),
        Event::Resize(cols, rows) => InputEvent::Resize(cols, rows),
        Event::FocusLost => InputEvent::Flush,
        Event::Mouse(mouse) => match mouse.kind {
            // Wheel first: the position decides whether it scrolls the panel or the buffer.
            MouseEventKind::ScrollDown => {
                InputEvent::Action(Action::MouseScroll(WHEEL_LINES, mouse.column, mouse.row))
            }
            MouseEventKind::ScrollUp => {
                InputEvent::Action(Action::MouseScroll(-WHEEL_LINES, mouse.column, mouse.row))
            }
            MouseEventKind::Down(button) => InputEvent::Action(Action::MouseDown(
                mouse_button(button),
                mouse.column,
                mouse.row,
            )),
            MouseEventKind::Drag(crossterm::event::MouseButton::Left) | MouseEventKind::Moved => {
                InputEvent::Action(Action::MouseDrag(mouse.column, mouse.row))
            }
            MouseEventKind::Up(_) => InputEvent::Action(Action::MouseUp),
            _ => InputEvent::Ignore,
        },
        _ => InputEvent::Ignore,
    }
}

fn mouse_button(button: crossterm::event::MouseButton) -> MouseButton {
    use crossterm::event::MouseButton as CT;
    match button {
        CT::Left => MouseButton::Left,
        CT::Middle => MouseButton::Middle,
        CT::Right => MouseButton::Right,
    }
}

/// Rows one wheel report scrolls. One: the terminal already decides how many
/// reports a notch is worth — Ghostty sends three (its `mouse-scroll-multiplier`),
/// xterm.js one — so multiplying here would scroll Ghostty nine rows a notch.
pub const WHEEL_LINES: isize = 1;

fn ct_code(code: crossterm::event::KeyCode) -> Option<KeyCode> {
    use crossterm::event::KeyCode as CT;
    Some(match code {
        CT::Char(c) => KeyCode::Char(c),
        CT::Enter => KeyCode::Enter,
        CT::Esc => KeyCode::Esc,
        CT::Backspace => KeyCode::Backspace,
        CT::Delete => KeyCode::Delete,
        CT::PageUp => KeyCode::PageUp,
        CT::PageDown => KeyCode::PageDown,
        CT::Up => KeyCode::Up,
        CT::Down => KeyCode::Down,
        CT::Left => KeyCode::Left,
        CT::Right => KeyCode::Right,
        CT::Home => KeyCode::Home,
        CT::End => KeyCode::End,
        CT::Tab => KeyCode::Tab,
        CT::F(n) => KeyCode::F(n),
        _ => return None,
    })
}

fn ct_mods(mods: crossterm::event::KeyModifiers) -> KeyModifiers {
    use crossterm::event::KeyModifiers as CT;
    let mut result = KeyModifiers::NONE;
    if mods.contains(CT::CONTROL) {
        result |= KeyModifiers::CONTROL;
    }
    if mods.contains(CT::SHIFT) {
        result |= KeyModifiers::SHIFT;
    }
    if mods.contains(CT::ALT) {
        result |= KeyModifiers::ALT;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::MouseButton;
    use crate::state::EditorMode;
    use crossterm::event::{KeyCode as CtKeyCode, KeyEvent, KeyModifiers as CtKeyModifiers};

    #[test]
    fn ctrl_p_enters_command_mode() {
        let editor = EditorState::new(None);
        let event = Event::Key(KeyEvent::new(CtKeyCode::Char('p'), CtKeyModifiers::CONTROL));

        assert!(matches!(
            map_event(&editor, event),
            InputEvent::Action(Action::EnterMode(EditorMode::Command))
        ));
    }

    #[test]
    fn paste_maps_to_insert_string() {
        let editor = EditorState::new(None);

        assert_eq!(
            match map_event(&editor, Event::Paste("abc".to_string())) {
                InputEvent::Action(action) => Some(action),
                _ => None,
            },
            Some(Action::InsertString("abc".to_string()))
        );
    }

    #[test]
    fn resize_stays_host_side() {
        let editor = EditorState::new(None);

        assert!(matches!(
            map_event(&editor, Event::Resize(80, 24)),
            InputEvent::Resize(80, 24)
        ));
    }

    fn mouse(kind: crossterm::event::MouseEventKind) -> Event {
        Event::Mouse(crossterm::event::MouseEvent {
            kind,
            column: 10,
            row: 5,
            modifiers: CtKeyModifiers::NONE,
        })
    }

    /// The wheel carries its position now: over the panel it walks the tree, elsewhere it
    /// stays the scroll it always was (`reducer::sidebar::mouse_scroll` decides).
    #[test]
    fn a_wheel_report_carries_its_cell() {
        use crossterm::event::MouseEventKind as K;
        let editor = EditorState::new(None);
        assert!(matches!(
            map_event(&editor, mouse(K::ScrollDown)),
            InputEvent::Action(Action::MouseScroll(1, 10, 5))
        ));
        assert!(matches!(
            map_event(&editor, mouse(K::ScrollUp)),
            InputEvent::Action(Action::MouseScroll(-1, 10, 5))
        ));
    }

    /// A press is reported with the cell it landed on — the panel is mouse-first now.
    #[test]
    fn a_left_click_is_reported_with_its_cell() {
        use crossterm::event::{MouseButton as CtButton, MouseEventKind as K};
        let editor = EditorState::new(None);
        assert!(matches!(
            map_event(&editor, mouse(K::Down(CtButton::Left))),
            InputEvent::Action(Action::MouseDown(MouseButton::Left, 10, 5))
        ));
        assert!(matches!(
            map_event(&editor, mouse(K::Up(CtButton::Left))),
            InputEvent::Action(Action::MouseUp)
        ));
        assert!(matches!(
            map_event(&editor, mouse(K::Drag(CtButton::Left))),
            InputEvent::Action(Action::MouseDrag(10, 5))
        ));
    }
}
