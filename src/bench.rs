use anyhow::Result;
pub fn run() -> Result<()> {
    let _perf = crate::perf::span(crate::perf::Metric::Discovery);
    let start = std::time::Instant::now();
    let config = crate::config::Config::load()?;
    let paths = config.scan();
    let loaded = start.elapsed();
    let strings = paths
        .iter()
        .map(|p| p.to_string_lossy().to_lowercase())
        .collect::<Vec<_>>();
    let search = std::time::Instant::now();
    for _ in 0..1000 {
        std::hint::black_box(strings.iter().filter(|s| s.contains("dev")).count());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"projects":paths.len(),"scan_ms":loaded.as_secs_f64()*1000.0,"search_mean_ms":search.elapsed().as_secs_f64(),"note":"Run the 100-project/8-session release integration benchmark for process CPU/memory; this command measures discovery/search only."})
        )?
    );
    Ok(())
}
