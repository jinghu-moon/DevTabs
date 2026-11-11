param([ValidateSet('before', 'after')][string]$Label = 'after')
$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
$reference = Join-Path $workspace 'refer/DevTabs/src/stats.rs'
if (-not (Test-Path -LiteralPath $reference)) { throw "Missing reference source: $reference" }
$harness = Join-Path $workspace 'artifacts/stats-reference-bench'
[IO.Directory]::CreateDirectory((Join-Path $harness 'src')) | Out-Null
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText((Join-Path $harness 'Cargo.toml'), @'
[package]
name="devtabs-stats-reference-bench"
version="0.1.0"
edition="2024"
[workspace]
[dependencies]
serde_json="=1.0.154"
[profile.release]
lto="thin"
strip=true
'@, $utf8)
$program = @'
#![allow(dead_code)]
mod config { pub fn data_dir() -> std::path::PathBuf { std::env::temp_dir().join("devtabs-legacy-stats-benchmark-unused") } }
mod model { pub fn now_secs() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() } }
mod sys { pub fn quiet(c: &mut std::process::Command) { use std::os::windows::process::CommandExt; c.creation_flags(0x08000000); } }
#[path = r###"@REFERENCE@"###] mod reference;
fn main() {
    let root=std::path::PathBuf::from(std::env::args_os().nth(1).unwrap());
    let mut source=Vec::new(); let mut disk=Vec::new(); let mut result=serde_json::Value::Null;
    for i in 0..8 {
        let start=std::time::Instant::now(); let s=reference::source_stats(&root);
        let us=start.elapsed().as_micros(); if i>0 { source.push(us); }
        let start=std::time::Instant::now(); let d=reference::disk_walk(&root,&||false).unwrap();
        let us=start.elapsed().as_micros(); if i>0 { disk.push(us); }
        assert!(!s.partial && !d.partial);
        result=serde_json::json!({"source_files":s.files,"source_bytes":s.bytes,"disk_files":d.files,"disk_bytes":d.bytes});
    }
    println!("{}",serde_json::json!({"source_us":source,"disk_us":disk,"counts":result}));
}
'@
[IO.File]::WriteAllText((Join-Path $harness 'src/main.rs'), $program.Replace('@REFERENCE@', $reference.Replace('\','/')), $utf8)
$identity = [ordered]@{
    source = $reference
    sha256 = (Get-FileHash -LiteralPath $reference -Algorithm SHA256).Hash.ToLowerInvariant()
    profile = 'Release opt-level 3, thin LTO, default codegen units; unmodified reference source with config/time/quiet adapters'
}
[IO.File]::WriteAllText((Join-Path $harness 'reference.json'), ($identity | ConvertTo-Json), $utf8)
$previousLabel = $env:DEVTABS_STATS_BENCH_LABEL
Push-Location $workspace
try {
    & cargo build --release --offline --manifest-path "$harness/Cargo.toml" --target-dir target/stats-reference
    if ($LASTEXITCODE -ne 0) { throw 'Reference harness build failed' }
    $env:DEVTABS_STATS_BENCH_LABEL = $Label
    & cargo test --release --locked --target-dir target/perf --lib statistics_ -- --ignored --nocapture --test-threads=1
    if ($LASTEXITCODE -ne 0) { throw 'Statistics benchmark failed' }
} finally {
    $env:DEVTABS_STATS_BENCH_LABEL = $previousLabel
    Pop-Location
}
