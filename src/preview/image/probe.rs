//! Recognize only replies to our own bounded DA/cell queries; preserve unrelated input.
use super::{CellSize, Display, ProtocolChoice};
use crate::events::{Events, Input};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use std::{
    collections::VecDeque,
    io::{self, Write},
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Probe {
    pending: Vec<Input>,
    text: String,
    started: Option<Instant>,
    pub sixel: Option<bool>,
    pub cell: Option<CellSize>,
    awaiting_da: bool,
    awaiting_cell: bool,
    draining_reply: bool,
}
fn native_vk(input: &Input) -> Option<&[u8]> {
    input
        .win32
        .as_deref()?
        .strip_prefix(b"\x1b[")?
        .split(|b| *b == b';')
        .next()
}
impl Probe {
    pub fn begin(&mut self) {
        self.awaiting_da = true;
        self.awaiting_cell = true;
        self.sixel = None;
        self.cell = None;
    }
    pub fn feed(&mut self, input: Input) -> Vec<Input> {
        // Physical keys are never part of console-injected terminal responses.
        // This includes Escape and keys interleaved with a partial/late reply.
        if native_vk(&input).is_some_and(|vk| vk != b"0") {
            return vec![input];
        }
        let c = match &input.event {
            Event::Key(k)
                if k.kind != KeyEventKind::Release
                    && !k
                        .modifiers
                        .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
            {
                match k.code {
                    KeyCode::Esc => Some('\x1b'),
                    KeyCode::Char(c) => Some(c),
                    _ => None,
                }
            }
            _ => None,
        };
        if self.draining_reply
            && native_vk(&input) == Some(b"0".as_slice())
            && let Some(c) = c
        {
            if c == '\x1b' {
                self.draining_reply = false;
            } else {
                if matches!(c, 'c' | 't') {
                    self.draining_reply = false;
                }
                return vec![];
            }
        }
        if self.pending.is_empty()
            && (c != Some('\x1b') || (!self.awaiting_da && !self.awaiting_cell))
        {
            return vec![input];
        }
        let Some(c) = c else {
            let mut out = self.finish();
            out.push(input);
            return out;
        };
        self.started.get_or_insert_with(Instant::now);
        self.text
            .extend(std::iter::repeat_n(c, input.repeat.min(128)));
        self.pending.push(input);
        let text = &self.text;
        if self.awaiting_da
            && text.starts_with("\x1b[?")
            && text.ends_with('c')
            && let Some(params) = parameters(&text[3..text.len() - 1])
        {
            self.sixel = Some(params.contains(&4));
            self.awaiting_da = false;
            self.discard();
            return vec![];
        }
        if self.awaiting_cell
            && text.starts_with("\x1b[6;")
            && text.ends_with('t')
            && let Some(params) = parameters(&text[4..text.len() - 1])
            && params.len() == 2
            && (1..=256).contains(&params[0])
            && (1..=256).contains(&params[1])
        {
            self.cell = Some(CellSize {
                width: params[1],
                height: params[0],
            });
            self.awaiting_cell = false;
            self.discard();
            return vec![];
        }
        let prefix = ["\x1b[?", "\x1b[6;"].iter().any(|prefix| {
            prefix.starts_with(text)
                || (text.starts_with(prefix)
                    && text[prefix.len()..]
                        .chars()
                        .all(|c| c.is_ascii_digit() || c == ';'))
        });
        if text.len() > 128 || !prefix {
            return self.finish();
        }
        vec![]
    }
    fn discard(&mut self) {
        self.pending.clear();
        self.text.clear();
        self.started = None;
    }
    pub fn flush(&mut self) -> Vec<Input> {
        self.text.clear();
        self.started = None;
        std::mem::take(&mut self.pending)
    }
    /// Finish a fragment without forwarding identified Windows query bytes to a PTY.
    /// Keep a constant-size drain state for its suffix; physical keys and Paste events pass.
    fn finish(&mut self) -> Vec<Input> {
        if !self.pending.is_empty()
            && self
                .pending
                .iter()
                .all(|i| native_vk(i) == Some(b"0".as_slice()))
            && ["\x1b[?", "\x1b[6;"]
                .iter()
                .any(|prefix| prefix.starts_with(&self.text) || self.text.starts_with(prefix))
        {
            self.draining_reply = !self.text.ends_with('c') && !self.text.ends_with('t');
            self.discard();
            vec![]
        } else {
            self.flush()
        }
    }
    pub fn expire(&mut self, now: Instant) -> Vec<Input> {
        if self
            .started
            .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_millis(100))
        {
            self.finish()
        } else {
            vec![]
        }
    }
}
fn parameters(value: &str) -> Option<Vec<u16>> {
    if value.len() > 120 {
        return None;
    }
    value.split(';').map(|s| s.parse().ok()).collect()
}

