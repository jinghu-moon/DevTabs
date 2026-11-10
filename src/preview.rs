//! Bounded, lossless decoding and structured text rendering. No terminal escapes are executed.
use anyhow::{Context, Result, ensure};
use ratatui::text::{Line, Span, Text};
use std::ops::Range;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use syntect::parsing::SyntaxSet;

mod layout;
mod markdown;
mod math;
mod theme;
mod viewport;
pub use markdown::{MarkdownCache, MarkdownStats};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
pub use viewport::{CodeHighlighter, CodeViewport, HighlightStats, HighlightWindow};

pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_LINES: usize = 10_000;
pub const MAX_LINE: usize = 16_384;
const MAX_SPANS: usize = 100_000;
const MAX_RENDER_ROWS: usize = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

#[derive(Clone, Debug)]
pub struct Loaded {
    pub identity: Option<(u64, [u8; 16])>,
    pub path: PathBuf,
    /// Decoded, bounded source; never replace this with the display's sanitized text.
    pub source: Arc<str>,
    pub bytes: Vec<u8>,
    pub encoding: Encoding,
    pub truncated: bool,
    pub editable: bool,
    pub newline: &'static str,
    pub note: String,
}

impl Loaded {
    pub fn markdown(&self) -> bool {
        self.path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
    }
}

pub fn read(path: &Path) -> Result<Loaded> {
    let _perf = crate::perf::span(crate::perf::Metric::Read);
    let mut file = open_read(path).with_context(|| format!("Cannot read {}", path.display()))?;
    let meta = file.metadata()?;
    ensure!(meta.is_file(), "Not a regular file");
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_BYTES + 4) as u64)
        .read_to_end(&mut bytes)?;
    let truncated = meta.len() > MAX_BYTES as u64 || bytes.len() > MAX_BYTES;
    let (encoding, start) = if bytes.starts_with(&[0xff, 0xfe]) {
        (Encoding::Utf16Le, 2)
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        (Encoding::Utf16Be, 2)
    } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        (Encoding::Utf8Bom, 3)
    } else {
        (Encoding::Utf8, 0)
    };
    let mut end = bytes.len().min(MAX_BYTES);
    let source = match encoding {
        Encoding::Utf8 | Encoding::Utf8Bom => {
            if truncated {
                while end > start && end < bytes.len() && bytes[end] & 0xc0 == 0x80 {
                    end -= 1;
                }
            }
            String::from_utf8(bytes[start..end].to_vec())
                .context("Unsupported encoding or binary file; open in default application")?
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            end -= (end - start) % 2;
            let mut units: Vec<u16> = bytes[start..end]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| {
                    if encoding == Encoding::Utf16Le {
                        u16::from_le_bytes([b[0], b[1]])
                    } else {
                        u16::from_be_bytes([b[0], b[1]])
                    }
                })
                .collect();
            if truncated && units.last().is_some_and(|u| (0xd800..=0xdbff).contains(u)) {
                units.pop();
            }
            ensure!(
                truncated || (bytes.len() - start).is_multiple_of(2),
                "Invalid UTF-16 byte length"
            );
            String::from_utf16(&units).context("Invalid UTF-16; open in default application")?
        }
    };
    ensure!(
        !source.contains('\0'),
        "Binary file; use default application"
    );
    let crlf = source.matches("\r\n").count();
    let lf = source.matches('\n').count();
    let mixed = (crlf > 0 && crlf != lf) || source.replace("\r\n", "").contains('\r');
    let controls = source
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'));
    let mut clipped = truncated;
    let needs_clipping = source
        .split_inclusive('\n')
        .enumerate()
        .any(|(i, line)| i >= MAX_LINES || line.chars().count() > MAX_LINE);
    let bounded = if needs_clipping {
        let mut bounded = String::new();
        for (i, line) in source.split_inclusive('\n').enumerate() {
            if i >= MAX_LINES {
                clipped = true;
                break;
            }
            if line.chars().count() > MAX_LINE {
                bounded.extend(line.chars().take(MAX_LINE));
                bounded.push('\n');
                clipped = true;
            } else {
                bounded.push_str(line);
            }
        }
        bounded
    } else {
        source
    };
    let eligibility = crate::editor::eligible(path);
    let editable = !clipped
        && !mixed
        && !controls
        && bytes.len() <= crate::editor::MAX_EDIT_BYTES
        && !meta.permissions().readonly()
        && eligibility.is_ok();
    let note = if clipped {
        "Truncated: preview only"
    } else if mixed {
        "Mixed newlines: preview only"
    } else if controls {
        "Control characters: preview only"
    } else if bytes.len() > crate::editor::MAX_EDIT_BYTES {
        "Over 512 KiB edit budget: preview only"
    } else if meta.permissions().readonly() {
        "Read-only file"
    } else {
        ""
    }
    .to_owned();
    let note = if note.is_empty() {
        eligibility
            .err()
            .map_or(note, |e| format!("Preview only: {e:#}"))
    } else {
        note
    };
    bytes.truncate(end);
    Ok(Loaded {
        identity: crate::editor::file_id(&file).ok(),
        path: path.into(),
        source: Arc::from(bounded),
        bytes,
        encoding,
        truncated: clipped,
        editable,
        newline: if crlf > 0 { "\r\n" } else { "\n" },
        note,
    })
}
fn open_read(path: &Path) -> std::io::Result<File> {
    // Windows ReplaceFileW can briefly unbind or deny sharing of the destination.
    // Keep retries bounded on the worker; deletion/persistent locks still surface.
    for _ in 0..4 {
        match File::open(path) {
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    || (cfg!(windows) && e.raw_os_error() == Some(32)) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            result => return result,
        }
    }
    File::open(path)
}

