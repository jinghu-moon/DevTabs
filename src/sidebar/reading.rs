use super::*;
use preview::reading::Destination;
use std::collections::{BTreeSet, VecDeque};

#[derive(Default)]
pub(super) struct DocumentReading {
    pub expanded: BTreeSet<usize>,
    pub seek: Option<usize>,
    pub fragment: Option<String>,
    pub copied: Option<(usize, Instant)>,
    pub pressed: Option<((usize, usize), (u16, u16))>,
}
#[derive(Clone)]
struct Bookmark {
    path: PathBuf,
    byte: usize,
    left: usize,
    markdown: bool,
    expanded: BTreeSet<usize>,
}
#[derive(Default)]
pub(super) struct ReadingUi {
    pub outline: Option<Outline>,
    memory: VecDeque<Bookmark>,
    back: VecDeque<Bookmark>,
    pub drag: Option<super::scroll::Drag>,
}
pub(super) struct Outline {
    filter: String,
    selected: usize,
}
impl Outline {
    pub(super) fn paste(&mut self, text: &str) {
        let remain = 256usize.saturating_sub(self.filter.chars().count());
        self.filter
            .extend(text.chars().filter(|c| !c.is_control()).take(remain));
        self.selected = 0;
    }
    fn matches<'a>(&self, d: &'a Document) -> Vec<&'a preview::reading::Heading> {
        d.rendered.as_ref().map_or(Vec::new(), |r| {
            r.reading
                .headings
                .iter()
                .filter(|h| h.title.to_lowercase().contains(&self.filter.to_lowercase()))
                .collect()
        })
    }
}
impl Document {
    pub(super) fn reading_byte(&self) -> usize {
        self.rendered
            .as_ref()
            .and_then(|r| {
                r.rows
                    .iter()
                    .skip(self.offset().0)
                    .flat_map(|row| &row.segments)
                    .next()
                    .map(|s| s.source.start)
            })
            .unwrap_or_else(|| self.reader.seek.unwrap_or(0))
    }
    fn bookmark(&self) -> Bookmark {
        Bookmark {
            path: self.path.clone(),
            byte: self.reading_byte(),
            left: self.offset().1,
            markdown: self.markdown,
            expanded: self.reader.expanded.clone(),
        }
    }
    pub(super) fn restore_reading_position(&mut self) {
        if let Some(fragment) = self.reader.fragment.take() {
            self.reader.seek = if fragment.is_empty() {
                Some(0)
            } else {
                self.rendered.as_ref().and_then(|r| {
                    r.reading
                        .headings
                        .iter()
                        .find(|h| h.anchor == fragment)
                        .map(|h| h.source.start)
                })
            };
        }
        if let Some(byte) = self.reader.seek {
            let ancestors = self.rendered.as_ref().map_or(Vec::new(), |r| {
                r.reading
                    .details
                    .iter()
                    .filter(|v| v.body.contains(&byte))
                    .map(|v| (v.source.start, v.open))
                    .collect()
            });
            let mut changed = false;
            for (id, default_open) in ancestors {
                changed |= if default_open {
                    self.reader.expanded.remove(&id)
                } else {
                    self.reader.expanded.insert(id)
                };
            }
            if changed {
                self.rendered = None;
                self.render_pending = false;
                self.render_id = 0;
                return;
            }
        }
        if let Some(r) = &self.rendered
            && let Some(byte) = self.reader.seek
        {
            let known = r.complete
                || r.rows
                    .iter()
                    .flat_map(|r| &r.segments)
                    .any(|s| s.source.end > byte);
            if known {
                let top = r.row_for_source(byte);
                self.offset_mut().0 = top;
                self.reader.seek = None;
            }
        }
    }
    /// Screen rows include a pinned table header; all hit-testing uses the same projection.
    pub(super) fn screen_rows(&self, height: usize) -> Vec<usize> {
        let top = self.offset().0;
        let Some(r) = &self.rendered else {
            return Vec::new();
        };
        let mut rows = Vec::with_capacity(height);
        if let Some(t) = r.rows.get(top).and_then(|r| r.table)
            && t.offset >= t.header_rows
            && t.header_rows < height
        {
            let start = top.saturating_sub(t.offset);
            rows.extend(start..start + t.header_rows);
        }
        rows.extend(top..top + height.saturating_sub(rows.len()));
        rows
    }
}
impl Sidebar {
    pub(super) fn remember_reading(&mut self) {
        if let Some(d) = &self.document {
            let b = d.bookmark();
            self.reader.memory.retain(|v| v.path != b.path);
            self.reader.memory.push_back(b);
            while self.reader.memory.len() > 64 {
                self.reader.memory.pop_front();
            }
        }
        self.reader.outline = None;
        self.reader.drag = None;
    }
    pub(super) fn restore_reading(&mut self) {
        if let Some(d) = &mut self.document
            && let Some(b) = self.reader.memory.iter().find(|b| b.path == d.path)
        {
            d.markdown = b.markdown;
            d.reader.seek = Some(b.byte);
            d.scroll[usize::from(b.markdown)].1 = b.left;
            d.reader.expanded = b.expanded.clone();
        }
    }
    fn push_back(&mut self) {
        if let Some(d) = &self.document {
            self.reader.back.push_back(d.bookmark());
            while self.reader.back.len() > 64 {
                self.reader.back.pop_front();
            }
        }
    }
    pub(super) fn seek_reading(&mut self, byte: usize) {
        if let Some(d) = &mut self.document {
            d.reader.seek = Some(byte);
            d.selection = None;
            d.restore_reading_position();
        }
    }
    pub(super) fn reading_back(&mut self) -> Result<()> {
        if self.dirty()
            && self
                .reader
                .back
                .back()
                .is_some_and(|b| self.document.as_ref().is_some_and(|d| d.path != b.path))
        {
            self.prompt = Some(Prompt::Unsaved(Pending::Reading(
                preview::ReadingAction::Back,
            )));
            return Ok(());
        }
        if let Some(b) = self.reader.back.pop_back() {
            self.open(b.path.clone(), true)?;
            if let Some(d) = &mut self.document {
                if d.markdown != b.markdown || d.reader.expanded != b.expanded {
                    d.rendered = None;
                    d.render_pending = false;
                    d.render_id = 0;
                }
                d.reader.expanded = b.expanded;
                d.markdown = b.markdown;
                d.offset_mut().1 = b.left;
            }
            self.seek_reading(b.byte);
        } else {
            self.say("No reading history", true);
        }
        Ok(())
    }
    pub fn activate_reading(&mut self, action: preview::ReadingAction) -> Result<()> {
        use preview::ReadingAction::*;
        let image = matches!(&action, Image(_));
        match action {
            Back => self.reading_back()?,
            Toggle(id) => {
                if let Some(d) = &mut self.document {
                    if !d.reader.expanded.remove(&id) {
                        d.reader.expanded.insert(id);
                    }
                    d.reader.seek = Some(id);
                    d.rendered = None;
                    d.render_pending = false;
                    d.render_id = 0;
                    d.selection = None;
                }
            }
            Footnote(id) => {
                let byte = self
                    .document
                    .as_ref()
                    .and_then(|d| d.rendered.as_ref())
                    .and_then(|r| {
                        r.reading
                            .footnotes
                            .iter()
                            .find(|(name, _)| *name == id)
                            .map(|(_, b)| *b)
                    });
                if let Some(byte) = byte {
                    self.push_back();
                    self.seek_reading(byte);
                } else {
                    self.say("Footnote not found", true);
                }
            }
            Link(target) | Image(target) => {
                let Some(d) = &self.document else {
                    return Ok(());
                };
                let destination = preview::reading::destination(&d.path, &target)?;
                match destination {
                    Destination::Web(url) => files::external(std::path::Path::new(&url), false)?,
                    Destination::Anchor(anchor) => {
                        let byte = if anchor.is_empty() {
                            Some(0)
                        } else {
                            d.rendered.as_ref().and_then(|r| {
                                r.reading
                                    .headings
                                    .iter()
                                    .find(|h| h.anchor == anchor)
                                    .map(|h| h.source.start)
                            })
                        };
                        if let Some(byte) = byte {
                            self.push_back();
                            self.seek_reading(byte);
                        } else {
                            self.say("Heading not found", true);
                        }
                    }
                    Destination::File(path, fragment) => {
                        let path = dunce::canonicalize(&path)
                            .with_context(|| format!("Cannot open {}", path.display()))?;
                        ensure!(path.is_file(), "Link is not a file");
                        if image {
                            files::external(&path, false)?;
                        } else {
                            if self.dirty()
                                && self.document.as_ref().is_some_and(|d| d.path != path)
                            {
                                self.prompt = Some(Prompt::Unsaved(Pending::Reading(Link(target))));
                                return Ok(());
                            }
                            self.push_back();
                            self.open(path, true)?;
                            if let Some(d) = &mut self.document {
                                if fragment.is_some()
                                    && d.path.extension().is_some_and(|e| {
                                        e.eq_ignore_ascii_case("md")
                                            || e.eq_ignore_ascii_case("markdown")
                                    })
                                {
                                    d.markdown = true;
                                }
                                d.reader.fragment = fragment;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn start_outline(&mut self) {
        if self.document.as_ref().is_some_and(|d| d.markdown) {
            self.reader.outline = Some(Outline {
                filter: String::new(),
                selected: 0,
            });
        }
    }
    pub(super) fn outline_key(&mut self, key: KeyEvent) -> bool {
        let Some(outline) = &mut self.reader.outline else {
            return false;
        };
        let len = self
            .document
            .as_ref()
            .map_or(0, |d| outline.matches(d).len());
        match key.code {
            KeyCode::Esc => self.reader.outline = None,
            KeyCode::Up => outline.selected = outline.selected.saturating_sub(1),
            KeyCode::Down => outline.selected = (outline.selected + 1).min(len.saturating_sub(1)),
            KeyCode::Backspace => {
                outline.filter.pop();
                outline.selected = 0;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && outline.filter.chars().count() < 256 =>
            {
                outline.filter.push(c);
                outline.selected = 0;
            }
            KeyCode::Enter => {
                let byte = self.document.as_ref().and_then(|d| {
                    outline
                        .matches(d)
                        .get(outline.selected)
                        .map(|h| h.source.start)
                });
                self.reader.outline = None;
                if let Some(byte) = byte {
                    self.push_back();
                    self.seek_reading(byte);
                }
            }
            _ => {}
        }
        true
    }
    pub(super) fn draw_outline(&self, frame: &mut Frame) {
        let Some(o) = &self.reader.outline else {
            return;
        };
        let Some(d) = &self.document else {
            return;
        };
        let area = frame.area();
        let r = Rect::new(
            area.x + area.width.saturating_sub(72) / 2,
            area.y + area.height.saturating_sub(22) / 2,
            area.width.min(72),
            area.height.min(22),
        );
        let rows = o.matches(d);
        let height = r.height.saturating_sub(2) as usize;
        let skip = o.selected.saturating_sub(height.saturating_sub(1));
        let lines = rows
            .iter()
            .enumerate()
            .skip(skip)
            .take(height)
            .map(|(i, h)| {
                Line::styled(
                    format!(
                        "{}{}{}",
                        if i == o.selected { "> " } else { "  " },
                        "  ".repeat(h.level.saturating_sub(1)),
                        h.title
                    ),
                    if i == o.selected {
                        Style::new().fg(Color::Black).bg(Color::Cyan)
                    } else {
                        Style::new()
                    },
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(Clear, r);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::bordered().title(format!(" Outline: {} · ↑↓ Enter · Esc ", o.filter)),
            ),
            r,
        );
    }
    pub(super) fn draw_reading_header(&self, frame: &mut Frame, row: usize, content: Rect) {
        let Some(d) = &self.document else {
            return;
        };
        let Some(r) = &d.rendered else {
            return;
        };
        let Some(mapping) = r.rows.get(row) else {
            return;
        };
        let Some(language) = &mapping.code_language else {
            return;
        };
        let line = r.text.lines.get(row).unwrap();
        let copied = d
            .reader
            .copied
            .is_some_and(|(r, t)| r == row && t.elapsed() < Duration::from_secs(2));
        let label = if copied { "[Copied]" } else { "[Copy]" };
        let available = content.width.saturating_sub(label.len() as u16 + 1) as usize;
        let text = if language.width() <= available {
            format!("{language} {label}")
        } else {
            label.into()
        };
        if text.width() <= content.width as usize {
            frame.render_widget(
                Paragraph::new(text)
                    .style(line.style)
                    .alignment(ratatui::layout::Alignment::Right),
                content,
            );
        }
    }
    pub(super) fn reading_click(
        &mut self,
        row: usize,
        col: usize,
        point: (u16, u16),
        inner: Rect,
    ) -> Result<bool> {
        let Some(d) = &mut self.document else {
            return Ok(false);
        };
        let Some(r) = &d.rendered else {
            return Ok(false);
        };
        let copy_width = if d
            .reader
            .copied
            .is_some_and(|(r, t)| r == row && t.elapsed() < Duration::from_secs(2))
        {
            8
        } else {
            6
        };
        if let Some(copy) = r.rows.get(row).and_then(|r| r.code_copy.as_ref())
            && inner.width >= copy_width
            && point.0 >= inner.right().saturating_sub(copy_width)
        {
            files::copy(copy.to_string())?;
            d.reader.copied = Some((row, Instant::now()));
            self.say("Copied code block", true);
            return Ok(true);
        }
        if let Some(action) = r.action_at(row, col) {
            self.activate_reading(action)?;
            return Ok(true);
        }
        Ok(false)
    }
}
