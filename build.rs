use std::{
    env, fs,
    hash::{DefaultHasher, Hash, Hasher},
    io,
    path::{Path, PathBuf},
    process::Command,
};

fn collect(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if !matches!(entry.file_name().to_str(), Some("target" | ".git")) {
                collect(&entry.path(), files)?;
            }
        }
    } else {
        files.push(path.to_path_buf());
    }
    Ok(())
}

fn main() -> io::Result<()> {
    // Development builds can change configuration or IPC without changing the
    // package version. Compute once at build time, never in the UI/RPC loop.
    let mut files = Vec::new();
    for input in [
        "build.rs",
        "Cargo.toml",
        "Cargo.lock",
        "src",
        "crates",
        "vendor",
    ] {
        println!("cargo:rerun-if-changed={input}");
        collect(Path::new(input), &mut files)?;
    }
    files.sort();
    let mut hash = DefaultHasher::new();
    for path in files {
        path.to_string_lossy().replace('\\', "/").hash(&mut hash);
        fs::read(path)?.hash(&mut hash);
    }
    for name in [
        "TARGET",
        "PROFILE",
        "CARGO_CFG_FEATURE",
        "CARGO_ENCODED_RUSTFLAGS",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
        env::var_os(name).hash(&mut hash);
    }
    let rustc = Command::new(env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("-vV")
        .output()?;
    if !rustc.status.success() {
        return Err(io::Error::other("Read Rust compiler identity"));
    }
    rustc.stdout.hash(&mut hash);
    println!("cargo:rustc-env=DEVTABS_BUILD_ID={:016x}", hash.finish());
    Ok(())
}