pub fn safe_text(source: &str) -> String {
    let mut col = 0;
    let mut out = String::with_capacity(source.len());
    for c in source.chars() {
        match c {
            '\n' => {
                out.push(c);
                col = 0;
            }
            '\r' => {}
            '\t' => {
                let n = 4 - col % 4;
                out.extend(std::iter::repeat_n(' ', n));
                col += n;
            }
            c if c.is_control() => {
                out.push('�');
                col += 1;
            }
            c => {
                out.push(c);
                col += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            }
        }
    }
    out
}
pub fn copy_text(source: &str) -> String {
    source
        .chars()
        .map(|c| {
            if c.is_control() && !matches!(c, '\r' | '\n' | '\t') {
                '�'
            } else {
                c
            }
        })
        .collect()
}

fn display_text(source: &str) -> std::borrow::Cow<'_, str> {
    if source.chars().any(|c| c.is_control() && c != '\n') {
        std::borrow::Cow::Owned(safe_text(source))
    } else {
        std::borrow::Cow::Borrowed(source)
    }
}
fn plain_text(source: &str) -> Text<'static> {
    let mut lines = source
        .lines()
        .map(|line| Line::raw(line.to_owned()))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(Line::default());
    }
    Text::from(lines)
}

pub const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;
pub const MAX_HIGHLIGHT_LINE_BYTES: usize = 4 * 1024;
pub fn highlight_allowed(source: &str) -> bool {
    source.len() <= MAX_HIGHLIGHT_BYTES
        && source.lines().count() <= MAX_LINES
        && source.lines().all(|l| l.len() <= MAX_HIGHLIGHT_LINE_BYTES)
}

fn syntax_for(
    path: &Path,
) -> Option<(
    &'static SyntaxSet,
    &'static syntect::parsing::SyntaxReference,
)> {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("txt") || e.eq_ignore_ascii_case("log"))
    {
        return None;
    }
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    static EXTRA_SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    let defaults = SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines);
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let token = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let token = match token.to_ascii_lowercase().as_str() {
        "golang" => "go",
        "python3" => "python",
        "shell" => "bash",
        "csharp" | "c-sharp" => "cs",
        "cuda" | "cu" | "cppm" | "cxxm" | "ixx" => "cpp",
        _ => token,
    };
    let find = |set: &'static SyntaxSet| {
        set.find_syntax_by_extension(file_name)
            .or_else(|| set.find_syntax_by_token(token))
    };
    let set = if find(defaults).is_some() {
        defaults
    } else {
        EXTRA_SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
    };
    find(set).map(|syntax| (set, syntax))
}

