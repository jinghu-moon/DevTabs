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
    Task {
        #[command(subcommand)]
        action: TaskCommand,
    },
    Overlap {
        tasks: Vec<String>,
    },
    Check {
        workspace: String,
        task: String,
        #[arg(long)]
        initialize: bool,
        #[arg(long)]
        status: bool,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Record or review disk changes without parsing Agent text.
    Record {
        workspace: String,
        task: String,
        #[command(subcommand)]
        action: RecordCommand,
    },
    Hooks {
        provider: String,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        remove: bool,
    },
    Notify {
        #[command(subcommand)]
        action: NotifyCommand,
    },
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
enum TaskCommand {
    Close {
        task: String,
        #[arg(long)]
        confirm: String,
    },
    List,
    Merge {
        source: String,
        target: String,
        #[arg(long)]
        require_check: bool,
    },
    Cleanup {
        source: String,
        target: String,
    },
    Apply {
        operation: String,
        #[arg(long)]
        confirm: String,
    },
    Show {
        operation: String,
    },
    Archive {
        task: String,
        #[arg(long)]
        confirm: String,
    },
}
#[derive(Subcommand)]
enum RecordCommand {
    Begin {
        #[arg(long)]
        scope: Vec<PathBuf>,
    },
    End,
    List,
    Open {
        round: String,
        #[arg(long)]
        file: Option<usize>,
    },
    Ack {
        round: String,
        revision: u64,
    },
}
#[derive(Subcommand)]
enum NotifyCommand {
    Category {
        category: String,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        off: bool,
    },
    AfterCheck {
        task: String,
        #[arg(long)]
        off: bool,
    },
    Sound {
        #[arg(long)]
        off: bool,
    },
    Register {
        #[arg(long)]
        remove: bool,
    },
    List,
    Diagnostics,
    Test,
    Global {
        enabled: bool,
    },
    Mute {
        task: String,
        #[arg(long)]
        off: bool,
    },
    Once {
        task: String,
    },
}
#[derive(Subcommand)]
enum ConfigAction {
    /// Persist application UI language.
    Language {
        #[arg(value_enum)]
        language: crate::i18n::Language,
    },
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
        "record",
        "hooks",
        "notify",
        "check",
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
        "task",
        "overlap",
    ]
    .contains(&first.as_str())
    {
        return Ok(false);
    }
    let cli = Cli::parse();
    match cli.action {
        Action::Task { action } => {
            use crate::lifecycle::Action as L;
            let action = match action {
                TaskCommand::List => L::List,
                TaskCommand::Merge {
                    source,
                    target,
                    require_check,
                } => L::Plan {
                    source,
                    target,
                    require_check,
                    cleanup: false,
                },
                TaskCommand::Cleanup { source, target } => L::Plan {
                    source,
                    target,
                    require_check: false,
                    cleanup: true,
                },
                TaskCommand::Apply { operation, confirm } => L::Apply { operation, confirm },
                TaskCommand::Show { operation } => L::Query { operation },
                TaskCommand::Archive { task, confirm } => L::Archive { task, confirm },
                TaskCommand::Close { task, confirm } => L::Close { task, confirm },
            };
            crate::host::ensure()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&protocol::rpc(Request::Lifecycle { action })?)?
            );
        }
        Action::Overlap { tasks } => {
            let catalog = crate::tasks::Catalog::load(&config::home())?;
            let selected = tasks
                .iter()
                .map(|id| catalog.tasks.get(id).cloned().context("Unknown task"))
                .collect::<Result<Vec<_>>>()?;
            anyhow::ensure!(selected.len() >= 2, "Select at least two tasks");
            let common = crate::tasks::Repository::discover(&selected[0].checkout)?.common;
            for t in &selected {
                anyhow::ensure!(
                    crate::tasks::Repository::discover(&t.checkout)?.common == common,
                    "Tasks belong to different repositories"
                );
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&crate::tasks::overlaps(&selected)?)?
            );
        }
        Action::Check {
            workspace,
            task,
            initialize,
            status,
            command,
        } => {
            let request = if status {
                Request::CheckStatus { task }
            } else {
                Request::Check {
                    workspace,
                    task,
                    initialize,
                    spec: if command.is_empty() {
                        None
                    } else {
                        Some(crate::checks::Spec {
                            argv: command,
                            cwd: ".".into(),
                            env: Default::default(),
                            timeout_ms: 300_000,
                        })
                    },
                }
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&protocol::rpc(request)?)?
            );
        }
        Action::Hooks {
            provider,
            config,
            remove,
        } => {
            let path = match config {
                Some(p) => p,
                None => crate::hooks::config_path(&provider)?,
            };
            crate::hooks::install(&path, &provider, &std::env::current_exe()?, remove)?;
            println!(
                "{} · configured, runtime events and Provider trust remain unverified",
                path.display()
            );
        }
        Action::Record {
            workspace,
            task,
            action,
        } => {
            use crate::rounds::service::Action as R;
            let action = match action {
                RecordCommand::Begin { scope } => R::Begin { scope },
                RecordCommand::End => R::End,
                RecordCommand::List => R::List,
                RecordCommand::Open { round, file } => R::Open { round, path: file },
                RecordCommand::Ack { round, revision } => R::Ack { round, revision },
            };
            let r = protocol::rpc(Request::Review {
                workspace,
                task,
                action,
            })?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        Action::Notify { action } => {
            use crate::notifications::Action as N;
            let action = match action {
                NotifyCommand::Register { remove } => {
                    crate::notifications::register(remove)?;
                    return Ok(true);
                }
                NotifyCommand::List => N::List,
                NotifyCommand::Category {
                    category,
                    task,
                    off,
                } => N::Category {
                    category,
                    task,
                    enabled: !off,
                },
                NotifyCommand::AfterCheck { task, off } => N::AfterCheck {
                    task,
                    enabled: !off,
                },
                NotifyCommand::Sound { off } => N::Sound { enabled: !off },
                NotifyCommand::Diagnostics => N::Diagnostics,
                NotifyCommand::Test => N::Test,
                NotifyCommand::Global { enabled } => N::Global { enabled },
                NotifyCommand::Mute { task, off } => N::Mute { task, muted: !off },
                NotifyCommand::Once { task } => N::Once { task },
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&protocol::rpc(Request::Notifications { action })?)?
            );
        }
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
                ConfigAction::Language { language } => c.language = language,
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
    let project = crate::project_profile::effective(&config, &path)?;
    let composition = project.terminal_composition();
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
    if !supplied("--composition") {
        options.composition = composition;
    }
    if !supplied("--title") {
        options.title = project.title;
    }
    if !supplied("--color") {
        options.color = project.color;
    }
    if !supplied("--agent-arg") {
        options.agent_args = project.agent_args;
    }
    if !options.fresh && !options.resume && options.resume_id.is_none() {
        options.continue_latest |= project.resume_latest.unwrap_or(true);
    }
    options.project = path;
    Ok(options)
}
