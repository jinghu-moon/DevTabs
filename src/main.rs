use anyhow::Result;
use clap::Parser;
use devtabs::{
    app::{self, App, TerminalGuard},
    launch::Options,
};

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    if let Some(first) = args.next() {
        match first.to_string_lossy().as_ref() {
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
    // Set a caller-specified data location before any worker thread is created.
    if let Some(index) = std::env::args().position(|arg| arg == "--data-dir")
        && let Some(path) = std::env::args().nth(index + 1)
    {
        unsafe {
            std::env::set_var("DEVTABS_HOME", path);
        }
    }
    if devtabs::cli::dispatch()? {
        return Ok(());
    }
    let overview = std::env::args_os().len() == 1;
    let options = if overview {
        None
    } else {
        Some(devtabs::cli::resolve(Options::parse())?)
    };
    if let Some(options) = &options
        && options.headless
    {
        devtabs::host::ensure()?;
        if let devtabs::protocol::Response::Workspace(workspace) =
            devtabs::protocol::rpc(devtabs::protocol::Request::Open {
                options: options.clone(),
            })?
        {
            println!("{}", serde_json::to_string(&workspace)?);
        }
        return Ok(());
    }
    #[cfg(windows)]
    let _debug_lease = if options.as_ref().and_then(|o| o.view.as_deref()) == Some("debug-shared") {
        let start = std::time::Instant::now();
        loop {
            if let Some(lease) = devtabs::windows::WindowLease::shared_debug()? {
                break Some(lease);
            }
            if start.elapsed() > std::time::Duration::from_secs(4) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    } else {
        None
    };
    let mut terminal = ratatui::try_init()?;
    let _guard = TerminalGuard::enter()?;
    let mut next = options;
    loop {
        let chosen = match next.take() {
            Some(options) => Some(options),
            None => devtabs::overview::run(&mut terminal)?,
        };
        let Some(options) = chosen else {
            return Ok(());
        };
        let size = terminal.size()?;
        let mut app = App::new(
            options,
            &std::env::current_exe()?,
            ratatui::layout::Rect::new(0, 0, size.width, size.height),
        )?;
        app::run(&mut terminal, &mut app)?;
        if app.return_overview {
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
