//! Source-based reading actions, independent of viewport layout and terminal cells.
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::{
    collections::BTreeSet,
    ops::Range,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Default)]
pub struct ReadingOptions {
    pub expanded: BTreeSet<usize>,
    pub seek: Option<usize>,
}

pub(super) fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
}
#[derive(Clone, Debug)]
pub struct Heading {
    pub title: String,
    pub level: usize,
    pub anchor: String,
    pub source: Range<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadingAction {
    Link(String),
    Image(String),
    Footnote(String),
    Back,
    Forward,
    Toggle(usize),
}
#[derive(Clone, Debug)]
pub struct ReadingTarget {
    pub source: Range<usize>,
    pub action: ReadingAction,
}
#[derive(Clone, Debug)]
pub struct Details {
    pub source: Range<usize>,
    pub summary: Range<usize>,
    pub body: Range<usize>,
    pub title: String,
    pub open: bool,
}
#[derive(Clone, Debug, Default)]
pub struct ReadingIndex {
    pub headings: Vec<Heading>,
    pub targets: Vec<ReadingTarget>,
    pub footnotes: Vec<(String, usize)>,
    pub details: Vec<Details>,
}

pub(super) struct Prepared {
    pub text: String,
    pub index: ReadingIndex,
    pub summaries: Vec<(String, Range<usize>)>,
    events: Option<Vec<(Event<'static>, Range<usize>)>>,
}
impl Prepared {
    pub(super) fn events(&mut self, markdown: &str) -> Vec<(Event<'static>, Range<usize>)> {
        if markdown == self.text
            && let Some(events) = self.events.take()
        {
            return events;
        }
        Parser::new_ext(markdown, options())
            .into_offset_iter()
            .map(|(e, r)| (e.into_static(), r))
            .collect()
    }
}
pub(super) fn with_summaries<'a>(
    events: impl IntoIterator<Item = (Event<'a>, Range<usize>)>,
    summaries: &[(String, Range<usize>)],
) -> Vec<(Event<'a>, Range<usize>)> {
    let mut out = Vec::new();
    let mut summaries = summaries.iter().peekable();
    for (event, range) in events {
        while summaries
            .peek()
            .is_some_and(|(_, r)| r.start <= range.start)
        {
            let (text, r) = summaries.next().unwrap();
            out.push((Event::Start(Tag::Paragraph), r.clone()));
            out.push((Event::Text(text.clone().into()), r.clone()));
            out.push((Event::End(TagEnd::Paragraph), r.clone()));
        }
        out.push((event, range));
    }
    for (text, r) in summaries {
        out.push((Event::Start(Tag::Paragraph), r.clone()));
        out.push((Event::Text(text.clone().into()), r.clone()));
        out.push((Event::End(TagEnd::Paragraph), r.clone()));
    }
    out
}
fn mask(bytes: &mut [u8], range: Range<usize>) {
    for b in &mut bytes[range] {
        if !matches!(*b, b'\n' | b'\r') {
            *b = b' ';
        }
    }
}
pub(super) fn prepare(source: &str, expanded: &BTreeSet<usize>) -> Prepared {
    static TAGS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let tags = TAGS.get_or_init(|| {
        regex::Regex::new(r"(?is)<details\b[^>]*>|</details\s*>|<summary\b[^>]*>.*?</summary\s*>")
            .unwrap()
    });
    let html: Vec<_> = if tags.is_match(source) {
        Parser::new_ext(source, options())
            .into_offset_iter()
            .filter_map(|(e, r)| matches!(e, Event::Html(_) | Event::InlineHtml(_)).then_some(r))
            .collect()
    } else {
        Vec::new()
    };
    let mut details: Vec<Details> = Vec::new();
    static ATTRIBUTES: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let attributes = ATTRIBUTES.get_or_init(|| {
        regex::Regex::new(r#"([^\s=<>/]+)(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?"#).unwrap()
    });
    let mut stack = Vec::new();
    let mut erase = Vec::new();
    for m in tags.find_iter(source) {
        let at = html.partition_point(|r| r.end <= m.start());
        if !html.get(at).is_some_and(|r| r.contains(&m.start())) {
            continue;
        }
        let lower = m.as_str().to_ascii_lowercase();
        if lower.starts_with("<details") {
            erase.push(m.range());
            stack.push(details.len());
            details.push(Details {
                source: m.start()..source.len(),
                summary: m.range(),
                body: m.end()..source.len(),
                title: "Details".into(),
                open: attributes
                    .captures_iter(m.as_str())
                    .any(|c| c[1].eq_ignore_ascii_case("open")),
            });
        } else if lower.starts_with("</details") {
            if let Some(i) = stack.pop() {
                erase.push(m.range());
                details[i].source.end = m.end();
                details[i].body.end = m.start();
            }
        } else if let Some(&i) = stack.last() {
            erase.push(m.range());
            let start = m.as_str().find('>').unwrap_or(0) + 1;
            let end = lower.rfind("</summary").unwrap_or(m.len());
            details[i].title = inline_text(&m.as_str()[start..end]);
            details[i].summary = m.range();
            details[i].body.start = m.end();
        }
    }
    let mut bytes = source.as_bytes().to_vec();
    for r in erase {
        mask(&mut bytes, r);
    }
    let index_text = String::from_utf8(bytes.clone()).expect("masked whole UTF-8 ranges");
    let mut ranges: Vec<_> = details
        .iter()
        .filter(|d| d.open == expanded.contains(&d.source.start))
        .map(|d| d.body.clone())
        .collect();
    ranges.sort_by_key(|r| r.start);
    let mut closed: Vec<Range<usize>> = Vec::new();
    for r in ranges {
        if let Some(last) = closed.last_mut()
            && r.start <= last.end
        {
            last.end = last.end.max(r.end);
        } else {
            closed.push(r);
        }
    }
    let hidden = |byte: usize| {
        let at = closed.partition_point(|r| r.end <= byte);
        closed.get(at).is_some_and(|r| r.contains(&byte))
    };
    for r in &closed {
        mask(&mut bytes, r.clone());
    }
    let summaries: Vec<_> = details
        .iter()
        .filter(|d| !hidden(d.source.start))
        .map(|d| {
            (
                format!(
                    "{} {}",
                    if d.open != expanded.contains(&d.source.start) {
                        "▾"
                    } else {
                        "▸"
                    },
                    d.title
                ),
                d.summary.clone(),
            )
        })
        .collect();
    let text = String::from_utf8(bytes).expect("masked whole UTF-8 ranges");
    let mut index = ReadingIndex {
        details,
        ..Default::default()
    };
    let mut heading: Option<Heading> = None;
    let mut used = std::collections::HashSet::new();
    let mut suffixes = std::collections::HashMap::<String, usize>::new();
    let events: Vec<_> = Parser::new_ext(&index_text, options())
        .into_offset_iter()
        .map(|(e, r)| (e.into_static(), r))
        .collect();
    for (e, range) in events.iter().cloned() {
        match e {
            Event::Start(Tag::Heading { level, id, .. }) => {
                heading = Some(Heading {
                    title: String::new(),
                    level: level as usize,
                    anchor: id.map(|s| s.to_string()).unwrap_or_default(),
                    source: range,
                })
            }
            Event::Text(t) | Event::Code(t) if heading.is_some() => {
                heading.as_mut().unwrap().title.push_str(&t)
            }
            Event::SoftBreak | Event::HardBreak if heading.is_some() => {
                heading.as_mut().unwrap().title.push(' ')
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(mut h) = heading.take() {
                    if h.anchor.is_empty() {
                        let base = slug(&h.title);
                        h.anchor = base.clone();
                        let n = suffixes.entry(base.clone()).or_default();
                        while used.contains(&h.anchor) {
                            *n += 1;
                            h.anchor = format!("{base}-{n}");
                        }
                    }
                    used.insert(h.anchor.clone());
                    index.headings.push(h);
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => index.targets.push(ReadingTarget {
                source: range,
                action: ReadingAction::Link(dest_url.to_string()),
            }),
            Event::Start(Tag::Image { dest_url, .. }) => index.targets.push(ReadingTarget {
                source: range,
                action: ReadingAction::Image(dest_url.to_string()),
            }),
            Event::FootnoteReference(id) => index.targets.push(ReadingTarget {
                source: range,
                action: ReadingAction::Footnote(id.to_string()),
            }),
            Event::Start(Tag::FootnoteDefinition(id)) => {
                index.footnotes.push((id.to_string(), range.start));
                index.targets.push(ReadingTarget {
                    source: range.start..(range.start + id.len() + 3).min(range.end),
                    action: ReadingAction::Back,
                });
            }
            _ => {}
        }
    }
    for d in &index.details {
        if !hidden(d.source.start) {
            index.targets.push(ReadingTarget {
                source: d.summary.clone(),
                action: ReadingAction::Toggle(d.source.start),
            });
        }
    }
    index.targets.sort_by_key(|t| t.source.start);
    Prepared {
        events: (text == index_text).then_some(events),
        text,
        index,
        summaries,
    }
}
fn inline_text(source: &str) -> String {
    Parser::new(source)
        .filter_map(|e| match e {
            Event::Text(t) | Event::Code(t) => Some(t.to_string()),
            _ => None,
        })
        .collect()
}
pub fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_whitespace() {
                Some('-')
            } else if c.is_alphanumeric() || matches!(c, '-' | '_') {
                Some(c)
            } else {
                None
            }
        })
        .collect()
}
pub fn decode_url(text: &str) -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if text.as_bytes()[i] == b'%' {
            anyhow::ensure!(i + 2 < text.len(), "Invalid URL escape");
            let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            let high =
                hex(text.as_bytes()[i + 1]).ok_or_else(|| anyhow::anyhow!("Invalid URL escape"))?;
            let low =
                hex(text.as_bytes()[i + 2]).ok_or_else(|| anyhow::anyhow!("Invalid URL escape"))?;
            bytes.push(high * 16 + low);
            i += 3;
        } else {
            bytes.push(text.as_bytes()[i]);
            i += 1;
        }
    }
    Ok(String::from_utf8(bytes)?)
}
#[derive(Debug, PartialEq, Eq)]
pub enum Destination {
    Web(String),
    File(PathBuf, Option<String>),
    Anchor(String),
}
pub fn destination(base: &Path, target: &str) -> anyhow::Result<Destination> {
    anyhow::ensure!(!target.chars().any(char::is_control), "Invalid link");
    if target.to_ascii_lowercase().starts_with("https://")
        || target.to_ascii_lowercase().starts_with("http://")
    {
        return Ok(Destination::Web(target.into()));
    }
    let (path, anchor) = target
        .split_once('#')
        .map_or((target, None), |(p, a)| (p, Some(a)));
    if path.is_empty() {
        return Ok(Destination::Anchor(decode_url(anchor.unwrap_or(""))?));
    }
    let path = decode_url(path)?;
    anyhow::ensure!(!path.chars().any(char::is_control), "Invalid path");
    anyhow::ensure!(
        !path.contains("://") && (!path.contains(':') || Path::new(&path).is_absolute()),
        "Unsupported link scheme"
    );
    let path = base.parent().unwrap_or(Path::new(".")).join(path);
    Ok(Destination::File(path, anchor.map(decode_url).transpose()?))
}

impl super::Rendered {
    pub fn row_for_source(&self, byte: usize) -> usize {
        self.rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.segments.iter().any(|s| s.source.end > byte))
            .map_or(self.rows.len().saturating_sub(1), |(i, _)| i)
    }
    pub fn action_at(&self, row: usize, col: usize) -> Option<ReadingAction> {
        let mapping = self.rows.get(row)?;
        let segment = mapping.segments.iter().find(|s| s.display.contains(&col))?;
        let at = if segment.exact {
            segment.source.start
                + self.plain[row]
                    .chars()
                    .skip(segment.display.start)
                    .take(col - segment.display.start)
                    .map(char::len_utf8)
                    .sum::<usize>()
        } else {
            segment.source.start
        };
        let index = &self.reading;
        let end = index.targets.partition_point(|t| t.source.start <= at);
        index.targets[..end]
            .iter()
            .rev()
            .find(|t| t.source.contains(&at))
            .map(|t| t.action.clone())
    }
}
