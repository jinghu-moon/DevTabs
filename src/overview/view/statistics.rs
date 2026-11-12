//! Width-aware statistics card. Collection and cache semantics live in stats.rs.
mod pulse;
use super::*;
use ratatui::style::Color;
use unicode_width::UnicodeWidthStr;

fn clip(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    crate::preview::truncate_name(&crate::preview::safe_text(text), width)
}
fn fit(text: &str, width: usize) -> String {
    super::super::dashboard::fit(text, width)
}
fn dim() -> Style {
    Style::new().fg(palette().muted)
}
fn strong() -> Style {
    Style::new().fg(palette().fg).add_modifier(Modifier::BOLD)
}
fn heading(title: &str, extra: &str, width: usize) -> Line<'static> {
    let title = clip(title, width);
    let mut spans = vec![Span::styled(title, strong())];
    let used = Line::from(spans.clone()).width();
    let extra = format!(" · {extra}");
    if extra != " · " && used + extra.width() + 2 <= width {
        spans.push(Span::styled(extra, dim()));
    }
    let used = Line::from(spans.clone()).width();
    if used < width {
        spans.push(Span::styled(
            format!(" {}", "─".repeat(width.saturating_sub(used + 1))),
            dim(),
        ));
    }
    Line::from(spans)
}
fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub(super) fn lines(s: &stats::Stats, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![];
    }
    let mut lines = pulse::lines(s, width);
    lines.push(heading(&t("overview.codebase"), "", width));
    if s.disk_scanning {
        let message = crate::i18n::format(
            "overview.stats_scanning",
            &[("count", count(s.scanned_entries))],
        );
        for text in textwrap::wrap(&message, width.max(1)) {
            lines.push(Line::from(Span::styled(text.into_owned(), dim())));
        }
    }
    if s.partial && s.source_issues.is_empty() && s.disk_issues.is_empty() {
        lines.push(Line::from(Span::styled(
            clip(&t("overview.stats_partial"), width),
            Style::new().fg(amber()),
        )));
    }
    let label_width = t("overview.stats_source")
        .width()
        .max(t("overview.stats_disk").width())
        + 2;
    for (label, bytes, files, ready, hint) in [
        (
            "overview.stats_source",
            s.source_bytes,
            s.source_files,
            s.source_ready,
            "overview.stats_source_hint",
        ),
        (
            "overview.stats_disk",
            s.bytes,
            s.files,
            s.disk_ready || s.disk_scanning,
            "overview.stats_disk_hint",
        ),
    ] {
        let mut row = vec![Span::styled(fit(&t(label), label_width.min(width)), dim())];
        if ready {
            let value = format!(
                "{}{}",
                if if label == "overview.stats_source" {
                    !s.source_issues.is_empty()
                } else {
                    !s.disk_issues.is_empty() || s.disk_scanning
                } {
                    "≥ "
                } else {
                    ""
                },
                human_bytes(bytes)
            );
            let value_width = value.width().max(11);
            let value = format!("{}{}", " ".repeat(value_width - value.width()), value);
            row.push(Span::styled(
                clip(&value, width.saturating_sub(label_width)),
                strong(),
            ));
            let files = format!("{} {}", count(files), t("overview.files"));
            if label_width + value_width + 2 + files.width() <= width {
                row.push(Span::styled(format!("  {files}"), dim()));
            } else {
                lines.push(Line::from(row));
                row = vec![Span::styled(
                    clip(&format!("{}{files}", " ".repeat(label_width)), width),
                    dim(),
                )];
            }
            let used = Line::from(row.clone()).width();
            if used + 2 + t(hint).width() <= width {
                row.push(Span::styled(format!("  {}", t(hint)), dim()));
            }
        } else {
            row.push(Span::styled(
                clip(&t("loading"), width.saturating_sub(label_width)),
                dim(),
            ));
        }
        lines.push(Line::from(row));
        let issues = if label == "overview.stats_source" {
            &s.source_issues
        } else {
            &s.disk_issues
        };
        if !issues.is_empty() {
            let reasons = issues
                .iter()
                .map(|reason| {
                    t(match reason {
                        stats::ScanIssue::EntryLimit => "overview.limit_entries",
                        stats::ScanIssue::TimeLimit => "overview.limit_time",
                        stats::ScanIssue::ReadError => "overview.limit_read",
                    })
                })
                .collect::<Vec<_>>();
            for message in std::iter::once(t("overview.lower_bound")).chain(reasons) {
                for text in textwrap::wrap(&message, width.max(1)) {
                    lines.push(Line::from(Span::styled(
                        text.into_owned(),
                        Style::new().fg(amber()),
                    )));
                }
            }
        }
    }
    for (label, bytes) in [
        ("overview.build_artifacts", s.artifact_bytes),
        ("overview.dependencies", s.dependency_bytes),
    ] {
        if s.disk_ready || s.disk_scanning {
            lines.push(Line::from(vec![
                Span::styled(format!("{}  ", t(label)), dim()),
                Span::styled(human_bytes(bytes), Style::new().fg(amber())),
            ]));
        }
    }
    if !s.categories.is_empty() {
        let categories = s
            .categories
            .iter()
            .map(|(name, bytes)| {
                format!(
                    "{} {}",
                    t(&format!("overview.category.{name}")),
                    human_bytes(*bytes)
                )
            })
            .collect::<Vec<_>>()
            .join(" · ");
        for text in textwrap::wrap(&categories, width.max(1)) {
            lines.push(Line::from(Span::styled(text.into_owned(), dim())));
        }
    }

    lines.push(Line::default());
    lines.push(heading(&t("overview.stats_languages"), "", width));
    let mut languages = s
        .languages
        .iter()
        .filter(|(_, bytes)| *bytes > 0)
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    let shown = languages
        .iter()
        .map(|(_, bytes)| u128::from(*bytes))
        .sum::<u128>();
    let other = u128::from(s.source_bytes).saturating_sub(shown) as u64;
    if other > 0 {
        languages.push((t("overview.other").into_owned(), other));
    }
    if !s.source_ready || languages.is_empty() {
        lines.push(Line::from(Span::styled(
            clip(
                &t(if s.source_ready {
                    "overview.stats_no_languages"
                } else {
                    "loading"
                }),
                width,
            ),
            dim(),
        )));
    } else {
        let weights = languages
            .iter()
            .map(|(_, bytes)| *bytes)
            .collect::<Vec<_>>();
        let cells = allocate(&weights, width);
        lines.push(Line::from(
            languages
                .iter()
                .zip(cells)
                .map(|((name, _), cells)| {
                    Span::styled(" ".repeat(cells), Style::new().bg(language_color(name)))
                })
                .collect::<Vec<_>>(),
        ));
        lines.extend(legend(&languages, width));
    }

    lines.push(Line::default());
    lines.push(heading(&t("largest"), "", width));
    for text in textwrap::wrap(&t("overview.ordinary_directories"), width.max(1)) {
        lines.push(Line::from(Span::styled(text.into_owned(), dim())));
    }
    if (!s.disk_ready && !s.disk_scanning) || s.directories.is_empty() {
        lines.push(Line::from(Span::styled(
            clip(
                &t(if s.disk_ready {
                    "overview.stats_no_directories"
                } else {
                    "loading"
                }),
                width,
            ),
            dim(),
        )));
    } else {
        let max = s
            .directories
            .iter()
            .take(5)
            .map(|(_, bytes)| *bytes)
            .max()
            .unwrap_or(0)
            .max(1);
        let size_width = s
            .directories
            .iter()
            .take(5)
            .map(|(_, bytes)| human_bytes(*bytes).width())
            .max()
            .unwrap_or(1);
        let name_width = width.saturating_sub(size_width + 8).clamp(1, 14);
        let bar_width = width.saturating_sub(name_width + size_width + 3);
        for (name, bytes) in s.directories.iter().take(5) {
            let name = if name == "." {
                t("overview.root_files")
            } else {
                name.as_str().into()
            };
            let artifact = artifact(&name);
            let color = if artifact { amber() } else { palette().accent };
            let value = human_bytes(*bytes);
            let mut row = vec![
                Span::styled(
                    fit(&name, name_width),
                    Style::new().fg(if artifact { color } else { palette().fg }),
                ),
                Span::styled(
                    format!(" {}{value}  ", " ".repeat(size_width - value.width())),
                    dim(),
                ),
            ];
            if bar_width > 0 {
                let filled = if *bytes == 0 {
                    0
                } else {
                    (u128::from(*bytes) * bar_width as u128 / u128::from(max)).max(1) as usize
                };
                row.push(Span::styled("━".repeat(filled), Style::new().fg(color)));
                row.push(Span::styled("─".repeat(bar_width - filled), dim()));
            }
            if Line::from(row.clone()).width() <= width {
                lines.push(Line::from(row));
            } else {
                lines.push(Line::from(clip(&format!("{name} {value}"), width)));
            }
        }
        if s.directories.iter().take(5).any(|(name, _)| artifact(name)) {
            lines.push(Line::from(Span::styled(
                clip(&t("overview.stats_artifact_hint"), width),
                dim(),
            )));
        }
    }

    if !s.numstat.is_empty() {
        lines.push(Line::default());
        lines.push(heading(&t("parity.numstat"), "", width));
        for (file, added, removed) in s.numstat.iter().take(6) {
            let prefix = format!("+{added} -{removed} ");
            if prefix.width() < width {
                lines.push(Line::from(vec![
                    Span::styled(format!("+{added} "), Style::new().fg(green())),
                    Span::styled(format!("-{removed} "), Style::new().fg(palette().error)),
                    Span::raw(clip(file, width - prefix.width())),
                ]));
            } else {
                lines.push(Line::from(clip(&prefix, width)));
            }
        }
    }
    lines
        .into_iter()
        .map(|l| {
            if l.width() > width {
                Line::from(clip(&l.to_string(), width))
            } else {
                l
            }
        })
        .collect()
}