pub fn highlight(source: &str, path: &Path, light: bool) -> Text<'static> {
    let _perf = crate::perf::span(crate::perf::Metric::Highlight);
    if !highlight_allowed(source) {
        return plain_text(source);
    }
    let Some((set, syntax)) = syntax_for(path) else {
        return plain_text(source);
    };
    let mut h = syntect::easy::HighlightLines::new(syntax, theme::syntax(light));
    let mut lines = Vec::new();
    let mut span_count = 0;
    for raw in syntect::util::LinesWithEndings::from(source) {
        let Ok(highlighted) = h.highlight_line(raw, set) else {
            return plain_text(source);
        };
        let spans = highlighted
            .into_iter()
            .filter_map(|(style, text)| {
                let text = text.trim_end_matches(['\r', '\n']);
                (!text.is_empty()).then(|| Span::styled(text.to_owned(), theme::convert(style)))
            })
            .collect::<Vec<_>>();
        span_count += spans.len();
        if span_count > MAX_SPANS {
            return plain_text(source);
        }
        lines.push(Line::from(spans));
    }
    if lines.is_empty() {
        lines.push(Line::default());
    }
    Text::from(lines)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Prose,
    Code,
    Table,
    Diagram,
    Math,
    Decoration,
}
#[derive(Clone, Debug)]
pub struct SourceSegment {
    /// Character coordinates in the displayed row; source coordinates are UTF-8 bytes.
    pub display: Range<usize>,
    pub source: Range<usize>,
    pub exact: bool,
}
#[derive(Clone, Debug)]
pub struct RowSource {
    pub kind: RowKind,
    pub segments: Vec<SourceSegment>,
    /// Fixed viewport decoration on a code header; excluded from text and source selection.
    pub code_language: Option<Arc<str>>,
}

