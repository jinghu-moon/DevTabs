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
#[derive(Clone, Debug, Parser, Serialize, Deserialize)]
#[command(
    name = "devtabs",
    version,
    about = "Agent and Debug together in a Windows terminal"
)]
pub struct Options {
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
    pub fn validate(&mut self) -> Result<()> {
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
            match self.agent {
                Agent::Codex => Some(
                    if self.resume {
                        "& codex resume"
                    } else {
                        "& codex"
                    }
                    .into(),
                ),
                Agent::Claude => Some(
                    if self.resume {
                        "& claude --resume"
                    } else {
                        "& claude"
                    }
                    .into(),
                ),
                Agent::Shell => None,
            }
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
