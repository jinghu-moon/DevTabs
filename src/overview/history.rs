//! Bounded, read-only adapters for optional Agent JSONL history.
//! Unsupported records never prevent the Agent's native picker from opening.
use crate::{config, launch::Agent};
use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
const TAIL: u64 = 256 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub user: bool,
    pub text: String,
}
#[derive(Clone, Debug)]
pub struct Conversation {
    pub id: String,
    pub timestamp: String,
    pub prompt: String,
    pub turns: Vec<Turn>,
    pub file: PathBuf,
}
#[derive(Clone, Debug, Default)]
pub struct History {
    pub sessions: Vec<Conversation>,
    pub skipped: usize,
    pub limited: bool,
    pub updated: std::collections::BTreeMap<String, u64>,
}
pub fn age_label(updated: u64) -> String {
    if updated == 0 {
        return "—".into();
    }
    let days = crate::config::now().saturating_sub(updated) / 86400;
    if days == 0 {
        crate::i18n::t("overview.today").into_owned()
    } else {
        format!("{days}{}", crate::i18n::t("overview.days"))
    }
}
pub fn matches(c: &Conversation, query: &str) -> bool {
    let query = query.to_lowercase();
    query.is_empty()
        || c.prompt.to_lowercase().contains(&query)
        || c.id.to_lowercase().contains(&query)
}
fn content(value: &Value) -> String {
    if let Some(s) = value.as_str() {
        return s.into();
    }
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|v| {
                    matches!(
                        v["type"].as_str(),
                        Some("text" | "Text" | "input_text" | "output_text")
                    )
                })
                .filter_map(|v| v["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
fn real_user(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty()
        && ![
            "<environment_context",
            "<permissions",
            "<instructions",
            "<developer_instructions",
            "<system",
            "<turn_aborted",
            "<local-command",
            "<command-name",
            "<user_instructions",
            "<skill",
        ]
        .iter()
        .any(|p| t.starts_with(p))
        && !t.starts_with("# AGENTS.md")
        && !t.starts_with("Caveat:")
        && !t.starts_with("[Request interrupted")
}
fn clean_user(text: &str) -> String {
    let mut value = text.trim();
    for _ in 0..8 {
        let Some(tag) = [
            "environment_context",
            "permissions",
            "instructions",
            "user_instructions",
            "developer_instructions",
            "system",
        ]
        .iter()
        .find(|tag| value.starts_with(&format!("<{tag}"))) else {
            break;
        };
        let closing = format!("</{tag}>");
        let Some(end) = value.find(&closing) else {
            break;
        };
        value = value[end + closing.len()..].trim();
    }
    value.into()
}
pub(crate) fn message(v: &Value) -> Option<Turn> {
    if v["isSidechain"] == true || v["isMeta"] == true {
        return None;
    }
    let p = &v["payload"];
    let pair = match v["type"].as_str() {
        Some("user") if v["isMeta"] != true && v["isSidechain"] != true => {
            Some((true, content(&v["message"]["content"])))
        }
        Some("assistant") => Some((false, content(&v["message"]["content"]))),
        Some("event_msg") if p["type"] == "user_message" => Some((true, content(&p["message"]))),
        Some("event_msg") if p["type"] == "agent_message" && p["phase"] != "commentary" => {
            Some((false, content(&p["message"])))
        }
        Some("response_item") if p["type"] == "message" && p["phase"] != "commentary" => {
            match p["role"].as_str() {
                Some("user") => Some((
                    true,
                    p["content"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|v| clean_user(&content(&v["text"])))
                                .filter(|s| real_user(s))
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default(),
                )),
                Some("assistant") => Some((false, content(&p["content"]))),
                _ => None,
            }
        }
        Some("event_msg") if p["type"] == "item_completed" => {
            let i = &p["item"];
            match i["type"].as_str() {
                Some("UserMessage") => Some((true, content(&i["content"]))),
                Some("AgentMessage") if i["phase"] != "commentary" => {
                    Some((false, content(&i["content"])))
                }
                _ => None,
            }
        }
        _ => None,
    };
    let (user, text) = pair?;
    let text = if user { clean_user(&text) } else { text };
    if text.trim().is_empty() || (user && !real_user(&text)) {
        return None;
    }
    Some(Turn { user, text })
}
pub fn turns(text: &str) -> Vec<Turn> {
    let mut out: Vec<Turn> = vec![];
    let mut previous = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(Turn { user, text }) = message(&v) else {
            continue;
        };
        let text = text.chars().take(4000).collect::<String>();
        if previous.as_ref() == Some(&(user, text.clone())) {
            continue;
        }
        previous = Some((user, text.clone()));
        append_turn(&mut out, Turn { user, text });
    }
    let skip = out.len().saturating_sub(6);
    out.drain(..skip);
    out
}
fn append_turn(out: &mut Vec<Turn>, turn: Turn) {
    if out.last() == Some(&turn) {
        return;
    }
    if !turn.user
        && let Some(last) = out.last_mut().filter(|t| !t.user)
    {
        last.text.push_str("\n\n");
        last.text.push_str(&turn.text);
        if last.text.chars().count() > 8000 {
            last.text = last
                .text
                .chars()
                .rev()
                .take(8000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
        }
    } else {
        out.push(turn);
    }
}
fn segment(file: &mut File, start: u64, limit: u64) -> std::io::Result<String> {
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![];
    file.take(limit).read_to_end(&mut bytes)?;
    crate::perf::add(crate::perf::Count::OverviewHistoryBytes, bytes.len() as u64);
    if start > 0 {
        if let Some(i) = bytes.iter().position(|b| *b == b'\n') {
            bytes.drain(..=i);
        } else {
            bytes.clear()
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
pub fn read(path: &Path, project: &Path, agent: Agent) -> anyhow::Result<Option<Conversation>> {
    read_inner(path, project, agent, true)
}
pub(crate) fn read_inner(
    path: &Path,
    project: &Path,
    agent: Agent,
    body: bool,
) -> anyhow::Result<Option<Conversation>> {
    let mut f = File::open(path)?;
    let size = f.metadata()?.len();
    let head = segment(&mut f, 0, 64 * 1024)?;
    let mut id = None;
    let mut cwd = None;
    let mut timestamp = None;
    for line in head.lines().take(200) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let meta = if v["type"] == "session_meta" {
            &v["payload"]
        } else {
            &v
        };
        if match agent {
            Agent::Codex => v["type"] != "session_meta",
            Agent::Claude => {
                !matches!(v["type"].as_str(), Some("user" | "assistant"))
                    || v["sessionId"].as_str().is_none()
            }
            Agent::Shell => true,
        } {
            continue;
        }
        if cwd.is_none() {
            cwd = meta["cwd"].as_str().map(PathBuf::from)
        }
        if id.is_none() {
            id = meta["id"]
                .as_str()
                .or(meta["sessionId"].as_str())
                .map(str::to_owned)
        }
        if timestamp.is_none() {
            timestamp = meta["timestamp"]
                .as_str()
                .or(v["timestamp"].as_str())
                .map(str::to_owned)
        }
    }
    if cwd
        .as_ref()
        .is_none_or(|p| history_key(p) != history_key(project))
    {
        return Ok(None);
    }
    let Some(id) = id.filter(|s| valid_id(s)) else {
        return Ok(None);
    };
    let turns = if body {
        recent_turns(&mut f, size)?
    } else {
        Vec::new()
    };
    let prompt = turns
        .iter()
        .rev()
        .find(|t| t.user)
        .map(|t| t.text.chars().take(160).collect())
        .or_else(|| {
            self::turns(&head)
                .into_iter()
                .find(|t| t.user)
                .map(|t| t.text.chars().take(160).collect())
        })
        .unwrap_or_default();
    if agent == Agent::Shell {
        return Ok(None);
    }
    Ok(Some(Conversation {
        id,
        timestamp: timestamp.unwrap_or_default(),
        prompt,
        turns,
        file: path.into(),
    }))
}
// Read backwards so large tool output does not push the latest user out of a
// fixed tail. Only message records are retained, never the whole rollout.
fn recent_turns(file: &mut File, size: u64) -> std::io::Result<Vec<Turn>> {
    let start = Instant::now();
    let mut offset = size;
    let mut carry = Vec::new();
    let mut records = Vec::new();
    let mut roles = Vec::new();
    let mut bytes_read = 0;
    while offset > 0 && bytes_read < 64 * 1024 * 1024 && start.elapsed() < Duration::from_secs(2) {
        let n = offset.min(TAIL);
        offset -= n;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; n as usize];
        file.read_exact(&mut bytes)?;
        bytes_read += n;
        crate::perf::add(crate::perf::Count::OverviewHistoryBytes, n);
        bytes.extend_from_slice(&carry);
        carry.clear();
        let boundary = if offset == 0 {
            0
        } else {
            bytes
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |i| i + 1)
        };
        // Giant tool/base64 records are irrelevant to the textual summary.
        if boundary <= 4 * 1024 * 1024 {
            carry.extend_from_slice(&bytes[..boundary]);
        }
        for line in bytes[boundary..].rsplit(|b| *b == b'\n') {
            if line.len() > 4 * 1024 * 1024 {
                continue;
            }
            let text = String::from_utf8_lossy(line);
            if let Some(turn) = turns(&text).into_iter().next() {
                if roles.last() != Some(&turn.user) {
                    roles.push(turn.user);
                }
                records.push(turn);
                if roles.len() >= 6 && roles.contains(&true) {
                    break;
                }
            }
        }
        if roles.len() >= 6 && roles.contains(&true) {
            break;
        }
    }
    records.reverse();
    let mut out = Vec::new();
    let mut previous = None;
    for turn in records {
        if previous.as_ref() == Some(&turn) {
            continue;
        }
        previous = Some(turn.clone());
        append_turn(&mut out, turn);
    }
    let skip = out.len().saturating_sub(6);
    out.drain(..skip);
    Ok(out)
}
fn history_key(path: &Path) -> String {
    config::key(dunce::simplified(path)).replace('/', "\\")
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
pub fn load(
    root: &Path,
    project: &Path,
    agent: Agent,
    generation: &Arc<AtomicU64>,
    ticket: u64,
) -> History {
    let start = Instant::now();
    let mut result = History::default();
    if agent == Agent::Shell {
        return result;
    }
    let mut candidates = vec![];
    let walk = ignore::WalkBuilder::new(root)
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false)
        .follow_links(false)
        .max_depth(Some(6))
        .build();
    for (n, entry) in walk.enumerate() {
        if generation.load(Ordering::Relaxed) != ticket {
            return History::default();
        }
        if n >= 30000 || start.elapsed() > Duration::from_secs(3) {
            result.limited = true;
            break;
        }
        let Ok(e) = entry else {
            result.skipped += 1;
            continue;
        };
        if e.file_type().is_some_and(|f| f.is_file())
            && e.path().extension().is_some_and(|e| e == "jsonl")
        {
            let time = e.metadata().ok().and_then(|m| m.modified().ok());
            candidates.push((time, e.into_path()));
        }
    }
    candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (modified, path) in candidates {
        if generation.load(Ordering::Relaxed) != ticket {
            break;
        }
        if start.elapsed() > Duration::from_secs(8) {
            result.limited = true;
            break;
        }
        match read_inner(&path, project, agent, false) {
            Ok(Some(c)) => {
                if let Some(time) =
                    modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                {
                    result.updated.insert(c.id.clone(), time.as_secs());
                }
                result.sessions.push(c)
            }
            Err(_) => result.skipped += 1,
            _ => {}
        }
    }
    // Candidates are ordered by last file modification, not conversation creation.
    // A resumed older session can be the latest active conversation.
    let mut seen = std::collections::BTreeSet::new();
    result.sessions.retain(|c| seen.insert(c.id.clone()));
    if agent == Agent::Codex
        && let Some(home) = root.parent()
    {
        // Codex's append-only name index is an overlay, not a catalogue:
        // entries have no cwd and must never bypass rollout cwd verification.
        if let Ok(mut file) = File::open(home.join("session_index.jsonl"))
            && let Ok(meta) = file.metadata()
            && let Ok(text) = segment(
                &mut file,
                meta.len().saturating_sub(4 * 1024 * 1024),
                4 * 1024 * 1024,
            )
        {
            let names: std::collections::BTreeMap<String, String> = text
                .lines()
                .filter_map(|l| {
                    let v: Value = serde_json::from_str(l).ok()?;
                    Some((
                        v["id"].as_str()?.into(),
                        v["thread_name"].as_str()?.chars().take(160).collect(),
                    ))
                })
                .collect();
            for c in &mut result.sessions {
                if let Some(name) = names.get(&c.id) {
                    c.prompt.clone_from(name);
                }
            }
        }
    }
    if let Some(first) = result.sessions.first_mut()
        && let Ok(Some(c)) = read(&first.file, project, agent)
    {
        first.turns = c.turns;
    }
    result
}
pub fn root(agent: Agent) -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default();
    match agent {
        Agent::Codex => std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"))
            .join("sessions"),
        Agent::Claude => std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".claude"))
            .join("projects"),
        Agent::Shell => PathBuf::new(),
    }
}
/// One bounded metadata pass per Agent, shared by every discovered project.
/// A timestamp is accepted only after a known conversation record identifies
/// its working directory; directory/Git mtimes never become usage evidence.
pub fn usage_index(
    root: &Path,
    agent: Agent,
    generation: &Arc<AtomicU64>,
    ticket: u64,
) -> std::collections::BTreeMap<String, u64> {
    let mut out = std::collections::BTreeMap::<String, u64>::new();
    let start = Instant::now();
    let mut files = 0;
    for (n, entry) in ignore::WalkBuilder::new(root)
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .follow_links(false)
        .max_depth(Some(6))
        .build()
        .enumerate()
    {
        if generation.load(Ordering::Relaxed) != ticket
            || n >= 30000
            || files >= 512
            || start.elapsed() > Duration::from_secs(3)
        {
            break;
        }
        let Ok(e) = entry else {
            continue;
        };
        if !e.file_type().is_some_and(|f| f.is_file())
            || e.path().extension().is_none_or(|e| e != "jsonl")
        {
            continue;
        }
        files += 1;
        let Ok(mut file) = File::open(e.path()) else {
            continue;
        };
        let Ok(head) = segment(&mut file, 0, 64 * 1024) else {
            continue;
        };
        for line in head.lines().take(200) {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let meta = if agent == Agent::Codex
                && v["type"] == "session_meta"
                && v["payload"]["id"].as_str().is_some_and(valid_id)
            {
                &v["payload"]
            } else if agent == Agent::Claude
                && matches!(v["type"].as_str(), Some("user" | "assistant"))
                && v["sessionId"].as_str().is_some_and(valid_id)
            {
                &v
            } else {
                continue;
            };
            if let Some(cwd) = meta["cwd"].as_str() {
                let timestamp = e
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |t| t.as_secs());
                out.entry(history_key(Path::new(cwd)))
                    .and_modify(|t| *t = (*t).max(timestamp))
                    .or_insert(timestamp);
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_bounded_codex_and_claude_turns() {
        let text = r#"{"type":"user","message":{"content":"hello"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"world"}]}}
{"type":"event_msg","payload":{"type":"user_message","message":"again"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"done"}}"#;
        let turns = turns(text);
        assert_eq!(turns.len(), 4);
        assert!(turns[0].user);
        assert_eq!(turns[3].text, "done");
    }
    #[test]
    fn conversation_ids_are_safe() {
        assert!(valid_id("abc-123_X"));
        assert!(!valid_id("../session"));
    }
}
