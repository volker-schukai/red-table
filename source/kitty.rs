use std::{
    collections::{HashSet, VecDeque},
    env,
    ffi::OsStr,
    fmt::Write,
    io::{self, Write as IoWrite},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use flate2::{Compression, write::ZlibEncoder};
use image::DynamicImage;
use ratatui::{
    buffer::{Buffer, CellDiffOption},
    layout::{Rect, Size},
    widgets::Widget,
};
use ratatui_image::picker::cap_parser::Parser;

const PLACEHOLDER: char = '\u{10EEEE}';
const UNIT_WIDTH: CellDiffOption =
    CellDiffOption::ForcedWidth(std::num::NonZeroU16::new(1).unwrap());
const MAX_IMAGE_ID: u32 = 0x00FF_FFFF;
const CHARS_PER_CHUNK: usize = 4096;
const BYTES_PER_CHUNK: usize = (CHARS_PER_CHUNK / 4) * 3;
const MAX_TRACKED_IMAGE_IDS: usize = 512;
/// RGBA payloads at or above this size are zlib-compressed (`o=z`) before
/// Base64 encoding.
const COMPRESSION_THRESHOLD_BYTES: usize = 1024 * 1024;
static NEXT_IMAGE_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Clone)]
pub(crate) struct KittySession {
    inner: Arc<KittySessionInner>,
}

struct KittySessionInner {
    is_tmux: bool,
    state: Mutex<KittySessionState>,
}

#[derive(Default)]
struct KittySessionState {
    active: HashSet<u32>,
    pending_deletions: VecDeque<u32>,
}

impl KittySession {
    pub(crate) fn from_process() -> Self {
        Self::with_tmux(is_tmux_session())
    }

    pub(crate) fn with_tmux(is_tmux: bool) -> Self {
        Self {
            inner: Arc::new(KittySessionInner {
                is_tmux,
                state: Mutex::new(KittySessionState::default()),
            }),
        }
    }

    pub(crate) fn drain_cleanup(&self, writer: &mut dyn IoWrite) -> io::Result<usize> {
        let pending = {
            let state = self
                .inner
                .state
                .lock()
                .expect("Kitty session state should not be poisoned");
            state.pending_deletions.iter().copied().collect::<Vec<_>>()
        };
        if pending.is_empty() {
            return Ok(0);
        }

        let mut commands = String::new();
        for id in &pending {
            commands.push_str(&delete_image(*id, self.inner.is_tmux));
        }
        writer.write_all(commands.as_bytes())?;
        writer.flush()?;

        let mut state = self
            .inner
            .state
            .lock()
            .expect("Kitty session state should not be poisoned");
        for expected in &pending {
            if state.pending_deletions.front() == Some(expected) {
                state.pending_deletions.pop_front();
            }
        }
        Ok(pending.len())
    }

    fn allocate_image_id(&self) -> Result<u32, String> {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("Kitty session state should not be poisoned");
        if state.active.len() + state.pending_deletions.len() >= MAX_TRACKED_IMAGE_IDS {
            return Err(format!(
                "Kitty image lifecycle reached its {MAX_TRACKED_IMAGE_IDS}-image bound"
            ));
        }
        for _ in 0..MAX_TRACKED_IMAGE_IDS {
            let id = next_image_id();
            if !state.active.contains(&id) && !state.pending_deletions.contains(&id) {
                state.active.insert(id);
                return Ok(id);
            }
        }
        Err("cannot allocate a unique Kitty image identifier".into())
    }

    fn release_image_id(&self, id: u32, transmitted: bool) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("Kitty session state should not be poisoned");
        state.active.remove(&id);
        if transmitted {
            debug_assert!(
                state.active.len() + state.pending_deletions.len() < MAX_TRACKED_IMAGE_IDS
            );
            state.pending_deletions.push_back(id);
        }
    }

    pub(crate) fn has_pending_cleanup(&self) -> bool {
        !self
            .inner
            .state
            .lock()
            .expect("Kitty session state should not be poisoned")
            .pending_deletions
            .is_empty()
    }

    #[cfg(test)]
    fn pending_cleanup_count(&self) -> usize {
        self.inner
            .state
            .lock()
            .expect("Kitty session state should not be poisoned")
            .pending_deletions
            .len()
    }
}

