use std::ops::Range;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect, Size},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use ratatui_image::Image;

use crate::{
    actions::{Action, ActionContext, Bindings},
    app::{App, InputMode},
    browser::{ComparePane, CompareState},
    inspection::{InspectionDisplay, ViewState},
    kitty::KittyImage,
    settings::{BackgroundMode, ThumbnailQuality, ThumbnailSize},
    thumbnails::{PreparedThumbnail, ThumbnailDisplay, ThumbnailKey, Thumbnails},
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct GridLayout {
    pub(crate) area: Rect,
    pub(crate) columns: usize,
    pub(crate) rows: usize,
    pub(crate) cell_width: u16,
    pub(crate) cell_height: u16,
    pub(crate) image_size: Size,
}

#[derive(Clone, Copy)]
pub(crate) struct RenderOptions<'a> {
    pub(crate) protocol_name: &'a str,
    pub(crate) thumbnail_size: ThumbnailSize,
    pub(crate) thumbnail_quality: ThumbnailQuality,
    pub(crate) bindings: &'a Bindings,
    pub(crate) show_help: bool,
    pub(crate) debug_status: bool,
    pub(crate) selection_mode: bool,
    pub(crate) notice: Option<&'a str>,
}

const FILMSTRIP_CELL_WIDTH: u16 = 14;
const FILMSTRIP_CELL_HEIGHT: u16 = 7;
/// Rows the inspected image keeps at least before the filmstrip is dropped.
const FILMSTRIP_MIN_IMAGE_ROWS: u16 = 10;

/// One row of fixed-size thumbnail cells below the inspected image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FilmstripLayout {
    pub(crate) area: Rect,
    pub(crate) columns: usize,
    pub(crate) image_size: Size,
}

impl FilmstripLayout {
    #[cfg(test)]
    pub(crate) fn new_for_test(area: Rect) -> Self {
        Self::new(area)
    }

    fn new(area: Rect) -> Self {
        Self {
            area,
            columns: (area.width / FILMSTRIP_CELL_WIDTH) as usize,
            image_size: Size::new(FILMSTRIP_CELL_WIDTH - 2, FILMSTRIP_CELL_HEIGHT - 2),
        }
    }

    /// Filtered positions shown in the strip: centred on `selected` and
    /// clamped so both ends of the ordering fill the strip.
    pub(crate) fn window(self, selected: usize, len: usize) -> Range<usize> {
        let count = self.columns.min(len);
        let start = selected.saturating_sub(count / 2).min(len - count);
        start..start + count
    }

    fn cell(self, index: usize, count: usize) -> Rect {
        let used = count as u16 * FILMSTRIP_CELL_WIDTH;
        Rect::new(
            self.area.x + (self.area.width - used) / 2 + index as u16 * FILMSTRIP_CELL_WIDTH,
            self.area.y,
            FILMSTRIP_CELL_WIDTH,
            FILMSTRIP_CELL_HEIGHT,
        )
    }
}

/// Inspection areas whose sizes drive background scheduling.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InspectionLayout {
    pub(crate) image_size: Size,
    pub(crate) filmstrip: Option<FilmstripLayout>,
}

impl GridLayout {
    /// Layout the grid renderer would compute for `area`, exposed so frame
    /// tests can derive the exact thumbnail key.
    #[cfg(test)]
    pub(crate) fn new_for_test(area: Rect, thumbnail_size: ThumbnailSize) -> Self {
        let chrome = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(area);
        Self::new(chrome[1], thumbnail_size)
    }

    fn new(area: Rect, thumbnail_size: ThumbnailSize) -> Self {
        let columns = (area.width / thumbnail_size.width).max(1) as usize;
        let rows = (area.height / thumbnail_size.height).max(1) as usize;
        let cell_width = area.width / columns as u16;
        let cell_height = area.height / rows as u16;
        let image_size = Size::new(cell_width.saturating_sub(2), cell_height.saturating_sub(3));
        Self {
            area,
            columns,
            rows,
            cell_width,
            cell_height,
            image_size,
        }
    }

