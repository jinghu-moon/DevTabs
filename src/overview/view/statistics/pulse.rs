//! Project pulse uses background Git data only; no commands run during drawing.
use super::*;
mod calendar;
pub(super) use calendar::render as chart;

pub(super) fn lines_for(s: &stats::Stats, width: usize, year: bool) -> Vec<Line<'static>> {
    lines_at(s, width, crate::config::now() / 86400, year)
}
fn lines_at(s: &stats::Stats, width: usize, today: u64, year: bool) -> Vec<Line<'static>> {
    let mut lines = calendar::render(s, width, today, year).lines;
    let ready =
        (s.source_ready || !s.weeks.is_empty() || !s.commits.is_empty()) && s.git_error.is_none();
    let times = t("overview.times");
    let binary = t("overview.binary");
    // Both rankings share column widths; file names never move numeric columns.
    let visible = || {
        s.hotspots
            .iter()
            .take(5)
            .chain(s.directory_hotspots.iter().take(3))
    };
    let count_width = visible()
        .map(|h| h.commits.to_string().len() + times.width())
        .max()
        .unwrap_or(0);
    let added_width = visible()
        .filter(|h| !h.binary)
        .map(|h| h.added.to_string().len() + 1)
        .max()
        .unwrap_or(2);
    let mut removed_width = visible()
        .filter(|h| !h.binary)
        .map(|h| h.removed.to_string().len() + 1)
        .max()
        .unwrap_or(2);
    if visible().any(|h| h.binary) {
        removed_width = removed_width.max(binary.width().saturating_sub(added_width + 1));
    }
    let numeric_width = count_width + added_width + removed_width + 2;
    let right = |text: &str, width: usize| {
        format!("{}{}", " ".repeat(width.saturating_sub(text.width())), text)
    };
    for (label, rows) in [
        ("overview.file_hotspots", &s.hotspots),
        ("overview.directory_hotspots", &s.directory_hotspots),
    ] {
        lines.push(Line::default());
        lines.push(heading(&t(label), "", width));
        if let Some(error) = s.hotspots_error.as_ref().or(s.git_error.as_ref()) {
            lines.push(Line::from(Span::styled(clip(error, width), dim())));
        } else if !ready {
            lines.push(Line::from(Span::styled(clip(&t("loading"), width), dim())));
        } else if rows.is_empty() {
            lines.push(Line::from(Span::styled(
                clip(&t("overview.no_hotspots"), width),
                dim(),
            )));
        }
        for h in rows.iter().take(if label == "overview.file_hotspots" {
            5
        } else {
            3
        }) {
            let mut metrics = vec![Span::styled(
                right(&format!("{}{times}", h.commits), count_width),
                dim(),
            )];
            metrics.push(Span::raw(" "));
            if h.binary {
                metrics.push(Span::styled(
                    right(&binary, added_width + 1 + removed_width),
                    dim(),
                ));
            } else {
                metrics.push(Span::styled(
                    right(&format!("+{}", h.added), added_width),
                    Style::new().fg(green()),
                ));
                metrics.push(Span::raw(" "));
                metrics.push(Span::styled(
                    right(&format!("-{}", h.removed), removed_width),
                    Style::new().fg(palette().error),
                ));
            }
            if numeric_width + 9 <= width {
                let mut row = vec![
                    Span::raw(fit(&h.path, width - numeric_width - 1)),
                    Span::raw(" "),
                ];
                row.extend(metrics);
                lines.push(Line::from(row));
            } else {
                lines.push(Line::from(clip(&h.path, width)));
                if numeric_width <= width {
                    let mut row = vec![Span::raw(" ".repeat(width - numeric_width))];
                    row.extend(metrics);
                    lines.push(Line::from(row));
                } else {
                    lines.push(Line::from(clip(&Line::from(metrics).to_string(), width)));
                }
            }
        }
        if s.hotspots_limited {
            lines.push(Line::from(Span::styled(
                clip(&t("overview.hotspots_limited"), width),
                Style::new().fg(amber()),
            )));
        }
    }
    lines.push(Line::default());
    lines
}

