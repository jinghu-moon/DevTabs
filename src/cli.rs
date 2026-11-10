use crate::{
    config::{self, Config},
    launch::{Agent, Layout, Options},
    protocol::{self, Request, Response},
};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, process::Command};

#[derive(Parser)]
#[command(name = "devtabs")]
struct Cli {
    #[command(subcommand)]
    action: Action,
}
#[derive(Subcommand)]
enum Action {
    List,
    Close {
        workspace: String,
    },
    Stop {
        workspace: String,
        session: String,
        #[arg(long)]
        force: bool,
    },
    Run {
        workspace: String,
        session: String,
        #[arg(long)]
        source: Option<String>,
    },
    Layout {
        workspace: String,
        #[arg(value_enum)]
        layout: Layout,
    },
    Shutdown,
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    Install {
        #[arg(long)]
        directory: Option<PathBuf>,
        #[arg(long)]
        no_path: bool,
    },
    Uninstall {
        #[arg(long)]
        directory: Option<PathBuf>,
        #[arg(long)]
        no_path: bool,
    },
    Bench,
}
#[derive(Subcommand)]
enum ConfigAction {
    Show,
    Edit,
    Root {
        path: PathBuf,
    },
    Add {
        path: PathBuf,
    },
    Exclude {
        path: PathBuf,
    },
    Favorite {
        path: PathBuf,
        #[arg(long)]
        off: bool,
    },
    Agent {
        #[arg(value_enum)]
        agent: Agent,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    Debug {
        project: PathBuf,
        source: String,
    },
}

pub fn dispatch() -> Result<bool> {
    let Some(first) = std::env::args().nth(1) else {
        return Ok(false);
    };
    if ![
        "list",
        "close",
        "stop",
        "run",
        "layout",
        "shutdown",
        "config",
        "install",
        "uninstall",
        "bench",
    ]
    .contains(&first.as_str())
    {
        return Ok(false);
    }
    let cli = Cli::parse();
    match cli.action {
        Action::List => {
            crate::host::ensure()?;
            if let Response::Workspaces(work) = protocol::rpc(Request::List)? {
                println!("{}", serde_json::to_string_pretty(&work)?);
            }
        }
        Action::Close { workspace } => {
            protocol::rpc(Request::Close { workspace })?;
        }
        Action::Stop {
            workspace,
            session,
            force,
        } => {
            protocol::rpc(Request::Stop {
                workspace,
                session,
                force,
            })?;
        }
        Action::Run {
            workspace,
            session,
            source,
        } => {
            protocol::rpc(Request::Run {
                workspace,
                session,
                source,
            })?;
        }
        Action::Layout { workspace, layout } => {
            protocol::rpc(Request::Layout { workspace, layout })?;
        }
        Action::Shutdown => {
            protocol::rpc(Request::Shutdown)?;
        }
        Action::Config { action } => {
            let _lock = config::ConfigLock::acquire()?;
            let mut c = Config::load()?;
            match action {
                ConfigAction::Show => {
                    println!(
                        "# {}\n{}",
                        config::home().join("config.toml").display(),
                        toml::to_string_pretty(&c)?
                    );
                    return Ok(true);
                }
                ConfigAction::Edit => {
                    c.save()?;
                    drop(_lock);
                    Command::new("notepad.exe")
                        .arg(config::home().join("config.toml"))
                        .status()?;
                    Config::load()?;
                    return Ok(true);
                }
                ConfigAction::Root { path } => c.roots.push(dunce::canonicalize(path)?),
                ConfigAction::Add { path } => c.added.push(dunce::canonicalize(path)?),
                ConfigAction::Exclude { path } => c.excluded.push(dunce::canonicalize(path)?),
                ConfigAction::Favorite { path, off } => {
                    c.projects
                        .entry(config::key(&dunce::canonicalize(path)?))
                        .or_default()
                        .favorite = !off
                }
                ConfigAction::Agent { agent, project } => {
                    if let Some(project) = project {
                        c.projects
                            .entry(config::key(&dunce::canonicalize(project)?))
                            .or_default()
                            .agent = Some(agent);
                    } else {
                        c.default_agent = Some(agent);
                    }
                }
                ConfigAction::Debug { project, source } => {
                    c.projects
                        .entry(config::key(&dunce::canonicalize(project)?))
                        .or_default()
                        .debug_command = Some(source)
                }
            }
            c.save()?;
        }
        Action::Install { directory, no_path } => crate::install::install(directory, no_path)?,
        Action::Uninstall { directory, no_path } => crate::install::uninstall(directory, no_path)?,
        Action::Bench => crate::bench::run()?,
    }
    Ok(true)
}
pub fn resolve(mut options: Options) -> Result<Options> {
    if options.workspace.is_some() {
        return Ok(options);
    }
    let args = std::env::args().collect::<Vec<_>>();
    let supplied = |flag: &str| {
        args.iter()
            .any(|arg| arg == flag || arg.starts_with(&format!("{flag}=")))
    };
    let config = Config::load()?;
    let path = dunce::canonicalize(&options.project)?;
    let project = config
        .projects
        .get(&config::key(&path))
        .cloned()
        .unwrap_or_default();
    if !supplied("--agent") {
        options.agent=project.agent.or(config.default_agent).context("Choose a global Agent once: devtabs config agent codex (or claude); --agent overrides for this launch")?;
    }
    if !supplied("--debug-command") {
        options.debug_command = project.debug_command;
    }
    if !supplied("--layout") {
        options.layout = project.layout.unwrap_or_default();
    }
    options.extra_sessions = project.sessions;
    options.project = path;
    Ok(options)
}