    fn cell(self, column: usize, row: usize) -> Rect {
        Rect::new(
            self.area.x + column as u16 * self.cell_width,
            self.area.y + row as u16 * self.cell_height,
            self.cell_width,
            self.cell_height,
        )
    }
}

pub(crate) fn render_grid(
    frame: &mut Frame<'_>,
    app: &mut App,
    thumbnails: &mut Thumbnails,
    options: RenderOptions<'_>,
) -> GridLayout {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(frame.area());
    let grid = GridLayout::new(areas[1], options.thumbnail_size);
    app.ensure_visible(grid.columns, grid.rows);
    let visible_keys = app
        .viewport_entry_ids(grid.columns, grid.rows, 0)
        .filter_map(|id| {
            app.entry(id).map(|entry| ThumbnailKey {
                entry_id: entry.id,
                revision: entry.revision,
                size: grid.image_size,
                quality: options.thumbnail_quality,
            })
        })
        .collect::<Vec<_>>();
    thumbnails.update_visible(visible_keys);

    render_header(
        frame,
        app,
        thumbnails,
        options.protocol_name,
        areas[0],
        options,
    );
    let context = match app.mode {
        InputMode::Normal => ActionContext::Normal,
        InputMode::Search => ActionContext::Search,
    };
    let overlay = options
        .show_help
        .then(|| help_area(frame.area(), options.bindings, context));
    render_grid_cards(
        frame,
        app,
        thumbnails,
        grid,
        options.thumbnail_quality,
        overlay,
    );
    render_footer(frame, app, options, areas[2]);
    if options.show_help {
        render_help(frame, options.bindings, context);
    }
    grid
}

fn render_header(
    frame: &mut Frame<'_>,
    app: &App,
    thumbnails: &Thumbnails,
    protocol_name: &str,
    area: Rect,
    options: RenderOptions<'_>,
) {
    let state = if app.scan_done { "ready" } else { "scanning" };
    let selected = if app.filtered.is_empty() {
        0
    } else {
        app.selected + 1
    };
    let title = Line::from(vec![
        Span::styled(
            " red-table ",
            Style::default().fg(Color::Black).bg(Color::Red),
        ),
        Span::raw("  "),
        Span::styled(
            app.root.to_string_lossy(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]);
    let pending = thumbnails.pending_count();
    let (disk_hits, disk_misses) = thumbnails.persistent_stats();
    let (memory_entries, memory_bytes) = thumbnails.cache_stats();
    let errors = app.scan_errors + app.thumbnail_errors;
    let status = if options.debug_status {
        Line::from(format!(
            " {selected}/{} · {} marked · {} found · {state} · {} · {pending} load · {} · Q{} · {protocol_name} · M{memory_entries}/{}MiB · D{disk_hits}/{disk_misses} · {errors} err",
            app.filtered.len(),
            app.marked_count(),
            app.entries.len(),
            app.order_label(),
            options.thumbnail_size,
            options.thumbnail_quality,
            memory_bytes.div_ceil(1024 * 1024),
        ))
    } else {
        let loading = if pending > 0 {
            format!(" · {pending} loading")
        } else {
            String::new()
        };
        let failures = if errors > 0 {
            format!(" · {errors} errors")
        } else {
            String::new()
        };
        Line::from(format!(
            " {selected}/{} · {} marked{} · {} found · {state} · {}{loading}{failures}",
            app.filtered.len(),
            app.marked_count(),
            if app.marked_only() { " (only)" } else { "" },
            app.entries.len(),
            app.order_label(),
        ))
    };
    frame.render_widget(Paragraph::new(vec![title, status]), area);
}

fn render_grid_cards(
    frame: &mut Frame<'_>,
    app: &App,
    thumbnails: &mut Thumbnails,
    grid: GridLayout,
    thumbnail_quality: ThumbnailQuality,
    overlay: Option<Rect>,
) {
    if app.filtered.is_empty() {
        let message = if app.scan_done {
            if app.query.is_empty() {
                "No supported images found"
            } else {
                "No images match the current search"
            }
        } else {
            "Scanning for images…"
        };
        frame.render_widget(
            Paragraph::new(message)
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray)),
            grid.area,
        );
        return;
    }

    let start = app.row_offset.saturating_mul(grid.columns);
    let end = (start + grid.columns * grid.rows).min(app.filtered.len());
    for (visible_index, filtered_position) in (start..end).enumerate() {
        let column = visible_index % grid.columns;
        let row = visible_index / grid.columns;
        let outer = grid.cell(column, row);
        let Some(entry) = app
            .filtered
            .get(filtered_position)
            .and_then(|id| app.entry(*id))
        else {
            continue;
        };
        let selected = filtered_position == app.selected;
        let marked = app.is_marked(entry.id);
        let block = grid_card_block(selected, marked);
        let inner = block.inner(outer);
        frame.render_widget(block, outer);

        let image_area = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(1),
        );
        let label_area = Rect::new(
            inner.x,
            inner.y + inner.height.saturating_sub(1),
            inner.width,
            inner.height.min(1),
        );
        let key = ThumbnailKey {
            entry_id: entry.id,
            revision: entry.revision,
            size: grid.image_size,
            quality: thumbnail_quality,
        };
        match thumbnails.display(key) {
            ThumbnailDisplay::Ready(thumbnail) => {
                render_prepared(frame, thumbnail, image_area, overlay);
            }
            ThumbnailDisplay::Loading => frame.render_widget(
                Paragraph::new("loading…")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                image_area,
            ),
            ThumbnailDisplay::Error(error) => frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        "Cannot load image",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(error),
                ])
                .alignment(Alignment::Center),
                image_area,
            ),
        }
        let label = grid_label(app, entry);
        frame.render_widget(
            Paragraph::new(label).style(if selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            }),
            label_area,
        );
    }
}

