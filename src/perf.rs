//! Opt-in, bounded aggregate timings. No file names, input or source are recorded.
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
    time::{Duration, Instant},
};

macro_rules! metrics {
    ($($name:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy)]
        #[repr(usize)]
        pub enum Metric { $($name),+ }
        const NAMES: &[&str] = &[$($label),+];
    };
}
metrics! {
    RoundBegin=>"round.begin",RoundEnd=>"round.end",RoundDiff=>"round.diff",RoundStore=>"round.store",TaskLifecycle=>"task.lifecycle",
    FloatingFirstFrame => "terminal.floating_first_frame", FloatingOpen => "terminal.panel_open", FloatingAttach => "terminal.floating_attach",
    OverviewStats => "overview.stats", OverviewHistory => "overview.history", OverviewGit => "overview.git", OverviewLaunch => "overview.launch",
    ProjectIndex => "project.index", ProjectSearch => "project.fuzzy_search", ProjectGit => "project.git_status", ProjectChange => "project.file_operation",
    ImageOutput => "output.write", ImageFlush => "output.flush", ImageSvgRender => "image.svg_render",
    ImageRead => "image.read", ImageInspect => "image.inspect_dimensions", ImageDecode => "image.decode_orient", ImageResize => "image.crop_alpha_resize", ImageEncode => "image.quantize_encode", ImageQueueWait => "image.request_wait", ImageSlice => "image.slice_prepare",
    Frame => "ui.frame_and_flush", Ui => "ui.compose", Input => "ui.input_dispatch",
    Tick => "sidebar.tick", Watch => "files.watch_reconcile",
    TreeItems => "tree.build_items", TreeRender => "tree.widget_render", TreeBlit => "tree.copy_viewport", TreeMerge => "tree.merge_batch",
    SidebarDraw => "sidebar.draw", PreviewDraw => "preview.draw", PreviewSearch => "preview.search", Highlight => "preview.highlight",
    Render => "preview.render", ViewportHighlight => "preview.viewport_highlight", Read => "files.read_decode", Directory => "files.directory_batch",
    Markdown => "markdown.parse_layout", MarkdownViewport => "markdown.viewport_layout", TableLayout => "markdown.table_layout", CodeLayout => "markdown.code_layout", Mermaid => "markdown.mermaid", Math => "markdown.math",
    QueueWait => "files.queue_wait", ResultWait => "files.result_backpressure", QueueFull => "files.queue_rejected",
    EditorEdit => "editor.edit", EditorClone => "editor.clone_candidate", EditorValidate => "editor.validate",
    EditorDirty => "editor.dirty_compare", EditorDraw => "editor.draw", Save => "editor.save", SaveEncode => "editor.stream_encode", SaveVerify => "editor.stream_verify",
    Rpc => "ipc.rpc", RpcPoll => "ipc.poll", RemoteApply => "terminal.apply_snapshot",
    HostRequest => "host.request", HostSnapshot => "host.snapshot", TerminalParse => "terminal.parse",
    OverviewFilter => "overview.filter", OverviewFrame => "overview.frame_and_flush", Discovery => "overview.discovery_benchmark",
    OverviewInputToFrame => "overview.input_to_frame",
    OverviewSourceScan => "overview.stats_source_scan",
    OverviewDiskScan => "overview.stats_disk_scan",
    OverviewStatsGit => "overview.stats_git",
}
const BUCKETS: usize = 64;
struct Counter {
    calls: AtomicU64,
    total: AtomicU64,
    max: AtomicU64,
    slow: AtomicU64,
    buckets: [AtomicU64; BUCKETS],
}
impl Counter {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            total: AtomicU64::new(0),
            max: AtomicU64::new(0),
            slow: AtomicU64::new(0),
            buckets: [const { AtomicU64::new(0) }; BUCKETS],
        }
    }
    fn record(&self, nanos: u64) {
        let bucket = if nanos == 0 {
            0
        } else {
            (64 - (nanos - 1).leading_zeros()) as usize
        }
        .min(BUCKETS - 1);
        self.buckets[bucket].fetch_add(1, Relaxed);
        self.total.fetch_add(nanos, Relaxed);
        self.max.fetch_max(nanos, Relaxed);
        if nanos >= 50_000_000 {
            self.slow.fetch_add(1, Relaxed);
        }
        self.calls.fetch_add(1, Relaxed);
    }
    fn percentile(&self, count: u64, numerator: u64) -> f64 {
        let rank = count.saturating_mul(numerator).div_ceil(100).max(1);
        let mut seen = 0;
        for (i, bucket) in self.buckets.iter().enumerate() {
            seen += bucket.load(Relaxed);
            if seen >= rank {
                return (1u64 << i) as f64 / 1_000_000.0;
            }
        }
        self.max.load(Relaxed) as f64 / 1_000_000.0
    }
}
static COUNTERS: [Counter; NAMES.len()] = [const { Counter::new() }; NAMES.len()];
struct Config {
    directory: Option<PathBuf>,
    started: Instant,
    instance: String,
}
static CONFIG: OnceLock<Config> = OnceLock::new();
fn config() -> &'static Config {
    CONFIG.get_or_init(|| {
        let directory = std::env::var_os("DEVTABS_PERF_DIR")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        if directory.is_some() {
            // Only enabled sessions get a writer. Flush is off the UI/PTY threads.
            let _ = std::thread::Builder::new()
                .name("perf-export".into())
                .spawn(|| {
                    loop {
                        std::thread::sleep(Duration::from_secs(2));
                        let _ = flush();
                    }
                });
        }
        Config {
            directory,
            started: Instant::now(),
            instance: uuid::Uuid::new_v4().simple().to_string(),
        }
    })
}
pub fn enabled() -> bool {
    config().directory.is_some()
}
pub struct Span {
    metric: Metric,
    start: Option<Instant>,
}
#[inline]
pub fn span(metric: Metric) -> Span {
    Span {
        metric,
        start: enabled().then(Instant::now),
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            record(self.metric, start.elapsed());
        }
    }
}
pub fn record(metric: Metric, elapsed: Duration) {
    if enabled() {
        COUNTERS[metric as usize].record(elapsed.as_nanos().min(u64::MAX as u128) as u64);
    }
}
#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Count {
    OverviewContextHits,
    OverviewStale,
    OverviewCanceled,
    OverviewHistoryBytes,
    FloatingFailures,
    TerminalAttaches,
    TerminalDetaches,
    TerminalResizes,
    TerminalFrameBytes,
    ImageInspections,
    ImageCacheHits,
    ImageCacheMisses,
    ImageDecodes,
    ImageEncodes,
    ImageStale,
    ImageReplaced,
    ImageTransfers,
    ImageBytes,
    ImageClears,
    ImageFallbacks,
    ImageClips,
    ImageCompositionHits,
}
const COUNT_NAMES: &[&str] = &[
    "overview.context_cache_hits",
    "overview.stale_results",
    "overview.canceled_requests",
    "overview.history_read_bytes",
    "terminal.window_failures",
    "terminal.attaches",
    "terminal.detaches",
    "terminal.resizes",
    "terminal.frame_bytes",
    "image.inspections",
    "image.source_cache_hits",
    "image.source_cache_misses",
    "image.decodes",
    "image.encodes",
    "image.stale_results",
    "image.replaced_requests",
    "image.transfers",
    "image.output_bytes",
    "image.clears",
    "image.fallbacks",
    "image.clips",
    "image.composition_cache_hits",
];
static COUNTS: [AtomicU64; COUNT_NAMES.len()] = [const { AtomicU64::new(0) }; COUNT_NAMES.len()];
pub fn add(counter: Count, value: u64) {
    if enabled() {
        COUNTS[counter as usize].fetch_add(value, Relaxed);
    }
}
#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Gauge {
    SourcePixels,
    TargetPixels,
    SourceBytes,
    TargetBytes,
    CacheBytes,
}
const GAUGE_NAMES: &[&str] = &[
    "image.source_pixels",
    "image.target_pixels",
    "image.source_bytes",
    "image.target_bytes",
    "image.cache_bytes",
];
static VALUES: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static PEAKS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
pub fn gauge(gauge: Gauge, value: u64) {
    if enabled() {
        VALUES[gauge as usize].store(value, Relaxed);
        PEAKS[gauge as usize].fetch_max(value, Relaxed);
    }
}
#[derive(Serialize)]
pub struct GaugeValue {
    pub name: &'static str,
    pub current: u64,
    pub peak: u64,
}
#[derive(Serialize)]
pub struct Memory {
    pub working_set_bytes: u64,
    pub peak_working_set_bytes: u64,
}
#[cfg(windows)]
pub fn memory() -> Option<Memory> {
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };
    let mut m = PROCESS_MEMORY_COUNTERS::default();
    m.cb = std::mem::size_of_val(&m) as u32;
    if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb) } != 0 {
        Some(Memory {
            working_set_bytes: m.WorkingSetSize as u64,
            peak_working_set_bytes: m.PeakWorkingSetSize as u64,
        })
    } else {
        None
    }
}
#[cfg(not(windows))]
pub fn memory() -> Option<Memory> {
    None
}
#[derive(Serialize)]
pub struct Quantity {
    pub name: &'static str,
    pub value: u64,
}
#[derive(Serialize)]
pub struct Timing {
    pub name: &'static str,
    pub calls: u64,
    pub total_ms: f64,
    pub mean_ms: f64,
    pub p50_upper_ms: f64,
    pub p95_upper_ms: f64,
    pub p99_upper_ms: f64,
    pub max_ms: f64,
    pub over_50_ms: u64,
}
#[derive(Serialize)]
pub struct Report {
    pub schema: u32,
    pub pid: u32,
    pub version: &'static str,
    pub uptime_seconds: f64,
    pub enabled: bool,
    pub semantics: &'static str,
    pub timings: Vec<Timing>,
    pub counts: Vec<Quantity>,
    pub gauges: Vec<GaugeValue>,
    pub process_memory: Option<Memory>,
}
pub fn snapshot() -> Report {
    let mut timings = Vec::new();
    for (name, counter) in NAMES.iter().zip(COUNTERS.iter()) {
        let count = counter.calls.load(Relaxed);
        if count == 0 {
            continue;
        }
        let total_ms = counter.total.load(Relaxed) as f64 / 1_000_000.0;
        timings.push(Timing {
            name,
            calls: count,
            total_ms,
            mean_ms: total_ms / count as f64,
            p50_upper_ms: counter.percentile(count, 50),
            p95_upper_ms: counter.percentile(count, 95),
            p99_upper_ms: counter.percentile(count, 99),
            max_ms: counter.max.load(Relaxed) as f64 / 1_000_000.0,
            over_50_ms: counter.slow.load(Relaxed),
        });
    }
    timings.sort_by(|a, b| b.total_ms.total_cmp(&a.total_ms));
    Report {
        schema: 2,
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: config().started.elapsed().as_secs_f64(),
        enabled: enabled(),
        semantics: "inclusive wall-clock time; nested/thread totals overlap; power-of-two percentile upper bounds; relaxed live snapshots are approximate; queue rejection counts are not durations",
        counts: COUNT_NAMES
            .iter()
            .zip(COUNTS.iter())
            .map(|(name, c)| Quantity {
                name,
                value: c.load(Relaxed),
            })
            .collect(),
        process_memory: enabled().then(memory).flatten(),
        gauges: GAUGE_NAMES
            .iter()
            .enumerate()
            .map(|(i, name)| GaugeValue {
                name,
                current: VALUES[i].load(Relaxed),
                peak: PEAKS[i].load(Relaxed),
            })
            .collect(),
        timings,
    }
}
pub fn flush() -> std::io::Result<()> {
    let Some(directory) = &config().directory else {
        return Ok(());
    };
    std::fs::create_dir_all(directory)?;
    let path = directory.join(format!(
        "devtabs-{}-{}.json",
        std::process::id(),
        config().instance
    ));
    // Serialize concurrent periodic/final exports. This lock is never on record paths.
    static EXPORT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = EXPORT.lock().unwrap_or_else(|e| e.into_inner());
    let bytes = serde_json::to_vec_pretty(&snapshot())?;
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(temp, path)
}
/// Normal exits export the last partial interval; forced exits retain the last periodic report.
pub struct ExitGuard;
impl Default for ExitGuard {
    fn default() -> Self {
        Self::new()
    }
}
impl ExitGuard {
    pub fn new() -> Self {
        let _ = config();
        Self
    }
}
impl Drop for ExitGuard {
    fn drop(&mut self) {
        let _ = flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn histograms_count_tail_and_percentile_bounds_without_storing_samples() {
        let c = Counter::new();
        for n in [0, 1, 2, 3, 4, 8, 100_000_000] {
            c.record(n);
        }
        assert_eq!(c.calls.load(Relaxed), 7);
        assert_eq!(c.max.load(Relaxed), 100_000_000);
        assert_eq!(c.slow.load(Relaxed), 1);
        assert_eq!(c.percentile(7, 50), 4.0 / 1_000_000.0);
        assert!(c.percentile(7, 95) >= 100.0);
    }
}
