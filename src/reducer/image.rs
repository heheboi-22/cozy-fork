//! The image view: a picture opened in place of the text.
//!
//! The heavy lifting lives in `imageview` (the same `Viewer` `czv` uses); this module is
//! only the three verbs the editor offers — leave, zoom, next/previous picture — so the
//! key handling never has to know about protocols or decoders.
//!
//! 🚨 Opening a picture must never touch `filename`: that name is what `Ctrl+S` writes
//! to, and a text save over a PNG would destroy it.

use crate::reducer::EventResult;
use crate::state::{EditorState, StatusKind};

/// `q` / `Esc`: back to whatever was open before the picture.
pub fn close(editor: &mut EditorState) -> EventResult {
    #[cfg(feature = "imageview")]
    {
        editor.image_view = None;
    }
    editor.enter_mode(editor.home_mode());
    editor.status_message = None;
    EventResult::Continue
}

/// `z`: fit the window ↔ 1:1.
pub fn zoom(editor: &mut EditorState) -> EventResult {
    #[cfg(feature = "imageview")]
    if let Some(viewer) = editor.image_view.as_mut() {
        viewer.toggle_zoom();
    }
    EventResult::Continue
}

/// `←` / `→`: the previous / next picture in the same folder.
pub fn step(editor: &mut EditorState, delta: isize) -> EventResult {
    #[cfg(feature = "imageview")]
    if let Some(viewer) = editor.image_view.as_mut() {
        // The status bar already shows the page/picture number — a message on top of it
        // would print the same thing twice on one line.
        viewer.step(delta);
    }
    let _ = StatusKind::Info;
    let _ = delta;
    EventResult::Continue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{EditorMode, TextBuffer};

    /// Closing the picture returns to the buffer that was there before it — the picture
    /// never became a document.
    #[test]
    fn closing_the_image_returns_to_the_text() {
        let mut editor = EditorState::new(None);
        editor.buffer = TextBuffer::from_lines(vec!["fn main() {}".to_string()]);
        editor.enter_mode(EditorMode::Image);
        close(&mut editor);
        assert_eq!(editor.mode, EditorMode::Edit);
        assert_eq!(editor.buffer.lines, vec!["fn main() {}"]);
    }

    /// An image is not a save target: opening one must leave `filename` alone.
    #[test]
    fn opening_an_image_does_not_become_the_save_target() {
        let tmp = tempfile::Builder::new()
            .prefix("cozy-img-")
            .tempdir()
            .unwrap();
        let path = tmp.path().join("pic.png");
        // 1×1 transparent PNG.
        std::fs::write(
            &path,
            [
                0x89u8, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49,
                0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00,
                0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54,
                0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4,
                0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
            ],
        )
        .unwrap();

        let mut editor = EditorState::new(None);
        editor.buffer = TextBuffer::from_lines(vec!["keep me".to_string()]);
        editor.open_file(path.to_str().unwrap()).unwrap();

        assert_eq!(editor.mode, EditorMode::Image, "PNG dibuka sebagai gambar");
        assert!(editor.filename.is_none(), "gambar bukan target Ctrl+S");
        assert_eq!(
            editor.buffer.lines,
            vec!["keep me"],
            "buffer tidak disentuh"
        );
        close(&mut editor);
        assert_eq!(editor.mode, EditorMode::Edit);
    }

    /// A PDF opens as a page, not as `%PDF-1.4` text — provided a rasterizer exists.
    /// The document never becomes the save target either.
    #[test]
    fn a_pdf_opens_as_a_page() {
        if !crate::pdfview::renderer_available() {
            // No poppler / ghostscript here: the feature is honest about it, and there is
            // nothing this test could assert without a rasterizer.
            return;
        }
        let tmp = tempfile::Builder::new()
            .prefix("cozy-pdf-")
            .tempdir()
            .unwrap();
        let path = tmp.path().join("doc.pdf");
        std::fs::write(
            &path,
            b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
              2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
              3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] >> endobj\n\
              trailer << /Root 1 0 R >>\n",
        )
        .unwrap();

        let mut editor = EditorState::new(None);
        editor.buffer = TextBuffer::from_lines(vec!["keep me".to_string()]);
        editor.open_file(path.to_str().unwrap()).unwrap();

        assert_eq!(editor.mode, EditorMode::Image, "PDF dibuka sebagai halaman");
        assert!(editor.filename.is_none(), "PDF bukan target Ctrl+S");
        assert_eq!(
            editor.buffer.lines,
            vec!["keep me"],
            "buffer tidak disentuh"
        );
        let status = editor
            .image_view
            .as_ref()
            .map(|v| v.status())
            .unwrap_or_default();
        assert!(status.contains("PDF"), "status: {status}");
        assert!(status.contains("page 1/1"), "status: {status}");
    }

    /// A text file still goes the old way.
    #[test]
    fn a_text_file_is_still_text() {
        let tmp = tempfile::Builder::new()
            .prefix("cozy-txt-")
            .tempdir()
            .unwrap();
        let path = tmp.path().join("note.txt");
        std::fs::write(&path, "hello\n").unwrap();
        let mut editor = EditorState::new(None);
        editor.open_file(path.to_str().unwrap()).unwrap();
        // `open_file` loads the buffer; which mode rests there is the caller's choice.
        assert_ne!(
            editor.mode,
            EditorMode::Image,
            "teks tidak masuk mode gambar"
        );
        assert_eq!(editor.buffer.lines, vec!["hello"]);
        assert_eq!(
            editor.filename.as_deref(),
            Some(path.as_path()),
            "teks tetap jadi target simpan"
        );
    }
}
