// A clipboard handle that lives for the whole process. On X11/Wayland the
// clipboard contents are served by the owning connection, so a throwaway
// `Clipboard::new()` per call relinquishes ownership the instant it drops.
// Holding one handle alive for the session keeps the selection owned and
// pasteable.
#[cfg(feature = "clipboard")]
thread_local! {
    static CLIPBOARD: std::cell::RefCell<Option<arboard::Clipboard>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `f` with the process-lifetime clipboard, lazily creating it. Returns
/// `None` when no clipboard is available (e.g. headless / no display).
#[cfg(feature = "clipboard")]
fn with_clipboard<R>(f: impl FnOnce(&mut arboard::Clipboard) -> R) -> Option<R> {
    CLIPBOARD.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = arboard::Clipboard::new().ok();
        }
        slot.as_mut().map(f)
    })
}

pub(crate) fn set_text(text: &str) {
    let _ = set_text_ok(text);
}

/// `set_text`, but says whether it worked — the copy key reports the difference between
/// "on the clipboard" and "nothing took it".
///
/// Two ways out, in order: the system clipboard (arboard, which needs X11/Wayland), and
/// when that is not there — an SSH session, a bare framebuffer, a host with the feature
/// off — `OSC 52`, which asks the *terminal* to hold the text. The terminal is the one
/// piece that exists in every case.
pub(crate) fn set_text_ok(text: &str) -> bool {
    #[cfg(feature = "clipboard")]
    if with_clipboard(|cb| cb.set_text(text).is_ok()).unwrap_or(false) {
        return true;
    }
    osc52(text)
}

/// Ask the terminal to put `text` on its clipboard: `ESC ] 52 ; c ; <base64> BEL`.
///
/// Written straight to stdout — the same place the frame goes, and never in the middle of
/// a draw (the reducer runs between frames). Terminals that do not know the sequence
/// ignore it, which is why this is the fallback and not the primary.
fn osc52(text: &str) -> bool {
    use std::io::{IsTerminal, Write};
    if !std::io::stdout().is_terminal() {
        return false;
    }
    let mut out = std::io::stdout().lock();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()))
        .and_then(|_| out.flush())
        .is_ok()
}

/// Standard base64 (with padding). Hand-rolled to keep this module dependency-free — the
/// alphabet and the 3-byte/4-char packing are the whole format.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(n >> 12) as usize & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(feature = "clipboard")]
pub(crate) fn get_text() -> ClipboardRead {
    match with_clipboard(|cb| cb.get_text()) {
        Some(Ok(text)) => ClipboardRead::Text(text),
        _ => ClipboardRead::Failed,
    }
}

#[cfg(not(feature = "clipboard"))]
pub(crate) fn get_text() -> ClipboardRead {
    ClipboardRead::Unavailable
}

pub(crate) enum ClipboardRead {
    #[cfg(feature = "clipboard")]
    Text(String),
    #[cfg(feature = "clipboard")]
    Failed,
    #[cfg(not(feature = "clipboard"))]
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::base64;

    /// The padding cases are the ones an encoder gets wrong.
    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"hello world"), "aGVsbG8gd29ybGQ=");
        // A newline (what a two-line selection carries) round-trips as its own bytes.
        assert_eq!(base64(b"a\nb"), "YQpi");
    }
}
