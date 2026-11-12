//! Overview layout and transient interaction state. No I/O belongs here.
use super::*;
use crate::appearance::{palette, style};
use ratatui::{
    Frame,
    layout::Margin,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Focus {
    Search,
    #[default]
    Projects,
    Detail,
    Stats,
    Queue,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeatmapAction {
    Period(bool),
    Expand,
    Day { day: u64, commits: Option<u32> },
}
impl HeatmapAction {
    fn description(self) -> Option<String> {
        let Self::Day { day, commits } = self else {
            return None;
        };
        Some(match commits {
            Some(count) => crate::i18n::format(
                "overview.activity_day",
                &[
                    ("date", crate::utc::label(day)),
                    ("count", count.to_string()),
                ],
            ),
            None => crate::i18n::format(
                "overview.activity_future_day",
                &[("date", crate::utc::label(day))],
            ),
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Columns {
    pub list: Rect,
    pub detail: Option<Rect>,
    pub stats: Option<Rect>,
    pub dividers: [Option<Rect>; 2],
}
// 24 heatmap columns + widest weekday label (3) + panel padding (4),
// rounded up to leave one spare column. Applies even to saved narrow ratios.
const MIN_STATS_WIDTH: u16 = 32;
pub(super) fn columns(
    body: Rect,
    detail: bool,
    stats: bool,
    left: Option<u16>,
    middle: Option<u16>,
) -> Columns {
    let mut out = Columns {
        list: body,
        detail: None,
        stats: None,
        dividers: [None, None],
    };
    if body.width < 104 || (!detail && !stats) {
        return out;
    }
    let three = body.width >= 160 && detail && stats;
    let usable = body.width.saturating_sub(if three { 2 } else { 1 });
    let mut a = ((u32::from(usable)
        * u32::from(left.unwrap_or(if three { 3800 } else { 4400 }).min(10000)))
        / 10000) as u16;
    a = a.clamp(
        if three { 44 } else { 42 },
        usable.saturating_sub(if three { 60 + MIN_STATS_WIDTH } else { 60 }),
    );
    out.list.width = a;
    out.dividers[0] = Some(Rect::new(body.x + a, body.y, 1, body.height));
    let x = body.x + a + 1;
    let remaining = usable - a;
    if three {
        let b = ((u32::from(usable) * u32::from(middle.unwrap_or(3800).min(10000))) / 10000) as u16;
        let b = b.clamp(60, remaining - MIN_STATS_WIDTH);
        out.detail = Some(Rect::new(x, body.y, b, body.height));
        out.dividers[1] = Some(Rect::new(x + b, body.y, 1, body.height));
        out.stats = Some(Rect::new(x + b + 1, body.y, remaining - b, body.height));
    } else if detail {
        out.detail = Some(Rect::new(x, body.y, remaining, body.height));
    } else {
        out.stats = Some(Rect::new(x, body.y, remaining, body.height));
    }
    out
}
#[derive(Default)]
pub(super) struct Dashboard {
    pub focus: Focus,
    pub overlay: Option<Focus>,
    pub size: Rect,
    pub columns: Option<Columns>,
    pub row_hits: Vec<(Rect, usize)>,
    pub summary: Rect,
    pub stats_scroll: u16,
    pub stats_year: bool,
    pub heatmap_hits: Vec<(Rect, HeatmapAction)>,
    pub heatmap_hover: Option<(Rect, String)>,
    pub detail_max: u16,
    pub stats_max: u16,
    pub drag: Option<(usize, Option<u16>, Option<u16>)>,
    pub hover: Option<(usize, Instant)>,
    pub expanded: bool,
    pub undo: Option<model::Queue>,
    pub pressed: Option<(usize, u16, u16)>,
    pub queue_drag: Option<usize>,
    pub selected_since: Option<(String, Instant)>,
    pub context_target: Option<String>,
    pub context_since: Option<Instant>,
    pub context_pending: Option<u64>,
    pub history_ticket: u64,
    pub history_pending: bool,
    pub contexts: BTreeMap<String, history::History>,
    pub context_order: std::collections::VecDeque<String>,
    pub sources: BTreeMap<String, stats::Stats>,
    pub source_pending: Option<u64>,
    pub source_ticket: u64,
    pub summary_cache: Option<Summary>,
    pub summary_links: Vec<String>,
    pub viewer: Option<Box<crate::sidebar::Sidebar>>,
    pub detail_actions: Vec<(usize, DetailAction)>,
    pub detail_hits: Vec<(Rect, DetailAction)>,
    pub file_selected: Option<(PathBuf, Option<usize>)>,
    pub viewer_line: Option<usize>,
    pub press_dragged: bool,
    pub scrollbar: Option<(Focus, Rect)>,
    pub list_detached: bool,
    pub pressed_path: Option<PathBuf>,
    pub hover_displayed: bool,
    pub agent_hits: Vec<(Rect, usize)>,
    pub stats_at: BTreeMap<String, Instant>,
    pub sources_at: BTreeMap<String, Instant>,
    pub pending_hit: Option<Rect>,
    pub text_selection: Option<TextSelection>,
    pub divider_hover: Option<usize>,
    pub search_hit: Option<Rect>,
    pub session_pending: Option<(String, u64)>,
    pub session_loaded: BTreeSet<String>,
    pub add_hit: Option<Rect>,
    pub expand_hits: Vec<(Rect, String)>,
    pub expanded_projects: BTreeSet<String>,
    pub history_query: String,
    pub history_preview: Option<(String, Vec<Line<'static>>)>,
}
pub(super) struct TextSelection {
    pub buffer: ratatui::buffer::Buffer,
    pub range: crate::selection::Selection,
    pub dragging: bool,
}
impl TextSelection {
    pub fn text(&self) -> String {
        let (start, end) = self.range.bounds();
        let area = self.buffer.area;
        let mut rows = Vec::new();
        for y in start.row..=end.row {
            let left = if y == start.row { start.col } else { area.x };
            let right = if y == end.row {
                end.col.saturating_add(1)
            } else {
                area.right()
            };
            let mut row = String::new();
            let mut x = area.x;
            while x < right.min(area.right()) {
                let symbol = self.buffer[(x, y)].symbol();
                let width = symbol.width().max(1) as u16;
                if x + width > left {
                    row.push_str(symbol);
                }
                x += width;
            }
            rows.push(row.trim_end().to_string());
        }
        rows.join("\n")
    }
}
#[derive(Clone)]
pub(super) enum DetailAction {
    Note,
    Options,
    History,
    Expand,
    File(PathBuf),
    Link(String),
    Instances,
    Diagnostics,
    Resume(String),
}
pub(super) struct Summary {
    pub key: String,
    pub lines: Vec<Line<'static>>,
    pub links: Vec<String>,
}
/// Strip numeric location suffixes from the right, preserving the drive colon.
pub(super) fn local_reference(
    root: &std::path::Path,
    target: &str,
) -> Option<(PathBuf, Option<usize>)> {
    let mut path = target;
    let mut line = None;
    if let Some((prefix, number)) = path.rsplit_once(':')
        && let Ok(n) = number.parse::<usize>()
    {
        path = prefix;
        line = Some(n);
        if let Some((prefix, number)) = path.rsplit_once(':')
            && let Ok(n) = number.parse::<usize>()
        {
            path = prefix;
            line = Some(n);
        }
    }
    if let Some((prefix, fragment)) = path.split_once("#L")
        && let Ok(n) = fragment.parse::<usize>()
    {
        path = prefix;
        line = Some(n);
    }
    match crate::preview::reading::destination(&root.join("__overview__.md"), path).ok()? {
        crate::preview::reading::Destination::File(path, _) => {
            Some((path, line.filter(|n| *n > 0)))
        }
        _ => None,
    }
}
impl Dashboard {
    pub fn cancel(&mut self, prefs: &mut model::Preferences) {
        if let Some((_, a, b)) = self.drag.take() {
            prefs.list_ratio = a;
            prefs.detail_ratio = b;
        }
        self.hover = None;
        self.pressed = None;
        self.queue_drag = None;
        self.press_dragged = false;
        self.scrollbar = None;
        self.pressed_path = None;
        self.text_selection = None;
        self.divider_hover = None;
    }
    pub fn visible(&self) -> Vec<Focus> {
        let mut v = vec![Focus::Search, Focus::Projects];
        if self.columns.is_some_and(|c| c.detail.is_some()) {
            v.push(Focus::Detail);
        }
        if self.columns.is_some_and(|c| c.stats.is_some()) {
            v.push(Focus::Stats);
        }
        v.push(Focus::Queue);
        v
    }
    pub fn store_context(&mut self, key: String, history: history::History) {
        self.summary_cache = None;
        self.context_order.retain(|k| *k != key);
        self.context_order.push_back(key.clone());
        self.contexts.insert(key, history);
        while self.context_order.len() > 32 {
            if let Some(k) = self.context_order.pop_front() {
                self.contexts.remove(&k);
            }
        }
    }
}
pub(super) fn context_key(path: &std::path::Path, agent: Agent) -> String {
    format!("{}|{agent:?}", config::key(path))
}
fn panel(
    frame: &mut Frame,
    title: String,
    lines: Vec<Line<'static>>,
    rect: Rect,
    scroll: &mut u16,
    focused: bool,
) -> u16 {
    let inner = rect.inner(Margin::new(2, 1));
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let count = paragraph.line_count(inner.width).min(u16::MAX as usize) as u16;
    let max = count.saturating_sub(inner.height);
    *scroll = (*scroll).min(max);
    frame.render_widget(
        Block::bordered()
            .title(format!(" {title} "))
            .border_style(Style::new().fg(if focused {
                palette().accent
            } else {
                palette().muted
            })),
        rect,
    );
    frame.render_widget(paragraph.scroll((*scroll, 0)), inner);
    if max > 0 && inner.height > 0 {
        let y =
            inner.y + (u32::from(*scroll) * u32::from(inner.height - 1) / u32::from(max)) as u16;
        for row in inner.y..inner.bottom() {
            frame.render_widget(
                Paragraph::new(if row == y { "█" } else { "│" })
                    .style(Style::new().fg(palette().muted)),
                Rect::new(rect.right() - 1, row, 1, 1),
            );
        }
    }
    max
}
pub(super) fn fit(text: &str, width: usize) -> String {
    let text = crate::preview::safe_text(text);
    let value = crate::preview::truncate_name(&text, width);
    format!(
        "{}{}",
        value,
        " ".repeat(width.saturating_sub(value.width()))
    )
}
fn draw_search(frame: &mut Frame, app: &mut Overview, top: Rect) {
    use unicode_segmentation::UnicodeSegmentation;
    let p = palette();
    frame.render_widget(
        Paragraph::new(" DevTabs ").style(Style::new().fg(p.accent).bold()),
        Rect::new(top.x, top.y, 10, 1),
    );
    let sort = format!(" {}: {} ", t("sort"), app.config.overview.sort.label());
    let sort_width = (sort.width() as u16).min(24);
    let rect = Rect::new(
        top.x + 11,
        top.y,
        top.width.saturating_sub(12 + sort_width).min(64),
        1,
    );
    app.dashboard.search_hit = Some(rect);
    let focused = app.dashboard.focus == Focus::Search
        && app.dashboard.overlay.is_none()
        && app.dialog.is_none();
    // Search editing appends at the end. Keep that end and its cursor visible,
    // measuring terminal cells and preserving complete grapheme clusters.
    let query = crate::preview::safe_text(&app.query);
    let room = rect.width.saturating_sub(6) as usize;
    let mut start = query.len();
    let mut used = 0;
    for (index, cluster) in query.grapheme_indices(true).rev() {
        if used + cluster.width() > room {
            break;
        }
        start = index;
        used += cluster.width();
    }
    let text = if query.is_empty() {
        fit(&t("overview.search_hint"), room)
    } else {
        fit(&query[start..], room)
    };
    let input_style = Style::new().fg(p.fg).bg(p.selected);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "[ / ",
                input_style.fg(if focused { p.accent } else { p.muted }),
            ),
            Span::styled(
                text,
                if query.is_empty() {
                    input_style.fg(p.muted)
                } else {
                    input_style
                },
            ),
            Span::styled(
                " ]",
                input_style.fg(if focused { p.accent } else { p.muted }),
            ),
        ])),
        rect,
    );
    frame.render_widget(
        Paragraph::new(sort).style(Style::new().fg(p.muted)),
        Rect::new(top.right() - sort_width, top.y, sort_width, 1),
    );
    if focused {
        frame.set_cursor_position((rect.x + 4 + used as u16, rect.y));
    }
}
pub(super) fn draw(frame: &mut Frame, app: &mut Overview) {
    let area = frame.area();
    app.dashboard.size = area;
    app.detail_areas.clear();
    app.dashboard.heatmap_hits.clear();
    app.filter_hits.clear();
    app.dashboard.row_hits.clear();
    app.dashboard.agent_hits.clear();
    app.dashboard.pending_hit = None;
    app.dashboard.search_hit = None;
    app.dashboard.add_hit = None;
    app.dashboard.expand_hits.clear();
    app.dashboard.detail_hits.clear();
    app.dialog_list = None;
    app.dialog_preview = Rect::default();
    app.list_area = Rect::default();
    app.dashboard.columns = None;
    app.dashboard.summary = Rect::default();
    frame.render_widget(Block::default().style(style()), area);
    if area.width < 60 || area.height < 10 {
        frame.render_widget(
            Paragraph::new(format!(
                "DevTabs · {}\n/ {}: {} · Esc",
                t("overview.resize"),
                t("search"),
                app.query
            )),
            area,
        );
        return;
    }
    let top_height = 2;
    let top = Rect::new(area.x, area.y, area.width, top_height);
    draw_search(frame, app, top);
    let mut x = area.x + 1;
    let filters = [
        model::Filter::All,
        model::Filter::Running,
        model::Filter::Attention,
        model::Filter::Dirty,
        model::Filter::Pending,
        model::Filter::Codex,
        model::Filter::Claude,
        model::Filter::Shell,
    ];
    let labels: Vec<_> = filters
        .iter()
        .map(|filter| {
            let unknown = (*filter == model::Filter::Dirty && app.git.len() < app.projects.len())
                || (*filter == model::Filter::Running && !app.host_error.is_empty());
            let count = if unknown {
                "—".into()
            } else {
                app.filter_counts
                    .get(filter)
                    .copied()
                    .unwrap_or(0)
                    .to_string()
            };
            format!(" {} {count} ", filter.label())
        })
        .collect();
    let active = filters
        .iter()
        .position(|f| *f == app.config.overview.filter)
        .unwrap_or(0);
    let mut first = 0;
    while labels[first..=active]
        .iter()
        .map(|s| s.width() + 1)
        .sum::<usize>()
        > area.width.saturating_sub(3) as usize
        && first < active
    {
        first += 1;
    }
    if first > 0 {
        frame.render_widget(
            Paragraph::new("‹").style(Style::new().fg(palette().muted)),
            Rect::new(area.x, top.y + 1, 1, 1),
        );
    }
    for (filter, label) in filters.into_iter().zip(labels).skip(first) {
        let w = label.width() as u16;
        if x + w >= area.right() {
            frame.render_widget(
                Paragraph::new("›").style(Style::new().fg(palette().muted)),
                Rect::new(area.right() - 1, top.y + 1, 1, 1),
            );
            break;
        }
        let rect = Rect::new(x, top.bottom() - 1, w, 1);
        frame.render_widget(
            Paragraph::new(label).style(if filter == app.config.overview.filter {
                Style::new()
                    .fg(ratatui::style::Color::Black)
                    .bg(ratatui::style::Color::Rgb(237, 174, 85))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(palette().fg).bg(palette().selected)
            }),
            rect,
        );
        app.filter_hits.push((rect, filter));
        x += w + 1;
    }
    let footer_height = if area.height <= 15 { 1 } else { 2 };
    let footer = Rect::new(
        area.x,
        area.bottom() - footer_height,
        area.width,
        footer_height,
    );
    let body = Rect::new(
        area.x,
        top.bottom(),
        area.width,
        area.height - top_height - footer_height,
    );
    let cols = columns(
        body,
        app.show_detail,
        app.show_stats,
        app.config.overview.list_ratio,
        app.config.overview.detail_ratio,
    );
    app.dashboard.columns = Some(cols);
    if !app.dashboard.visible().contains(&app.dashboard.focus) {
        app.dashboard.focus = Focus::Projects;
    }
    for (i, r) in cols
        .dividers
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.map(|r| (i, r)))
    {
        frame.render_widget(
            Paragraph::new("│\n".repeat(r.height as usize)).style(Style::new().fg(
                if app.dashboard.drag.is_some_and(|(index, _, _)| index == i)
                    || app.dashboard.divider_hover == Some(i)
                {
                    palette().accent
                } else {
                    palette().muted
                },
            )),
            r,
        );
    }
    draw_projects(frame, app, cols.list);
    if let Some(r) = cols.detail {
        let lines = super::view::detail_lines(app, r.width.saturating_sub(4));
        app.detail_areas.push(r);
        detail_hits(app, &lines, r);
        app.dashboard.detail_max = panel(
            frame,
            t("detail").into_owned(),
            lines,
            r,
            &mut app.detail_scroll,
            app.dashboard.focus == Focus::Detail,
        );
    }
    if let Some(r) = cols.stats {
        app.detail_areas.push(r);
        app.dashboard.stats_max = panel(
            frame,
            t("statistics").into_owned(),
            super::view::stats_lines(app, r.width.saturating_sub(4)),
            r,
            &mut app.dashboard.stats_scroll,
            app.dashboard.focus == Focus::Stats,
        );
        heatmap_hits(app, r);
    }
    app.dashboard.summary = Rect::new(footer.x, footer.y, footer.width, 1);
    let summary = if app.marked.is_empty() {
        app.path()
            .map(|p| {
                format!(
                    "Enter → {} · {}",
                    p.file_name().unwrap_or_default().to_string_lossy(),
                    app.launch_description(&app.options_for(&p))
                )
            })
            .unwrap_or_else(|| t("no_project").into_owned())
    } else {
        let mut text = String::from("Ctrl+Enter → ");
        let mut shown = 0;
        for (i, entry) in app.marked.0.iter().enumerate() {
            let options = entry
                .restore
                .clone()
                .or_else(|| entry.connect.clone())
                .unwrap_or_else(|| app.options_for(&entry.path));
            let part = format!(
                "{} {} · {}{}  ",
                i + 1,
                entry.path.file_name().unwrap_or_default().to_string_lossy(),
                app.launch_description(&options),
                if entry.restore.is_some() {
                    format!(" · {}", t("parity.pending"))
                } else {
                    String::new()
                }
            );
            if shown > 0 && text.width() + part.width() > area.width.saturating_sub(16) as usize {
                break;
            }
            text.push_str(&part);
            shown += 1;
        }
        if shown < app.marked.len() {
            text.push_str(&format!(" +{} [Enter]", app.marked.len() - shown));
        }
        text
    };
    let summary = if footer_height == 1 {
        format!(
            "[{} {}] Tab · Enter · F1",
            t("overview.queue"),
            app.marked.len()
        )
    } else {
        summary
    };
    frame.render_widget(
        Paragraph::new(summary).style(Style::new().fg(palette().accent).bg(
            if app.dashboard.focus == Focus::Queue {
                palette().selected
            } else {
                palette().bg
            },
        )),
        app.dashboard.summary,
    );
    let hint = if !app.message.is_empty() {
        app.message.clone()
    } else if let Some((i, _)) = app
        .dashboard
        .hover
        .filter(|(_, since)| since.elapsed() >= Duration::from_millis(400))
    {
        app.dashboard.hover_displayed = true;
        app.rows
            .get(i)
            .and_then(Row::path)
            .map(|p| {
                let key = config::key(p);
                let source = if app.usage.get(&key).copied().unwrap_or(0)
                    > app.config.projects.get(&key).map_or(0, |p| p.last_opened)
                {
                    t("overview.agent_history_time")
                } else {
                    t("last_opened")
                };
                format!(
                    "{} · {}",
                    crate::preview::safe_text(&p.display().to_string()),
                    source
                )
            })
            .unwrap_or_default()
    } else {
        t("overview.hint").into_owned()
    };
    if footer_height > 1 {
        frame.render_widget(
            Paragraph::new(hint).style(Style::new().fg(palette().muted)),
            Rect::new(footer.x, footer.y + 1, footer.width, 1),
        );
    }
    if let Some(focus) = app.dashboard.overlay.filter(|_| app.dialog.is_none()) {
        app.dashboard.heatmap_hits.clear();
        // This is a modal view, not another statistics column. Clear the whole
        // body so the original panels cannot peek out beside a centered popup.
        // Clear also resets old glyphs; a styled empty Block alone only paints.
        crate::ui::clear_popup(frame, body);
        frame.render_widget(Block::default().style(style()), body);
        let r = super::view::popup(
            frame,
            if focus == Focus::Stats {
                area.width.saturating_sub(4).min(180)
            } else {
                100
            },
            area.height.saturating_sub(4),
        );
        app.detail_areas = vec![r];
        if focus == Focus::Stats {
            app.dashboard.detail_hits.clear();
            app.dashboard.stats_max = panel(
                frame,
                format!("{} · Esc", t("statistics")),
                super::view::stats_lines(app, r.width.saturating_sub(4)),
                r,
                &mut app.dashboard.stats_scroll,
                true,
            );
            heatmap_hits(app, r);
        } else {
            let lines = super::view::detail_lines(app, r.width.saturating_sub(4));
            app.dashboard.detail_hits.clear();
            detail_hits(app, &lines, r);
            app.dashboard.detail_max = panel(
                frame,
                t("detail").into_owned(),
                lines,
                r,
                &mut app.detail_scroll,
                true,
            );
        }
    }
    if app.dialog.is_none()
        && app.dashboard.text_selection.is_none()
        && let Some((hit, text)) = &app.dashboard.heatmap_hover
        && app
            .dashboard
            .heatmap_hits
            .iter()
            .any(|(rect, action)| rect == hit && action.description().as_ref() == Some(text))
    {
        let width = (text.width() as u16 + 2).min(area.width);
        let x = hit.x.min(area.right().saturating_sub(width));
        let y = if hit.y + 1 < area.bottom() {
            hit.y + 1
        } else {
            hit.y.saturating_sub(1)
        };
        frame.render_widget(
            Paragraph::new(format!(" {text} "))
                .style(Style::new().fg(palette().fg).bg(palette().selected)),
            Rect::new(x, y, width, 1),
        );
    }
    if let Some(selection) = &app.dashboard.text_selection {
        let r = selection.buffer.area;
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                frame.buffer_mut()[(x, y)] = selection.buffer[(x, y)].clone();
                if selection
                    .range
                    .contains(crate::selection::Point { row: y, col: x })
                {
                    frame.buffer_mut()[(x, y)].set_bg(palette().selected);
                }
            }
        }
    }
}
fn heatmap_hits(app: &mut Overview, rect: Rect) {
    let inner = rect.inner(Margin::new(2, 1));
    for (row, col, width, action) in super::view::stats_chart_hits(app, inner.width) {
        if row < app.dashboard.stats_scroll as usize {
            continue;
        }
        let y = row - app.dashboard.stats_scroll as usize;
        if y < inner.height as usize && col + width <= inner.width as usize {
            app.dashboard.heatmap_hits.push((
                Rect::new(inner.x + col as u16, inner.y + y as u16, width as u16, 1),
                action,
            ));
        }
    }
}
fn detail_hits(app: &mut Overview, lines: &[Line<'static>], rect: Rect) {
    let inner = rect.inner(Margin::new(2, 1));
    let mut row = 0usize;
    let starts = lines
        .iter()
        .map(|line| {
            let start = row;
            row += Paragraph::new(line.clone())
                .wrap(Wrap { trim: false })
                .line_count(inner.width);
            start
        })
        .collect::<Vec<_>>();
    app.detail_scroll = app.detail_scroll.min(
        row.saturating_sub(inner.height as usize)
            .min(u16::MAX as usize) as u16,
    );
    for (i, row) in starts.into_iter().enumerate() {
        if let Some((_, action)) = app.dashboard.detail_actions.iter().find(|(n, _)| *n == i)
            && row >= usize::from(app.detail_scroll)
            && row - usize::from(app.detail_scroll) < usize::from(inner.height)
        {
            app.dashboard.detail_hits.push((
                Rect::new(
                    inner.x,
                    inner.y + (row - usize::from(app.detail_scroll)) as u16,
                    inner.width,
                    1,
                ),
                action.clone(),
            ));
        }
    }
}
fn agent_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Codex => "Codex",
        Agent::Claude => "Claude",
        Agent::Shell => "Shell",
    }
}
fn draw_projects(frame: &mut Frame, app: &mut Overview, rect: Rect) {
    frame.render_widget(
        Block::bordered()
            .title(format!(
                " {} {} ",
                t(if app.scanning { "scan" } else { "projects" }),
                app.projects.len()
            ))
            .border_style(Style::new().fg(if app.dashboard.focus == Focus::Projects {
                palette().accent
            } else {
                palette().muted
            })),
        rect,
    );
    let mut inner = rect.inner(Margin::new(1, 1));
    if inner.height > 1 {
        let add = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
        let label = format!("[＋ {}]", t("add"));
        frame.render_widget(
            Paragraph::new(label).style(Style::new().fg(palette().accent).bg(palette().selected)),
            add,
        );
        app.dashboard.add_hit = Some(add);
        inner.height -= 1;
    }
    app.list_area = inner;
    let height = |_: &Row| 1u16;
    let mut offset = app.list.offset().min(app.rows.len().saturating_sub(1));
    if !app.dashboard.list_detached {
        offset = offset.min(app.selected);
        let mut first = app.selected.min(app.rows.len().saturating_sub(1));
        let mut used = app.rows.get(first).map_or(0, height);
        while first > offset && used.saturating_add(height(&app.rows[first - 1])) <= inner.height {
            first -= 1;
            used += height(&app.rows[first]);
        }
        offset = offset.max(first);
    }
    *app.list.offset_mut() = offset;
    let mut y = inner.y;
    for (i, row) in app.rows.iter().enumerate().skip(offset) {
        if y >= inner.bottom() {
            break;
        }
        let h = height(row).min(inner.bottom() - y);
        let hit = Rect::new(inner.x, y, inner.width, h);
        app.dashboard.row_hits.push((hit, i));
        let selected = i == app.selected;
        let base = if selected {
            Style::new().bg(palette().selected)
        } else {
            style()
        };
        match row {
            Row::Group(g) => {
                let count = app.group_counts.get(g).copied().unwrap_or(0);
                frame.render_widget(
                    Paragraph::new(format!(
                        " {} {}  {count}",
                        if app.config.overview.collapsed.contains(g) {
                            "▸"
                        } else {
                            "▾"
                        },
                        model::group_label(*g)
                    ))
                    .style(base.fg(palette().accent).add_modifier(Modifier::BOLD)),
                    hit,
                );
                if *g == 4 {
                    let label = format!("[{}]", t("overview.add_pending"));
                    let w = label.width() as u16;
                    if w + model::group_label(*g).width() as u16 + 10 < inner.width {
                        let r = Rect::new(inner.right() - w, y, w, 1);
                        frame.render_widget(
                            Paragraph::new(label).style(Style::new().fg(palette().accent)),
                            r,
                        );
                        app.dashboard.pending_hit = Some(r);
                    }
                }
            }
            Row::Project(path) => {
                let key = config::key(path);
                let profile = app
                    .profiles
                    .get(&key)
                    .or_else(|| app.config.projects.get(&key));
                let name = profile.and_then(|p| p.title.clone()).unwrap_or_else(|| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
                let color = profile
                    .and_then(|p| p.color.as_deref())
                    .and_then(|c| c.parse().ok())
                    .unwrap_or_else(|| super::view::stable_color(&key));
                let number = app
                    .marked
                    .iter()
                    .position(|p| config::key(p) == key)
                    .map(|n| format!("{:>2}", n + 1))
                    .unwrap_or_else(|| " □".into());
                let git = app.git.get(&key).and_then(|g| g.as_ref().ok());
                let failed = app.profile_errors.contains_key(&key)
                    || app.batch.items.iter().any(|i| {
                        config::key(&i.options.project) == key
                            && matches!(i.state, LaunchState::Failed(_))
                    });
                let (state, state_color) = if failed {
                    (t("failed"), palette().error)
                } else if git.is_some_and(|g| g.conflicts > 0) {
                    (t("overview.conflict"), palette().error)
                } else if app.workspaces.contains_key(&key) {
                    (t("running"), ratatui::style::Color::Green)
                } else if !app.host_error.is_empty() {
                    (t("overview.state_unknown"), palette().muted)
                } else if app.config.overview.pending.contains(&key) {
                    (t("parity.pending"), ratatui::style::Color::Yellow)
                } else if app
                    .config
                    .projects
                    .get(&key)
                    .is_some_and(|p| p.last_opened > 0)
                {
                    (t("overview.stopped"), palette().muted)
                } else {
                    (t("overview.not_started"), palette().muted)
                };
                let change = git.map_or_else(
                    || "—".into(),
                    |g| {
                        if g.changed == 0 {
                            t("overview.clean").into_owned()
                        } else {
                            format!(
                                "{} {}{}{}",
                                g.changed,
                                t("overview.changes"),
                                if g.ahead > 0 { " ↑" } else { "" },
                                if g.behind > 0 { " ↓" } else { "" }
                            )
                        }
                    },
                );
                let state_width = state.width().max(8);
                let changes_width = change.width().clamp(6, 14);
                let content_width =
                    (inner.width as usize).saturating_sub(9 + state_width + changes_width);
                let branch = git.map_or("", |g| {
                    if matches!(g.branch.as_str(), "main" | "master") {
                        ""
                    } else {
                        &g.branch
                    }
                });
                let branch_width = if !branch.is_empty() && content_width >= 32 {
                    14
                } else {
                    0
                };
                let name_width = content_width.saturating_sub(branch_width);
                let agent = app.staged.get(&key).map_or_else(
                    || {
                        profile
                            .and_then(|p| p.agent)
                            .or(app.config.default_agent)
                            .unwrap_or(Agent::Codex)
                    },
                    |o| o.agent,
                );
                let history = app.dashboard.contexts.get(&context_key(path, agent));
                let has_children = history.is_some_and(|h| !h.sessions.is_empty());
                let expanded = app.dashboard.expanded_projects.contains(&key);
                let arrow = if has_children {
                    if expanded { "▾" } else { "▸" }
                } else {
                    " "
                };
                if has_children {
                    app.dashboard
                        .expand_hits
                        .push((Rect::new(inner.x + 4, y, 1, 1), key.clone()));
                }
                let mut spans = vec![
                    Span::styled(
                        if selected { "▎" } else { " " },
                        Style::new().fg(palette().accent),
                    ),
                    Span::raw(format!("{number} {arrow}")),
                    Span::styled("■ ", Style::new().fg(color)),
                    Span::styled(
                        fit(
                            &format!(
                                "{}{}",
                                if profile.is_some_and(|p| p.favorite) {
                                    "★ "
                                } else {
                                    ""
                                },
                                name
                            ),
                            name_width,
                        ),
                        if selected {
                            Style::new().fg(palette().fg).bold()
                        } else {
                            Style::new().fg(palette().fg)
                        },
                    ),
                    Span::styled(
                        format!(" {}", fit(&state, state_width)),
                        Style::new().fg(state_color),
                    ),
                    Span::styled(
                        format!(" {}", fit(&change, changes_width)),
                        Style::new().fg(if git.is_some_and(|g| g.changed > 0) {
                            ratatui::style::Color::Yellow
                        } else {
                            palette().muted
                        }),
                    ),
                ];
                if branch_width > 0 {
                    spans.insert(
                        4,
                        Span::styled(
                            fit(&format!(" {branch}"), branch_width),
                            Style::new().fg(palette().accent),
                        ),
                    );
                }
                if !app.query.is_empty() {
                    let mut needle = app.query.chars().flat_map(char::to_lowercase).peekable();
                    let normal = spans[3].style;
                    let highlighted = spans[3]
                        .content
                        .chars()
                        .map(|c| {
                            let matched = needle
                                .peek()
                                .is_some_and(|q| c.to_lowercase().next() == Some(*q));
                            if matched {
                                needle.next();
                            }
                            Span::styled(
                                c.to_string(),
                                if matched {
                                    normal.fg(palette().accent).bold()
                                } else {
                                    normal
                                },
                            )
                        })
                        .collect::<Vec<_>>();
                    spans.splice(3..4, highlighted);
                }
                frame.render_widget(Paragraph::new(Line::from(spans)).style(base), hit);
            }
            Row::Task(path, workspace, task) => {
                if let Some(work) = app
                    .workspaces
                    .get(&config::key(path))
                    .into_iter()
                    .flatten()
                    .find(|w| w.id == *workspace)
                    && let Some(task) = work.tasks.iter().find(|t| t.id == *task)
                {
                    let agent = work.sessions.iter().find(|s| {
                        s.task.task_id == task.id && s.task.role == crate::tasks::Role::Agent
                    });
                    let state = agent.map_or_else(
                        || t("failed").into_owned(),
                        |s| {
                            if s.activity.phase != crate::activity::Phase::Unknown {
                                format!(
                                    "{} {}",
                                    if s.activity.pending() { "◆" } else { "" },
                                    t(s.activity.phase.label())
                                )
                            } else {
                                match &s.status {
                                    crate::session::Status::Running => t("running").into_owned(),
                                    crate::session::Status::Exited(c) => {
                                        format!("{} {c}", t("exited"))
                                    }
                                    crate::session::Status::Failed(e) => e.clone(),
                                }
                            }
                        },
                    );
                    frame.render_widget(
                        Paragraph::new(fit(
                            &format!(
                                "    {} {} · {} · {}",
                                if selected { "▸" } else { "↳" },
                                task.name,
                                task.branch,
                                state
                            ),
                            inner.width as usize,
                        ))
                        .style(base.fg(palette().accent)),
                        hit,
                    );
                }
            }
            Row::Session(path, agent, id) => {
                let c = app
                    .dashboard
                    .contexts
                    .get(&context_key(path, *agent))
                    .and_then(|h| h.sessions.iter().find(|c| c.id == *id));
                let label = c.map_or(id.as_str(), |c| {
                    if c.prompt.is_empty() {
                        &c.id
                    } else {
                        &c.prompt
                    }
                });
                let label = format!(
                    "    {} {} · {}",
                    if selected { "▸" } else { "↳" },
                    agent_name(*agent),
                    label
                );
                frame.render_widget(
                    Paragraph::new(fit(&label, inner.width as usize)).style(base.fg(palette().fg)),
                    hit,
                );
            }
        }
        y += h;
    }
    if app.rows.is_empty() {
        frame.render_widget(Paragraph::new(t("no_matches")), inner);
    }
    if app.rows.len() > inner.height as usize && inner.height > 0 {
        let thumb = inner.y
            + (offset * inner.height.saturating_sub(1) as usize
                / app.rows.len().saturating_sub(1).max(1)) as u16;
        for y in inner.y..inner.bottom() {
            frame.render_widget(
                Paragraph::new(if y == thumb { "█" } else { "│" })
                    .style(Style::new().fg(palette().muted)),
                Rect::new(rect.right() - 1, y, 1, 1),
            );
        }
    }
}
impl Overview {
    pub(super) fn dashboard_key(&mut self, w: &Workers, key: KeyEvent) -> Result<Option<Outcome>> {
        use Focus::*;
        if let Some(viewer) = self.dashboard.viewer.as_mut() {
            if matches!(key.code, KeyCode::Esc) && !viewer.modal() {
                self.dashboard.viewer = None;
            } else {
                let _ = viewer.key(key)?;
            }
            return Ok(Some(Outcome::Stay));
        }
        if self.dialog.is_some() {
            return Ok(None);
        }
        if key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && let Some(selection) = &self.dashboard.text_selection
        {
            crate::files::copy(selection.text())?;
            self.message = t("overview.copied").into_owned();
            return Ok(Some(Outcome::Stay));
        }
        if key.code == KeyCode::Esc && self.dashboard.text_selection.take().is_some() {
            return Ok(Some(Outcome::Stay));
        }
        self.dashboard.text_selection = None;
        self.dashboard.heatmap_hover = None;
        if (self.dashboard.overlay == Some(Stats) || self.dashboard.focus == Stats)
            && !key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        {
            match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.heatmap_action(HeatmapAction::Period(!self.dashboard.stats_year));
                    return Ok(Some(Outcome::Stay));
                }
                KeyCode::Char('z' | 'Z') => {
                    self.heatmap_action(HeatmapAction::Expand);
                    return Ok(Some(Outcome::Stay));
                }
                _ => {}
            }
        }
        if self.dashboard.focus == Projects && scroll_key(key.code).is_some() {
            self.dashboard.list_detached = false;
        }
        if key.code == KeyCode::Esc
            && (self.dashboard.drag.is_some()
                || self.dashboard.scrollbar.is_some()
                || self.dashboard.pressed.is_some())
        {
            self.dashboard.cancel(&mut self.config.overview);
            return Ok(Some(Outcome::Stay));
        }
        if self.dashboard.overlay.is_some() {
            if key.code == KeyCode::Esc {
                self.dashboard.overlay = None;
                return Ok(Some(Outcome::Stay));
            }
            let stats = self.dashboard.overlay == Some(Stats);
            if let Some(delta) = scroll_key(key.code) {
                self.scroll_panel(stats, delta);
                return Ok(Some(Outcome::Stay));
            }
            if key.code == KeyCode::Enter
                && !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
            {
                if !stats && let Some((file, line)) = self.dashboard.file_selected.clone() {
                    self.open_file(file, line)?;
                } else {
                    self.dashboard.expanded = !self.dashboard.expanded;
                }
                return Ok(Some(Outcome::Stay));
            }
            if key.code == KeyCode::Char('e')
                && !stats
                && !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
            {
                self.dashboard.expanded = !self.dashboard.expanded;
                return Ok(Some(Outcome::Stay));
            }
            if !matches!(key.code, KeyCode::F(_) | KeyCode::Enter)
                && !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
            {
                return Ok(Some(Outcome::Stay));
            }
        }
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            let visible = self.dashboard.visible();
            let i = visible
                .iter()
                .position(|f| *f == self.dashboard.focus)
                .unwrap_or(0);
            let backwards =
                key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
            self.dashboard.focus =
                visible[(i + if backwards { visible.len() - 1 } else { 1 }) % visible.len()];
            return Ok(Some(Outcome::Stay));
        }
        if self.dashboard.focus == Projects
            && key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
            && let Some(Row::Project(path)) = self.rows.get(self.selected)
        {
            let keypath = config::key(path);
            if key.code == KeyCode::Right {
                self.dashboard.expanded_projects.insert(keypath);
            } else {
                self.dashboard.expanded_projects.remove(&keypath);
            }
            self.rebuild();
            return Ok(Some(Outcome::Stay));
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Char('/') && !ctrl && self.dashboard.focus != Search {
            self.dashboard.focus = Search;
            return Ok(Some(Outcome::Stay));
        }
        if self.dashboard.focus == Search && !ctrl {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.dashboard.focus = Projects,
                KeyCode::Char(c) => {
                    append_bounded(&mut self.query, &c.to_string(), 256);
                    self.rebuild();
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    self.rebuild();
                }
                _ => return Ok(None),
            }
            return Ok(Some(Outcome::Stay));
        }
        if matches!(self.dashboard.focus, Detail | Stats) {
            if let Some(delta) = scroll_key(key.code) {
                self.scroll_panel(self.dashboard.focus == Stats, delta);
                return Ok(Some(Outcome::Stay));
            }
            if key.code == KeyCode::Char(' ') {
                self.scroll_panel(self.dashboard.focus == Stats, 10);
                return Ok(Some(Outcome::Stay));
            }
            if key.code == KeyCode::Char('e')
                && !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
            {
                self.dashboard.expanded = !self.dashboard.expanded;
                return Ok(Some(Outcome::Stay));
            }
            if key.code == KeyCode::Enter
                && !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
            {
                if let Some((file, line)) = self.dashboard.file_selected.clone() {
                    self.open_file(file, line)?;
                } else {
                    self.dashboard.expanded = !self.dashboard.expanded;
                }
                return Ok(Some(Outcome::Stay));
            }
        }
        if self.dashboard.focus == Queue && key.code == KeyCode::Delete && !self.marked.is_empty() {
            self.dashboard.undo = Some(self.marked.clone());
            self.marked.0.remove(0);
            return Ok(Some(Outcome::Stay));
        }
        if self.dashboard.focus != Projects && key.code == KeyCode::Char(' ') && !ctrl {
            return Ok(Some(Outcome::Stay));
        }
        if self.dashboard.focus == Queue
            && key.code == KeyCode::Enter
            && !key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        {
            self.dialog = Some(Dialog::Queue { selected: 0 });
            return Ok(Some(Outcome::Stay));
        }
        if ctrl && key.code == KeyCode::Char('z') {
            if let Some(previous) = self.dashboard.undo.take() {
                self.marked = previous;
            }
            return Ok(Some(Outcome::Stay));
        }
        if self.dashboard.focus == Projects
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
            && self.path().is_some()
        {
            let path = self.path().unwrap();
            let mut o = self.options_for(&path);
            o.agent = match (o.agent, key.code == KeyCode::Right) {
                (Agent::Codex, true) | (Agent::Shell, false) => Agent::Claude,
                (Agent::Claude, true) | (Agent::Codex, false) => Agent::Shell,
                _ => Agent::Codex,
            };
            o.resume_id = None;
            o.resume = false;
            if o.agent == Agent::Shell {
                o.continue_latest = false;
                o.fresh = true;
            }
            self.staged.insert(config::key(&path), o);
            return Ok(Some(Outcome::Stay));
        }
        if self.dashboard.focus == Projects && key.code == KeyCode::Char(' ') && !ctrl {
            self.dashboard.undo = Some(self.marked.clone());
        }
        // Existing printable-character search remains available and has a visible focus.
        if matches!(key.code, KeyCode::Char(c) if !c.is_whitespace())
            && !ctrl
            && !key.modifiers.contains(KeyModifiers::ALT)
        {
            self.dashboard.focus = Search;
        }
        let _ = w;
        Ok(None)
    }
    fn scroll_panel(&mut self, stats: bool, delta: i32) {
        self.dashboard.heatmap_hover = None;
        let (scroll, max) = if stats {
            (&mut self.dashboard.stats_scroll, self.dashboard.stats_max)
        } else {
            (&mut self.detail_scroll, self.dashboard.detail_max)
        };
        *scroll = i32::from(*scroll)
            .saturating_add(delta)
            .clamp(0, i32::from(max)) as u16;
    }
    fn heatmap_action(&mut self, action: HeatmapAction) {
        self.dashboard.focus = Focus::Stats;
        self.dashboard.heatmap_hover = None;
        match action {
            HeatmapAction::Period(year) => {
                self.dashboard.stats_year = year;
                self.dashboard.stats_scroll = 0;
                if year {
                    self.dashboard.overlay = Some(Focus::Stats);
                    if self.dashboard.size.width < 116 {
                        self.message = t("overview.activity_narrow").into_owned();
                    }
                }
            }
            HeatmapAction::Expand => {
                self.dashboard.overlay = if self.dashboard.overlay == Some(Focus::Stats) {
                    None
                } else {
                    Some(Focus::Stats)
                };
                self.dashboard.stats_scroll = 0;
            }
            HeatmapAction::Day { .. } => {
                if let Some(text) = action.description() {
                    self.message = text;
                }
            }
        }
    }
    pub(super) fn dashboard_mouse(
        &mut self,
        w: &Workers,
        mouse: crossterm::event::MouseEvent,
    ) -> Result<Option<Outcome>> {
        let point = (mouse.column, mouse.row).into();
        if let Some(viewer) = self.dashboard.viewer.as_mut() {
            let _ = viewer.mouse(mouse)?;
            return Ok(Some(Outcome::Stay));
        }
        if let Some(Dialog::Queue { selected }) = self.dialog.as_mut() {
            if let Some((area, first)) = self.dialog_list {
                let target = first + usize::from(mouse.row.saturating_sub(area.y));
                if area.contains(point) {
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left)
                            if target < self.marked.len()
                                && mouse.column >= area.right().saturating_sub(3) =>
                        {
                            self.dashboard.undo = Some(self.marked.clone());
                            self.marked.0.remove(target);
                            self.dashboard.queue_drag = None;
                        }
                        MouseEventKind::Down(MouseButton::Left) if target < self.marked.len() => {
                            *selected = target;
                            self.dashboard.queue_drag = Some(target);
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            if let Some(from) = self.dashboard.queue_drag.take()
                                && from < self.marked.len()
                                && target < self.marked.len()
                                && from != target
                            {
                                self.dashboard.undo = Some(self.marked.clone());
                                let entry = self.marked.0.remove(from);
                                self.marked.0.insert(target, entry);
                                *selected = target;
                            }
                        }
                        MouseEventKind::ScrollUp => *selected = selected.saturating_sub(3),
                        MouseEventKind::ScrollDown => {
                            *selected = (*selected + 3).min(self.marked.len().saturating_sub(1))
                        }
                        _ => {}
                    }
                }
            }
            if matches!(mouse.kind, MouseEventKind::Up(_)) {
                self.dashboard.queue_drag = None;
            }
            return Ok(Some(Outcome::Stay));
        }
        if self.dialog.is_some() {
            self.last_click = None;
            return Ok(None);
        }
        if self.dashboard.text_selection.is_none()
            && self.dashboard.drag.is_none()
            && self.dashboard.scrollbar.is_none()
            && !mouse.modifiers.contains(KeyModifiers::ALT)
        {
            let hit = self
                .dashboard
                .heatmap_hits
                .iter()
                .find(|(r, _)| r.contains(point))
                .copied();
            if matches!(mouse.kind, MouseEventKind::Moved) {
                self.dashboard.heatmap_hover =
                    hit.and_then(|(rect, action)| action.description().map(|text| (rect, text)));
            }
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                && let Some((_, action)) = hit
            {
                self.heatmap_action(action);
                self.last_click = None;
                return Ok(Some(Outcome::Stay));
            }
        }
        if self.dashboard.overlay.is_none()
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.dashboard.search_hit.is_some_and(|r| r.contains(point))
        {
            self.dashboard.focus = Focus::Search;
            self.dashboard.text_selection = None;
            self.last_click = None;
            return Ok(Some(Outcome::Stay));
        }
        if matches!(mouse.kind, MouseEventKind::Moved) {
            self.dashboard.divider_hover = self.dashboard.columns.and_then(|c| {
                c.dividers
                    .iter()
                    .position(|r| r.is_some_and(|r| r.contains(point)))
            });
        }
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            self.dashboard.text_selection = None;
        }
        if let Some(selection) = &mut self.dashboard.text_selection
            && selection.dragging
        {
            let r = selection.buffer.area;
            if matches!(
                mouse.kind,
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
            ) {
                selection.range.end = crate::selection::Point {
                    row: mouse.row.clamp(r.y, r.bottom() - 1),
                    col: mouse.column.clamp(r.x, r.right() - 1),
                };
                selection.dragging = !matches!(mouse.kind, MouseEventKind::Up(_));
            }
            return Ok(Some(Outcome::Stay));
        }
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            self.dashboard.text_selection = None;
            if mouse.modifiers.contains(KeyModifiers::ALT)
                && let Some(rect) = self
                    .detail_areas
                    .iter()
                    .copied()
                    .find(|r| r.inner(Margin::new(2, 1)).contains(point))
            {
                use ratatui::widgets::Widget;
                let stats = self.dashboard.overlay == Some(Focus::Stats)
                    || (self.dashboard.overlay.is_none()
                        && self
                            .dashboard
                            .columns
                            .is_some_and(|c| c.stats == Some(rect)));
                let lines = if stats {
                    super::view::stats_lines(self, rect.width.saturating_sub(4))
                } else {
                    super::view::detail_lines(self, rect.width.saturating_sub(4))
                };
                let inner = rect.inner(Margin::new(2, 1));
                let mut buffer = ratatui::buffer::Buffer::empty(inner);
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .scroll((
                        if stats {
                            self.dashboard.stats_scroll
                        } else {
                            self.detail_scroll
                        },
                        0,
                    ))
                    .render(inner, &mut buffer);
                let at = crate::selection::Point {
                    row: mouse.row,
                    col: mouse.column,
                };
                self.dashboard.text_selection = Some(TextSelection {
                    buffer,
                    range: crate::selection::Selection { start: at, end: at },
                    dragging: true,
                });
                self.last_click = None;
                return Ok(Some(Outcome::Stay));
            }
        }
        if self.dashboard.size.width < 60 || self.dashboard.size.height < 10 {
            return Ok(Some(Outcome::Stay));
        }
        if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left))
            && self
                .dashboard
                .pressed
                .is_some_and(|(_, x, y)| x.abs_diff(mouse.column) + y.abs_diff(mouse.row) > 2)
        {
            self.dashboard.press_dragged = true;
            self.last_click = None;
        }
        if let Some((focus, rect)) = self.dashboard.scrollbar {
            if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
                self.dashboard.scrollbar = None;
            } else if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left)) {
                self.drag_scrollbar(focus, rect, mouse.row);
            }
            return Ok(Some(Outcome::Stay));
        }
        if self.dashboard.overlay.is_none()
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self
                .dashboard
                .pending_hit
                .is_some_and(|r| r.contains(point))
        {
            return self.command(w, Command::Restore).map(Some);
        }
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some((_, action)) = self
                .dashboard
                .detail_hits
                .iter()
                .find(|(r, _)| r.contains(point))
                .cloned()
        {
            self.last_click = None;
            self.dashboard.focus = Focus::Detail;
            match action {
                DetailAction::Note => return self.command(w, Command::Note).map(Some),
                DetailAction::Options => return self.command(w, Command::Options).map(Some),
                DetailAction::History => return self.command(w, Command::History).map(Some),
                DetailAction::Expand => self.dashboard.expanded = !self.dashboard.expanded,
                DetailAction::File(path) => {
                    self.dashboard.file_selected = Some((path.clone(), None));
                    if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                        self.open_file(path, None)?;
                    }
                }
                DetailAction::Link(target) => {
                    if let Some(root) = self.path()
                        && let Some((path, line)) = local_reference(&root, &target)
                    {
                        self.dashboard.file_selected = Some((path.clone(), line));
                        if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                            self.open_file(path, line)?;
                        }
                    }
                }
                DetailAction::Instances => {
                    if let Some(path) = self.path() {
                        self.dialog = Some(Dialog::Instances { path, selected: 0 });
                    }
                }
                DetailAction::Diagnostics => {
                    return self.command(w, Command::Diagnostics).map(Some);
                }
                DetailAction::Resume(id) => {
                    if let Some(path) = self.path() {
                        let agent = self.options_for(&path).agent;
                        self.stage_resume(&path, agent, id);
                        self.detail_scroll = 0;
                    }
                }
            }
            return Ok(Some(Outcome::Stay));
        }
        if let Some((index, _, _)) = self.dashboard.drag {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    if let Some(c) = self.dashboard.columns {
                        let width = self
                            .dashboard
                            .size
                            .width
                            .saturating_sub(if c.stats.is_some() && c.detail.is_some() {
                                2
                            } else {
                                1
                            })
                            .max(1);
                        if index == 0 {
                            let min = if c.detail.is_some() && c.stats.is_some() {
                                44
                            } else {
                                42
                            };
                            let adjacent = c.detail.or(c.stats).map_or(60, |r| r.width);
                            let max = c.list.width + adjacent - 60;
                            let wanted = mouse
                                .column
                                .saturating_sub(c.list.x)
                                .clamp(min, max.max(min));
                            self.config.overview.list_ratio =
                                Some((u32::from(wanted) * 10000 / u32::from(width)) as u16);
                            if c.detail.is_some() && c.stats.is_some() {
                                self.config.overview.detail_ratio = Some(
                                    (u32::from(c.list.width + adjacent - wanted) * 10000
                                        / u32::from(width))
                                        as u16,
                                );
                            }
                        } else if let Some(r) = c.detail {
                            let max =
                                r.width + c.stats.map_or(0, |stats| stats.width) - MIN_STATS_WIDTH;
                            let wanted = mouse.column.saturating_sub(r.x).clamp(60, max);
                            self.config.overview.detail_ratio =
                                Some((u32::from(wanted) * 10000 / u32::from(width)).min(10000)
                                    as u16);
                        }
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.dashboard.drag = None;
                    self.edit(w, Edit::Preferences(self.config.overview.clone()))?;
                }
                _ => {}
            }
            return Ok(Some(Outcome::Stay));
        }
        if let Some(overlay) = self.dashboard.overlay {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(r) = self
                    .detail_areas
                    .first()
                    .copied()
                    .filter(|r| r.right() - 1 == mouse.column && r.contains(point))
            {
                self.dashboard.scrollbar = Some((overlay, r));
                self.drag_scrollbar(overlay, r, mouse.row);
            } else if matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            ) {
                self.scroll_panel(
                    overlay == Focus::Stats,
                    if mouse.kind == MouseEventKind::ScrollUp {
                        -3
                    } else {
                        3
                    },
                );
            } else if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                && !self.detail_areas.iter().any(|r| r.contains(point))
            {
                self.dashboard.overlay = None;
            }
            return Ok(Some(Outcome::Stay));
        }
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            if self.dashboard.add_hit.is_some_and(|r| r.contains(point)) {
                return self.command(w, Command::Add).map(Some);
            }
            if let Some((_, key)) = self
                .dashboard
                .expand_hits
                .iter()
                .find(|(r, _)| r.contains(point))
            {
                let key = key.clone();
                if !self.dashboard.expanded_projects.remove(&key) {
                    self.dashboard.expanded_projects.insert(key);
                }
                self.rebuild();
                self.last_click = None;
                return Ok(Some(Outcome::Stay));
            }
        }
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some(c) = self.dashboard.columns
            && let Some((i, _)) = c
                .dividers
                .iter()
                .enumerate()
                .find(|(_, r)| r.is_some_and(|r| r.contains(point)))
        {
            self.dashboard.drag = Some((
                i,
                self.config.overview.list_ratio,
                self.config.overview.detail_ratio,
            ));
            self.last_click = None;
            return Ok(Some(Outcome::Stay));
        }
        if let Some(c) = self.dashboard.columns {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                && let Some((_, i)) = self
                    .dashboard
                    .agent_hits
                    .iter()
                    .find(|(r, _)| r.contains(point))
            {
                self.selected = *i;
                self.dashboard.focus = Focus::Projects;
                self.last_click = None;
                self.dashboard.pressed = None;
                self.dashboard_key(w, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))?;
                return Ok(Some(Outcome::Stay));
            }
            if mouse.column == c.list.right() - 1
                && mouse.row > c.list.y
                && mouse.row < c.list.bottom() - 1
                && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            {
                self.dashboard.scrollbar = Some((Focus::Projects, c.list));
                self.drag_scrollbar(Focus::Projects, c.list, mouse.row);
                self.last_click = None;
                return Ok(Some(Outcome::Stay));
            }
            for (rect, focus) in [(c.detail, Focus::Detail), (c.stats, Focus::Stats)] {
                if let Some(rect) = rect.filter(|r| r.contains(point)) {
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                        self.dashboard.focus = focus;
                    }
                    if matches!(
                        mouse.kind,
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    ) {
                        self.scroll_panel(
                            focus == Focus::Stats,
                            if mouse.kind == MouseEventKind::ScrollUp {
                                -3
                            } else {
                                3
                            },
                        );
                    }
                    if mouse.column == rect.right() - 1
                        && matches!(
                            mouse.kind,
                            MouseEventKind::Down(MouseButton::Left)
                                | MouseEventKind::Drag(MouseButton::Left)
                        )
                    {
                        self.dashboard.scrollbar = Some((focus, rect));
                        self.drag_scrollbar(focus, rect, mouse.row);
                    }
                    return Ok(Some(Outcome::Stay));
                }
            }
        }
        if self.dashboard.summary.contains(point) {
            if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left))
                && self.dashboard.press_dragged
                && let Some(path) = self.dashboard.pressed_path.take()
            {
                self.dashboard.undo = Some(self.marked.clone());
                self.marked.push(path);
                self.dashboard.press_dragged = false;
                self.dashboard.pressed = None;
            } else if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.dashboard.focus = Focus::Queue;
                self.dialog = Some(Dialog::Queue { selected: 0 });
            }
            self.last_click = None;
            return Ok(Some(Outcome::Stay));
        }
        let hit = self
            .dashboard
            .row_hits
            .iter()
            .find(|(r, _)| r.contains(point))
            .map(|(_, i)| *i);
        if let Some(index) = hit {
            match mouse.kind {
                MouseEventKind::Moved => {
                    if self.dashboard.hover.as_ref().map(|(i, _)| *i) != Some(index) {
                        self.dashboard.hover = Some((index, Instant::now()));
                        self.dashboard.hover_displayed = false;
                    }
                }
                MouseEventKind::Down(
                    MouseButton::Left | MouseButton::Middle | MouseButton::Right,
                ) => {
                    let old = self.selected;
                    self.selected = index;
                    self.dashboard.focus = Focus::Projects;
                    self.dashboard.list_detached = false;
                    self.dashboard.pressed = Some((index, mouse.column, mouse.row));
                    self.dashboard.pressed_path = self.path();
                    self.dashboard.press_dragged = false;
                    let badge = mouse.column < self.list_area.x + 4;
                    if badge
                        || mouse.kind == MouseEventKind::Down(MouseButton::Middle)
                        || mouse.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        self.dashboard.undo = Some(self.marked.clone());
                        self.toggle_selected_queue();
                        self.last_click = None;
                    } else if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                        self.dashboard.undo = Some(self.marked.clone());
                        for i in old.min(index)..=old.max(index) {
                            if let Some(p) = self.rows.get(i).and_then(Row::path) {
                                self.marked.push(p.to_path_buf());
                            }
                        }
                        self.last_click = None;
                    } else if mouse.kind == MouseEventKind::Down(MouseButton::Right) {
                        self.last_click = None;
                        self.dialog = Some(Dialog::Palette {
                            query: String::new(),
                            selected: 0,
                            original: crate::appearance::theme(),
                        });
                    } else if matches!(self.rows[index], Row::Group(_)) {
                        self.last_click = None;
                        return self.command(w, Command::Group).map(Some);
                    } else if self.last_click.is_some_and(|(i, t)| {
                        i == index && t.elapsed() < Duration::from_millis(400)
                    }) {
                        self.last_click = None;
                        return self.command(w, Command::Open).map(Some);
                    } else {
                        self.last_click = Some((index, Instant::now()));
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if self.dashboard.pressed.is_some_and(|(_, x, y)| {
                        x.abs_diff(mouse.column) + y.abs_diff(mouse.row) > 2
                    }) {
                        self.last_click = None;
                        self.dashboard.press_dragged = true;
                    }
                }
                MouseEventKind::ScrollUp => {
                    self.dashboard.list_detached = true;
                    *self.list.offset_mut() = self.list.offset().saturating_sub(3);
                }
                MouseEventKind::ScrollDown => {
                    self.dashboard.list_detached = true;
                    *self.list.offset_mut() =
                        (self.list.offset() + 3).min(self.rows.len().saturating_sub(1));
                }
                MouseEventKind::Up(_) => {
                    self.dashboard.pressed = None;
                }
                _ => {}
            }
            return Ok(Some(Outcome::Stay));
        }
        if self.list_area.contains(point) {
            self.last_click = None;
            return Ok(Some(Outcome::Stay));
        }
        if matches!(mouse.kind, MouseEventKind::Moved) {
            self.dashboard.hover = None;
        }
        if matches!(mouse.kind, MouseEventKind::Up(_)) {
            self.dashboard.pressed = None;
        }
        Ok(None)
    }
    fn open_file(&mut self, path: PathBuf, line: Option<usize>) -> Result<()> {
        let root = self
            .path()
            .ok_or_else(|| anyhow::anyhow!("{}", t("no_project")))?;
        let mut viewer = Box::new(crate::sidebar::Sidebar::new(root));
        viewer.read_only = true;
        viewer.visible = false;
        viewer.enlarged = Some(crate::sidebar::Focus::Preview);
        viewer.open(path, true)?;
        viewer.update_layout(self.dashboard.size, false);
        self.dashboard.viewer = Some(viewer);
        self.dashboard.viewer_line = line;
        Ok(())
    }
    fn drag_scrollbar(&mut self, focus: Focus, rect: Rect, y: u16) {
        let max = match focus {
            Focus::Projects => self.rows.len().saturating_sub(1).min(u16::MAX as usize) as u16,
            Focus::Stats => self.dashboard.stats_max,
            _ => self.dashboard.detail_max,
        };
        let value = (u32::from(y.saturating_sub(rect.y + 1)) * u32::from(max)
            / u32::from(rect.height.saturating_sub(3).max(1)))
        .min(u32::from(max)) as u16;
        match focus {
            Focus::Projects => {
                self.dashboard.list_detached = true;
                *self.list.offset_mut() = usize::from(value);
            }
            Focus::Stats => self.dashboard.stats_scroll = value,
            _ => self.detail_scroll = value,
        }
    }
}
fn scroll_key(key: KeyCode) -> Option<i32> {
    match key {
        KeyCode::Up => Some(-1),
        KeyCode::Down => Some(1),
        KeyCode::PageUp => Some(-10),
        KeyCode::PageDown => Some(10),
        KeyCode::Home => Some(-i32::MAX),
        KeyCode::End => Some(i32::MAX),
        _ => None,
    }
}