fn grid_card_block(focused: bool, marked: bool) -> Block<'static> {
    let border_style = if focused {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else if marked {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let border_type = if marked {
        BorderType::Double
    } else {
        BorderType::Plain
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(border_style)
}

fn grid_label(app: &App, entry: &crate::app::ImageEntry) -> String {
    if app.is_marked(entry.id) {
        format!("[x] {}", entry.label)
    } else {
        format!("[ ] {}", entry.label)
    }
}

/// The status line below a view: a pending notice, otherwise the key hints.
fn status_line(notice: Option<&str>, help: String) -> Line<'static> {
    match notice {
        Some(notice) => Line::from(Span::styled(
            format!(" {notice}"),
            Style::default().fg(Color::Yellow),
        )),
        None => Line::from(Span::styled(help, Style::default().fg(Color::DarkGray))),
    }
}

fn render_footer(frame: &mut Frame<'_>, app: &App, options: RenderOptions<'_>, area: Rect) {
    let (bindings, selection_mode) = (options.bindings, options.selection_mode);
    let selected_path = app
        .selected_entry()
        .map(|entry| entry.path.to_string_lossy())
        .unwrap_or_default();
    let help = match app.mode {
        InputMode::Normal if selection_mode => format!(
            " {} mark · {} range · {} marked only · {} sort · {} confirm · {} help",
            bindings.keys_for(ActionContext::Normal, Action::ToggleMark),
            bindings.keys_for(ActionContext::Normal, Action::MarkRange),
            bindings.keys_for(ActionContext::Normal, Action::ToggleMarkedOnly),
            bindings.keys_for(ActionContext::Normal, Action::ToggleSort),
            bindings.keys_for(ActionContext::Normal, Action::ConfirmSelection),
            bindings.keys_for(ActionContext::Normal, Action::ToggleHelp),
        ),
        InputMode::Normal => format!(
            " {} mark · {} range · {} marked only · {} sort · {} help",
            bindings.keys_for(ActionContext::Normal, Action::ToggleMark),
            bindings.keys_for(ActionContext::Normal, Action::MarkRange),
            bindings.keys_for(ActionContext::Normal, Action::ToggleMarkedOnly),
            bindings.keys_for(ActionContext::Normal, Action::ToggleSort),
            bindings.keys_for(ActionContext::Normal, Action::ToggleHelp),
        ),
        InputMode::Search => format!(
            " type to filter · {} apply · {} cancel",
            bindings.keys_for(ActionContext::Search, Action::CommitSearch),
            bindings.keys_for(ActionContext::Search, Action::CancelSearch)
        ),
    };
    let input = match app.mode {
        InputMode::Normal if app.query.is_empty() => selected_path.into_owned(),
        InputMode::Normal => format!("/{}  ·  {selected_path}", app.query),
        InputMode::Search => format!("/{}█", app.query),
    };
    frame.render_widget(
        Paragraph::new(vec![Line::from(input), status_line(options.notice, help)]),
        area,
    );
}

fn help_lines(bindings: &Bindings, context: ActionContext) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "Effective key bindings",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.extend(bindings.help_lines(context).into_iter().map(Line::from));
    lines.push(Line::from(""));
    lines.push(Line::from("Esc closes this help"));
    lines
}

