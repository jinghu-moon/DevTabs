use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Codex,
    Claude,
    /// Open a second PowerShell without an Agent, for terminal diagnostics.
    Shell,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Float,
    Split,
    Tabs,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Composition {
    AgentOnly,
    #[default]
    AgentDebug,
    Custom,
}
impl Composition {
    pub fn label(self) -> std::borrow::Cow<'static, str> {
        crate::i18n::t(match self {
            Self::AgentOnly => "overview.agent_only",
            Self::AgentDebug => "overview.agent_debug",
            Self::Custom => "overview.custom",
        })
    }
    pub fn next(self) -> Self {
        match self {
            Self::AgentOnly => Self::AgentDebug,
            Self::AgentDebug => Self::Custom,
            Self::Custom => Self::AgentOnly,
        }
    }
}
#[derive(Clone, Debug, Parser, Serialize, Deserialize)]
#[command(
    name = "devtabs",
    version,
    long_version = concat!(env!("CARGO_PKG_VERSION"), "\nbuild ", env!("DEVTABS_BUILD_ID"), "\nIPC DevTabs-v5"),
    about = "Agent and Debug together in a Windows terminal"
)]
pub struct Options {
    /// Application UI language; does not change Agent output or shell locale.
    #[arg(long, value_enum)]
    #[serde(default)]
    pub language: Option<crate::i18n::Language>,
    /// Continue a specific Agent conversation (otherwise --resume opens its picker).
    #[arg(long)]
    #[serde(default)]
    pub resume_id: Option<String>,
    /// Restore workspaces whose host ended; leaves running workspaces untouched.
    #[arg(long, conflicts_with = "last")]
    #[serde(default)]
    pub restore: bool,
    /// Reopen the most recently launched ordered group.
    #[arg(long)]
    #[serde(default)]
    pub last: bool,
    /// Start a fresh Agent conversation, overriding automatic continuation.
    #[arg(long, conflicts_with_all = ["resume", "resume_id"]) ]
    #[serde(default)]
    pub fresh: bool,
    /// Continue the latest conversation if readable history exists.
    #[arg(long, conflicts_with = "fresh")]
    #[serde(default)]
    pub continue_latest: bool,
    /// Extra Agent arguments, repeated once per argument; passed as literal values.
    #[arg(long = "agent-arg", allow_hyphen_values = true)]
    #[serde(default)]
    pub agent_args: Vec<String>,
    /// Custom workspace title.
    #[arg(long)]
    #[serde(default)]
    pub title: Option<String>,
    /// Windows Terminal tab color in #RRGGBB form.
    #[arg(long)]
    #[serde(default)]
    pub color: Option<String>,
    /// Open a new Windows Terminal window rather than append to the current one.
    #[arg(long)]
    #[serde(default)]
    pub new_window: bool,
    #[arg(skip)]
    #[serde(default)]
    pub recovery_id: Option<String>,
    #[arg(skip)]
    #[serde(default)]
    pub window_group: Option<String>,
    /// Image protocol for this frontend. Sixel requires confirmed cell pixels.
    #[arg(long, value_enum, default_value = "auto")]
    #[serde(default)]
    pub image_protocol: crate::preview::image::ProtocolChoice,
    /// Diagnostic override: terminal character cell pixels, e.g. 9x18.
    #[arg(long)]
    #[serde(default)]
    pub image_cell_size: Option<crate::preview::image::CellSize>,
    #[arg(long, default_value = ".")]
    pub project: PathBuf,
    #[arg(long, value_enum, default_value = "codex")]
    pub agent: Agent,
    /// Open the Agent's own conversation picker.
    #[arg(long)]
    pub resume: bool,
    /// PowerShell source executed once in Debug; the shell remains afterwards.
    #[arg(long)]
    pub debug_command: Option<String>,
    #[arg(long, default_value = "pwsh")]
    pub shell: PathBuf,
    /// Diagnostic comparison only: skip the personal PowerShell Profile.
    #[arg(long)]
    pub no_profile: bool,
    #[arg(long, default_value_t = 5000, value_parser = clap::value_parser!(u16).range(0..=20000))]
    pub history: u16,
    #[arg(long, value_enum, default_value = "float")]
    pub layout: Layout,
    /// Which terminals to create; independent of their visual arrangement.
    #[arg(long, value_enum, default_value = "agent-debug")]
    #[serde(default)]
    pub composition: Composition,
    /// Create another workspace instead of reusing the existing one.
    #[arg(long)]
    pub new_instance: bool,
    #[arg(long)]
    pub workspace: Option<String>,
    #[arg(long, hide = true)]
    pub view: Option<String>,
    #[arg(skip)]
    pub extra_sessions: Vec<crate::config::SessionConfig>,
    /// Open/reuse the workspace and print its identity as JSON without a view.
    #[arg(long)]
    pub headless: bool,
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}

