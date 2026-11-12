//! Project pulse uses background Git data only; no commands run during drawing.
use super::*;
pub(super) fn lines(s: &stats::Stats, width: usize) -> Vec<Line<'static>> {
    lines_at(s, width, crate::config::now() / 86400)
}
fn lines_at(s: &stats::Stats, width: usize, today: u64) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let ready =
        (s.source_ready || !s.weeks.is_empty() || !s.commits.is_empty()) && s.git_error.is_none();
    let streak = s.weeks.iter().rev().take_while(|n| **n > 0).count();
    let latest = s
        .commits
        .first()
        .and_then(|c| c.split_whitespace().nth(1))
        .unwrap_or("—");
    let recent = s.weeks.iter().map(|n| u64::from(*n)).sum::<u64>();
    let recent_total =
        crate::i18n::format("overview.stats_commit_count", &[("count", count(recent))]);
    lines.push(heading(
        &t("overview.stats_weeks"),
        if ready { &recent_total } else { "" },
        width,
    ));
    if ready && t("overview.stats_weeks").width() + recent_total.width() + 5 > width {
        lines.push(Line::from(Span::styled(clip(&recent_total, width), dim())));
    }
    if ready {
        let mut summary = crate::i18n::format(
            "overview.pulse_summary",
            &[("date", latest.into()), ("streak", streak.to_string())],
        );
        // The heading already shows the period total. Keep the lifetime total
        // only when it adds information instead of repeating the same number.
        if let Some(total) = s.total_commits.filter(|total| *total != recent) {
            summary.push_str(" · ");
            summary.push_str(&crate::i18n::format(
                "overview.activity_total",
                &[("total", count(total))],
            ));
        }
        for text in textwrap::wrap(&summary, width.max(1)) {
            lines.push(Line::from(Span::styled(text.into_owned(), dim())));
        }
    }
    if let Some(error) = &s.git_error {
        lines.push(Line::from(Span::styled(clip(error, width), dim())));
    } else if s.weeks.is_empty() {
        lines.push(Line::from(Span::styled(clip(&t("loading"), width), dim())));
    } else {
        let max = s.days.iter().copied().max().unwrap_or(0).max(1);
        if s.days.len() == 84 && width >= 28 {
            let monday = today - (today + 3) % 7;
            let first = monday.saturating_sub(77);
            let label_width = (0..7)
                .map(|day| t(&format!("overview.weekday.{day}")).width())
                .max()
                .unwrap_or(0)
                + 1;
            let start = crate::i18n::format("overview.weeks_ago", &[("count", "11".into())]);
            let end = t("overview.this_week");
            lines.push(Line::from(Span::styled(
                format!(
                    "{}{start}{}{end}",
                    " ".repeat(label_width),
                    " ".repeat(24usize.saturating_sub(start.width() + end.width())),
                ),
                dim(),
            )));
            for day in 0..7 {
                let mut row = vec![Span::styled(
                    fit(&t(&format!("overview.weekday.{day}")), label_width),
                    dim(),
                )];
                for week in 0..12 {
                    let index = week * 7 + day;
                    let n = s.days[index];
                    let future = first + index as u64 > today;
                    let color = if future {
                        palette().muted
                    } else {
                        activity_color(if n == 0 {
                            0
                        } else {
                            1 + (u64::from(n) * 3 / u64::from(max)) as usize
                        })
                    };
                    row.push(Span::styled(
                        if future { "··" } else { "██" },
                        Style::new().fg(color),
                    ));
                }
                lines.push(Line::from(row));
            }
            let mut legend = vec![Span::styled(
                format!("UTC {} ", t("overview.activity_low")),
                dim(),
            )];
            legend.extend(
                (0..5).map(|level| Span::styled("■", Style::new().fg(activity_color(level)))),
            );
            legend.push(Span::styled(
                format!(
                    " {} ·· {}",
                    t("overview.activity_high"),
                    t("overview.activity_future")
                ),
                dim(),
            ));
            lines.push(Line::from(legend));
        } else {
            let max = s.weeks.iter().copied().max().unwrap_or(1).max(1);
            let glyphs = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
            lines.push(Line::from(
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
        }
    }
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

fn activity_color(level: usize) -> Color {
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
    fn every_weekday_keeps_twelve_visible_cells_and_distinguishes_future_days() {
        for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
            crate::i18n::with_language(language, || {
                // 2026-11-09 Monday, then all seven possible current weekdays.
                for elapsed in 0..7 {
                    let today = 20_766 + elapsed;
                    assert_eq!((today + 3) % 7, elapsed);
                    let rendered = lines_at(&fixture(), 76, today);
                    let rows: Vec<_> = rendered
                        .iter()
                        .filter(|line| {
                            line.spans.len() == 13
                                && line.spans[1..]
                                    .iter()
                                    .all(|span| matches!(span.content.as_ref(), "██" | "··" | "  "))
                        })
                        .collect();
                    assert_eq!(rows.len(), 7);
                    for (day, row) in rows.iter().enumerate() {
                        for (week, cell) in row.spans[1..].iter().enumerate() {
                            if week == 11 && day as u64 > elapsed {
                                assert_eq!(cell.content, "··", "missing future day {day}");
                                assert_ne!(cell.style.fg, row.spans[1].style.fg);
                            } else {
                                assert_eq!(cell.content, "██");
                            }
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
                let rendered = lines_at(&fixture(), 76, 20_769);
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
                    first_hotspots <= 13,
                    "too many explanatory rows: {first_hotspots}"
                );
                let mut historical = fixture();
                historical.total_commits = Some(1_000);
                let text = lines_at(&historical, 76, 20_769)
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("1,000"), "retain distinct lifetime total");
            });
        }
    }
}