/// Screen area the help overlay covers; images below it must know it.
fn help_area(screen: Rect, bindings: &Bindings, context: ActionContext) -> Rect {
    let lines = bindings.help_lines(context).len() as u16 + 3;
    let width = screen.width.saturating_sub(4).clamp(1, 72);
    let height = (lines + 2).min(screen.height.saturating_sub(2)).max(1);
    Rect::new(
        screen.x + screen.width.saturating_sub(width) / 2,
        screen.y + screen.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn render_help(frame: &mut Frame<'_>, bindings: &Bindings, context: ActionContext) {
    let lines = help_lines(bindings, context);
    let area = help_area(frame.area(), bindings, context);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" Help ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red)),
        ),
        area,
    );
}

pub(crate) struct InspectionOptions<'a> {
    pub(crate) display: InspectionDisplay<'a>,
    pub(crate) state: ViewState,
    pub(crate) background: BackgroundMode,
    pub(crate) bindings: &'a Bindings,
    pub(crate) show_help: bool,
    pub(crate) debug_status: bool,
    pub(crate) protocol_name: &'a str,
    pub(crate) selection_mode: bool,
    pub(crate) filmstrip: bool,
    pub(crate) thumbnail_quality: ThumbnailQuality,
    pub(crate) notice: Option<&'a str>,
}

fn filmstrip_height(enabled: bool, area: Rect) -> u16 {
    let fits = area.width >= FILMSTRIP_CELL_WIDTH
        && area.height >= 4 + FILMSTRIP_CELL_HEIGHT + FILMSTRIP_MIN_IMAGE_ROWS;
    if enabled && fits {
        FILMSTRIP_CELL_HEIGHT
    } else {
        0
    }
}

