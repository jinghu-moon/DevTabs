//! Incremental, task-bound explicit Markdown file references. No terminal scraping.
use crate::{launch::Agent, overview::history};
use anyhow::{Context, Result, ensure};
use pulldown_cmark::{Event, Parser, Tag};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

const BLOCK: u64 = 256 * 1024;
const INITIAL: u64 = 32 * 1024 * 1024;
const LINE_LIMIT: usize = 4 * 1024 * 1024;
const REF_LIMIT: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub path: PathBuf,
    pub line: Option<usize>,
}
#[derive(Default)]
pub struct Cursor {
    source: Option<(PathBuf, Option<SystemTime>, Vec<u8>)>,
    binding: Option<(PathBuf, Agent, String)>,
    offset: u64,
    target: Option<String>,
    active: Option<String>,
    anchored: bool,
    refs: BTreeMap<PathBuf, Reference>,
    pub partial: bool,
    /// Actual bytes read by this cursor; useful for isolated performance checks.
    pub bytes: u64,
}
impl Cursor {
    pub fn read(
        &mut self,
        path: &Path,
        root: &Path,
        agent: Agent,
        conversation: &str,
        turn: Option<&str>,
    ) -> Result<Vec<Reference>> {
        let _perf = crate::perf::span(crate::perf::Metric::FileReferences);
        ensure!(
            path.is_absolute() && path.extension().is_some_and(|e| e == "jsonl"),
            "No usable transcript path"
        );
        let mut file = File::open(path)?;
        let meta = file.metadata()?;
        let size = meta.len();
        let mut head = vec![0; size.min(1024) as usize];
        file.read_exact(&mut head)?;
        self.bytes += head.len() as u64;
        crate::perf::add(crate::perf::Count::ReferenceBytes, head.len() as u64);
        let source = (path.to_path_buf(), meta.created().ok(), head);
        // Truncation/replacement invalidates the cursor and forces identity revalidation.
        let binding = (root.to_path_buf(), agent, conversation.to_owned());
        let changed_turn = self.target.as_deref() != turn;
        let seek_turn = changed_turn && self.active.as_deref() != self.target.as_deref();
        if changed_turn {
            self.refs.clear();
            self.anchored = false;
            self.partial = false;
        }
        let reset = self.binding.as_ref() != Some(&binding)
            || self.source.as_ref().is_none_or(|old| {
                old.0 != source.0
                    || old.1 != source.1
                    || !source.2.starts_with(&old.2)
                    || size < self.offset
            });
        if reset {
            self.bytes += size.min(64 * 1024);
            crate::perf::add(crate::perf::Count::ReferenceBytes, size.min(64 * 1024));
            let verified = history::read_inner(path, root, agent, false)?
                .context("Transcript does not belong to this task directory")?;
            ensure!(
                verified.id == conversation,
                "Transcript conversation mismatch"
            );
            self.refs.clear();
            self.anchored = false;
            self.active = None;
            self.partial = false;
            self.offset = self.latest_start(&mut file, size, turn)?;
        }
        if !reset && seek_turn {
            self.offset = self.latest_start(&mut file, size, turn)?;
        }
        if changed_turn {
            self.active = None;
        }
        self.binding = Some(binding);
        self.target = turn.map(str::to_owned);
        self.source = Some(source);
        file.seek(SeekFrom::Start(self.offset))?;
        // Freeze at the file length seen above. Concurrent appends are read next time.
        let mut reader = BufReader::new(file.take(size.saturating_sub(self.offset)));
        let mut record = Vec::new();
        let mut length = 0u64;
        let mut oversized = false;
        loop {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                break;
            }
            let n = buf
                .iter()
                .position(|b| *b == b'\n')
                .map_or(buf.len(), |n| n + 1);
            let end = buf[n - 1] == b'\n';
            length += n as u64;
            self.bytes += n as u64;
            crate::perf::add(crate::perf::Count::ReferenceBytes, n as u64);
            if record.len() + n <= LINE_LIMIT && !oversized {
                record.extend_from_slice(&buf[..n]);
            } else {
                oversized = true;
                record.clear();
            }
            reader.consume(n);
            if end {
                if oversized {
                    self.partial = true;
                } else if let Ok(v) = serde_json::from_slice::<Value>(&record) {
                    self.record(&v, root);
                }
                self.offset += length;
                length = 0;
                oversized = false;
                record.clear();
            }
        }
        if self.target.is_some() && !self.anchored {
            self.partial = true;
        }
        // No cursor advance for an incomplete final JSONL record (including split UTF-8).
        Ok(self.refs.values().cloned().collect())
    }
    fn latest_start(&mut self, file: &mut File, size: u64, turn: Option<&str>) -> Result<u64> {
        let floor = size.saturating_sub(INITIAL);
        let mut offset = size;
        let mut carry = Vec::new();
        while offset > floor {
            let n = BLOCK.min(offset - floor);
            offset -= n;
            file.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0; n as usize];
            file.read_exact(&mut bytes)?;
            self.bytes += n;
            crate::perf::add(crate::perf::Count::ReferenceBytes, n);
            bytes.extend_from_slice(&carry);
            let start = if offset == 0 {
                0
            } else {
                bytes
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(bytes.len(), |i| i + 1)
            };
            let mut end = bytes.len();
            for line in bytes[start..].rsplit(|b| *b == b'\n') {
                end = end.saturating_sub(line.len());
                if line.len() <= LINE_LIMIT
                    && let Ok(v) = serde_json::from_slice::<Value>(line)
                {
                    let boundary = if let Some(turn) = turn {
                        v["type"] == "event_msg"
                            && v["payload"]["type"] == "task_started"
                            && v["payload"]["turn_id"].as_str() == Some(turn)
                    } else {
                        history::message(&v).is_some_and(|t| t.user)
                    };
                    if boundary {
                        return Ok(offset + end as u64);
                    }
                }
                end = end.saturating_sub(1);
            }
            carry = if start <= LINE_LIMIT {
                bytes[..start].to_vec()
            } else {
                self.partial = true;
                Vec::new()
            };
        }
        self.partial = floor > 0;
        if floor == 0 { Ok(0) } else { Ok(size) } // Never attribute an unanchored fragment to a turn.
    }
    fn record(&mut self, v: &Value, root: &Path) {
        if v["isSidechain"] == true || v["isMeta"] == true {
            return;
        }
        let p = &v["payload"];
        let marker = match v["type"].as_str() {
            Some("turn_context") => p["turn_id"].as_str(),
            Some("event_msg") if p["type"] == "task_started" => p["turn_id"].as_str(),
            _ => None,
        };
        if let Some(id) = marker {
            // Keep the requested completed turn if a later prompt is already being appended.
            self.active = Some(id.to_owned());
            if self.active == self.target {
                self.anchored = true;
            }
        }
        if self.target.is_some() && self.active != self.target {
            return;
        }
        let Some(message) = history::message(v) else {
            return;
        };
        if message.user {
            if self.target.is_none() {
                self.refs.clear();
            }
            return;
        }
        for event in Parser::new(&message.text) {
            if let Event::Start(Tag::Link { dest_url, .. }) = event
                && let Some((path, line)) = local_reference(root, &dest_url)
            {
                if self.refs.len() < REF_LIMIT || self.refs.contains_key(&path) {
                    self.refs
                        .entry(path.clone())
                        .or_insert(Reference { path, line });
                } else {
                    self.partial = true;
                }
            }
        }
    }
}

/// Strip numeric location suffixes from the right, preserving the drive colon.
pub fn local_reference(root: &std::path::Path, target: &str) -> Option<(PathBuf, Option<usize>)> {
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
