use std::{
    ffi::OsString,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_RECORD_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_RECORDS: usize = 1_000_000;

pub(crate) fn read_nul_paths(reader: impl Read) -> io::Result<Vec<PathBuf>> {
    let mut reader = reader;
    let mut buffer = [0_u8; 8192];
    let mut record = Vec::new();
    let mut paths = Vec::new();
    let mut total = 0_usize;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            total = total.saturating_add(1);
            if total > MAX_TOTAL_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "NUL path input exceeds the 64 MiB limit",
                ));
            }
            if *byte == 0 {
                if record.is_empty() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "NUL path input contains an empty record",
                    ));
                }
                if paths.len() == MAX_RECORDS {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "NUL path input exceeds 1000000 records",
                    ));
                }
                paths.push(PathBuf::from(os_string_from_bytes(std::mem::take(
                    &mut record,
                ))?));
            } else {
                if record.len() == MAX_RECORD_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "NUL path record exceeds the 1 MiB limit",
                    ));
                }
                record.push(*byte);
            }
        }
    }
    if !record.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "NUL path input ends without a NUL delimiter",
        ));
    }
    Ok(paths)
}

/// Escapes control characters so untrusted text cannot drive a terminal.
pub(crate) fn escape_controls(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() {
            escaped.extend(character.escape_default());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

/// Writes a confirmed selection. NUL output is byte-exact. Newline output is
/// escaped for a terminal and refuses paths whose newline would split records
/// for a consuming program.
pub(crate) fn write_selection(
    writer: &mut impl Write,
    paths: &[PathBuf],
    nul_delimited: bool,
    terminal: bool,
) -> io::Result<()> {
    if nul_delimited {
        return write_paths(writer, paths, true);
    }
    if terminal {
        for path in paths {
            writeln!(writer, "{}", escape_controls(&path.to_string_lossy()))?;
        }
        return writer.flush();
    }
    if let Some(path) = paths.iter().find(|path| path_bytes(path).contains(&b'\n')) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "selected path {} contains a newline; use --print0 for exact output",
                escape_controls(&path.to_string_lossy())
            ),
        ));
    }
    write_paths(writer, paths, false)
}

fn write_paths(writer: &mut impl Write, paths: &[PathBuf], nul_delimited: bool) -> io::Result<()> {
    let delimiter = if nul_delimited { 0 } else { b'\n' };
    for path in paths {
        writer.write_all(path_bytes(path))?;
        writer.write_all(&[delimiter])?;
    }
    writer.flush()
}

#[cfg(unix)]
fn os_string_from_bytes(bytes: Vec<u8>) -> io::Result<OsString> {
    use std::os::unix::ffi::OsStringExt;
    Ok(OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn os_string_from_bytes(bytes: Vec<u8>) -> io::Result<OsString> {
    String::from_utf8(bytes)
        .map(OsString::from)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "path is not valid UTF-8"))
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> &[u8] {
    path.as_os_str().as_encoded_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_nul_records_and_rejects_malformed_input() {
        assert_eq!(
            read_nul_paths(&b"one.jpg\0nested/two.png\0"[..]).unwrap(),
            vec![PathBuf::from("one.jpg"), PathBuf::from("nested/two.png")]
        );
        assert!(read_nul_paths(&b"unterminated"[..]).is_err());
        assert!(read_nul_paths(&b"one\0\0"[..]).is_err());
        assert!(
            read_nul_paths(
                std::iter::repeat_n(b'x', MAX_RECORD_BYTES + 1)
                    .collect::<Vec<_>>()
                    .as_slice()
            )
            .is_err()
        );
    }

    #[test]
    fn control_characters_are_escaped_and_text_is_kept() {
        assert_eq!(escape_controls("plain ä.png"), "plain ä.png");
        assert_eq!(
            escape_controls("x\u{1b}]52;c;Zm9v\u{7}\n\u{9b}.png"),
            "x\\u{1b}]52;c;Zm9v\\u{7}\\n\\u{9b}.png"
        );
    }

    #[test]
    fn newline_selection_is_escaped_on_terminals_and_refused_for_pipes() {
        let hostile = vec![
            PathBuf::from("/photos/a.png"),
            PathBuf::from("/photos/x\u{1b}[2J\n/etc/passwd"),
        ];
        let mut terminal = Vec::new();
        write_selection(&mut terminal, &hostile, false, true).unwrap();
        let terminal = String::from_utf8(terminal).unwrap();
        assert_eq!(terminal.lines().count(), 2);
        assert!(!terminal.contains('\u{1b}'));

        let mut pipe = Vec::new();
        let error = write_selection(&mut pipe, &hostile, false, false).unwrap_err();
        assert!(error.to_string().contains("--print0"));
        assert!(pipe.is_empty(), "nothing is written before the refusal");

        let mut plain = Vec::new();
        write_selection(&mut plain, &hostile[..1], false, false).unwrap();
        assert_eq!(plain, b"/photos/a.png\n");

        let mut nul = Vec::new();
        write_selection(&mut nul, &hostile, true, false).unwrap();
        assert_eq!(nul.iter().filter(|byte| **byte == 0).count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn nul_output_preserves_non_utf8_and_embedded_newline_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let paths = vec![PathBuf::from(OsString::from_vec(vec![
            b'/', b't', b'm', b'p', b'/', 0xff, b'\n', b'x',
        ]))];
        let mut output = Vec::new();
        write_selection(&mut output, &paths, true, true).unwrap();
        assert_eq!(output, b"/tmp/\xff\nx\0");
    }
}
