use std::{
    io,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui_image::picker::Picker;

use crate::{
    BoxError,
    actions::{Action, ActionContext, Bindings},
    app::{App, EntryId, EntryOrder, InputMode},
    config::RuntimeSettings,
    graphics,
    inspection::{DecodedSources, Inspection, InspectionDisplay, InspectionKey, ViewState},
    kitty::KittySession,
    scan::{self, ScanEvent},
    settings::{BackgroundMode, DEFAULT_THUMBNAIL_SIZE, ThumbnailQuality, ThumbnailSize},
    terminal::{AppTerminal, TerminalSession, TerminalTarget},
    thumbnails::{ThumbnailKey, Thumbnails},
    ui::{self, ComparisonOptions, GridLayout, InspectionOptions, RenderOptions},
};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);
/// Delays after a frame that transmitted Kitty image data at which a
/// state-neutral repaint nudge is written. The terminal resolves new virtual
/// placements only on a repaint that follows the transmitting input burst;
/// the second nudge covers slow renderers.
const REPAINT_NUDGE_DELAYS: [Duration; 2] = [Duration::from_millis(50), Duration::from_millis(300)];
/// Poll interval while no scan, thumbnail, inspection, or cleanup work is
/// pending; only input and resize events need to wake the loop.
const IDLE_INTERVAL: Duration = Duration::from_millis(250);
const PREFETCH_ROWS: usize = 1;

pub(crate) enum BrowseOutcome {
    Completed,
    Cancelled,
    Selection(Vec<PathBuf>),
}

pub(crate) fn run(
    path: PathBuf,
    settings: RuntimeSettings,
    selection_mode: bool,
    input_paths: Option<Vec<PathBuf>>,
) -> Result<BrowseOutcome, BoxError> {
    let root = path
        .canonicalize()
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    if !root.is_dir() {
        return Err(format!("not a directory: {}", root.display()).into());
    }

    let mut terminal = TerminalSession::new(TerminalTarget::for_result_mode(selection_mode))?;
    let (picker, protocol_name) =
        graphics::select_picker(settings.graphics_protocol, !selection_mode);
    let kitty_session = KittySession::from_process();
    let mut browser = Browser::new(
        root,
        picker,
        protocol_name,
        settings,
        selection_mode,
        input_paths,
        kitty_session.clone(),
    );
    let loop_result = browser.run(terminal.terminal_mut());
    let close_result = close(
        browser,
        &kitty_session,
        terminal.terminal_mut().backend_mut(),
    );
    terminal.restore();
    loop_result?;
    Ok(close_result?)
}

/// Finishes the browser and releases every terminal-side image it still owns.
///
/// Dropping the browser queues deletions for all transmitted Kitty images, so
/// the drain must run afterwards and before the terminal is restored.
fn close(
    browser: Browser,
    kitty_session: &KittySession,
    writer: &mut dyn io::Write,
) -> io::Result<BrowseOutcome> {
    let outcome = browser.finish();
    kitty_session.drain_cleanup(writer)?;
    Ok(outcome)
}

struct Browser {
    app: App,
    scanner: mpsc::Receiver<ScanEvent>,
    thumbnails: Thumbnails,
    kitty_session: KittySession,
    inspection: Inspection,
    comparison_inspection: Inspection,
    view: BrowserView,
    inspection_size: ratatui::layout::Size,
    inspection_key: Option<InspectionKey>,
    comparison_size: ratatui::layout::Size,
    comparison_key: Option<InspectionKey>,
    protocol_name: String,
    grid: GridLayout,
    viewport_ids: Vec<EntryId>,
    viewport_size: ratatui::layout::Size,
    viewport_quality: ThumbnailQuality,
    viewport_generation: u64,
    thumbnail_size: ThumbnailSize,
    thumbnail_quality: ThumbnailQuality,
    bindings: Bindings,
    show_help: bool,
    debug_status: bool,
    background: BackgroundMode,
    running: bool,
    selection_mode: bool,
    confirmed: bool,
    repaint_nudges: Vec<Instant>,
}

