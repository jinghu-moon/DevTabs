//! Project pulse uses background Git data only; no commands run during drawing.
use super::*;
pub(super) fn lines(s: &stats::Stats, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![heading(&t("overview.pulse"), "", width)];
    let ready =
        (s.source_ready || !s.weeks.is_empty() || !s.commits.is_empty()) && s.git_error.is_none();
    let active = s.weeks.iter().filter(|n| **n > 0).count();
    let streak = s.weeks.iter().rev().take_while(|n| **n > 0).count();
    let latest = s
        .commits
        .first()
        .and_then(|c| c.split_whitespace().nth(1))
        .unwrap_or("—");
    let total = s.total_commits.map_or_else(|| "—".into(), count);
    let summary = crate::i18n::format(
        "overview.pulse_summary",
        &[
            ("date", latest.into()),
            (
                "active",
                if ready {
                    active.to_string()
                } else {
                    "—".into()
                },
            ),
            ("total", total),
        ],
    );
    for text in textwrap::wrap(&summary, width.max(1)) {
        lines.push(Line::from(Span::styled(text.into_owned(), strong())));
    }
    lines.push(Line::default());
    let recent_total = crate::i18n::format(
        "overview.stats_commit_count",
        &[(
            "count",
            s.weeks
                .iter()
                .map(|n| u64::from(*n))
                .sum::<u64>()
                .to_string(),
        )],
    );
    lines.push(heading(
        &t("overview.stats_weeks"),
        if ready { &recent_total } else { "" },
        width,
    ));
    if ready && t("overview.stats_weeks").width() + recent_total.width() + 5 > width {
        lines.push(Line::from(Span::styled(clip(&recent_total, width), dim())));
    }
    if let Some(error) = &s.git_error {
        lines.push(Line::from(Span::styled(clip(error, width), dim())));
    } else if s.weeks.is_empty() {
        lines.push(Line::from(Span::styled(clip(&t("loading"), width), dim())));
    } else {
        let max = s.days.iter().copied().max().unwrap_or(0).max(1);
        if s.days.len() == 84 && width >= 28 {
            let today = crate::config::now() / 86400;
            let monday = today - (today + 3) % 7;
            let first = monday.saturating_sub(77);
            for day in 0..7 {
                let mut row = vec![Span::styled(
                    format!("{} ", t(&format!("overview.weekday.{day}"))),
                    dim(),
                )];
                for week in 0..12 {
                    let index = week * 7 + day;
                    let n = s.days[index];
                    let future = first + index as u64 > today;
                    let level = (u64::from(n) * 3 / u64::from(max)) as usize;
                    let color = if n == 0 {
                        palette().selected
                    } else if crate::appearance::theme() == crate::appearance::Theme::Light {
                        [
                            Color::Rgb(155, 210, 165),
                            Color::Rgb(95, 180, 125),
                            Color::Rgb(40, 140, 85),
                            Color::Rgb(0, 100, 45),
                        ][level]
                    } else {
                        [
                            Color::Rgb(45, 85, 65),
                            Color::Rgb(65, 125, 80),
                            Color::Rgb(105, 165, 100),
                            Color::Rgb(165, 210, 130),
                        ][level]
                    };
                    row.push(Span::styled(
                        if future { "  " } else { "██" },
                        Style::new().fg(color),
                    ));
                }
                lines.push(Line::from(row));
            }
            lines.push(Line::from(Span::styled(
                clip(&t("overview.activity_legend"), width),
                dim(),
            )));
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
        let peak = s.weeks.iter().max().copied().unwrap_or(0);
        let week = s
            .weeks
            .iter()
            .rposition(|n| *n == peak)
            .map(|i| {
                if i == 11 {
                    t("overview.this_week").into_owned()
                } else {
                    crate::i18n::format("overview.weeks_ago", &[("count", (11 - i).to_string())])
                }
            })
            .unwrap_or_else(|| "—".into());
        let note = crate::i18n::format(
            "overview.activity_note",
            &[
                ("peak", peak.to_string()),
                ("streak", streak.to_string()),
                ("week", week),
            ],
        );
        for text in textwrap::wrap(&note, width.max(1)) {
            lines.push(Line::from(Span::styled(text.into_owned(), dim())));
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