impl Options {
    pub fn terminal_count(&self) -> usize {
        match self.composition {
            Composition::AgentOnly => 1,
            Composition::AgentDebug => 2,
            Composition::Custom => 2 + self.extra_sessions.len(),
        }
    }
    pub fn effective_composition(&self) -> Composition {
        self.composition
    }
    pub fn parse_for_view() -> Self {
        Self::parse_from(["devtabs", "--agent", "shell"])
    }
    pub fn resolve_latest(&mut self) {
        if self.workspace.is_none()
            && self.continue_latest
            && !self.fresh
            && !self.resume
            && self.resume_id.is_none()
            && self.agent != Agent::Shell
        {
            let flag = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1));
            let history = crate::overview::history::load(
                &crate::overview::history::root(self.agent),
                &self.project,
                self.agent,
                &flag,
                1,
            );
            self.resume_id = history.sessions.first().map(|s| s.id.clone());
        }
    }
    pub fn validate(&mut self) -> Result<()> {
        crate::project_profile::validate_launch(self)?;
        if let Some(id) = &self.resume_id {
            anyhow::ensure!(
                crate::overview::history::valid_id(id),
                "Invalid conversation id"
            );
            anyhow::ensure!(
                self.agent != Agent::Shell,
                "--resume-id requires codex or claude"
            );
        }
        if !cfg!(windows) {
            bail!("DevTabs currently supports Windows only");
        }
        self.project =
            dunce::canonicalize(&self.project).context("Project directory does not exist")?;
        if !self.project.is_dir() {
            bail!("--project must be a directory");
        }
        if !on_path(&self.shell.to_string_lossy()) {
            bail!("PowerShell not found: {}", self.shell.display());
        }
        let tool = match self.agent {
            Agent::Codex => "codex",
            Agent::Claude => "claude",
            Agent::Shell if self.resume => bail!("--resume requires codex or claude"),
            Agent::Shell => return Ok(()),
        };
        if !on_path(tool) {
            bail!("{tool} not found on PATH; install it or use --agent shell for diagnostics");
        }
        Ok(())
    }

    pub fn source(&self, role: usize) -> Option<String> {
        if role == 0 {
            let command = match self.agent {
                Agent::Shell => return None,
                Agent::Codex => "& codex",
                Agent::Claude => "& claude",
            };
            let mut command = command.to_string();
            if let Some(id) = &self.resume_id {
                command.push_str(if self.agent == Agent::Codex {
                    " resume "
                } else {
                    " --resume "
                });
                command.push_str(&quote_argument(id));
            } else if self.resume {
                command.push_str(if self.agent == Agent::Codex {
                    " resume"
                } else {
                    " --resume"
                });
            }
            for arg in &self.agent_args {
                command.push(' ');
                command.push_str(&quote_argument(arg));
            }
            Some(command)
        } else if role == 1 {
            self.debug_command.clone()
        } else {
            self.extra_sessions
                .get(role - 2)
                .and_then(|s| s.command.clone())
        }
    }
    pub fn shell_args(&self, role: usize) -> Vec<String> {
        let mut args = vec!["-NoLogo".into(), "-NoExit".into()];
        if self.no_profile {
            args.push("-NoProfile".into());
        }
        let script = self.source(role);
        if let Some(script) = script {
            args.push("-EncodedCommand".into());
            args.push(encode_script(&script));
        }
        args
    }
}

pub fn quote_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub fn encode_script(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    STANDARD.encode(bytes)
}

fn on_path(name: &str) -> bool {
    let exists = |path: &std::path::Path| std::fs::symlink_metadata(path).is_ok();
    if name.contains(['/', '\\']) {
        return exists(std::path::Path::new(name));
    }
    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|dir| {
            ["", ".exe", ".cmd", ".bat", ".ps1"]
                .iter()
                .any(|suffix| exists(&dir.join(format!("{name}{suffix}"))))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_resume_preserves_native_picker_and_quotes_explicit_id() {
        for (agent, new, picker, named) in [
            (
                "codex",
                "& codex",
                "& codex resume",
                "& codex resume 'session-1_a'",
            ),
            (
                "claude",
                "& claude",
                "& claude --resume",
                "& claude --resume 'session-1_a'",
            ),
        ] {
            let mut options = Options::parse_from(["devtabs", "--agent", agent]);
            assert_eq!(options.source(0).as_deref(), Some(new));
            options.resume = true;
            assert_eq!(options.source(0).as_deref(), Some(picker));
            options.resume_id = Some("session-1_a".into());
            assert_eq!(options.source(0).as_deref(), Some(named));
            for invalid in ["", "a'; Write-Host injected", "a/b", "a\nb", "中文"] {
                options.resume_id = Some(invalid.into());
                assert!(
                    options
                        .validate()
                        .unwrap_err()
                        .to_string()
                        .contains("Invalid conversation id")
                );
            }
        }
        let mut options =
            Options::parse_from(["devtabs", "--agent", "shell", "--resume-id", "safe-id"]);
        assert!(options.source(0).is_none());
        assert!(
            options
                .validate()
                .unwrap_err()
                .to_string()
                .contains("requires codex or claude")
        );
    }

    #[test]
    fn powershell_source_is_preserved_as_utf16_not_split_into_arguments() {
        let script =
            "Set-Location -LiteralPath 'D:\\项目 with spaces'; Write-Output \"a;b\" | Out-String";
        let bytes = STANDARD.decode(encode_script(script)).unwrap();
        let wide: Vec<_> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(String::from_utf16(&wide).unwrap(), script);
    }
}