#[derive(Clone, Copy, Debug)]
enum BrowserView {
    Grid,
    Inspect(ViewState),
    Compare(CompareState),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ComparePane {
    Reference,
    Candidate,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CompareState {
    pub(crate) reference_id: EntryId,
    pub(crate) reference_view: ViewState,
    pub(crate) candidate_view: ViewState,
    pub(crate) active: ComparePane,
    pub(crate) synchronized: bool,
}

impl CompareState {
    fn mutate_view(&mut self, mutation: impl FnOnce(&mut ViewState)) {
        let active = match self.active {
            ComparePane::Reference => &mut self.reference_view,
            ComparePane::Candidate => &mut self.candidate_view,
        };
        mutation(active);
        if self.synchronized {
            let shared = *active;
            self.reference_view = shared;
            self.candidate_view = shared;
        }
    }

    fn toggle_sync(&mut self) {
        self.synchronized = !self.synchronized;
        if self.synchronized {
            let shared = match self.active {
                ComparePane::Reference => self.reference_view,
                ComparePane::Candidate => self.candidate_view,
            };
            self.reference_view = shared;
            self.candidate_view = shared;
        }
    }

    fn recenter_candidate(&mut self) {
        self.candidate_view.recenter();
        if self.synchronized {
            self.reference_view.recenter();
        }
    }
}

impl Browser {
    fn new(
        root: PathBuf,
        picker: Picker,
        protocol_name: String,
        settings: RuntimeSettings,
        selection_mode: bool,
        input_paths: Option<Vec<PathBuf>>,
        kitty_session: KittySession,
    ) -> Self {
        let decoded_sources = DecodedSources::new(settings.decoded_cache_bytes);
        let inspection = Inspection::with_session(
            picker.clone(),
            decoded_sources.clone(),
            kitty_session.clone(),
        );
        let comparison_inspection =
            Inspection::with_session(picker.clone(), decoded_sources, kitty_session.clone());
        let order = if input_paths.is_some() {
            EntryOrder::Arrival
        } else {
            EntryOrder::Path
        };
        Self {
            scanner: match input_paths {
                Some(paths) => scan::start_paths(root.clone(), paths),
                None => scan::start(root.clone()),
            },
            app: App::with_order(root, order),
            thumbnails: Thumbnails::with_session(picker, kitty_session.clone()),
            kitty_session,
            inspection,
            comparison_inspection,
            view: BrowserView::Grid,
            inspection_size: Default::default(),
            inspection_key: None,
            comparison_size: Default::default(),
            comparison_key: None,
            protocol_name,
            grid: GridLayout {
                area: Default::default(),
                columns: 1,
                rows: 1,
                cell_width: 0,
                cell_height: 0,
                image_size: Default::default(),
            },
            viewport_ids: Vec::new(),
            viewport_size: Default::default(),
            viewport_quality: settings.thumbnail_quality,
            viewport_generation: 0,
            thumbnail_size: settings.thumbnail_size,
            thumbnail_quality: settings.thumbnail_quality,
            bindings: settings.bindings,
            show_help: false,
            debug_status: settings.debug_status,
            background: settings.background,
            running: true,
            selection_mode,
            confirmed: false,
            repaint_nudges: Vec::new(),
        }
    }

    fn run(&mut self, terminal: &mut AppTerminal) -> Result<(), BoxError> {
        let mut dirty = true;
        while self.running {
            let frame_started = Instant::now();
            dirty |= self.drain_background_work();
            dirty |= self.kitty_session.drain_cleanup(terminal.backend_mut())? > 0;
            self.write_due_repaint_nudges(terminal.backend_mut(), frame_started)?;
            if dirty {
                self.render_frame(terminal)?;
                dirty = self.schedule_work();
            }

            let now = Instant::now();
            let wait = self
                .poll_wait(now)
                .saturating_sub(now.duration_since(frame_started));
            if event::poll(wait)? {
                dirty |= self.handle_event(event::read()?);
            }
        }
        Ok(())
    }

    /// Time to wait for input: the frame or idle interval, shortened so a due
    /// repaint nudge is not delayed past its schedule.
    fn poll_wait(&self, now: Instant) -> Duration {
        let interval = poll_interval(self.has_background_work());
        self.repaint_nudges
            .iter()
            .map(|due| due.saturating_duration_since(now))
            .min()
            .map_or(interval, |until_nudge| interval.min(until_nudge))
    }

    /// Writes one repaint nudge when at least one scheduled nudge is due and
    /// drops every due entry. Returns how many entries were due.
    fn write_due_repaint_nudges(
        &mut self,
        writer: &mut dyn io::Write,
        now: Instant,
    ) -> io::Result<usize> {
        let due = self.repaint_nudges.iter().filter(|at| **at <= now).count();
        if due == 0 {
            return Ok(0);
        }
        self.repaint_nudges.retain(|at| *at > now);
        KittySession::write_repaint_nudge(writer)?;
        Ok(due)
    }

    /// Draws one frame and, when it transmitted Kitty image data, schedules
    /// the repaint nudges so the new placements are painted without input.
    fn render_frame(&mut self, terminal: &mut AppTerminal) -> Result<(), BoxError> {
        self.render_frame_at(terminal, Instant::now())
    }

    fn render_frame_at(
        &mut self,
        terminal: &mut AppTerminal,
        now: Instant,
    ) -> Result<(), BoxError> {
        self.prepare_render_visibility();
        terminal.draw(|frame| match self.view {
            BrowserView::Grid => {
                self.grid = ui::render_grid(
                    frame,
                    &mut self.app,
                    &mut self.thumbnails,
                    RenderOptions {
                        protocol_name: &self.protocol_name,
                        thumbnail_size: self.thumbnail_size,
                        thumbnail_quality: self.thumbnail_quality,
                        bindings: &self.bindings,
                        show_help: self.show_help,
                        debug_status: self.debug_status,
                        selection_mode: self.selection_mode,
                    },
                );
            }
            BrowserView::Inspect(state) => {
                let display = self
                    .inspection_key
                    .map(|key| self.inspection.display(key))
                    .unwrap_or(InspectionDisplay::Loading);
                self.inspection_size = ui::render_inspection(
                    frame,
                    &self.app,
                    InspectionOptions {
                        display,
                        state,
                        background: self.background,
                        bindings: &self.bindings,
                        show_help: self.show_help,
                        debug_status: self.debug_status,
                        protocol_name: &self.protocol_name,
                        selection_mode: self.selection_mode,
                    },
                );
            }
            BrowserView::Compare(state) => {
                let reference_display = self
                    .inspection_key
                    .map(|key| self.inspection.display(key))
                    .unwrap_or(InspectionDisplay::Loading);
                let candidate_display = self
                    .comparison_key
                    .map(|key| self.comparison_inspection.display(key))
                    .unwrap_or(InspectionDisplay::Loading);
                let sizes = ui::render_comparison(
                    frame,
                    &self.app,
                    ComparisonOptions {
                        state,
                        reference_display,
                        candidate_display,
                        background: self.background,
                        bindings: &self.bindings,
                        show_help: self.show_help,
                        debug_status: self.debug_status,
                        protocol_name: &self.protocol_name,
                        selection_mode: self.selection_mode,
                    },
                );
                self.inspection_size = sizes[0];
                self.comparison_size = sizes[1];
            }
        })?;
        if self.kitty_session.take_transmissions() > 0 {
            self.repaint_nudges
                .extend(REPAINT_NUDGE_DELAYS.iter().map(|delay| now + *delay));
        }
        Ok(())
    }

    fn finish(self) -> BrowseOutcome {
        if self.confirmed {
            BrowseOutcome::Selection(self.app.marked_paths())
        } else if self.selection_mode {
            BrowseOutcome::Cancelled
        } else {
            BrowseOutcome::Completed
        }
    }

    fn has_background_work(&self) -> bool {
        !self.app.scan_done
            || self.thumbnails.pending_count() > 0
            || self.inspection.is_pending()
            || self.comparison_inspection.is_pending()
            || self.kitty_session.has_pending_cleanup()
    }

    fn drain_background_work(&mut self) -> bool {
        let mut changed = false;
        if !self.app.scan_done {
            for _ in 0..2048 {
                match self.scanner.try_recv() {
                    Ok(ScanEvent::Found { path, revision }) => {
                        self.app.add_path(path, revision);
                        changed = true;
                    }
                    Ok(ScanEvent::Error) => {
                        self.app.scan_errors += 1;
                        changed = true;
                    }
                    Ok(ScanEvent::Done) => {
                        self.app.scan_done = true;
                        changed = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.app.scan_done = true;
                        changed = true;
                        break;
                    }
                }
            }
            changed |= self.app.integrate_pending();
        }
        let (completed, errors) = self.thumbnails.drain();
        self.app.thumbnail_errors += errors;
        let inspection_changed = self.inspection.drain();
        let comparison_changed = self.comparison_inspection.drain();
        changed || completed > 0 || inspection_changed || comparison_changed
    }

    fn schedule_work(&mut self) -> bool {
        match self.view {
            BrowserView::Grid => self.schedule_viewport(),
            BrowserView::Inspect(state) => self.schedule_inspection(state),
            BrowserView::Compare(state) => self.schedule_comparison(state),
        }
    }

    fn prepare_render_visibility(&mut self) {
        match self.view {
            BrowserView::Grid => {
                self.inspection.update_visible(None);
                self.comparison_inspection.update_visible(None);
            }
            BrowserView::Inspect(_) => {
                self.thumbnails.update_visible([]);
                self.inspection.update_visible(self.inspection_key);
                self.comparison_inspection.update_visible(None);
            }
            BrowserView::Compare(_) => {
                self.thumbnails.update_visible([]);
                self.inspection.update_visible(self.inspection_key);
                self.comparison_inspection
                    .update_visible(self.comparison_key);
            }
        }
    }

    fn schedule_viewport(&mut self) -> bool {
        let mut scheduled = false;
        let ids = self
            .app
            .viewport_entry_ids(self.grid.columns, self.grid.rows, PREFETCH_ROWS)
            .collect::<Vec<_>>();
        if ids != self.viewport_ids
            || self.grid.image_size != self.viewport_size
            || self.thumbnail_quality != self.viewport_quality
        {
            self.viewport_generation = self.thumbnails.begin_generation();
            self.viewport_ids.clone_from(&ids);
            self.viewport_size = self.grid.image_size;
            self.viewport_quality = self.thumbnail_quality;
        }
        for id in ids {
            if let Some(entry) = self.app.entry(id) {
                scheduled |= self.thumbnails.request(
                    ThumbnailKey {
                        entry_id: entry.id,
                        revision: entry.revision,
                        size: self.grid.image_size,
                        quality: self.thumbnail_quality,
                    },
                    entry.path.clone(),
                    self.viewport_generation,
                );
            }
        }
        scheduled
    }

    fn schedule_inspection(&mut self, state: ViewState) -> bool {
        let Some(entry) = self.app.selected_entry() else {
            self.inspection_key = None;
            return false;
        };
        let key = InspectionKey {
            entry_id: entry.id,
            revision: entry.revision,
            size: self.inspection_size,
            zoom: state.zoom,
            center_x: state.center_x,
            center_y: state.center_y,
            background: self.background,
            quality: self.thumbnail_quality,
        };
        self.inspection_key = Some(key);
        self.inspection.request(key, entry.path.clone())
    }

    fn schedule_comparison(&mut self, state: CompareState) -> bool {
        let Some(candidate) = self.app.selected_entry() else {
            self.view = BrowserView::Grid;
            return false;
        };
        if candidate.id == state.reference_id
            || self.app.filtered_position(state.reference_id).is_none()
        {
            self.view = BrowserView::Grid;
            return false;
        }
        let candidate_id = candidate.id;
        let candidate_revision = candidate.revision;
        let candidate_path = candidate.path.clone();
        let Some(reference) = self.app.entry(state.reference_id) else {
            self.view = BrowserView::Grid;
            return false;
        };
        let reference_key = InspectionKey {
            entry_id: reference.id,
            revision: reference.revision,
            size: self.inspection_size,
            zoom: state.reference_view.zoom,
            center_x: state.reference_view.center_x,
            center_y: state.reference_view.center_y,
            background: self.background,
            quality: self.thumbnail_quality,
        };
        let reference_path = reference.path.clone();
        let candidate_key = InspectionKey {
            entry_id: candidate_id,
            revision: candidate_revision,
            size: self.comparison_size,
            zoom: state.candidate_view.zoom,
            center_x: state.candidate_view.center_x,
            center_y: state.candidate_view.center_y,
            background: self.background,
            quality: self.thumbnail_quality,
        };
        self.inspection_key = Some(reference_key);
        self.comparison_key = Some(candidate_key);
        self.inspection.request(reference_key, reference_path)
            | self
                .comparison_inspection
                .request(candidate_key, candidate_path)
    }

    fn handle_event(&mut self, event: Event) -> bool {
        let Event::Key(key) = event else {
            return matches!(event, Event::Resize(_, _));
        };
        if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.running = false;
            return false;
        }

        let context = match self.view {
            BrowserView::Inspect(_) => ActionContext::Inspect,
            BrowserView::Compare(_) => ActionContext::Compare,
            BrowserView::Grid => match self.app.mode {
                InputMode::Normal => ActionContext::Normal,
                InputMode::Search => ActionContext::Search,
            },
        };
        let action = self.bindings.action(context, key);
        if self.show_help {
            match action {
                Some(Action::ToggleHelp)
                | Some(Action::CancelSearch)
                | Some(Action::CloseInspect) => self.show_help = false,
                Some(Action::Quit) => self.running = false,
                _ if key.code == KeyCode::Esc => self.show_help = false,
                _ => {}
            }
            return self.running;
        }

        match self.view {
            BrowserView::Inspect(_) => self.handle_inspect_action(action),
            BrowserView::Compare(_) => self.handle_compare_action(action),
            BrowserView::Grid => match self.app.mode {
                InputMode::Normal => self.handle_normal_action(action),
                InputMode::Search => self.handle_search_key(action, key),
            },
        }
        self.running
    }

    fn handle_normal_action(&mut self, action: Option<Action>) {
        match action {
            Some(Action::Quit) => self.running = false,
            Some(Action::StartSearch) => self.app.start_search(),
            Some(Action::ThumbnailLarger) => {
                self.thumbnail_size = self.thumbnail_size.larger_for_grid(
                    self.grid.area.width,
                    self.grid.area.height,
                    self.grid.columns,
                    self.grid.rows,
                );
            }
            Some(Action::ThumbnailSmaller) => {
                self.thumbnail_size = self.thumbnail_size.smaller_for_grid(
                    self.grid.area.width,
                    self.grid.area.height,
                    self.grid.columns,
                    self.grid.rows,
                );
            }
            Some(Action::ThumbnailReset) => {
                self.thumbnail_size = DEFAULT_THUMBNAIL_SIZE;
            }
            Some(Action::SetQuality(level)) => {
                self.thumbnail_quality = ThumbnailQuality::from_digit(
                    char::from_digit(u32::from(level), 10)
                        .expect("configured quality actions use levels 1 through 9"),
                )
                .expect("configured quality actions use levels 1 through 9");
            }
            Some(Action::MoveLeft) => self.app.move_left(self.grid.columns, self.grid.rows),
            Some(Action::MoveRight) => self.app.move_right(self.grid.columns, self.grid.rows),
            Some(Action::MoveUp) => self.app.move_up(self.grid.columns, self.grid.rows),
            Some(Action::MoveDown) => self.app.move_down(self.grid.columns, self.grid.rows),
            Some(Action::PageUp) => self.app.page_up(self.grid.columns, self.grid.rows),
            Some(Action::PageDown) => self.app.page_down(self.grid.columns, self.grid.rows),
            Some(Action::First) => self.app.select_first(),
            Some(Action::Last) => self.app.select_last(self.grid.columns, self.grid.rows),
            Some(Action::ToggleHelp) => self.show_help = true,
            Some(Action::ToggleDebug) => self.debug_status = !self.debug_status,
            Some(Action::ToggleMark) => self.app.toggle_mark(),
            Some(Action::MarkRange) => self.app.mark_range(),
            Some(Action::ClearMarks) => self.app.clear_marks(),
            Some(Action::ToggleMarkedOnly) => self.app.toggle_marked_only(),
            Some(Action::ConfirmSelection) if self.selection_mode => {
                self.confirmed = true;
                self.running = false;
            }
            Some(Action::OpenInspect) if self.app.selected_entry().is_some() => {
                self.view = BrowserView::Inspect(ViewState::default());
                self.inspection_key = None;
            }
            Some(Action::OpenCompare) => self.start_comparison(),
            _ => {}
        }
    }

    fn start_comparison(&mut self) {
        if self.app.filtered.len() < 2 {
            return;
        }
        let reference_id = self.app.filtered[self.app.selected];
        self.app.selected = if self.app.selected + 1 < self.app.filtered.len() {
            self.app.selected + 1
        } else {
            self.app.selected - 1
        };
        self.view = BrowserView::Compare(CompareState {
            reference_id,
            reference_view: ViewState::default(),
            candidate_view: ViewState::default(),
            active: ComparePane::Reference,
            synchronized: true,
        });
        self.inspection_key = None;
        self.comparison_key = None;
    }

    fn handle_inspect_action(&mut self, action: Option<Action>) {
        match action {
            Some(Action::Quit) => self.running = false,
            Some(Action::CloseInspect) => self.view = BrowserView::Grid,
            Some(Action::PreviousImage) => {
                self.app.move_left(self.grid.columns, self.grid.rows);
                if let BrowserView::Inspect(state) = &mut self.view {
                    state.recenter();
                }
            }
            Some(Action::NextImage) => {
                self.app.move_right(self.grid.columns, self.grid.rows);
                if let BrowserView::Inspect(state) = &mut self.view {
                    state.recenter();
                }
            }
            Some(Action::ToggleFitHundred) => {
                if let BrowserView::Inspect(state) = &mut self.view {
                    state.zoom = state.zoom.toggle_fit_hundred();
                }
            }
            Some(Action::ZoomIn) => {
                if let BrowserView::Inspect(state) = &mut self.view {
                    state.zoom = state.zoom.zoom_in();
                }
            }
            Some(Action::ZoomOut) => {
                if let BrowserView::Inspect(state) = &mut self.view {
                    state.zoom = state.zoom.zoom_out();
                }
            }
            Some(Action::PanLeft) => {
                if let BrowserView::Inspect(state) = &mut self.view {
                    state.pan_left();
                }
            }
            Some(Action::PanRight) => {
                if let BrowserView::Inspect(state) = &mut self.view {
                    state.pan_right();
                }
            }
            Some(Action::PanUp) => {
                if let BrowserView::Inspect(state) = &mut self.view {
                    state.pan_up();
                }
            }
            Some(Action::PanDown) => {
                if let BrowserView::Inspect(state) = &mut self.view {
                    state.pan_down();
                }
            }
            Some(Action::CycleBackground) => self.background = self.background.next(),
            Some(Action::ToggleHelp) => self.show_help = true,
            Some(Action::ToggleDebug) => self.debug_status = !self.debug_status,
            Some(Action::ToggleMark) => self.app.toggle_mark(),
            Some(Action::MarkRange) => self.app.mark_range(),
            Some(Action::ClearMarks) => self.app.clear_marks(),
            Some(Action::ToggleMarkedOnly) => self.app.toggle_marked_only(),
            Some(Action::ConfirmSelection) if self.selection_mode => {
                self.confirmed = true;
                self.running = false;
            }
            _ => {}
        }
    }

    fn handle_compare_action(&mut self, action: Option<Action>) {
        match action {
            Some(Action::Quit) => self.running = false,
            Some(Action::CloseInspect) => self.view = BrowserView::Grid,
            Some(Action::PreviousImage) => self.move_compare_candidate(false),
            Some(Action::NextImage) => self.move_compare_candidate(true),
            Some(Action::PromoteCandidate) => self.promote_candidate(),
            Some(Action::SwitchComparePane) => {
                if let BrowserView::Compare(state) = &mut self.view {
                    state.active = match state.active {
                        ComparePane::Reference => ComparePane::Candidate,
                        ComparePane::Candidate => ComparePane::Reference,
                    };
                }
            }
            Some(Action::ToggleCompareSync) => {
                if let BrowserView::Compare(state) = &mut self.view {
                    state.toggle_sync();
                }
            }
            Some(Action::ToggleFitHundred) => self.mutate_compare_view(|view| {
                view.zoom = view.zoom.toggle_fit_hundred();
            }),
            Some(Action::ZoomIn) => self.mutate_compare_view(|view| {
                view.zoom = view.zoom.zoom_in();
            }),
            Some(Action::ZoomOut) => self.mutate_compare_view(|view| {
                view.zoom = view.zoom.zoom_out();
            }),
            Some(Action::PanLeft) => self.mutate_compare_view(ViewState::pan_left),
            Some(Action::PanRight) => self.mutate_compare_view(ViewState::pan_right),
            Some(Action::PanUp) => self.mutate_compare_view(ViewState::pan_up),
            Some(Action::PanDown) => self.mutate_compare_view(ViewState::pan_down),
            Some(Action::CycleBackground) => self.background = self.background.next(),
            Some(Action::ToggleMark) => {
                let id = match self.view {
                    BrowserView::Compare(CompareState {
                        reference_id,
                        active: ComparePane::Reference,
                        ..
                    }) => Some(reference_id),
                    BrowserView::Compare(CompareState {
                        active: ComparePane::Candidate,
                        ..
                    }) => self.app.selected_entry().map(|entry| entry.id),
                    _ => None,
                };
                if let Some(id) = id {
                    self.app.toggle_mark_id(id);
                    self.close_invalid_comparison();
                }
            }
            Some(Action::ClearMarks) => {
                self.app.clear_marks();
                self.close_invalid_comparison();
            }
            Some(Action::ToggleMarkedOnly) => {
                self.app.toggle_marked_only();
                self.close_invalid_comparison();
            }
            Some(Action::ToggleHelp) => self.show_help = true,
            Some(Action::ToggleDebug) => self.debug_status = !self.debug_status,
            Some(Action::ConfirmSelection) if self.selection_mode => {
                self.confirmed = true;
                self.running = false;
            }
            _ => {}
        }
    }

    fn mutate_compare_view(&mut self, mutation: impl FnOnce(&mut ViewState)) {
        if let BrowserView::Compare(state) = &mut self.view {
            state.mutate_view(mutation);
        }
    }

    fn move_compare_candidate(&mut self, forward: bool) {
        let BrowserView::Compare(state) = &mut self.view else {
            return;
        };
        let Some(position) = comparison_candidate_position(
            &self.app.filtered,
            self.app.selected,
            state.reference_id,
            forward,
        ) else {
            return;
        };
        self.app.selected = position;
        state.recenter_candidate();
        self.comparison_key = None;
    }

    fn promote_candidate(&mut self) {
        let BrowserView::Compare(state) = &mut self.view else {
            return;
        };
        let Some(candidate_id) = self.app.filtered.get(self.app.selected).copied() else {
            self.view = BrowserView::Grid;
            return;
        };
        state.reference_id = candidate_id;
        let Some(position) = comparison_candidate_position(
            &self.app.filtered,
            self.app.selected,
            state.reference_id,
            true,
        ) else {
            self.view = BrowserView::Grid;
            return;
        };
        self.app.selected = position;
        state.recenter_candidate();
        self.inspection_key = None;
        self.comparison_key = None;
    }

    fn close_invalid_comparison(&mut self) {
        let BrowserView::Compare(state) = self.view else {
            return;
        };
        let candidate = self.app.filtered.get(self.app.selected).copied();
        if self.app.filtered_position(state.reference_id).is_none()
            || candidate.is_none_or(|id| id == state.reference_id)
        {
            self.view = BrowserView::Grid;
        }
    }

    fn handle_search_key(&mut self, action: Option<Action>, key: KeyEvent) {
        match action {
            Some(Action::CancelSearch) => self.app.cancel_search(),
            Some(Action::CommitSearch) => self.app.commit_search(),
            Some(Action::DeleteSearchCharacter) => self.app.pop_search(),
            None => {
                if let KeyCode::Char(character) = key.code
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    self.app.push_search(character);
                }
            }
            _ => {}
        }
    }
}