pub(crate) fn render_inspection(
    frame: &mut Frame<'_>,
    app: &App,
    thumbnails: &mut Thumbnails,
    options: InspectionOptions<'_>,
) -> InspectionLayout {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(filmstrip_height(options.filmstrip, frame.area())),
            Constraint::Length(2),
        ])
        .split(frame.area());
    let filmstrip = (areas[2].height > 0).then(|| FilmstripLayout::new(areas[2]));
    let overlay = options
        .show_help
        .then(|| help_area(frame.area(), options.bindings, ActionContext::Inspect));
    render_filmstrip(
        frame,
        app,
        thumbnails,
        filmstrip,
        options.thumbnail_quality,
        overlay,
    );
    let selected = if app.filtered.is_empty() {
        0
    } else {
        app.selected + 1
    };
    let entry = app.selected_entry();
    let title = Line::from(vec![
        Span::styled(
            " red-table · inspect ",
            Style::default().fg(Color::Black).bg(Color::Red),
        ),
        Span::raw("  "),
        Span::styled(
            entry.map(|entry| entry.label.as_str()).unwrap_or_default(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]);
    let mut status = format!(
        " {selected}/{} · {} marked · {} · {} background{}",
        app.filtered.len(),
        app.marked_count(),
        options.state.zoom.label(),
        options.background,
        entry
            .filter(|entry| app.is_marked(entry.id))
            .map(|_| " · [x] current")
            .unwrap_or_default(),
    );
    if options.debug_status {
        status.push_str(" · ");
        status.push_str(options.protocol_name);
        status.push_str(&format!(
            " · center {},{}",
            options.state.center_x, options.state.center_y
        ));
    }
    frame.render_widget(Paragraph::new(vec![title, Line::from(status)]), areas[0]);

    match options.display {
        InspectionDisplay::Loading => frame.render_widget(
            Paragraph::new("loading full image…")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray)),
            areas[1],
        ),
        InspectionDisplay::Ready(prepared) => render_prepared(frame, prepared, areas[1], overlay),
        InspectionDisplay::Error(error) => frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "Cannot display this image",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(error),
            ])
            .alignment(Alignment::Center),
            areas[1],
        ),
    }

    let path = entry
        .map(|entry| entry.path.to_string_lossy())
        .unwrap_or_default();
    let mut help = format!(
        " {} previous · {} next · {} mark · {} fit/100% · {} return",
        options
            .bindings
            .keys_for(ActionContext::Inspect, Action::PreviousImage),
        options
            .bindings
            .keys_for(ActionContext::Inspect, Action::NextImage),
        options
            .bindings
            .keys_for(ActionContext::Inspect, Action::ToggleMark),
        options
            .bindings
            .keys_for(ActionContext::Inspect, Action::ToggleFitHundred),
        options
            .bindings
            .keys_for(ActionContext::Inspect, Action::CloseInspect),
    );
    if options.selection_mode {
        help.push_str(&format!(
            " · {} confirm",
            options
                .bindings
                .keys_for(ActionContext::Inspect, Action::ConfirmSelection)
        ));
    }
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(path.into_owned()),
            status_line(options.notice, help),
        ]),
        areas[3],
    );
    if options.show_help {
        render_help(frame, options.bindings, ActionContext::Inspect);
    }
    InspectionLayout {
        image_size: Size::new(areas[1].width, areas[1].height),
        filmstrip,
    }
}

/// Publishes the strip's visible thumbnail keys (empty when hidden) so Kitty
/// images re-arm only when they newly appear, then draws the strip cells.
fn render_filmstrip(
    frame: &mut Frame<'_>,
    app: &App,
    thumbnails: &mut Thumbnails,
    filmstrip: Option<FilmstripLayout>,
    thumbnail_quality: ThumbnailQuality,
    overlay: Option<Rect>,
) {
    let Some(filmstrip) = filmstrip else {
        thumbnails.update_visible([]);
        return;
    };
    let window = filmstrip.window(app.selected, app.filtered.len());
    let count = window.len();
    let entries = app.filtered[window.clone()]
        .iter()
        .filter_map(|id| app.entry(*id))
        .collect::<Vec<_>>();
    let key = |entry: &crate::app::ImageEntry| ThumbnailKey {
        entry_id: entry.id,
        revision: entry.revision,
        size: filmstrip.image_size,
        quality: thumbnail_quality,
    };
    thumbnails.update_visible(entries.iter().map(|entry| key(entry)));

    for (index, (position, entry)) in window.zip(&entries).enumerate() {
        let block = grid_card_block(position == app.selected, app.is_marked(entry.id));
        let outer = filmstrip.cell(index, count);
        let inner = block.inner(outer);
        frame.render_widget(block, outer);
        match thumbnails.display(key(entry)) {
            ThumbnailDisplay::Ready(thumbnail) => render_prepared(frame, thumbnail, inner, overlay),
            ThumbnailDisplay::Loading => frame.render_widget(
                Paragraph::new("…")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                inner,
            ),
            ThumbnailDisplay::Error(_) => frame.render_widget(
                Paragraph::new("!")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::Red)),
                inner,
            ),
        }
    }
}