pub(super) fn activity_color(level: usize) -> Color {
    if level == 0 {
        palette().selected
    } else if crate::appearance::theme() == crate::appearance::Theme::Light {
        [
            Color::Rgb(155, 210, 165),
            Color::Rgb(95, 180, 125),
            Color::Rgb(40, 140, 85),
            Color::Rgb(0, 100, 45),
        ][level - 1]
    } else {
        [
            Color::Rgb(45, 85, 65),
            Color::Rgb(65, 125, 80),
            Color::Rgb(105, 165, 100),
            Color::Rgb(165, 210, 130),
        ][level - 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> stats::Stats {
        let mut days = vec![0; 84];
        days[70] = 143;
        days[77] = 27;
        stats::Stats {
            source_ready: true,
            weeks: days.chunks(7).map(|week| week.iter().sum()).collect(),
            days,
            commits: vec!["abcd123 2026-11-12 Summary".into()],
            total_commits: Some(170),
            ..Default::default()
        }
    }

    #[test]
    fn calendar_keeps_compact_cells_month_labels_and_a_local_legend() {
        for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
            crate::i18n::with_language(language, || {
                let rendered = lines_at(&fixture(), 76, 20_769, false);
                let rows: Vec<_> = rendered
                    .iter()
                    .filter(|line| {
                        line.spans.len() == 13
                            && line.spans[1..]
                                .iter()
                                .all(|span| span.content == "  " && span.style.bg.is_some())
                    })
                    .collect();
                assert_eq!(rows.len(), 7);
                assert!(rows.iter().all(|line| line.width() <= 27));
                let text = rendered
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(!text.contains("··"));
                assert!(!text.contains("□"));
                assert!(text.contains(if language == crate::i18n::Language::ZhCn {
                    "11月"
                } else {
                    "Nov"
                }));
                assert!(
                    text.contains(t("overview.month.9").as_ref()),
                    "do not lose September behind the year label"
                );
                let legend = rendered
                    .iter()
                    .find(|line| {
                        line.to_string()
                            .contains(t("overview.activity_low").as_ref())
                    })
                    .unwrap();
                let colors: Vec<_> = legend
                    .spans
                    .iter()
                    .filter(|span| span.style.bg.is_some())
                    .collect();
                assert_eq!(colors.len(), 5);
                for (level, color) in colors.iter().enumerate() {
                    assert_eq!(color.content, "  ");
                    assert_eq!(color.style.bg, Some(activity_color(level)));
                }
                assert_eq!(
                    legend.width(),
                    if language == crate::i18n::Language::ZhCn {
                        27
                    } else {
                        26
                    }
                );
            });
        }
    }

    #[test]
    fn every_weekday_keeps_twelve_visible_cells_and_distinguishes_future_days() {
        for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
            crate::i18n::with_language(language, || {
                // 2026-11-09 Monday, then all seven possible current weekdays.
                for elapsed in 0..7 {
                    let today = 20_766 + elapsed;
                    assert_eq!((today + 3) % 7, elapsed);
                    let chart = calendar::render(&fixture(), 76, today, false);
                    let cells: Vec<_> = chart
                        .hits
                        .iter()
                        .filter_map(|(row, col, width, action)| {
                            if let crate::overview::dashboard::HeatmapAction::Day { day, commits } =
                                action
                            {
                                Some((*row, *col, *width, *day, *commits))
                            } else {
                                None
                            }
                        })
                        .collect();
                    assert_eq!(cells.len(), 84);
                    for (row, col, width, day, commits) in cells {
                        assert_eq!(commits.is_none(), day > today);
                        let line = &chart.lines[row];
                        let mut offset = 0;
                        let cell = line
                            .spans
                            .iter()
                            .find(|span| {
                                let found = offset == col;
                                offset += span.width();
                                found
                            })
                            .unwrap();
                        assert_eq!(cell.width(), width);
                        if day > today {
                            assert_eq!(cell.content, "  ");
                            assert_eq!(cell.style.bg, Some(activity_color(0)));
                            assert!(cell.style.fg.is_none());
                        } else {
                            assert_eq!(cell.content, "  ");
                            assert!(cell.style.bg.is_some());
                        }
                    }
                }
            });
        }
    }

    #[test]
    fn activity_card_removes_duplicate_titles_counts_and_long_explanations() {
        for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
            crate::i18n::with_language(language, || {
                let rendered = lines_at(&fixture(), 76, 20_769, false);
                let text = rendered
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(!text.contains(t("overview.pulse").as_ref()));
                assert_eq!(text.matches("170").count(), 1, "duplicate count");
                assert!(text.contains("2026-11-12"));
                assert!(text.contains("UTC"));
                let first_hotspots = rendered
                    .iter()
                    .position(|line| {
                        line.to_string()
                            .contains(t("overview.file_hotspots").as_ref())
                    })
                    .unwrap();
                assert!(
                    first_hotspots <= 23,
                    "too many explanatory rows: {first_hotspots}"
                );
                let mut historical = fixture();
                historical.total_commits = Some(1_000);
                let text = lines_at(&historical, 76, 20_769, false)
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("1,000"), "retain distinct lifetime total");
            });
        }
    }
}
