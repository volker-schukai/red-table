use std::{
    fs::OpenOptions,
    io::{self, BufWriter, Write, stdout},
    sync::atomic::{AtomicU8, Ordering},
};

use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

pub(crate) type AppTerminal = Terminal<CrosstermBackend<Box<dyn Write + Send>>>;

/// Frames, Kitty transmissions, and control sequences are buffered; every
/// writer path flushes explicitly.
const OUTPUT_BUFFER_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalTarget {
    Stdout,
    Tty,
}

impl TerminalTarget {
    pub(crate) const fn for_result_mode(result_mode: bool) -> Self {
        if result_mode { Self::Tty } else { Self::Stdout }
    }

    const fn code(self) -> u8 {
        match self {
            Self::Stdout => 0,
            Self::Tty => 1,
        }
    }

    const fn from_code(code: u8) -> Self {
        if code == 1 { Self::Tty } else { Self::Stdout }
    }
}

static ACTIVE_TARGET: AtomicU8 = AtomicU8::new(0);

pub(crate) struct TerminalSession {
    terminal: AppTerminal,
    restored: bool,
}

impl TerminalSession {
    pub(crate) fn new(target: TerminalTarget) -> io::Result<Self> {
        install_panic_restore(target);
        ACTIVE_TARGET.store(target.code(), Ordering::Release);
        enable_raw_mode()?;
        let mut writer = writer(target)?;
        if let Err(error) = execute!(writer, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        let backend = CrosstermBackend::new(writer);
        let terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => {
                restore_target(target);
                return Err(error);
            }
        };
        Ok(Self {
            terminal,
            restored: false,
        })
    }

    pub(crate) fn terminal_mut(&mut self) -> &mut AppTerminal {
        &mut self.terminal
    }

    pub(crate) fn restore(&mut self) {
        if self.restored {
            return;
        }
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
        self.restored = true;
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.restore();
    }
}

fn writer(target: TerminalTarget) -> io::Result<Box<dyn Write + Send>> {
    Ok(match target {
        TerminalTarget::Stdout => buffered(stdout()),
        TerminalTarget::Tty => {
            buffered(OpenOptions::new().read(true).write(true).open("/dev/tty")?)
        }
    })
}

fn buffered(writer: impl Write + Send + 'static) -> Box<dyn Write + Send> {
    Box::new(BufWriter::with_capacity(OUTPUT_BUFFER_BYTES, writer))
}

fn restore_target(target: TerminalTarget) {
    let _ = disable_raw_mode();
    if let Ok(mut writer) = writer(target) {
        let _ = execute!(writer, LeaveAlternateScreen);
    }
}

fn install_panic_restore(target: TerminalTarget) {
    ACTIVE_TARGET.store(target.code(), Ordering::Release);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Worker panics are caught and reported as request errors; restoring
        // the terminal here would corrupt the still-running interface.
        if crate::worker::is_worker_thread() {
            return;
        }
        restore_target(TerminalTarget::from_code(
            ACTIVE_TARGET.load(Ordering::Acquire),
        ));
        previous(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_frame_reaches_the_terminal_in_few_writes() {
        use std::sync::{Arc, Mutex};

        use ratatui::{
            Terminal, TerminalOptions, Viewport, layout::Rect, style::Color, widgets::Paragraph,
        };

        #[derive(Clone, Default)]
        struct Counting(Arc<Mutex<(usize, usize)>>);
        impl Write for Counting {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                let mut counts = self.0.lock().unwrap();
                counts.0 += 1;
                counts.1 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let frame = |writer: Box<dyn Write + Send>| {
            let mut terminal = Terminal::with_options(
                CrosstermBackend::new(writer),
                TerminalOptions {
                    viewport: Viewport::Fixed(Rect::new(0, 0, 200, 60)),
                },
            )
            .unwrap();
            terminal
                .draw(|frame| {
                    for row in 0..60 {
                        let color = Color::Indexed(row as u8);
                        frame.render_widget(
                            Paragraph::new("x".repeat(200))
                                .style(ratatui::style::Style::default().fg(color)),
                            Rect::new(0, row, 200, 1),
                        );
                    }
                })
                .unwrap();
        };
        let raw = Counting::default();
        frame(Box::new(raw.clone()));
        let buffered_counts = Counting::default();
        frame(buffered(buffered_counts.clone()));

        let (raw_writes, raw_bytes) = *raw.0.lock().unwrap();
        let (buffered_writes, buffered_bytes) = *buffered_counts.0.lock().unwrap();
        assert_eq!(raw_bytes, buffered_bytes);
        assert!(raw_writes > 1_000, "unbuffered frame: {raw_writes} writes");
        assert!(
            buffered_writes <= 2,
            "buffered frame: {buffered_writes} writes for {buffered_bytes} bytes"
        );
    }

    #[test]
    fn target_code_round_trips() {
        for target in [TerminalTarget::Stdout, TerminalTarget::Tty] {
            assert_eq!(TerminalTarget::from_code(target.code()), target);
        }
    }
}