pub(crate) struct ComparisonOptions<'a> {
    pub(crate) state: CompareState,
    pub(crate) reference_display: InspectionDisplay<'a>,
    pub(crate) candidate_display: InspectionDisplay<'a>,
    pub(crate) background: BackgroundMode,
    pub(crate) bindings: &'a Bindings,
    pub(crate) show_help: bool,
    pub(crate) debug_status: bool,
    pub(crate) protocol_name: &'a str,
    pub(crate) selection_mode: bool,
}

pub(crate) fn render_comparison(
    frame: &mut Frame<'_>,
    app: &App,
    options: ComparisonOptions<'_>,
) -> [Size; 2] {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(frame.area());
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(areas[1]);
    let reference = app.entry(options.state.reference_id);
    let candidate = app.selected_entry();
    let active = match options.state.active {
        ComparePane::Reference => "reference",
        ComparePane::Candidate => "candidate",
    };
    let mut status = format!(
        " {} marked · sync {} · active {active} · {} background",
        app.marked_count(),
        if options.state.synchronized {
            "on"
        } else {
            "off"
        },
        options.background,
    );
    if options.debug_status {
        status.push_str(&format!(" · {}", options.protocol_name));
    }
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                " red-table · A/B compare ",
                Style::default().fg(Color::Black).bg(Color::Red),
            )),
            Line::from(status),
        ]),
        areas[0],
    );

    let overlay = options
        .show_help
        .then(|| help_area(frame.area(), options.bindings, ActionContext::Compare));
    let reference_size = render_comparison_pane(
        frame,
        panes[0],
        "REFERENCE",
        reference
            .map(|entry| entry.label.as_str())
            .unwrap_or_default(),
        reference.is_some_and(|entry| app.is_marked(entry.id)),
        options.state.active == ComparePane::Reference,
        options.state.reference_view,
        options.reference_display,
        overlay,
    );
    let candidate_size = render_comparison_pane(
        frame,
        panes[1],
        "CANDIDATE",
        candidate
            .map(|entry| entry.label.as_str())
            .unwrap_or_default(),
        candidate.is_some_and(|entry| app.is_marked(entry.id)),
        options.state.active == ComparePane::Candidate,
        options.state.candidate_view,
        options.candidate_display,
        overlay,
    );

    let mut help = format!(
        " {} pane · {} previous/{} next · {} promote · {} sync · {} return",
        options
            .bindings
            .keys_for(ActionContext::Compare, Action::SwitchComparePane),
        options
            .bindings
            .keys_for(ActionContext::Compare, Action::PreviousImage),
        options
            .bindings
            .keys_for(ActionContext::Compare, Action::NextImage),
        options
            .bindings
            .keys_for(ActionContext::Compare, Action::PromoteCandidate),
        options
            .bindings
            .keys_for(ActionContext::Compare, Action::ToggleCompareSync),
        options
            .bindings
            .keys_for(ActionContext::Compare, Action::CloseInspect),
    );
    if options.selection_mode {
        help.push_str(&format!(
            " · {} confirm",
            options
                .bindings
                .keys_for(ActionContext::Compare, Action::ConfirmSelection)
        ));
    }
    let paths = format!(
        "{}  ↔  {}",
        reference
            .map(|entry| entry.path.to_string_lossy())
            .unwrap_or_default(),
        candidate
            .map(|entry| entry.path.to_string_lossy())
            .unwrap_or_default(),
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(paths),
            Line::from(Span::styled(help, Style::default().fg(Color::DarkGray))),
        ]),
        areas[2],
    );
    if options.show_help {
        render_help(frame, options.bindings, ActionContext::Compare);
    }
    [reference_size, candidate_size]
}