/// Largest remainders keep the stacked bar exactly as wide as the viewport.
fn allocate(weights: &[u64], width: usize) -> Vec<usize> {
    let total = weights.iter().map(|n| u128::from(*n)).sum::<u128>();
    if total == 0 {
        return vec![0; weights.len()];
    }
    let mut cells = weights
        .iter()
        .map(|n| (u128::from(*n) * width as u128 / total) as usize)
        .collect::<Vec<_>>();
    let mut order = (0..weights.len()).collect::<Vec<_>>();
    order.sort_by_key(|i| {
        (
            std::cmp::Reverse(u128::from(weights[*i]) * width as u128 % total),
            *i,
        )
    });
    for i in order.into_iter().take(width - cells.iter().sum::<usize>()) {
        cells[i] += 1;
    }
    cells
}
fn legend(languages: &[(String, u64)], width: usize) -> Vec<Line<'static>> {
    let total = languages
        .iter()
        .map(|(_, n)| u128::from(*n))
        .sum::<u128>()
        .max(1);
    let mut lines = Vec::new();
    let mut row = Vec::new();
    let mut used = 0;
    for (name, bytes) in languages {
        let color = language_color(name);
        let percent = *bytes as f64 * 100.0 / total as f64;
        let percent = if percent < 0.1 {
            "<0.1%".into()
        } else {
            format!("{percent:.1}%")
        };
        if width < percent.width() + 4 {
            if !row.is_empty() {
                lines.push(Line::from(std::mem::take(&mut row)));
            }
            lines.push(Line::from(Span::styled(
                clip(&format!("{name} {percent}"), width),
                strong(),
            )));
            used = 0;
            continue;
        }
        let name = clip(name, width.saturating_sub(percent.width() + 3));
        let item_width = 3 + name.width() + percent.width();
        if used > 0 && used + 3 + item_width > width {
            lines.push(Line::from(std::mem::take(&mut row)));
            used = 0;
        }
        if used > 0 {
            row.push(Span::raw("   "));
            used += 3;
        }
        row.push(Span::styled("● ", Style::new().fg(color)));
        row.push(Span::styled(name, strong()));
        row.push(Span::styled(format!(" {percent}"), dim()));
        used += item_width;
    }
    if !row.is_empty() {
        lines.push(Line::from(row));
    }
    lines
}
fn artifact(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "target" | "node_modules" | "build" | "dist" | ".gradle" | "out"
    )
}
fn amber() -> Color {
    if crate::appearance::theme() == crate::appearance::Theme::Light {
        Color::Rgb(140, 88, 0)
    } else {
        Color::Rgb(229, 192, 123)
    }
}
fn green() -> Color {
    if crate::appearance::theme() == crate::appearance::Theme::Light {
        Color::Rgb(52, 112, 48)
    } else {
        Color::Rgb(152, 195, 121)
    }
}
fn language_color(name: &str) -> Color {
    match name {
        "Rust" => Color::Rgb(222, 165, 132),
        "HTML" => Color::Rgb(227, 76, 38),
        "Python" => Color::Rgb(65, 140, 180),
        "JavaScript" => Color::Rgb(241, 224, 90),
        "TypeScript" => Color::Rgb(49, 120, 198),
        "PowerShell" => Color::Rgb(83, 128, 244),
        "Other" | "其他" => palette().muted,
        _ => stable_color(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> stats::Stats {
        stats::Stats {
            source_ready: true,
            disk_ready: true,
            source_bytes: 100_000,
            source_files: 7515,
            bytes: 107_700_000_000,
            files: 181751,
            languages: vec![
                ("Rust".into(), 95600),
                ("HTML".into(), 2200),
                ("Python".into(), 1700),
                ("JavaScript".into(), 200),
                ("PowerShell".into(), 200),
            ],
            directories: vec![
                ("target".into(), 92_000_000_000),
                ("vendor".into(), 14_000_000_000),
                ("refer".into(), 1_500_000_000),
                (".git".into(), 88_900_000),
                ("docs".into(), 66_100_000),
            ],
            weeks: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 40, 122],
            commits: vec![
                "8704350 2026-11-11 this long commit subject belongs in project details".into(),
            ],
            ..Default::default()
        }
    }
    #[test]
    fn compact_statistics_fill_width_and_align_directory_columns() {
        let original = crate::appearance::theme();
        for theme in [
            crate::appearance::Theme::Catppuccin,
            crate::appearance::Theme::Light,
        ] {
            crate::appearance::set(theme);
            for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
                crate::i18n::with_language(language, || {
                    for width in [26, 40, 60, 76, 120] {
                        let lines = lines(&fixture(), width);
                        assert!(
                            lines.iter().all(|l| l.width() <= width),
                            "{language:?} {width}"
                        );
                        let paragraph = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
                        assert_eq!(
                            paragraph.line_count(width as u16),
                            lines.len(),
                            "unexpected wrapping"
                        );
                        let directories = lines
                            .iter()
                            .filter(|l| {
                                l.spans.first().is_some_and(|s| {
                                    ["target", "vendor", "refer", ".git", "docs"]
                                        .iter()
                                        .any(|name| s.content.starts_with(name))
                                })
                            })
                            .collect::<Vec<_>>();
                        assert_eq!(directories.len(), 5);
                        let bar_start = |line: &Line<'_>| {
                            line.spans
                                .iter()
                                .take(2)
                                .map(|s| s.content.width())
                                .sum::<usize>()
                        };
                        assert!(
                            directories
                                .iter()
                                .all(|l| bar_start(l) == bar_start(directories[0]))
                        );
                        assert!(
                            directories
                                .iter()
                                .all(|l| l.spans.iter().any(|s| s.content.contains('━'))),
                            "small directories remain visible"
                        );
                        assert_eq!(directories[0].spans[0].style.fg, Some(amber()));
                        assert_ne!(directories[1].spans[0].style.fg, Some(amber()));
                        if width >= 76 {
                            assert!(lines.len() <= 40);
                        }
                    }
                });
            }
        }
        crate::appearance::set(original);
    }
    #[test]
    fn language_color_legend_matches_exact_width_stacked_bar() {
        crate::i18n::with_language(crate::i18n::Language::EnUs, || {
            for width in [26, 40, 76, 120] {
                let lines = lines(&fixture(), width);
                let bar = lines
                    .iter()
                    .find(|l| !l.spans.is_empty() && l.spans.iter().all(|s| s.style.bg.is_some()))
                    .unwrap();
                assert_eq!(bar.width(), width);
                let colors = bar
                    .spans
                    .iter()
                    .map(|s| s.style.bg.unwrap())
                    .collect::<Vec<_>>();
                let legend = lines
                    .iter()
                    .filter(|l| l.spans.iter().any(|s| s.content == "● "))
                    .collect::<Vec<_>>();
                let marker_colors = legend
                    .iter()
                    .flat_map(|l| {
                        l.spans
                            .iter()
                            .filter(|s| s.content == "● ")
                            .map(|s| s.style.fg.unwrap())
                    })
                    .collect::<Vec<_>>();
                assert_eq!(marker_colors, colors);
                if width >= 76 {
                    assert!(legend.len() <= 2);
                }
                let text = lines
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("7,515") && text.contains("181,751"));
                assert!(text.contains("162 commits") && text.contains("2026-11-11"));
                assert!(!text.contains("this long commit subject"));
            }
            for (weights, width) in [
                (vec![1, 1, 1], 80),
                (vec![u64::MAX, u64::MAX, 1], 31),
                (vec![1, 99999], 1),
            ] {
                assert_eq!(allocate(&weights, width).iter().sum::<usize>(), width);
            }
            assert_eq!(allocate(&[0, 0], 80), vec![0, 0]);
        });
    }
    #[test]
    fn narrow_partial_empty_and_loading_statistics_remain_bounded() {
        crate::i18n::with_language(crate::i18n::Language::EnUs, || {
            let mut s = fixture();
            s.partial = true;
            s.languages[0].0 = "超长语言名 with a very long name".into();
            s.directories[0].0 = "超长目录\u{1b}[31m".into();
            for width in 0..100 {
                let lines = lines(&s, width);
                assert!(lines.iter().all(|l| l.width() <= width), "width {width}");
                assert!(!lines.iter().any(|l| l.to_string().contains('\u{1b}')));
            }
            let text = lines(&s, 80)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("Partial"));
            let text = lines(&stats::Stats::default(), 80)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains(t("loading").as_ref()));
            assert!(!text.contains("0 B") && !text.contains("0 commits"));
            let empty = stats::Stats {
                source_ready: true,
                disk_ready: true,
                weeks: vec![0; 12],
                ..Default::default()
            };
            let text = lines(&empty, 80)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("0 B") && text.contains("0 commits"));
        });
    }
    #[test]
    fn ongoing_scan_shows_live_lower_bound_and_names_root_files() {
        for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
            crate::i18n::with_language(language, || {
                let mut s = fixture();
                s.disk_ready = false;
                s.disk_scanning = true;
                s.scanned_entries = 234_567;
                s.directories = vec![(".".into(), 123), ("crates".into(), 12)];
                for width in [1, 8, 26, 40, 76] {
                    let rendered = lines(&s, width);
                    assert!(rendered.iter().all(|line| line.width() <= width));
                    if width >= 40 {
                        let text = rendered
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("\n");
                        assert!(text.contains("234,567"));
                        assert!(text.contains(t("overview.root_files").as_ref()));
                        assert!(text.contains("crates"));
                        assert!(text.contains('≥'));
                        assert!(!text.contains(t("overview.limit_entries").as_ref()));
                    }
                }
            });
        }
    }
    #[test]
    fn hotspot_numeric_columns_align_across_sections_without_ages() {
        for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
            crate::i18n::with_language(language, || {
                let mut s = fixture();
                s.hotspots = vec![
                    stats::Hotspot {
                        path: "src/短文件.rs".into(),
                        commits: 2,
                        added: 9,
                        removed: 1234,
                        latest: crate::config::now(),
                        binary: false,
                    },
                    stats::Hotspot {
                        path: "src/a-much-longer-file.rs".into(),
                        commits: 321,
                        added: 1575,
                        removed: 6,
                        latest: 1,
                        binary: false,
                    },
                ];
                s.directory_hotspots = vec![stats::Hotspot {
                    path: "docs/research".into(),
                    commits: 77,
                    added: 149152,
                    removed: 63,
                    latest: 1,
                    binary: false,
                }];
                for width in [26, 40, 60, 100] {
                    let rendered = lines(&s, width);
                    let numeric = rendered
                        .iter()
                        .filter(|line| {
                            line.spans.iter().any(|span| {
                                span.style.fg == Some(green()) && span.content.contains('+')
                            })
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(numeric.len(), 3, "{language:?} {width}");
                    let mut expected = None;
                    for line in numeric {
                        let text = line.to_string();
                        assert!(!text.contains('·'), "age remains: {text}");
                        let mut x = 0;
                        let mut edges = vec![];
                        for span in &line.spans {
                            x += span.width();
                            if span.content.contains(t("overview.times").as_ref())
                                || span.style.fg == Some(green())
                                || span.style.fg == Some(palette().error)
                            {
                                edges.push(x);
                            }
                        }
                        assert_eq!(edges.len(), 3);
                        assert_eq!(edges[2], width, "right edge: {text}");
                        if let Some(expected) = &expected {
                            assert_eq!(&edges, expected, "{language:?} {width}: {text}");
                        } else {
                            expected = Some(edges);
                        }
                    }
                }
                s.hotspots.push(stats::Hotspot {
                    path: "assets/logo.png".into(),
                    commits: 1,
                    binary: true,
                    ..Default::default()
                });
                for width in [1, 8, 26, 40, 60] {
                    let rendered = lines(&s, width);
                    assert!(rendered.iter().all(|line| line.width() <= width));
                    if width >= 40 {
                        let binary = rendered
                            .iter()
                            .find(|line| {
                                line.spans.iter().any(|span| {
                                    span.content.trim() == t("overview.binary").as_ref()
                                })
                            })
                            .unwrap();
                        assert_eq!(binary.width(), width);
                        assert!(!binary.to_string().contains("+0"));
                    }
                }
            });
        }
    }
    #[test]
    fn pulse_heatmap_hotspots_and_budget_labels_remain_readable_at_all_widths() {
        for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
            crate::i18n::with_language(language, || {
                let mut s = fixture();
                s.days = vec![0; 84];
                s.days[80] = 3;
                s.weeks = vec![0; 12];
                s.weeks[11] = 3;
                s.total_commits = Some(164);
                s.partial = true;
                s.disk_issues = vec![stats::ScanIssue::EntryLimit];
                s.hotspots = vec![stats::Hotspot {
                    path: "src/auth.rs".into(),
                    commits: 12,
                    added: 100,
                    removed: 30,
                    latest: crate::config::now(),
                    binary: false,
                }];
                s.directory_hotspots = vec![stats::Hotspot {
                    path: "src".into(),
                    commits: 12,
                    added: 100,
                    removed: 30,
                    latest: crate::config::now(),
                    binary: false,
                }];
                for width in [1, 2, 8, 26, 28, 40, 60, 76, 120] {
                    let lines = lines(&s, width);
                    assert!(
                        lines.iter().all(|l| l.width() <= width),
                        "{language:?} {width}"
                    );
                    if width >= 40 {
                        let text = lines
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("\n");
                        assert!(text.contains("164"));
                        assert!(text.contains("src/auth.rs"));
                        assert!(!text.contains("%{"));
                        assert!(text.contains(t("overview.limit_entries").as_ref()));
                        let source = lines
                            .iter()
                            .find(|l| {
                                l.to_string()
                                    .starts_with(t("overview.stats_source").as_ref())
                            })
                            .unwrap();
                        assert!(!source.to_string().contains('≥'));
                        let disk = lines
                            .iter()
                            .find(|l| l.to_string().starts_with(t("overview.stats_disk").as_ref()))
                            .unwrap();
                        assert!(disk.to_string().contains('≥'));
                        assert_eq!(
                            lines
                                .iter()
                                .filter(|l| l.spans.len() == 13
                                    && l.spans[1..]
                                        .iter()
                                        .all(|s| matches!(s.content.as_ref(), "██" | "··")))
                                .count(),
                            7
                        );
                        assert!(
                            lines
                                .iter()
                                .flat_map(|l| &l.spans)
                                .any(|s| s.content.contains("+100") && s.style.fg == Some(green()))
                        );
                        assert!(
                            lines
                                .iter()
                                .flat_map(|l| &l.spans)
                                .any(|s| s.content.contains("-30")
                                    && s.style.fg == Some(palette().error))
                        );
                    }
                }
            });
        }
    }
    #[test]
    #[ignore = "write statistics card text fixtures; TestBackend only"]
    fn statistics_layout_samples() {
        use ratatui::widgets::Widget;
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/statistics-layouts");
        std::fs::create_dir_all(&directory).unwrap();
        for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
            crate::i18n::with_language(language, || {
                for width in [26, 44, 60, 76, 100] {
                    let mut sample = fixture();
                    sample.total_commits = Some(164);
                    sample.days = (0..84)
                        .map(|i| if i % 5 == 0 { i % 9 } else { 0 })
                        .collect();
                    sample.weeks = sample
                        .days
                        .chunks(7)
                        .map(|days| days.iter().sum())
                        .collect();
                    sample.artifact_bytes = sample.directories.remove(0).1;
                    sample.dependency_bytes = sample.directories.remove(0).1;
                    sample.categories = [
                        ("source".into(), 75000),
                        ("tests".into(), 20000),
                        ("docs".into(), 5000),
                    ]
                    .into();
                    sample.hotspots = vec![stats::Hotspot {
                        path: "src/auth.rs".into(),
                        commits: 12,
                        added: 420,
                        removed: 80,
                        latest: crate::config::now(),
                        binary: false,
                    }];
                    sample.directory_hotspots = vec![stats::Hotspot {
                        path: "src".into(),
                        ..sample.hotspots[0].clone()
                    }];
                    let lines = lines(&sample, width);
                    let rect = Rect::new(0, 0, width as u16, lines.len() as u16);
                    let mut buffer = ratatui::buffer::Buffer::empty(rect);
                    Paragraph::new(lines.clone()).render(rect, &mut buffer);
                    let text = lines
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n");
                    std::fs::write(directory.join(format!("{language:?}-{width}.txt")), text)
                        .unwrap();
                }
            });
        }
    }
}