pub(crate) struct KittyProtocol {
    needs_transmission: AtomicBool,
    ever_transmitted: AtomicBool,
    transmission: String,
    id_color: String,
    id: u32,
    size: Size,
    session: KittySession,
}

impl KittyProtocol {
    pub(crate) fn new(
        image: DynamicImage,
        size: Size,
        session: KittySession,
    ) -> Result<Self, String> {
        if usize::from(size.height) > ROW_DIACRITICS.len() {
            return Err(format!(
                "Kitty image height {} exceeds the {}-row placeholder limit",
                size.height,
                ROW_DIACRITICS.len()
            ));
        }
        let id = session.allocate_image_id()?;
        let [_, red, green, blue] = id.to_be_bytes();
        Ok(Self {
            needs_transmission: AtomicBool::new(true),
            ever_transmitted: AtomicBool::new(false),
            transmission: transmit_virtual(image, size, id, session.inner.is_tmux),
            id_color: format!("\x1b[38;2;{red};{green};{blue}m"),
            id,
            size,
            session,
        })
    }

    pub(crate) const fn size(&self) -> Size {
        self.size
    }

    #[cfg(test)]
    pub(crate) const fn id(&self) -> u32 {
        self.id
    }

    pub(crate) fn resident_bytes(&self) -> u64 {
        (self.transmission.capacity() + self.id_color.capacity()) as u64
    }

    pub(crate) fn rearm(&self) -> bool {
        self.ever_transmitted.load(Ordering::Acquire)
            && !self.needs_transmission.swap(true, Ordering::AcqRel)
    }

    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let size = self.size();
        let width = area.width.min(size.width);
        let height = area
            .height
            .min(size.height)
            .min(ROW_DIACRITICS.len() as u16);
        if width == 0 || height == 0 {
            return;
        }

        let transmission = self
            .needs_transmission
            .swap(false, Ordering::AcqRel)
            .then_some(self.transmission.as_str());
        if transmission.is_some() {
            self.ever_transmitted.store(true, Ordering::Release);
        }
        let trailing_placeholders: String =
            std::iter::repeat_n(PLACEHOLDER, usize::from(width.saturating_sub(1))).collect();
        let right = area.width.saturating_sub(1);
        let down = area.height.saturating_sub(1);
        let restore_cursor = format!("\x1b[u\x1b[{right}C\x1b[{down}B");

        for row in 0..height {
            let mut symbol = String::new();
            if row == 0
                && let Some(transmission) = transmission
            {
                symbol.push_str(transmission);
            }
            write!(
                symbol,
                "\x1b[s{}{}{}{}{}",
                self.id_color,
                PLACEHOLDER,
                ROW_DIACRITICS[usize::from(row)],
                ROW_DIACRITICS[0],
                trailing_placeholders
            )
            .expect("writing to a String cannot fail");
            symbol.push_str(&restore_cursor);

            for column in 1..width {
                if let Some(cell) = buffer.cell_mut((area.x + column, area.y + row)) {
                    cell.set_diff_option(CellDiffOption::Skip);
                }
            }
            if let Some(cell) = buffer.cell_mut((area.x, area.y + row)) {
                cell.set_symbol(&symbol).set_diff_option(UNIT_WIDTH);
            }
        }
    }
}

impl Drop for KittyProtocol {
    fn drop(&mut self) {
        self.session
            .release_image_id(self.id, self.ever_transmitted.load(Ordering::Acquire));
    }
}

pub(crate) struct KittyImage<'a> {
    protocol: &'a KittyProtocol,
}

