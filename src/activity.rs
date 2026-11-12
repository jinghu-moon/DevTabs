//! Provider facts, never inferred from terminal text or silence.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    #[default]
    Unknown,
    Working,
    Permission,
    Input,
    Reported,
    Failed,
    Interrupted,
    Ended,
}
impl Phase {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "notify.unknown",
            Self::Working => "notify.working",
            Self::Permission => "notify.permission",
            Self::Input => "notify.input",
            Self::Reported => "notify.reported",
            Self::Failed => "notify.failed",
            Self::Interrupted => "notify.interrupted",
            Self::Ended => "notify.ended",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub provider: String,
    pub conversation: String,
    pub turn: Option<String>,
    pub kind: String,
    pub child: bool,
    pub cwd: std::path::PathBuf,
    #[serde(default)]
    pub transcript: Option<std::path::PathBuf>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub results: Vec<Outcome>,
    #[serde(default)]
    pub transcript: Option<std::path::PathBuf>,
    pub phase: Phase,
    pub conversation: Option<String>,
    pub turn: Option<String>,
    pub generation: u64,
    pub result: u64,
    pub read: u64,
    pub started: Option<u64>,
    pub ended: Option<u64>,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub retired: Vec<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub version: u64,
    pub phase: Phase,
    pub at: u64,
    pub read: bool,
}
impl State {
    pub fn apply(&mut self, e: &Event, now: u64) -> bool {
        if e.child
            || e.conversation.is_empty()
            || self
                .conversation
                .as_ref()
                .is_some_and(|c| c != &e.conversation)
        {
            return false;
        }
        let phase = match e.kind.as_str() {
            "UserPromptSubmit" => Phase::Working,
            "PermissionRequest" => Phase::Permission,
            "Stop" => Phase::Reported,
            "StopFailure" => Phase::Failed,
            "Interrupt" => Phase::Interrupted,
            "SessionEnd" => Phase::Ended,
            "Notification:permission_prompt" => Phase::Permission,
            "Notification:idle_prompt" => Phase::Input,
            _ => return false,
        };
        // Without a Provider turn ID we cannot prove delayed reports belong to this prompt.
        if phase != Phase::Working && (e.turn != self.turn || self.started.is_none()) {
            return false;
        }
        if phase == self.phase && e.turn == self.turn {
            return false;
        }
        if phase == Phase::Working {
            if e.turn.as_ref().is_some_and(|t| self.retired.contains(t)) {
                return false;
            }
            if e.turn != self.turn || self.started.is_none() || e.turn.is_none() {
                if let Some(old) = &self.turn {
                    self.retired.push(old.clone());
                    if self.retired.len() > 64 {
                        self.retired.remove(0);
                    }
                }
                self.generation += 1;
                self.started = Some(now);
            }
            self.turn = e.turn.clone();
            self.ended = None;
        } else if matches!(
            phase,
            Phase::Reported | Phase::Failed | Phase::Interrupted | Phase::Ended
        ) {
            self.result += 1;
            self.ended = Some(now);
            self.results.push(Outcome {
                version: self.result,
                phase: phase.clone(),
                at: now,
                read: false,
            });
            if self.results.len() > 20 {
                self.results.remove(0);
            }
        }
        if let Some(path) = &e.transcript {
            self.transcript = Some(path.clone());
        }
        self.conversation = Some(e.conversation.clone());
        self.phase = phase;
        if !self.capabilities.contains(&e.kind) {
            self.capabilities.push(e.kind.clone());
            self.capabilities.truncate(16);
        }
        true
    }
    pub fn ack(&mut self, version: u64) {
        if let Some(result) = self.results.iter_mut().find(|r| r.version == version) {
            result.read = true;
        }
        if version == self.result {
            self.read = version;
        }
    }
    pub fn pending(&self) -> bool {
        self.results.iter().any(|r| !r.read)
            || matches!(self.phase, Phase::Permission | Phase::Input)
    }
}
