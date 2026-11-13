//! PDF pages, drawn as pictures.
//!
//! ⚠️ **Why not a Rust PDF renderer.** The engine `tdf` uses is MuPDF, which is
//! **AGPL-3.0** — linking it would relicense cozy's whole binary (MIT/Apache-2.0) and
//! drags ~50 MB of C plus a git dependency behind it. So, like the Markdown preview's
//! remote images hand the fetching to `curl`, this hands *rasterizing* to the tool the
//! system already has: poppler's `pdftocairo` (the usual one on a desktop), Ghostscript
//! (`gs`). A process, not a link: nothing to license, nothing to ship, and if the tool is
//! missing the message says which package to install.
//!
//! The result is an ordinary [`DynamicImage`], so it goes straight into the image view
//! cozy already has — the sidebar, `z`, and the status bar come for free.

use std::path::Path;
use std::process::Command;

use image::DynamicImage;

/// PDFs always start with this. The extension is a hint; the bytes are the fact (the same
/// rule the image sniffing follows).
pub(crate) fn is_pdf(head: &[u8]) -> bool {
    head.starts_with(b"%PDF-")
}

/// How many pixels per inch to rasterize at. 144 keeps a page crisp at 1:1 without
/// turning an A4 into a 30 MB RGBA buffer (A4 at 144 dpi is 1190×1684).
pub(crate) const PDF_DPI: u32 = 144;

/// The rasterizer this machine has, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// poppler's `pdftocairo` — the best quality, and it writes the page to stdout.
    PdfToCairo,
    /// Ghostscript, the fallback: slower to start, always correct.
    Ghostscript,
}

impl Tool {
    fn command(self, path: &Path, page: usize, dpi: u32) -> Command {
        match self {
            Tool::PdfToCairo => {
                let mut cmd = Command::new("pdftocairo");
                cmd.args([
                    "-png",
                    "-f",
                    &page.to_string(),
                    "-l",
                    &page.to_string(),
                    "-singlefile",
                    "-r",
                    &dpi.to_string(),
                ])
                .arg(path)
                // `-` is stdout: the page never touches the disk.
                .arg("-");
                cmd
            }
            Tool::Ghostscript => {
                let mut cmd = Command::new("gs");
                cmd.args(["-q", "-dNOPAUSE", "-dBATCH", "-sDEVICE=png16m"]);
                cmd.arg(format!("-r{dpi}"));
                cmd.arg(format!("-dFirstPage={page}"));
                cmd.arg(format!("-dLastPage={page}"));
                cmd.args(["-sOutputFile=-"]);
                cmd.arg(path);
                cmd
            }
        }
    }
}

fn find_tool() -> Option<Tool> {
    [Tool::PdfToCairo, Tool::Ghostscript]
        .into_iter()
        .find(|tool| {
            which(match tool {
                Tool::PdfToCairo => "pdftocairo",
                Tool::Ghostscript => "gs",
            })
        })
}

/// `which`, without a dependency: PATH order decides, as a shell would.
fn which(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file()
    })
}

/// The longest side a page may be rasterized to. A poster-sized page at 144 dpi is
/// otherwise tens of megabytes of RGBA — and the terminal shows a few hundred cells.
const MAX_SIDE: u32 = 4000;

/// Whether a rasterizer exists here (`pdftocairo` or `gs`).
pub(crate) fn renderer_available_now() -> bool {
    find_tool().is_some()
}

/// The same question, for tests that must skip on a machine with neither tool.
#[cfg(test)]
pub(crate) fn renderer_available() -> bool {
    renderer_available_now()
}

/// How many pages the document has, when `pdfinfo` can say. `None` only means "unknown":
/// the first page still opens.
pub(crate) fn page_count(path: &Path) -> Option<usize> {
    let out = Command::new("pdfinfo").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_pages(&String::from_utf8_lossy(&out.stdout))
}

/// Pull `Pages: 12` out of `pdfinfo`'s report.
fn parse_pages(text: &str) -> Option<usize> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("Pages:"))
        .and_then(|rest| rest.trim().parse::<usize>().ok())
}

