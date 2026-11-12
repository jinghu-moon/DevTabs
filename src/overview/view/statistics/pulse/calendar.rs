//! Fixed date buckets rendered without I/O. Geometry also drives mouse hits.
use super::*;
use crate::overview::dashboard::HeatmapAction;

pub(in crate::overview) struct Chart {
    pub lines: Vec<Line<'static>>,
    pub hits: Vec<(usize, usize, usize, HeatmapAction)>,
}

pub(in crate::overview) fn render(s: &stats::Stats, width: usize, today: u64, year: bool) -> Chart {
    let mut out = Chart {
        lines: Vec::new(),
        hits: Vec::new(),
    };
    if width == 0 {
        return out;
    }
    let ready =
        (s.source_ready || !s.weeks.is_empty() || !s.commits.is_empty()) && s.git_error.is_none();
    let label_width = (0..7)
        .map(|day| t(&format!("overview.weekday.{day}")).width())
        .max()
        .unwrap_or(0)
        + 1;
    let annual = year && width >= label_width + 106 && s.calendar.is_some();
    let (first, today, days) =
        if let Some(calendar) = s.calendar.as_ref().filter(|c| c.days.len() == 371) {
            let skip = if annual { 0 } else { 287 };
            (
                calendar.first_day + skip as u64,
                calendar.today,
                &calendar.days[skip..],
            )
        } else {
            (
                today.saturating_sub((today + 3) % 7 + 77),
                today,
                s.days.as_slice(),
            )
        };
    let weeks = days.len() / 7;
    let calendar_width = label_width + weeks * 2;
    // The calendar is a compact card within the statistics panel. Its controls
    // and legend must not inherit the full panel's potentially huge width.
    let width = width.min(calendar_width.max(48));
    let total = if days.is_empty() {
        s.weeks.iter().map(|n| u64::from(*n)).sum()
    } else {
        days.iter().map(|n| u64::from(*n)).sum()
    };
    let title = t(if annual {
        "overview.activity_year"
    } else {
        "overview.stats_weeks"
    });
    let extra = if ready {
        crate::i18n::format("overview.activity_commits", &[("count", count(total))])
    } else {
        String::new()
    };
    let buttons = if s.calendar.is_some() && width >= 26 {
        format!(
            "[{}] [+]",
            t(if annual {
                "overview.activity_recent_button"
            } else {
                "overview.activity_year_button"
            })
        )
    } else {
        String::new()
    };
    let header_width = width.saturating_sub(buttons.width() + usize::from(!buttons.is_empty()));
    let mut header = heading(&title, &extra, header_width);
    if !buttons.is_empty() {
        header.spans.push(Span::raw(" "));
        let start = header.width();
        let period = format!(
            "[{}]",
            t(if annual {
                "overview.activity_recent_button"
            } else {
                "overview.activity_year_button"
            })
        );
        header.spans.push(Span::styled(
            period.clone(),
            Style::new().fg(palette().accent),
        ));
        out.hits
            .push((0, start, period.width(), HeatmapAction::Period(!annual)));
        header.spans.push(Span::raw(" "));
        out.hits.push((0, header.width(), 3, HeatmapAction::Expand));
        header
            .spans
            .push(Span::styled("[+]", Style::new().fg(palette().accent)));
    }
    out.lines.push(header);
    if ready && title.width() + extra.width() + 5 > header_width {
        out.lines
            .push(Line::from(Span::styled(clip(&extra, width), strong())));
    }
    if ready {
        let latest = s
            .commits
            .first()
            .and_then(|c| c.split_whitespace().nth(1))
            .unwrap_or("—");
        let streak = s.weeks.iter().rev().take_while(|n| **n > 0).count();
        let mut summary = crate::i18n::format(
            "overview.pulse_summary",
            &[("date", latest.into()), ("streak", streak.to_string())],
        );
        if let Some(lifetime) = s.total_commits.filter(|n| *n != total) {
            summary.push_str(" · ");
            summary.push_str(&crate::i18n::format(
                "overview.activity_total",
                &[("total", count(lifetime))],
            ));
        }
        for text in textwrap::wrap(&summary, width.max(1)) {
            out.lines
                .push(Line::from(Span::styled(clip(&text, width), dim())));
        }
    }
    if let Some(error) = &s.git_error {
        out.lines
            .push(Line::from(Span::styled(clip(error, width), dim())));
        return out;
    }
    if s.weeks.is_empty() {
        out.lines
            .push(Line::from(Span::styled(clip(&t("loading"), width), dim())));
        return out;
    }
    if !matches!(weeks, 12 | 53) || width < calendar_width {
        let max = s.weeks.iter().copied().max().unwrap_or(1).max(1);
        let glyphs = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        out.lines.push(Line::from(
            s.weeks
                .iter()
                .take(width.min(12))
                .map(|n| {
                    Span::styled(
                        glyphs[(u64::from(*n) * 7 / u64::from(max)) as usize].to_string(),
                        Style::new().fg(if *n == 0 { palette().muted } else { green() }),
                    )
                })
                .collect::<Vec<_>>(),
        ));
        return out;
    }
    let cell_width = 2;
    let step = cell_width;
    let grid_width = weeks * step;
    // Keep years on their own axis so a long year prefix never hides a month.
    let mut months = Vec::new();
    let mut years = Vec::new();
    let mut previous = None;
    for week in 0..weeks {
        let (year, month, _) = crate::utc::date((first + week as u64 * 7 + 6).min(today));
        if previous == Some((year, month)) {
            continue;
        }
        if previous.is_none() || previous.is_some_and(|(old_year, _)| old_year != year) {
            years.push((week * step, year.to_string()));
        }
        months.push((
            week * step,
            t(&format!("overview.month.{month}")).into_owned(),
        ));
        previous = Some((year, month));
    }
    for labels in [&years, &months] {
        let mut axis = " ".repeat(label_width);
        let mut used = 0;
        for (index, (position, text)) in labels.iter().enumerate() {
            let next = labels.get(index + 1).map_or(grid_width, |(pos, _)| *pos);
            // A partial opening month can be shorter than its label. Preserve
            // the next month at its correct week instead of silently hiding it.
            if *position >= used && position + text.width() <= next {
                axis.push_str(&" ".repeat(position - used));
                axis.push_str(text);
                used = position + text.width();
            }
        }
        out.lines.push(Line::from(Span::styled(axis, dim())));
    }
    let max = days.iter().copied().max().unwrap_or(0).max(1);
    for day in 0..7 {
        let row_index = out.lines.len();
        let mut row = vec![Span::styled(
            fit(&t(&format!("overview.weekday.{day}")), label_width),
            dim(),
        )];
        for week in 0..weeks {
            let index = week * 7 + day;
            let date = first + index as u64;
            let future = date > today;
            let n = days[index];
            row.push(if future {
                Span::styled("□ ", dim())
            } else {
                let level = if n == 0 {
                    0
                } else {
                    1 + (u64::from(n) * 3 / u64::from(max)) as usize
                };
                Span::styled("██", Style::new().fg(activity_color(level)))
            });
            out.hits.push((
                row_index,
                label_width + week * step,
                cell_width,
                HeatmapAction::Day {
                    day: date,
                    commits: if future { None } else { Some(n) },
                },
            ));
        }
        out.lines.push(Line::from(row));
    }
    out.lines.push(Line::default());
    let mut legend = vec![Span::styled(
        format!("{} ", t("overview.activity_low")),
        dim(),
    )];
    for level in 0..5 {
        legend.push(Span::styled("■", Style::new().fg(activity_color(level))));
    }
    legend.push(Span::styled(
        format!(" {}", t("overview.activity_high")),
        dim(),
    ));
    let legend_width = Line::from(legend.clone()).width();
    let mut footer = vec![
        Span::styled("UTC", dim()),
        Span::raw(" ".repeat(calendar_width.saturating_sub(3 + legend_width))),
    ];
    footer.extend(legend);
    out.lines.push(Line::from(footer));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> stats::Stats {
        let mut days = vec![0; 371];
        days[0] = 7;
        days[287] = 3;
        days[367] = 10;
        stats::Stats {
            source_ready: true,
            weeks: days[287..]
                .chunks(7)
                .map(|week| week.iter().sum())
                .collect(),
            days: days[287..].to_vec(),
            calendar: Some(stats::Calendar {
                first_day: 20_402,
                today: 20_769,
                days,
            }),
            ..Default::default()
        }
    }
    #[test]
    fn compact_rows_and_controls_do_not_stretch_to_the_panel_width() {
        for year in [false, true] {
            let chart = render(&fixture(), 180, 20_769, year);
            let cells: Vec<_> = chart
                .hits
                .iter()
                .filter(|(_, _, _, action)| matches!(action, HeatmapAction::Day { .. }))
                .collect();
            let weeks = if year { 53 } else { 12 };
            assert_eq!(cells.len(), weeks * 7);
            let mut rows: Vec<_> = cells.iter().map(|(row, _, _, _)| *row).collect();
            rows.sort_unstable();
            rows.dedup();
            assert_eq!(rows.len(), 7);
            assert!(
                rows.windows(2).all(|pair| pair[1] == pair[0] + 1),
                "remove blank rows between weekdays"
            );
            assert_eq!(cells[1].1, cells[0].1 + 2, "contiguous two-character cells");
            let grid_right = cells
                .iter()
                .map(|(_, col, width, _)| col + width)
                .max()
                .unwrap();
            let legend = chart
                .lines
                .iter()
                .find(|line| {
                    line.to_string()
                        .contains(t("overview.activity_low").as_ref())
                })
                .unwrap();
            assert_eq!(legend.width(), grid_right, "legend belongs to the calendar");
            assert!(
                chart
                    .lines
                    .iter()
                    .all(|line| line.width() <= grid_right.max(48)),
                "buttons must stay near the chart"
            );
        }
    }

    #[test]
    fn yearly_and_recent_views_share_anchored_day_buckets_across_midnight() {
        let stats = fixture();
        let actions = |year| {
            render(&stats, 176, 20_770, year)
                .hits
                .into_iter()
                .filter_map(|(_, _, _, hit)| {
                    if let HeatmapAction::Day { day, commits } = hit {
                        Some((day, commits))
                    } else {
                        None
                    }
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let year = actions(true);
        let recent = actions(false);
        assert_eq!(year.len(), 371);
        assert_eq!(recent.len(), 84);
        assert_eq!(year[&20_402], Some(7));
        assert_eq!(recent[&20_689], Some(3));
        assert_eq!(year[&20_769], Some(10));
        assert_eq!(
            year[&20_770], None,
            "cached calendar does not shift overnight"
        );
        assert!(recent.iter().all(|(day, commits)| year[day] == *commits));
    }
    #[test]
    fn all_widths_themes_and_locales_keep_geometry_and_month_labels_inside_the_card() {
        struct Restore(crate::appearance::Theme);
        impl Drop for Restore {
            fn drop(&mut self) {
                crate::appearance::set(self.0);
            }
        }
        let _restore = Restore(crate::appearance::theme());
        for theme in [
            crate::appearance::Theme::Terminal,
            crate::appearance::Theme::Tokyo,
            crate::appearance::Theme::Catppuccin,
            crate::appearance::Theme::Light,
        ] {
            crate::appearance::set(theme);
            for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
                crate::i18n::with_language(language, || {
                    for width in 0..=180 {
                        for year in [false, true] {
                            let chart = render(&fixture(), width, 20_769, year);
                            assert!(
                                chart.lines.iter().all(|line| line.width() <= width),
                                "{theme:?} {language:?} {width} {year}"
                            );
                            assert!(
                                chart
                                    .hits
                                    .iter()
                                    .all(|(row, col, size, _)| *row < chart.lines.len()
                                        && col + size <= width)
                            );
                        }
                    }
                    let chart = render(&fixture(), 176, 20_769, true);
                    let text = chart
                        .lines
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n");
                    assert!(text.contains("2025"));
                    assert!(text.contains("2026"));
                    assert!(text.contains(t("overview.month.1").as_ref()));
                    assert!(text.contains(t("overview.month.11").as_ref()));
                    assert!(!text.contains("%{"));
                    let cell = chart
                        .hits
                        .iter()
                        .find(|(_, _, _, action)| {
                            *action
                                == HeatmapAction::Day {
                                    day: 20_769,
                                    commits: Some(10),
                                }
                        })
                        .unwrap();
                    // Verify the restored contiguous foreground cells in a real buffer.
                    use ratatui::widgets::Widget;
                    let area = Rect::new(0, 0, 176, chart.lines.len() as u16);
                    let mut buffer = ratatui::buffer::Buffer::empty(area);
                    Paragraph::new(chart.lines).render(area, &mut buffer);
                    assert_eq!(buffer[(cell.1 as u16, cell.0 as u16)].fg, activity_color(4));
                    assert_eq!(buffer[(cell.1 as u16, cell.0 as u16)].symbol(), "█");
                    assert_eq!(
                        buffer[(cell.1 as u16 + 1, cell.0 as u16)].fg,
                        activity_color(4)
                    );
                    assert_eq!(buffer[(cell.1 as u16, cell.0 as u16)].bg, Color::Reset);
                });
            }
        }
    }
    #[test]
    fn compact_year_request_falls_back_to_twelve_weeks_without_clipping() {
        let chart = render(&fixture(), 60, 20_769, true);
        assert_eq!(
            chart
                .hits
                .iter()
                .filter(|(_, _, _, action)| matches!(action, HeatmapAction::Day { .. }))
                .count(),
            84
        );
        assert!(
            chart
                .hits
                .iter()
                .any(|(_, _, _, action)| *action == HeatmapAction::Period(true))
        );
        let error = stats::Stats {
            git_error: Some("Unavailable".into()),
            ..fixture()
        };
        assert!(
            !render(&error, 176, 20_769, true)
                .hits
                .iter()
                .any(|(_, _, _, action)| matches!(action, HeatmapAction::Day { .. }))
        );
    }
}
