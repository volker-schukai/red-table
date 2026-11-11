//! Clipboard text for image paths and its OSC 52 terminal transport.

use std::path::{Path, PathBuf};

/// Text above this size is refused; terminals bound OSC 52 payloads.
pub(crate) const MAX_TEXT_BYTES: usize = 64 * 1024;

/// One path per line. With `root`, paths below it are made relative; paths
/// outside it stay absolute.
pub(crate) fn text(paths: &[PathBuf], root: Option<&Path>) -> Vec<u8> {
    let mut text = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            text.push(b'\n');
        }
        let shown = root
            .and_then(|root| path.strip_prefix(root).ok())
            .filter(|relative| !relative.as_os_str().is_empty())
            .unwrap_or(path);
        text.extend_from_slice(shown.as_os_str().as_encoded_bytes());
    }
    text
}

/// OSC 52 sequence that asks the terminal to place `text` in the clipboard.
pub(crate) fn osc52(text: &[u8]) -> String {
    let mut sequence = String::from("\x1b]52;c;");
    base64_simd::STANDARD.encode_append(text, &mut sequence);
    sequence.push_str("\x1b\\");
    sequence
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_joins_paths_and_relativizes_only_below_the_root() {
        let paths = [
            PathBuf::from("/photos/trip/a b.jpg"),
            PathBuf::from("/elsewhere/c.png"),
            PathBuf::from("/photos"),
        ];
        assert_eq!(
            text(&paths, None),
            b"/photos/trip/a b.jpg\n/elsewhere/c.png\n/photos"
        );
        assert_eq!(
            text(&paths, Some(Path::new("/photos"))),
            b"trip/a b.jpg\n/elsewhere/c.png\n/photos"
        );
        assert!(text(&[], None).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_bytes_are_copied_unchanged() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let path = PathBuf::from(OsString::from_vec(b"/photos/\xff.jpg".to_vec()));
        assert_eq!(text(&[path], Some(Path::new("/photos"))), b"\xff.jpg");
    }

    #[test]
    fn osc52_frames_base64_text_for_the_clipboard_selection() {
        assert_eq!(osc52(b"/a.png"), "\x1b]52;c;L2EucG5n\x1b\\");
        let hostile = osc52(b"x\x1b]0;title\x07\n");
        assert_eq!(hostile.matches('\x1b').count(), 2, "payload is inert");
    }
}