const fn poll_interval(has_background_work: bool) -> Duration {
    if has_background_work {
        FRAME_INTERVAL
    } else {
        IDLE_INTERVAL
    }
}

fn comparison_candidate_position(
    filtered: &[EntryId],
    current: usize,
    reference_id: EntryId,
    forward: bool,
) -> Option<usize> {
    if filtered.len() < 2 {
        return None;
    }
    for offset in 1..=filtered.len() {
        let position = if forward {
            (current + offset) % filtered.len()
        } else {
            (current + filtered.len() - offset % filtered.len()) % filtered.len()
        };
        if filtered[position] != reference_id {
            return Some(position);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::*;
    use crate::{
        app::SourceRevision,
        config::{CliOverrides, ConfigSource},
        inspection::Zoom,
    };

    fn browser_with_images() -> (Browser, PathBuf) {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("red-table-browser-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let settings =
            RuntimeSettings::load(&ConfigSource::Disabled, &CliOverrides::default()).unwrap();
        let mut browser = Browser::new(
            root.clone(),
            Picker::halfblocks(),
            "Half 1x2/forced".into(),
            settings,
            false,
            None,
            KittySession::with_tmux(false),
        );
        for name in ["1-one.png", "2-two.png", "3-three.png"] {
            browser
                .app
                .add_path(root.join(name), SourceRevision::default());
        }
        browser.app.integrate_pending();
        browser.grid.columns = 2;
        browser.grid.rows = 1;
        (browser, root)
    }

    #[test]
    fn inspection_round_trip_preserves_grid_focus_and_scroll() {
        let (mut browser, root) = browser_with_images();
        browser.app.selected = 2;
        browser.app.row_offset = 1;
        browser.handle_normal_action(Some(Action::OpenInspect));
        assert!(matches!(browser.view, BrowserView::Inspect(_)));
        browser.handle_inspect_action(Some(Action::CloseInspect));
        assert!(matches!(browser.view, BrowserView::Grid));
        assert_eq!((browser.app.selected, browser.app.row_offset), (2, 1));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repeated_escape_after_inspection_returns_to_a_running_grid() {
        let (mut browser, root) = browser_with_images();
        let key = |code| {
            Event::Key(KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            })
        };

        assert!(browser.handle_event(key(KeyCode::Enter)));
        assert!(matches!(browser.view, BrowserView::Inspect(_)));
        assert!(browser.handle_event(key(KeyCode::Esc)));
        assert!(matches!(browser.view, BrowserView::Grid));
        assert!(browser.handle_event(key(KeyCode::Esc)));
        assert!(matches!(browser.view, BrowserView::Grid));
        assert!(browser.running);

        assert!(!browser.handle_event(key(KeyCode::Char('q'))));
        assert!(!browser.running);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inspection_navigation_retains_zoom_and_recenters() {
        let (mut browser, root) = browser_with_images();
        browser.app.selected = 1;
        browser.view = BrowserView::Inspect(ViewState {
            zoom: Zoom::Percent(200),
            center_x: 9_000,
            center_y: 1_000,
        });
        browser.handle_inspect_action(Some(Action::NextImage));
        assert_eq!(browser.app.selected, 2);
        let BrowserView::Inspect(state) = browser.view else {
            panic!("inspection must stay active");
        };
        assert_eq!(state.zoom, Zoom::Percent(200));
        assert_eq!((state.center_x, state.center_y), (5_000, 5_000));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn marking_in_inspection_is_visible_to_the_grid() {
        let (mut browser, root) = browser_with_images();
        browser.app.selected = 1;
        browser.view = BrowserView::Inspect(ViewState::default());
        browser.handle_inspect_action(Some(Action::ToggleMark));
        assert!(browser.app.is_marked(EntryId(1)));
        browser.handle_inspect_action(Some(Action::CloseInspect));
        assert!(matches!(browser.view, BrowserView::Grid));
        assert!(browser.app.is_marked(EntryId(1)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selection_confirmation_and_cancellation_are_distinct_outcomes() {
        let (mut confirmed, confirmed_root) = browser_with_images();
        confirmed.selection_mode = true;
        confirmed.handle_normal_action(Some(Action::ToggleMark));
        confirmed.handle_normal_action(Some(Action::ConfirmSelection));
        assert!(confirmed.confirmed);
        assert!(!confirmed.running);
        let BrowseOutcome::Selection(paths) = confirmed.finish() else {
            panic!("confirmation must produce a selection");
        };
        assert_eq!(paths, vec![confirmed_root.join("1-one.png")]);

        let (mut cancelled, cancelled_root) = browser_with_images();
        cancelled.selection_mode = true;
        cancelled.handle_normal_action(Some(Action::Quit));
        assert!(matches!(cancelled.finish(), BrowseOutcome::Cancelled));
        fs::remove_dir_all(confirmed_root).unwrap();
        fs::remove_dir_all(cancelled_root).unwrap();
    }

    #[test]
    fn comparison_pins_reference_and_skips_it_during_candidate_navigation() {
        let (mut browser, root) = browser_with_images();
        browser.app.selected = 0;
        browser.start_comparison();
        let BrowserView::Compare(state) = browser.view else {
            panic!("comparison must open with two or more images");
        };
        assert_eq!(state.reference_id, EntryId(0));
        assert_eq!(browser.app.selected_entry().unwrap().id, EntryId(1));

        browser.move_compare_candidate(true);
        assert_eq!(browser.app.selected_entry().unwrap().id, EntryId(2));
        browser.move_compare_candidate(true);
        assert_eq!(browser.app.selected_entry().unwrap().id, EntryId(1));
        browser.move_compare_candidate(false);
        assert_eq!(browser.app.selected_entry().unwrap().id, EntryId(2));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn comparison_from_end_falls_back_and_promotion_advances() {
        let (mut browser, root) = browser_with_images();
        browser.app.selected = 2;
        browser.start_comparison();
        let BrowserView::Compare(state) = browser.view else {
            panic!("comparison must open");
        };
        assert_eq!(state.reference_id, EntryId(2));
        assert_eq!(browser.app.selected_entry().unwrap().id, EntryId(1));

        browser.promote_candidate();
        let BrowserView::Compare(state) = browser.view else {
            panic!("comparison must remain open after promotion");
        };
        assert_eq!(state.reference_id, EntryId(1));
        assert_eq!(browser.app.selected_entry().unwrap().id, EntryId(2));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn two_image_promotion_swaps_reference_and_candidate() {
        let (mut browser, root) = browser_with_images();
        browser.app.filtered.truncate(2);
        browser.start_comparison();
        browser.promote_candidate();
        let BrowserView::Compare(state) = browser.view else {
            panic!("two-image comparison must remain open");
        };
        assert_eq!(state.reference_id, EntryId(1));
        assert_eq!(browser.app.selected_entry().unwrap().id, EntryId(0));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn comparison_sync_and_active_pane_control_geometry_and_marks() {
        let (mut browser, root) = browser_with_images();
        browser.start_comparison();
        browser.handle_compare_action(Some(Action::ZoomIn));
        let BrowserView::Compare(state) = browser.view else {
            panic!("comparison must stay open");
        };
        assert_eq!(state.reference_view.zoom, Zoom::Percent(100));
        assert_eq!(state.candidate_view.zoom, Zoom::Percent(100));

        browser.handle_compare_action(Some(Action::ToggleMark));
        assert!(browser.app.is_marked(EntryId(0)));
        browser.handle_compare_action(Some(Action::ToggleCompareSync));
        browser.handle_compare_action(Some(Action::SwitchComparePane));
        browser.handle_compare_action(Some(Action::ZoomIn));
        browser.handle_compare_action(Some(Action::ToggleMark));
        let BrowserView::Compare(state) = browser.view else {
            panic!("comparison must stay open");
        };
        assert!(!state.synchronized);
        assert_eq!(state.reference_view.zoom, Zoom::Percent(100));
        assert_eq!(state.candidate_view.zoom, Zoom::Percent(200));
        assert!(browser.app.is_marked(EntryId(1)));

        browser.handle_compare_action(Some(Action::CloseInspect));
        assert!(matches!(browser.view, BrowserView::Grid));
        assert_eq!(browser.app.selected_entry().unwrap().id, EntryId(1));
        assert_eq!(browser.app.marked_count(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn closing_the_browser_deletes_only_transmitted_kitty_images() {
        use image::DynamicImage;
        use ratatui::{Terminal, backend::TestBackend, layout::Size};

        use crate::{
            kitty::{KittyImage, KittyProtocol},
            thumbnails::{PreparedThumbnail, ThumbnailKey},
        };

        let (mut browser, root) = browser_with_images();
        let session = browser.kitty_session.clone();
        let key = |entry_id| ThumbnailKey {
            entry_id: EntryId(entry_id),
            revision: SourceRevision::default(),
            size: Size::new(1, 1),
            quality: "7".parse().unwrap(),
        };
        let transmitted = KittyProtocol::new(
            DynamicImage::new_rgba8(1, 1),
            Size::new(1, 1),
            session.clone(),
        )
        .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(KittyImage::new(&transmitted), frame.area()))
            .unwrap();
        let transmitted_id = transmitted.id();
        browser
            .thumbnails
            .insert(key(0), PreparedThumbnail::Kitty(transmitted));
        let never_sent = KittyProtocol::new(
            DynamicImage::new_rgba8(1, 1),
            Size::new(1, 1),
            session.clone(),
        )
        .unwrap();
        let never_sent_id = never_sent.id();
        browser
            .thumbnails
            .insert(key(1), PreparedThumbnail::Kitty(never_sent));

        let mut output = Vec::new();
        assert!(matches!(
            close(browser, &session, &mut output).unwrap(),
            BrowseOutcome::Completed
        ));
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.matches("a=d,d=I,i=").count(), 1);
        assert!(output.contains(&format!("a=d,d=I,i={transmitted_id}")));
        assert!(!output.contains(&format!("i={never_sent_id}")));
        assert!(!session.has_pending_cleanup());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn frames_that_transmit_kitty_images_schedule_delayed_repaint_nudges() {
        use std::sync::{Arc, Mutex};

        use image::DynamicImage;
        use ratatui::{
            Terminal, TerminalOptions, Viewport, backend::CrosstermBackend, layout::Rect,
        };

        use crate::{kitty::KittyProtocol, thumbnails::PreparedThumbnail, ui::GridLayout};

        #[derive(Clone)]
        struct SharedWriter(Arc<Mutex<Vec<u8>>>);
        impl io::Write for SharedWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let (mut browser, root) = browser_with_images();
        browser.app.scan_done = true;
        assert!(!browser.has_background_work());
        let output = SharedWriter(Arc::new(Mutex::new(Vec::new())));
        let backend = CrosstermBackend::new(Box::new(output.clone()) as Box<dyn io::Write + Send>);
        let area = Rect::new(0, 0, 40, 20);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .unwrap();
        let transmitted = |output: &SharedWriter| {
            String::from_utf8_lossy(&output.0.lock().unwrap()).contains("a=T,U=1")
        };
        let start = Instant::now();

        browser.render_frame_at(&mut terminal, start).unwrap();
        assert!(!transmitted(&output));
        assert!(
            browser.repaint_nudges.is_empty(),
            "no transmission, no nudge"
        );
        assert_eq!(
            browser.poll_wait(start),
            poll_interval(browser.has_background_work())
        );

        let image_size = GridLayout::new_for_test(area, DEFAULT_THUMBNAIL_SIZE).image_size;
        let entry = browser.app.selected_entry().unwrap();
        let key = ThumbnailKey {
            entry_id: entry.id,
            revision: entry.revision,
            size: image_size,
            quality: browser.thumbnail_quality,
        };
        let protocol = KittyProtocol::new(
            DynamicImage::new_rgba8(2, 2),
            image_size,
            browser.kitty_session.clone(),
        )
        .unwrap();
        browser
            .thumbnails
            .insert(key, PreparedThumbnail::Kitty(protocol));
        output.0.lock().unwrap().clear();
        browser.render_frame_at(&mut terminal, start).unwrap();
        assert!(
            transmitted(&output),
            "the frame must transmit the inserted image"
        );
        assert_eq!(
            browser.repaint_nudges,
            REPAINT_NUDGE_DELAYS.map(|delay| start + delay).to_vec()
        );
        assert_eq!(browser.poll_wait(start), REPAINT_NUDGE_DELAYS[0]);

        let mut nudges = Vec::new();
        assert_eq!(
            browser
                .write_due_repaint_nudges(&mut nudges, start + Duration::from_millis(10))
                .unwrap(),
            0
        );
        assert!(
            nudges.is_empty(),
            "nothing is written before the first delay"
        );
        assert_eq!(
            browser
                .write_due_repaint_nudges(&mut nudges, start + REPAINT_NUDGE_DELAYS[0])
                .unwrap(),
            1
        );
        assert_eq!(nudges, b"\x1b[?25l");
        assert_eq!(browser.repaint_nudges.len(), 1);
        assert_eq!(
            browser.poll_wait(start + REPAINT_NUDGE_DELAYS[0]),
            REPAINT_NUDGE_DELAYS[1] - REPAINT_NUDGE_DELAYS[0]
        );
        assert_eq!(
            browser
                .write_due_repaint_nudges(&mut nudges, start + Duration::from_secs(5))
                .unwrap(),
            1
        );
        assert!(browser.repaint_nudges.is_empty());

        output.0.lock().unwrap().clear();
        browser.render_frame_at(&mut terminal, start).unwrap();
        assert!(
            !transmitted(&output),
            "placeholder-only frames do not retransmit"
        );
        assert!(browser.repaint_nudges.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn idle_loop_polls_slowly_only_without_background_work() {
        assert_eq!(poll_interval(true), FRAME_INTERVAL);
        assert_eq!(poll_interval(false), IDLE_INTERVAL);
        assert!(IDLE_INTERVAL > FRAME_INTERVAL);

        let (mut browser, root) = browser_with_images();
        assert!(browser.has_background_work(), "the scan is still running");
        browser.app.scan_done = true;
        assert!(!browser.has_background_work());

        browser.view = BrowserView::Inspect(ViewState::default());
        browser.inspection_size = ratatui::layout::Size::new(8, 4);
        assert!(browser.schedule_inspection(ViewState::default()));
        assert!(
            browser.has_background_work(),
            "an inspection request is pending"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn comparison_requires_two_visible_images() {
        let (mut browser, root) = browser_with_images();
        browser.app.start_search();
        for character in "one".chars() {
            browser.app.push_search(character);
        }
        browser.app.commit_search();
        browser.start_comparison();
        assert!(matches!(browser.view, BrowserView::Grid));
        fs::remove_dir_all(root).unwrap();
    }
}