#[allow(clippy::too_many_arguments)]
fn render_comparison_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    role: &str,
    label: &str,
    marked: bool,
    active: bool,
    view: ViewState,
    display: InspectionDisplay<'_>,
    overlay: Option<Rect>,
) -> Size {
    let title = format!(
        " {role} · {} · {} · {} ",
        if marked { "[x]" } else { "[ ]" },
        label,
        view.zoom.label(),
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(if active {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match display {
        InspectionDisplay::Loading => frame.render_widget(
            Paragraph::new("loading…")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray)),
            inner,
        ),
        InspectionDisplay::Ready(prepared) => render_prepared(frame, prepared, inner, overlay),
        InspectionDisplay::Error(error) => frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "Cannot display",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(error),
            ])
            .alignment(Alignment::Center),
            inner,
        ),
    }
    Size::new(inner.width, inner.height)
}

/// `overlay` is the help area drawn over the image afterwards, if any.
fn render_prepared(
    frame: &mut Frame<'_>,
    prepared: &PreparedThumbnail,
    area: Rect,
    overlay: Option<Rect>,
) {
    match prepared {
        PreparedThumbnail::Standard(protocol) => {
            if let Some(placeholder) = protocol.needs_placeholder(area) {
                frame.render_widget(Clear, placeholder);
            } else {
                frame.render_widget(Image::new(protocol).allow_clipping(true), area);
            }
        }
        PreparedThumbnail::Kitty(protocol) => {
            frame.render_widget(KittyImage::new(protocol).occluded(overlay), area);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{buffer::Buffer, widgets::Widget};

    use crate::{app::SourceRevision, settings::DEFAULT_THUMBNAIL_SIZE};

    use super::*;

    #[test]
    fn help_owns_its_lines_over_kitty_thumbnails_in_the_grid() {
        use image::DynamicImage;
        use ratatui::{Terminal, backend::TestBackend};
        use ratatui_image::picker::Picker;

        use crate::kitty::{KittyProtocol, KittySession};

        let mut app = App::new("/photos".into());
        for index in 0..4 {
            app.add_path(
                format!("/photos/{index}.png").into(),
                SourceRevision::default(),
            );
        }
        app.integrate_pending();
        let session = KittySession::with_tmux(false);
        let mut thumbnails = Thumbnails::with_session(Picker::halfblocks(), session.clone());
        let bindings = Bindings::built_in();
        let screen = Rect::new(0, 0, 80, 40);
        let grid = GridLayout::new_for_test(screen, DEFAULT_THUMBNAIL_SIZE);
        let quality = "7".parse().unwrap();
        for id in app.filtered.clone() {
            thumbnails.insert(
                ThumbnailKey {
                    entry_id: id,
                    revision: SourceRevision::default(),
                    size: grid.image_size,
                    quality,
                },
                PreparedThumbnail::Kitty(
                    KittyProtocol::new(
                        DynamicImage::new_rgb8(8, 8),
                        grid.image_size,
                        session.clone(),
                    )
                    .unwrap(),
                ),
            );
        }
        let mut terminal = Terminal::new(TestBackend::new(screen.width, screen.height)).unwrap();
        let mut draw = |show_help| {
            terminal
                .draw(|frame| {
                    render_grid(
                        frame,
                        &mut app,
                        &mut thumbnails,
                        RenderOptions {
                            protocol_name: "Kitty",
                            thumbnail_size: DEFAULT_THUMBNAIL_SIZE,
                            thumbnail_quality: quality,
                            bindings: &bindings,
                            show_help,
                            debug_status: false,
                            selection_mode: false,
                            notice: None,
                        },
                    );
                })
                .unwrap();
            terminal.backend().buffer().clone()
        };

        let row_symbols = |buffer: &Buffer, lines: std::ops::Range<u16>| {
            lines
                .flat_map(|y| (0..screen.width).map(move |x| (x, y)))
                .filter(|position| buffer[*position].symbol().contains("\x1b[s"))
                .count()
        };
        let without_help = draw(false);
        assert!(
            row_symbols(&without_help, 0..screen.height) > 0,
            "the grid must show Kitty row symbols for this test to mean anything"
        );

        let help = help_area(screen, &bindings, ActionContext::Normal);
        let with_help = draw(true);
        assert_eq!(
            row_symbols(&with_help, help.top()..help.bottom()),
            0,
            "a row symbol on a help line would repaint over the help"
        );
        assert_eq!(with_help[(help.x + 1, help.y + 1)].symbol(), "E");
        assert_eq!(with_help[(help.x, help.y + 1)].symbol(), "│");
    }

    #[test]
    fn larger_target_dimensions_reduce_grid_density() {
        let area = Rect::new(0, 0, 100, 40);
        let default = GridLayout::new(area, DEFAULT_THUMBNAIL_SIZE);
        let larger = GridLayout::new(
            area,
            ThumbnailSize {
                width: 44,
                height: 20,
            },
        );

        assert!(larger.columns < default.columns);
        assert!(larger.image_size.width > default.image_size.width);
        assert!(larger.image_size.height >= default.image_size.height);
    }

    #[test]
    fn filmstrip_window_centres_on_the_selection_and_clamps_at_the_ends() {
        let strip = FilmstripLayout::new(Rect::new(0, 0, 5 * FILMSTRIP_CELL_WIDTH + 3, 7));
        assert_eq!(strip.columns, 5);
        assert_eq!(strip.window(0, 0), 0..0);
        assert_eq!(strip.window(1, 3), 0..3);
        assert_eq!(strip.window(0, 100), 0..5);
        assert_eq!(strip.window(50, 100), 48..53);
        assert_eq!(strip.window(99, 100), 95..100);
        assert_eq!(strip.window(97, 100), 95..100);
    }

    #[test]
    fn filmstrip_cells_are_centred_and_stay_inside_the_strip() {
        let area = Rect::new(2, 30, 80, FILMSTRIP_CELL_HEIGHT);
        let strip = FilmstripLayout::new(area);
        let first = strip.cell(0, 3);
        let last = strip.cell(2, 3);
        assert_eq!(first.x - area.x, area.right() - last.right());
        assert!(strip.cell(strip.columns - 1, strip.columns).right() <= area.right());
    }

    #[test]
    fn filmstrip_needs_room_for_the_main_image() {
        assert_eq!(
            filmstrip_height(true, Rect::new(0, 0, 80, 21)),
            FILMSTRIP_CELL_HEIGHT
        );
        assert_eq!(filmstrip_height(true, Rect::new(0, 0, 80, 20)), 0);
        assert_eq!(filmstrip_height(true, Rect::new(0, 0, 13, 40)), 0);
        assert_eq!(filmstrip_height(false, Rect::new(0, 0, 80, 40)), 0);
    }

    #[test]
    fn mark_label_is_literal_and_independent_from_focus_styling() {
        let mut app = App::new("/photos".into());
        let id = app.add_path("/photos/one.png".into(), SourceRevision::default());
        app.integrate_pending();
        assert_eq!(grid_label(&app, app.entry(id).unwrap()), "[ ] one.png");
        app.toggle_mark();
        assert_eq!(grid_label(&app, app.entry(id).unwrap()), "[x] one.png");
    }

    #[test]
    fn mark_border_shape_composes_with_focus_style() {
        let area = Rect::new(0, 0, 8, 3);
        let render = |focused, marked| {
            let mut buffer = Buffer::empty(area);
            grid_card_block(focused, marked).render(area, &mut buffer);
            buffer
        };

        let plain = render(false, false);
        assert_eq!(plain[(0, 0)].symbol(), "┌");
        assert_eq!(plain[(0, 0)].fg, Color::DarkGray);

        let marked = render(false, true);
        assert_eq!(marked[(0, 0)].symbol(), "╔");
        assert_eq!(marked[(0, 0)].fg, Color::Yellow);

        let focused = render(true, false);
        assert_eq!(focused[(0, 0)].symbol(), "┌");
        assert_eq!(focused[(0, 0)].fg, Color::Red);
        assert!(focused[(0, 0)].modifier.contains(Modifier::BOLD));

        let focused_mark = render(true, true);
        assert_eq!(focused_mark[(0, 0)].symbol(), "╔");
        assert_eq!(focused_mark[(0, 0)].fg, Color::Red);
        assert!(focused_mark[(0, 0)].modifier.contains(Modifier::BOLD));
    }
}