/// Rasterize one page (1-based). The error string is what the status bar shows, so it
/// names the missing package instead of the failed syscall.
pub(crate) fn render_page(path: &Path, page: usize, dpi: u32) -> Result<DynamicImage, String> {
    let page = page.max(1);
    let Some(tool) = find_tool() else {
        return Err(format!("no PDF renderer — {}", missing_tool_help()));
    };
    let out = tool
        .command(path, page, dpi)
        .output()
        .map_err(|e| format!("could not run the PDF renderer: {e}"))?;
    if !out.status.success() {
        let mut why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if why.is_empty() {
            why = format!("{} exited with {}", tool_name(tool), out.status);
        }
        return Err(why);
    }
    let image = image::load_from_memory(&out.stdout)
        .map_err(|e| format!("the PDF renderer did not return a picture: {e}"))?;
    Ok(cap_side(image))
}

fn tool_name(tool: Tool) -> &'static str {
    match tool {
        Tool::PdfToCairo => "pdftocairo",
        Tool::Ghostscript => "gs",
    }
}

/// Keep an enormous page from costing tens of megabytes; the terminal cannot show more
/// than a few hundred cells anyway.
fn cap_side(image: DynamicImage) -> DynamicImage {
    if image.width() > MAX_SIDE || image.height() > MAX_SIDE {
        image.thumbnail(MAX_SIDE, MAX_SIDE)
    } else {
        image
    }
}

/// The page's path as the viewer labels it (the file name, not the whole path).
pub(crate) fn label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// What to install when no rasterizer is found. One sentence, in the status bar.
fn missing_tool_help() -> String {
    "install poppler-utils (pdftocairo) or ghostscript to read PDFs".to_string()
}

/// Is this path a PDF, by its first bytes? Reuses the same bounded read the image sniff
/// uses, so the editor can ask about a file without loading it.
pub(crate) fn sniff_path(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 5];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(n) = file.read(&mut head) else {
        return false;
    };
    is_pdf(&head[..n])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pdf_is_recognized_by_its_signature() {
        assert!(is_pdf(b"%PDF-1.7\n%\xe2\xe3"));
        assert!(!is_pdf(b"%PDX-1.7"));
        assert!(!is_pdf(b"hello"));
    }

    #[test]
    fn pages_are_read_out_of_pdfinfo() {
        let report = "Title:          x\nPages:           12\nEncrypted:      no\n";
        assert_eq!(parse_pages(report), Some(12));
        assert_eq!(parse_pages("Pages: 1\n"), Some(1));
        assert_eq!(parse_pages("Title: x\n"), None);
        assert_eq!(parse_pages("Pages: many\n"), None);
    }

    /// Both commands put the page on **stdout** — that is the whole point (no temp files,
    /// nothing to clean up, and a pipe cannot leak a file into the folder).
    #[test]
    fn the_rasterizers_write_to_stdout() {
        let path = Path::new("/tmp/a file.pdf");
        let cairo = Tool::PdfToCairo.command(path, 3, 144);
        let args: Vec<_> = cairo
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            [
                "-png",
                "-f",
                "3",
                "-l",
                "3",
                "-singlefile",
                "-r",
                "144",
                "/tmp/a file.pdf",
                "-"
            ]
        );
        let gs = Tool::Ghostscript.command(path, 3, 144);
        let args: Vec<_> = gs
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(args.contains(&"-sOutputFile=-".to_string()));
        assert!(args.contains(&"-dFirstPage=3".to_string()));
        assert!(args.last().is_some_and(|a| a == "/tmp/a file.pdf"));
    }

    /// A missing rasterizer is reported by name, not by a failed syscall.
    #[test]
    fn no_renderer_says_what_to_install() {
        // Only meaningful when neither tool exists, which is not this machine — so the
        // message is checked directly rather than by removing PATH (that would break
        // every other test that spawns a process).
        assert!(missing_tool_help().contains("poppler-utils"));
    }

    #[test]
    fn the_label_is_the_file_name() {
        assert_eq!(label(Path::new("/deep/path/report.pdf")), "report.pdf");
    }
}
