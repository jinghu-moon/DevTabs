use anyhow::Result;
use devtabs::{
    app::{self, App, TerminalGuard},
    launch::Options,
};

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    if let Some(first) = args.next() {
        match first.to_string_lossy().as_ref() {
            "__check-child" => {
                let job = args.next().unwrap_or_default();
                let code =
                    devtabs::checks::child(&job.to_string_lossy(), &args.collect::<Vec<_>>())?;
                std::process::exit(code);
            }
            "__hook" => {
                devtabs::hooks::helper(&args.next().unwrap_or_default().to_string_lossy());
                return Ok(());
            }
            "__host" => {
                let code = devtabs::platform::run_host(args.collect())?;
                std::process::exit(code);
            }
            "__serve" => {
                devtabs::host::serve()?;
                return Ok(());
            }
            "__ping" => {
                println!(
                    "{:?}",
                    devtabs::protocol::rpc(devtabs::protocol::Request::Ping).map(|_| "ok")
                );
                return Ok(());
            }
            "__rpc" => {
                use std::io::{Read, Write};
                let mut json = String::new();
                std::io::stdin().read_to_string(&mut json)?;
                let response = devtabs::protocol::rpc(serde_json::from_str(&json)?)?;
                std::io::stdout().write_all(&serde_json::to_vec(&response)?)?;
                return Ok(());
            }
            _ => {}
        }
    }
    if devtabs::cli::dispatch()? {
        return Ok(());
    }
    let overview = std::env::args_os().len() == 1;
    let options = if overview {
        None
    } else {
        use clap::{CommandFactory, FromArgMatches};
        let matches = Options::command().get_matches();
        let parsed = Options::from_arg_matches(&matches)?;
        // Parse structurally so an Agent argument that resembles a DevTabs flag
        // cannot change the application's data directory.
        if let Some(path) = &parsed.data_dir {
            unsafe {
                std::env::set_var("DEVTABS_HOME", path);
            }
        }
        if parsed.restore || parsed.last {
            devtabs::recovery::launch_group(&parsed)?;
            return Ok(());
        }
        Some(devtabs::cli::resolve(parsed, &matches)?)
    };
    if let Some(o) = &options
        && o.workspace.is_none()
    {
        devtabs::recovery::remember(std::slice::from_ref(o))?;
    }
    if let Some(options) = &options
        && options.headless
    {
        devtabs::host::ensure()?;
        match devtabs::protocol::rpc(devtabs::protocol::Request::Open {
            options: Box::new(options.clone()),
        })? {
            devtabs::protocol::Response::Workspace(workspace) => {
                println!("{}", serde_json::to_string(&workspace)?)
            }
            devtabs::protocol::Response::Error(error) => anyhow::bail!("{error}"),
            _ => anyhow::bail!("Unexpected host response"),
        }
        return Ok(());
    }
    if let Some(options) = &options
        && options.new_window
        && options.workspace.is_none()
    {
        devtabs::overview::launch_tab(options)?;
        return Ok(());
    }
    let config = devtabs::config::Config::load()?;
    devtabs::i18n::set(
        options
            .as_ref()
            .and_then(|o| o.language)
            .unwrap_or(config.language),
    );
    devtabs::appearance::set(config.theme);
    let _setup = ratatui::try_init()?;
    let mut terminal =
        devtabs::image_output::Terminal::new(ratatui::backend::CrosstermBackend::new(
            devtabs::image_output::Writer::new(std::io::stdout()),
        ))?;
    let _guard = TerminalGuard::enter()?;
    let mut next = options;
    let mut overview_origin = None;
    loop {
        let chosen = match next.take() {
            Some(options) => Some(options),
            None => devtabs::overview::run_from(&mut terminal, overview_origin.take())?,
        };
        let Some(options) = chosen else {
            return Ok(());
        };
        if options.workspace.is_none() {
            devtabs::recovery::remember(std::slice::from_ref(&options))?;
        }
        let size = terminal.size()?;
        let mut app = App::new(
            options,
            &std::env::current_exe()?,
            ratatui::layout::Rect::new(0, 0, size.width, size.height),
        )?;
        app::run(&mut terminal, &mut app)?;
        terminal.clear()?;
        if app.return_overview {
            overview_origin = Some(app.overview_options());
            continue;
        }
        return Ok(());
    }
}

fn main() -> std::process::ExitCode {
    let _perf = devtabs::perf::ExitGuard::new();
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("DevTabs: {error:#}");
            let directory = devtabs::config::home();
            if std::fs::create_dir_all(&directory).is_ok() {
                let _ = std::fs::write(directory.join("last-error.txt"), format!("{error:#}"));
            }
            std::process::ExitCode::FAILURE
        }
    }
}