pub struct InputReader {
    events: Events,
    probe: Probe,
    queue: VecDeque<Input>,
    choice: ProtocolChoice,
    override_cell: Option<CellSize>,
    pub display: Display,
    query_started: Option<Instant>,
    last_query: Instant,
    requery: bool,
}
impl InputReader {
    pub fn new(choice: ProtocolChoice, cell: Option<CellSize>) -> io::Result<Self> {
        let mut source = Self {
            events: Events::new()?,
            probe: Probe::default(),
            queue: VecDeque::new(),
            choice,
            override_cell: cell,
            display: Display::explicit(choice, cell),
            query_started: None,
            last_query: Instant::now() - Duration::from_secs(10),
            requery: false,
        };
        if choice == ProtocolChoice::Auto || cell.is_none() {
            source.query()?;
        }
        // Same input owner throughout startup and normal dispatch. No Picker stdio thread.
        let start = Instant::now();
        while source.query_started.is_some() && start.elapsed() < Duration::from_millis(500) {
            if let Some(input) = source.events.read(Duration::from_millis(10))? {
                let ready = source.probe.feed(input);
                source.queue.extend(ready);
            }
            source.update()?;
        }
        source.queue.extend(source.probe.finish());
        source.query_started = None;
        source.update()?;
        Ok(source)
    }
    fn query(&mut self) -> io::Result<()> {
        if self.query_started.is_some() || self.last_query.elapsed() < Duration::from_millis(500) {
            return Ok(());
        }
        self.probe.begin();
        self.requery = false;
        self.query_started = Some(Instant::now());
        self.last_query = Instant::now();
        let mut out = io::stdout().lock();
        out.write_all(b"\x1b[c\x1b[16t")?;
        out.flush()
    }
    fn update(&mut self) -> io::Result<()> {
        let cell = self.override_cell.or(self.probe.cell);
        self.display = Display::explicit(
            if self.choice == ProtocolChoice::Auto {
                if self.probe.sixel == Some(true) {
                    ProtocolChoice::Sixel
                } else {
                    ProtocolChoice::Halfblock
                }
            } else {
                self.choice
            },
            cell,
        );
        self.display.automatic = self.choice == ProtocolChoice::Auto;
        if self.probe.sixel.is_some() && cell.is_some() {
            self.query_started = None;
        }
        if self
            .query_started
            .is_some_and(|at| at.elapsed() >= Duration::from_millis(500))
        {
            self.query_started = None;
            self.queue.extend(self.probe.finish());
        }
        // A resize inside the query cooldown must still be checked after it settles.
        // Keep the request instead of remaining on estimated pixels until another event.
        if self.requery && self.query_started.is_none() {
            self.query()?;
        }
        Ok(())
    }
    pub fn read(&mut self, timeout: Duration) -> io::Result<Option<Input>> {
        let started = Instant::now();
        loop {
            self.queue.extend(self.probe.expire(Instant::now()));
            self.update()?;
            if let Some(input) = self.queue.pop_front() {
                if matches!(input.event, Event::Resize(..) | Event::FocusGained) {
                    // Measured pixels may be stale after DPI/font changes. Safe fallback
                    // until the single input stream confirms new cell dimensions.
                    self.probe.cell = None;
                    self.display = Display::explicit(self.choice, self.override_cell);
                    self.requery = true;
                    self.query()?;
                }
                return Ok(Some(input));
            }
            if started.elapsed() >= timeout {
                return Ok(None);
            }
            if let Some(input) = self.events.read(
                timeout
                    .saturating_sub(started.elapsed())
                    .min(Duration::from_millis(20)),
            )? {
                self.queue.extend(self.probe.feed(input));
            }
        }
    }
}