#[derive(Debug)]
pub struct Rendered {
    pub text: Arc<Text<'static>>,
    pub window: Option<Arc<HighlightWindow>>,
    pub plain: Arc<Vec<String>>,
    pub source: Arc<str>,
    pub rows: Arc<Vec<RowSource>>,
    pub complete: bool,
    pub width: u16,
    pub markdown: bool,
    pub light: bool,
    pub note: String,
}
impl Rendered {
    /// Original Markdown for a visual selection. Synthetic gutters are excluded;
    /// transformed tokens (entities, math, diagrams) copy their complete source.
    pub fn selected_source(&self, mut start: (usize, usize), mut end: (usize, usize)) -> String {
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for (row, mapping) in self
            .rows
            .iter()
            .enumerate()
            .skip(start.0)
            .take(end.0.saturating_sub(start.0) + 1)
        {
            let a = if row == start.0 { start.1 } else { 0 };
            let b = if row == end.0 { end.1 } else { usize::MAX };
            for segment in &mapping.segments {
                let from = a.max(segment.display.start);
                let to = b.min(segment.display.end);
                if from >= to {
                    continue;
                }
                let range = if segment.exact {
                    let raw = &self.source[segment.source.clone()];
                    let byte =
                        |index: usize| raw.char_indices().nth(index).map_or(raw.len(), |(i, _)| i);
                    segment.source.start + byte(from - segment.display.start)
                        ..segment.source.start + byte(to - segment.display.start)
                } else {
                    segment.source.clone()
                };
                ranges.push(range);
            }
        }
        ranges.sort_unstable_by_key(|r| r.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in ranges {
            if let Some(last) = merged.last_mut()
                && range.start <= last.end
            {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        let mut out = String::new();
        let mut previous = None;
        for range in merged {
            if let Some(end) = previous {
                let gap = &self.source[end..range.start];
                // Recover whitespace removed by wrapping, but no unselected markup/gutters.
                if gap.chars().all(char::is_whitespace) {
                    out.push_str(gap);
                } else if gap.contains('\n') {
                    out.push('\n');
                }
            }
            out.push_str(&self.source[range.clone()]);
            previous = Some(range.end);
        }
        copy_text(&out)
    }
}

pub fn render(source: &str, path: &Path, markdown: bool, width: u16, light: bool) -> Rendered {
    render_shared(Arc::from(source), path, markdown, width, light)
}
pub(crate) fn render_shared(
    source: Arc<str>,
    path: &Path,
    markdown: bool,
    width: u16,
    light: bool,
) -> Rendered {
    render_internal(source, path, markdown, width, light, true)
}
pub(crate) fn render_plain(
    source: Arc<str>,
    path: &Path,
    markdown: bool,
    width: u16,
    light: bool,
) -> Rendered {
    render_internal(source, path, markdown, width, light, false)
}
fn render_internal(
    source: Arc<str>,
    path: &Path,
    markdown: bool,
    width: u16,
    light: bool,
    full_highlight: bool,
) -> Rendered {
    let _perf = crate::perf::span(crate::perf::Metric::Render);
    let mut note = String::new();
    let (text, rows) = if markdown {
        let output = markdown::render(&source, width.max(1) as usize, light);
        let mut lines = Vec::with_capacity(output.rows.len());
        let mut rows = Vec::with_capacity(output.rows.len());
        for row in output.rows {
            lines.push(row.line);
            rows.push(row.source);
        }
        note = output.notes.join("; ");
        (Text::from(lines), rows)
    } else {
        let safe = display_text(&source);
        if !highlight_allowed(&safe) {
            note = "Highlight budget exceeded; plain text".into();
        }
        let text = if full_highlight {
            highlight(&safe, path, light)
        } else {
            plain_text(&safe)
        };
        let mut offset = 0;
        let rows = source
            .split_inclusive('\n')
            .map(|raw| {
                let start = offset;
                offset += raw.len();
                let raw = raw.trim_end_matches(['\r', '\n']);
                if raw.chars().any(|c| c.is_control()) {
                    return layout::row(
                        layout::literal(raw, ratatui::style::Style::default(), start),
                        RowKind::Code,
                        ratatui::style::Style::default(),
                    )
                    .source;
                }
                let display = safe_text(raw);
                RowSource {
                    kind: RowKind::Code,
                    code_language: None,
                    segments: vec![SourceSegment {
                        display: 0..display.chars().count(),
                        source: start..start + raw.len(),
                        exact: display == raw,
                    }],
                }
            })
            .collect();
        (text, rows)
    };
    let plain = text.lines.iter().map(ToString::to_string).collect();
    Rendered {
        text: Arc::new(text),
        window: None,
        plain: Arc::new(plain),
        source,
        rows: Arc::new(rows),
        complete: true,
        width,
        markdown,
        light,
        note,
    }
}
pub fn render_editor(source: &str, path: &Path, width: u16, light: bool) -> Rendered {
    let safe = display_text(source);
    Rendered {
        text: Arc::new(highlight(&safe, path, light)),
        window: None,
        plain: Default::default(),
        source: Arc::from(""),
        rows: Default::default(),
        complete: true,
        width,
        markdown: false,
        light,
        note: if highlight_allowed(&safe) {
            String::new()
        } else {
            "Highlight budget exceeded; plain text".into()
        },
    }
}

pub(crate) fn render_editor_plain(source: Arc<str>, width: u16, light: bool) -> Rendered {
    Rendered {
        text: Arc::new(Text::default()),
        window: None,
        plain: Default::default(),
        source,
        rows: Default::default(),
        complete: true,
        width,
        markdown: false,
        light,
        note: String::new(),
    }
}

pub fn truncate_name(name: &str, width: usize) -> String {
    if name.width() <= width {
        return name.to_owned();
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let tail = if ext.width() + 2 < width {
        ext
    } else {
        String::new()
    };
    let budget = width.saturating_sub(tail.width() + 1);
    let mut head = String::new();
    for g in name.graphemes(true) {
        if head.width() + g.width() > budget {
            break;
        }
        head.push_str(g);
    }
    format!("{head}…{tail}")
}