impl<'a> KittyImage<'a> {
    pub(crate) const fn new(protocol: &'a KittyProtocol) -> Self {
        Self { protocol }
    }
}

impl Widget for KittyImage<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        self.protocol.render(area, buffer);
    }
}

fn is_tmux_session() -> bool {
    is_tmux_environment(
        env::var_os("TMUX").as_deref(),
        env::var_os("TERM").as_deref(),
        env::var_os("TERM_PROGRAM").as_deref(),
    )
}

fn is_tmux_environment(
    tmux: Option<&OsStr>,
    term: Option<&OsStr>,
    program: Option<&OsStr>,
) -> bool {
    tmux.is_some_and(|value| !value.is_empty())
        || term.is_some_and(|value| value.as_encoded_bytes().starts_with(b"tmux"))
        || program.is_some_and(|value| value == "tmux")
}

fn next_image_id() -> u32 {
    NEXT_IMAGE_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(if current >= MAX_IMAGE_ID {
                1
            } else {
                current + 1
            })
        })
        .expect("image ID update closure always returns a value")
}

fn transmit_virtual(image: DynamicImage, size: Size, id: u32, is_tmux: bool) -> String {
    let rgba = image.into_rgba8();
    let (pixel_width, pixel_height) = rgba.dimensions();
    let (bytes, compressed) = compress_payload(rgba.into_raw());
    let chunks = bytes.chunks(BYTES_PER_CHUNK);
    let chunk_count = chunks.len();
    let (start, escape, end) = Parser::tmux_start_escape_end(is_tmux);
    let encoded_chars = bytes.len().div_ceil(3).saturating_mul(4);
    let command_overhead =
        chunk_count.saturating_mul(start.len() + end.len() + escape.len().saturating_mul(2) + 96);
    let mut output = String::with_capacity(encoded_chars.saturating_add(command_overhead));

    for (index, chunk) in chunks.enumerate() {
        output.push_str(start);
        write!(output, "{escape}_Gq=2,").expect("writing to a String cannot fail");
        if index == 0 {
            write!(
                output,
                "i={id},a=T,U=1,N=1,c={},r={},f=32,t=d,s={pixel_width},v={pixel_height},",
                size.width, size.height,
            )
            .expect("writing to a String cannot fail");
            if compressed {
                output.push_str("o=z,");
            }
        }
        let more = u8::from(index + 1 < chunk_count);
        write!(output, "m={more};").expect("writing to a String cannot fail");
        base64_simd::STANDARD.encode_append(chunk, &mut output);
        write!(output, "{escape}\\").expect("writing to a String cannot fail");
        output.push_str(end);
    }
    output
}

/// Compresses payloads at or above the threshold with zlib at the fastest
/// level. Returns the raw bytes when compression is skipped or does not shrink
/// the payload.
fn compress_payload(raw: Vec<u8>) -> (Vec<u8>, bool) {
    if raw.len() < COMPRESSION_THRESHOLD_BYTES {
        return (raw, false);
    }
    let mut encoder = ZlibEncoder::new(Vec::with_capacity(raw.len() / 2), Compression::fast());
    if encoder.write_all(&raw).is_err() {
        return (raw, false);
    }
    match encoder.finish() {
        Ok(compressed) if compressed.len() < raw.len() => (compressed, true),
        _ => (raw, false),
    }
}

fn delete_image(id: u32, is_tmux: bool) -> String {
    let (start, escape, end) = Parser::tmux_start_escape_end(is_tmux);
    format!("{start}{escape}_Gq=2,a=d,d=I,i={id}{escape}\\{end}")
}

