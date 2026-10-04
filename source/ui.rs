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
}

impl GridLayout {
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
    render_grid_cards(frame, app, thumbnails, grid, options.thumbnail_quality);
    render_footer(
        frame,
        app,
        options.bindings,
        options.selection_mode,
        areas[2],
    );
    if options.show_help {
        let context = match app.mode {
            InputMode::Normal => ActionContext::Normal,
            InputMode::Search => ActionContext::Search,
        };
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
            " {selected}/{} · {} marked · {} found · {state} · {pending} load · {} · Q{} · {protocol_name} · M{memory_entries}/{}MiB · D{disk_hits}/{disk_misses} · {errors} err",
            app.filtered.len(),
            app.marked_count(),
            app.entries.len(),
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
            " {selected}/{} · {} marked{} · {} found · {state}{loading}{failures}",
            app.filtered.len(),
            app.marked_count(),
            if app.marked_only() { " (only)" } else { "" },
            app.entries.len(),
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
            ThumbnailDisplay::Ready(thumbnail) => render_prepared(frame, thumbnail, image_area),
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

fn render_footer(
    frame: &mut Frame<'_>,
    app: &App,
    bindings: &Bindings,
    selection_mode: bool,
    area: Rect,
) {
    let selected_path = app
        .selected_entry()
        .map(|entry| entry.path.to_string_lossy())
        .unwrap_or_default();
    let help = match app.mode {
        InputMode::Normal if selection_mode => format!(
            " {} mark · {} range · {} marked only · {} confirm · {} help",
            bindings.keys_for(ActionContext::Normal, Action::ToggleMark),
            bindings.keys_for(ActionContext::Normal, Action::MarkRange),
            bindings.keys_for(ActionContext::Normal, Action::ToggleMarkedOnly),
            bindings.keys_for(ActionContext::Normal, Action::ConfirmSelection),
            bindings.keys_for(ActionContext::Normal, Action::ToggleHelp),
        ),
        InputMode::Normal => format!(
            " {} mark · {} range · {} marked only · {} help",
            bindings.keys_for(ActionContext::Normal, Action::ToggleMark),
            bindings.keys_for(ActionContext::Normal, Action::MarkRange),
            bindings.keys_for(ActionContext::Normal, Action::ToggleMarkedOnly),
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
        Paragraph::new(vec![
            Line::from(input),
            Line::from(Span::styled(help, Style::default().fg(Color::DarkGray))),
        ]),
        area,
    );
}

fn render_help(frame: &mut Frame<'_>, bindings: &Bindings, context: ActionContext) {
    let mut lines = vec![Line::from(Span::styled(
        "Effective key bindings",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.extend(bindings.help_lines(context).into_iter().map(Line::from));
    lines.push(Line::from(""));
    lines.push(Line::from("Esc closes this help"));

    let screen = frame.area();
    let width = screen.width.saturating_sub(4).clamp(1, 72);
    let height = (lines.len() as u16 + 2)
        .min(screen.height.saturating_sub(2))
        .max(1);
    let area = Rect::new(
        screen.x + screen.width.saturating_sub(width) / 2,
        screen.y + screen.height.saturating_sub(height) / 2,
        width,
        height,
    );
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
}

pub(crate) fn render_inspection(
    frame: &mut Frame<'_>,
    app: &App,
    options: InspectionOptions<'_>,
) -> Size {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(frame.area());
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
        InspectionDisplay::Ready(prepared) => render_prepared(frame, prepared, areas[1]),
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
            Line::from(Span::styled(help, Style::default().fg(Color::DarkGray))),
        ]),
        areas[2],
    );
    if options.show_help {
        render_help(frame, options.bindings, ActionContext::Inspect);
    }
    Size::new(areas[1].width, areas[1].height)
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
        InspectionDisplay::Ready(prepared) => render_prepared(frame, prepared, inner),
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

fn render_prepared(frame: &mut Frame<'_>, prepared: &PreparedThumbnail, area: Rect) {
    match prepared {
        PreparedThumbnail::Standard(protocol) => {
            if let Some(placeholder) = protocol.needs_placeholder(area) {
                frame.render_widget(Clear, placeholder);
            } else {
                frame.render_widget(Image::new(protocol).allow_clipping(true), area);
            }
        }
        PreparedThumbnail::Kitty(protocol) => {
            frame.render_widget(KittyImage::new(protocol), area);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{buffer::Buffer, widgets::Widget};

    use crate::{app::SourceRevision, settings::DEFAULT_THUMBNAIL_SIZE};

    use super::*;

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