/// From https://sw.kovidgoyal.net/kitty/_downloads/1792bad15b12979994cd6ecc54c967a6/rowcolumn-diacritics.txt
/// See https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders for further explanation.
const ROW_DIACRITICS: [char; 297] = [
    '\u{305}',
    '\u{30D}',
    '\u{30E}',
    '\u{310}',
    '\u{312}',
    '\u{33D}',
    '\u{33E}',
    '\u{33F}',
    '\u{346}',
    '\u{34A}',
    '\u{34B}',
    '\u{34C}',
    '\u{350}',
    '\u{351}',
    '\u{352}',
    '\u{357}',
    '\u{35B}',
    '\u{363}',
    '\u{364}',
    '\u{365}',
    '\u{366}',
    '\u{367}',
    '\u{368}',
    '\u{369}',
    '\u{36A}',
    '\u{36B}',
    '\u{36C}',
    '\u{36D}',
    '\u{36E}',
    '\u{36F}',
    '\u{483}',
    '\u{484}',
    '\u{485}',
    '\u{486}',
    '\u{487}',
    '\u{592}',
    '\u{593}',
    '\u{594}',
    '\u{595}',
    '\u{597}',
    '\u{598}',
    '\u{599}',
    '\u{59C}',
    '\u{59D}',
    '\u{59E}',
    '\u{59F}',
    '\u{5A0}',
    '\u{5A1}',
    '\u{5A8}',
    '\u{5A9}',
    '\u{5AB}',
    '\u{5AC}',
    '\u{5AF}',
    '\u{5C4}',
    '\u{610}',
    '\u{611}',
    '\u{612}',
    '\u{613}',
    '\u{614}',
    '\u{615}',
    '\u{616}',
    '\u{617}',
    '\u{657}',
    '\u{658}',
    '\u{659}',
    '\u{65A}',
    '\u{65B}',
    '\u{65D}',
    '\u{65E}',
    '\u{6D6}',
    '\u{6D7}',
    '\u{6D8}',
    '\u{6D9}',
    '\u{6DA}',
    '\u{6DB}',
    '\u{6DC}',
    '\u{6DF}',
    '\u{6E0}',
    '\u{6E1}',
    '\u{6E2}',
    '\u{6E4}',
    '\u{6E7}',
    '\u{6E8}',
    '\u{6EB}',
    '\u{6EC}',
    '\u{730}',
    '\u{732}',
    '\u{733}',
    '\u{735}',
    '\u{736}',
    '\u{73A}',
    '\u{73D}',
    '\u{73F}',
    '\u{740}',
    '\u{741}',
    '\u{743}',
    '\u{745}',
    '\u{747}',
    '\u{749}',
    '\u{74A}',
    '\u{7EB}',
    '\u{7EC}',
    '\u{7ED}',
    '\u{7EE}',
    '\u{7EF}',
    '\u{7F0}',
    '\u{7F1}',
    '\u{7F3}',
    '\u{816}',
    '\u{817}',
    '\u{818}',
    '\u{819}',
    '\u{81B}',
    '\u{81C}',
    '\u{81D}',
    '\u{81E}',
    '\u{81F}',
    '\u{820}',
    '\u{821}',
    '\u{822}',
    '\u{823}',
    '\u{825}',
    '\u{826}',
    '\u{827}',
    '\u{829}',
    '\u{82A}',
    '\u{82B}',
    '\u{82C}',
    '\u{82D}',
    '\u{951}',
    '\u{953}',
    '\u{954}',
    '\u{F82}',
    '\u{F83}',
    '\u{F86}',
    '\u{F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use image::{Rgba, RgbaImage};
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn transmission_declares_virtual_placement_extent_and_exact_rgba_payload() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255])));
        let command = transmit_virtual(image, Size::new(32, 14), 42, false);

        assert!(command.starts_with("\x1b_Gq=2,i=42,a=T,U=1,N=1,c=32,r=14,f=32,t=d,s=1,v=1,m=0;"));
        assert!(command.ends_with("/wAA/w==\x1b\\"));
    }

    /// Splits a direct (non-tmux) transmission into its chunk payloads and
    /// returns the decoded bytes plus the first chunk's control keys.
    fn decode_transmission(command: &str) -> (String, Vec<u8>) {
        let mut header = None;
        let mut payload = Vec::new();
        for chunk in command.split("\x1b\\").filter(|chunk| !chunk.is_empty()) {
            let body = chunk
                .strip_prefix("\x1b_G")
                .expect("every chunk is a graphics command");
            let (keys, data) = body.split_once(';').expect("chunks carry a payload");
            assert!(data.len() <= CHARS_PER_CHUNK, "chunk exceeds 4096 chars");
            header.get_or_insert_with(|| keys.to_owned());
            payload.extend(base64_simd::STANDARD.decode_to_vec(data).unwrap());
        }
        (header.expect("at least one chunk"), payload)
    }

    #[test]
    fn small_payloads_stay_raw_and_large_payloads_compress_losslessly() {
        use std::io::Read;

        let small = RgbaImage::from_fn(256, 256, |x, y| Rgba([x as u8, y as u8, 7, 255]));
        let small_raw = small.as_raw().clone();
        assert!(small_raw.len() < COMPRESSION_THRESHOLD_BYTES);
        let (header, payload) = decode_transmission(&transmit_virtual(
            DynamicImage::ImageRgba8(small),
            Size::new(8, 4),
            7,
            false,
        ));
        assert!(!header.contains("o=z"), "{header}");
        assert_eq!(payload, small_raw);

        let large = RgbaImage::from_fn(640, 480, |x, y| {
            Rgba([(x / 4) as u8, (y / 4) as u8, ((x ^ y) & 15) as u8, 255])
        });
        let large_raw = large.as_raw().clone();
        assert!(large_raw.len() >= COMPRESSION_THRESHOLD_BYTES);
        let command =
            transmit_virtual(DynamicImage::ImageRgba8(large), Size::new(64, 24), 8, false);
        let (header, payload) = decode_transmission(&command);
        assert!(header.contains(",o=z,"), "{header}");
        assert!(header.contains("s=640,v=480,"), "{header}");
        assert!(payload.len() < large_raw.len());
        let mut inflated = Vec::new();
        flate2::read::ZlibDecoder::new(payload.as_slice())
            .read_to_end(&mut inflated)
            .unwrap();
        assert_eq!(inflated, large_raw);
    }

    #[test]
    fn incompressible_payloads_fall_back_to_raw_transmission() {
        let mut state = 0x9E37_79B9_u32;
        let mut noise = vec![0_u8; COMPRESSION_THRESHOLD_BYTES];
        for byte in &mut noise {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        let (payload, compressed) = compress_payload(noise.clone());
        assert!(!compressed);
        assert_eq!(payload, noise);
    }

    #[test]
    fn rendered_placeholders_transmit_once_and_encode_rows() {
        let session = KittySession::with_tmux(false);
        let protocol = KittyProtocol::new(
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 4, Rgba([10, 20, 30, 255]))),
            Size::new(2, 2),
            session,
        )
        .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(2, 2)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(KittyImage::new(&protocol), frame.area()))
            .unwrap();
        let first = terminal.backend().buffer().cell((0, 0)).unwrap().symbol();
        assert!(first.contains("a=T,U=1,N=1,c=2,r=2"));
        assert!(first.contains(ROW_DIACRITICS[0]));
        assert!(
            terminal
                .backend()
                .buffer()
                .cell((0, 1))
                .unwrap()
                .symbol()
                .contains(ROW_DIACRITICS[1])
        );

        terminal
            .draw(|frame| frame.render_widget(KittyImage::new(&protocol), frame.area()))
            .unwrap();
        assert!(
            !terminal
                .backend()
                .buffer()
                .cell((0, 0))
                .unwrap()
                .symbol()
                .contains("a=T,U=1")
        );

        assert!(protocol.rearm());
        assert!(!protocol.rearm());
        terminal
            .draw(|frame| frame.render_widget(KittyImage::new(&protocol), frame.area()))
            .unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .cell((0, 0))
                .unwrap()
                .symbol()
                .contains("a=T,U=1")
        );
    }

    #[test]
    fn supports_every_normative_placeholder_row_and_rejects_the_next() {
        for height in [61, 297] {
            let protocol = KittyProtocol::new(
                DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255]))),
                Size::new(1, height),
                KittySession::with_tmux(false),
            )
            .unwrap();
            let mut terminal = Terminal::new(TestBackend::new(1, height)).unwrap();
            terminal
                .draw(|frame| frame.render_widget(KittyImage::new(&protocol), frame.area()))
                .unwrap();
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .cell((0, height - 1))
                    .unwrap()
                    .symbol()
                    .contains(ROW_DIACRITICS[usize::from(height - 1)])
            );
        }

        let result = KittyProtocol::new(
            DynamicImage::new_rgba8(1, 1),
            Size::new(1, 298),
            KittySession::with_tmux(false),
        );
        let Err(error) = result else {
            panic!("row 298 must be rejected");
        };
        assert!(error.contains("297-row"));
    }

    #[test]
    fn tmux_detection_uses_the_authoritative_environment_variable() {
        assert!(is_tmux_environment(
            Some(OsStr::new("/tmp/tmux-1000/default,1,0")),
            Some(OsStr::new("xterm-256color")),
            None,
        ));
        assert!(is_tmux_environment(
            None,
            Some(OsStr::new("tmux-256color")),
            None,
        ));
        assert!(is_tmux_environment(None, None, Some(OsStr::new("tmux")),));
        assert!(!is_tmux_environment(
            Some(OsStr::new("")),
            Some(OsStr::new("xterm-kitty")),
            None,
        ));
    }

    #[test]
    fn dropping_transmitted_images_deletes_only_their_owned_data() {
        let session = KittySession::with_tmux(false);
        let protocol = KittyProtocol::new(
            DynamicImage::new_rgba8(1, 1),
            Size::new(1, 1),
            session.clone(),
        )
        .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(KittyImage::new(&protocol), frame.area()))
            .unwrap();
        drop(protocol);
        assert_eq!(session.pending_cleanup_count(), 1);

        let mut output = Vec::new();
        assert_eq!(session.drain_cleanup(&mut output).unwrap(), 1);
        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("\x1b_Gq=2,a=d,d=I,i="));
        assert!(output.ends_with("\x1b\\"));
        assert_eq!(session.pending_cleanup_count(), 0);

        let untransmitted = KittyProtocol::new(
            DynamicImage::new_rgba8(1, 1),
            Size::new(1, 1),
            session.clone(),
        )
        .unwrap();
        drop(untransmitted);
        assert_eq!(session.pending_cleanup_count(), 0);
    }

    #[test]
    fn tmux_cleanup_is_wrapped_and_lifecycle_tracking_is_bounded() {
        let tmux = KittySession::with_tmux(true);
        let protocol =
            KittyProtocol::new(DynamicImage::new_rgba8(1, 1), Size::new(1, 1), tmux.clone())
                .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(KittyImage::new(&protocol), frame.area()))
            .unwrap();
        drop(protocol);
        let mut output = Vec::new();
        tmux.drain_cleanup(&mut output).unwrap();
        assert!(String::from_utf8(output).unwrap().starts_with("\x1bPtmux;"));

        let session = KittySession::with_tmux(false);
        let protocols = (0..MAX_TRACKED_IMAGE_IDS)
            .map(|_| {
                KittyProtocol::new(
                    DynamicImage::new_rgba8(1, 1),
                    Size::new(1, 1),
                    session.clone(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let overflow = KittyProtocol::new(
            DynamicImage::new_rgba8(1, 1),
            Size::new(1, 1),
            session.clone(),
        );
        let Err(error) = overflow else {
            panic!("the lifecycle bound must reject another image");
        };
        assert!(error.contains("512-image bound"));
        drop(protocols);
        assert!(
            KittyProtocol::new(DynamicImage::new_rgba8(1, 1), Size::new(1, 1), session,).is_ok()
        );
    }
}
