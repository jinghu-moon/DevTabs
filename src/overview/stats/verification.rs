use super::*;
use std::{fs, path::PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("DevTabs-complete-stats-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, path: &str, contents: &[u8]) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn same_disk(a: &Stats, b: &Stats) {
    assert_eq!(
        (a.files, a.bytes, a.artifact_bytes, a.dependency_bytes),
        (b.files, b.bytes, b.artifact_bytes, b.dependency_bytes)
    );
    assert_eq!(a.directories, b.directories);
    assert_eq!(a.artifacts, b.artifacts);
    assert_eq!(a.disk_issues, b.disk_issues);
}
fn same_source(a: &Stats, b: &Stats) {
    assert_eq!(
        (a.source_files, a.source_bytes),
        (b.source_files, b.source_bytes)
    );
    assert_eq!(a.languages, b.languages);
    assert_eq!(a.categories, b.categories);
    assert_eq!(a.source_issues, b.source_issues);
}

#[test]
fn parallel_scans_match_complete_reference_and_preserve_nested_ignore_rules() {
    let f = Fixture::new();
    f.file("Cargo.toml", b"[workspace]\n");
    f.file("crates/组件/src/lib.rs", b"fn main() {}\n");
    f.file("crates/组件/target/build.bin", &[1; 17]);
    f.file("docs/a.md", b"# Hello\n");
    f.file("vendor/lib.c", &[2; 23]);
    f.file("target/debug/data.bin", &[3; 37]);
    f.file("src/main.rs", b"fn main() {}\n");
    f.file("src/tests/test_main.rs", b"#[test] fn test() {}\n");
    f.file("src/skip.rs", &[4; 101]);
    f.file("src/keep.rs", &[5; 103]);
    f.file("src/.ignore", b"skip.rs\n");
    f.file(".ignore", b"*.rs\n!src/\n!src/*.rs\n!crates/\n!crates/**\n");
    let flag = Arc::new(AtomicU64::new(1));
    let mut expected = Stats::default();
    assert!(disk_reference(
        &f.0,
        &flag,
        1,
        &mut expected,
        Instant::now()
    ));
    for threads in [1, 2, 4] {
        let mut actual = Stats::default();
        assert!(disk::scan(
            &f.0,
            &flag,
            1,
            &mut actual,
            threads,
            &mut |_| {}
        ));
        same_disk(&expected, &actual);
        assert!(actual.disk_ready && !actual.disk_scanning);
        assert!(actual.directories.iter().any(|(name, _)| name == "crates"));
        assert!(actual.directories.iter().any(|(name, _)| name == "."));
    }
    let mut expected = Stats::default();
    assert!(scan_source_serial(
        &f.0,
        &flag,
        1,
        &mut expected,
        Instant::now()
    ));
    for threads in [1, 2] {
        let mut actual = Stats::default();
        assert!(source::scan(&f.0, &flag, 1, &mut actual, threads));
        same_source(&expected, &actual);
    }
}

#[test]
fn canceled_progress_never_becomes_a_completed_cache_and_retry_finishes() {
    let f = Fixture::new();
    f.file("crates/core/src/lib.rs", b"fn main() {}\n");
    let home = Fixture::new();
    let flag = Arc::new(AtomicU64::new(1));
    let mut progress = vec![];
    let canceled = cached_at_with_progress(&f.0, &home.0, &flag, 1, false, &mut |s| {
        assert!(s.source_ready && s.disk_scanning && !s.disk_ready);
        progress.push(s.scanned_entries);
        flag.store(2, Ordering::Relaxed);
    });
    assert_eq!(progress, vec![0]);
    assert!(!canceled.disk_ready);
    assert!(!home.0.join("stats").exists());
    let result = cached_at(&f.0, &home.0, &flag, 2, false);
    assert!(result.disk_ready && !result.disk_scanning && !result.partial);
    let cached = cached_at_with_progress(&f.0, &home.0, &flag, 2, false, &mut |_| {
        panic!("cache hit traversed")
    });
    same_disk(&result, &cached);
}

#[test]
fn old_truncated_cache_and_in_progress_cache_are_not_reused() {
    let f = Fixture::new();
    let home = Fixture::new();
    f.file("crates/core/lib.rs", b"new code\n");
    let flag = Arc::new(AtomicU64::new(1));
    let key = crate::config::key(&f.0);
    let hash = key.bytes().fold(2166136261u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(16777619)
    });
    let path = home.0.join("stats").join(format!("{hash:016x}.json"));
    fs::create_dir(path.parent().unwrap()).unwrap();
    for (schema, scanning) in [(4, false), (5, true)] {
        let mut stamp = cache_stamp(&f.0);
        stamp.push(schema);
        stamp.extend(cache_stamp(&home.0.join("stats-refresh")));
        let bad = Cached {
            path: key.clone(),
            source_at: crate::config::now(),
            disk_at: crate::config::now(),
            stamp,
            stats: Stats {
                source_ready: true,
                disk_ready: true,
                files: 999,
                disk_scanning: scanning,
                ..Default::default()
            },
        };
        fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        let result = cached_at(&f.0, &home.0, &flag, 1, false);
        assert_eq!(result.files, 1);
        assert!(result.disk_ready && !result.disk_scanning);
    }
}

#[test]
#[cfg(windows)]
fn directory_junctions_do_not_count_files_outside_the_project() {
    use std::os::windows::process::CommandExt;
    let f = Fixture::new();
    let outside = Fixture::new();
    f.file("src/lib.rs", b"project\n");
    outside.file("outside.rs", &[9; 301]);
    let output = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(f.0.join("external"))
        .arg(&outside.0)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let flag = Arc::new(AtomicU64::new(1));
    let mut s = Stats::default();
    assert!(scan_disk(&f.0, &flag, 1, &mut s, Instant::now()));
    assert_eq!((s.files, s.bytes), (1, 8));
    assert!(scan_source(&f.0, &flag, 1, &mut s, Instant::now()));
    assert_eq!((s.source_files, s.source_bytes), (1, 8));
    assert_eq!(fs::read(outside.0.join("outside.rs")).unwrap().len(), 301);
}

#[test]
#[ignore = "explicit >200k-file completeness and aggregation fixture"]
fn over_two_hundred_thousand_entries_are_fully_counted_in_both_scans() {
    let f = Fixture::new();
    let contents = b"pub fn complete() {}\n";
    // Hard links keep the fixture small on disk. Logical bytes count each path,
    // exactly as the existing statistics contract and independent reference do.
    for group in 0..201 {
        let parent = f.0.join("crates").join(format!("part-{group}"));
        fs::create_dir_all(&parent).unwrap();
        let seed = parent.join("seed.rs");
        fs::write(&seed, contents).unwrap();
        for index in 0..999 {
            fs::hard_link(&seed, parent.join(format!("file-{index}.rs"))).unwrap();
        }
    }
    f.file("target/generated.bin", &[1; 73]);
    f.file("vendor/dependency.bin", &[2; 79]);
    f.file("docs/说明.md", b"# Complete\n");
    let flag = Arc::new(AtomicU64::new(1));
    let mut s = Stats::default();
    let start = Instant::now();
    assert!(scan_disk(&f.0, &flag, 1, &mut s, start));
    let disk_ms = start.elapsed().as_millis();
    assert!(s.scanned_entries > 200_000);
    assert_eq!(s.files, 201_003);
    assert_eq!(s.bytes, 201_000 * contents.len() as u64 + 73 + 79 + 11);
    assert!(
        s.directories
            .contains(&("crates".into(), 201_000 * contents.len() as u64))
    );
    assert_eq!((s.artifact_bytes, s.dependency_bytes), (73, 79));
    assert!(!s.partial && s.disk_issues.is_empty());
    let start = Instant::now();
    assert!(scan_source(&f.0, &flag, 1, &mut s, start));
    assert_eq!(s.source_files, 201_001);
    assert_eq!(s.source_bytes, 201_000 * contents.len() as u64 + 11);
    assert!(!s.partial && s.source_issues.is_empty());
    println!(
        "{}",
        serde_json::json!({"files":s.files,"bytes":s.bytes,"entries":s.scanned_entries,"source_files":s.source_files,"disk_ms":disk_ms,"source_ms":start.elapsed().as_millis()})
    );
}

#[test]
#[ignore = "explicit read-only complete-scan comparison; DEVTABS_STATS_BENCH_ROOT required"]
fn complete_project_scan_comparison() {
    let root = std::env::var_os("DEVTABS_STATS_BENCH_ROOT").expect("explicit read-only root");
    let root = Path::new(&root);
    let flag = Arc::new(AtomicU64::new(1));
    let mut disk_truth = Stats::default();
    let mut source_truth = Stats::default();
    assert!(disk_reference(
        root,
        &flag,
        1,
        &mut disk_truth,
        Instant::now()
    ));
    assert!(scan_source_serial(
        root,
        &flag,
        1,
        &mut source_truth,
        Instant::now()
    ));
    assert!(disk_truth.disk_issues.is_empty() && source_truth.source_issues.is_empty());
    for round in 0..6 {
        let mut modes = vec![
            "disk-std",
            "disk-dua1",
            "disk-dua2",
            "disk-dua4",
            "source-serial",
            "source-parallel2",
        ];
        if round % 2 == 1 {
            modes.reverse();
        }
        for mode in modes {
            let mut s = Stats::default();
            let before = resources();
            let start = Instant::now();
            assert!(match mode {
                "disk-std" => disk_reference(root, &flag, 1, &mut s, start),
                "disk-dua1" => disk::scan(root, &flag, 1, &mut s, 1, &mut |_| {}),
                "disk-dua2" => disk::scan(root, &flag, 1, &mut s, 2, &mut |_| {}),
                "disk-dua4" => disk::scan(root, &flag, 1, &mut s, 4, &mut |_| {}),
                "source-serial" => scan_source_serial(root, &flag, 1, &mut s, start),
                _ => source::scan(root, &flag, 1, &mut s, 2),
            });
            let ms = start.elapsed().as_secs_f64() * 1000.;
            let after = resources();
            if mode.starts_with("disk") {
                same_disk(&s, &disk_truth);
            } else {
                same_source(&s, &source_truth);
            }
            println!(
                "{}",
                serde_json::json!({"round":round,"mode":mode,"ms":ms,"cpu_ms":after.0.saturating_sub(before.0) as f64 / 10000.,"working_set_bytes":after.1,"process_peak_working_set_bytes":after.2,"disk_files":s.files,"disk_bytes":s.bytes,"source_files":s.source_files,"source_bytes":s.source_bytes,"directories":s.directories})
            );
        }
    }
}

// Process peak is cumulative across modes, not an isolated peak per algorithm.
#[cfg(windows)]
fn resources() -> (u64, usize, usize) {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    };
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let mut memory = PROCESS_MEMORY_COUNTERS::default();
    // SAFETY: these calls inspect this process with fully sized writable output
    // structs. The pseudo handle is valid and is not owned by this function.
    unsafe {
        let handle = GetCurrentProcess();
        assert_ne!(
            GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user),
            0
        );
        assert_ne!(
            K32GetProcessMemoryInfo(
                handle,
                &mut memory,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32
            ),
            0
        );
    }
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    (
        ticks(kernel) + ticks(user),
        memory.WorkingSetSize,
        memory.PeakWorkingSetSize,
    )
}
#[cfg(not(windows))]
fn resources() -> (u64, usize, usize) {
    (0, 0, 0)
}
fn disk_reference(
    root: &Path,
    generation: &Arc<AtomicU64>,
    ticket: u64,
    out: &mut Stats,
    _start: Instant,
) -> bool {
    let _perf = crate::perf::span(crate::perf::Metric::OverviewDiskScan);
    // ReadDir keeps Windows enumeration metadata, avoiding a new path-based stat
    // for each file. Aggregate once per directory rather than hashing/cloning
    // the top-level name for every file.
    let mut groups = vec![(".".to_string(), [0u64; 3], [0u64; 3])];
    let mut stack = vec![(root.to_path_buf(), 0usize, 0u8)];
    let mut entries = 0usize;
    while let Some((directory, group, category)) = stack.pop() {
        if generation.load(Ordering::Relaxed) != ticket {
            return false;
        }
        let Ok(children) = std::fs::read_dir(&directory) else {
            issue(out, false, ScanIssue::ReadError);
            continue;
        };
        for entry in children {
            if entries.is_multiple_of(64) && generation.load(Ordering::Relaxed) != ticket {
                return false;
            }
            entries += 1;
            let Ok(entry) = entry else {
                issue(out, false, ScanIssue::ReadError);
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                issue(out, false, ScanIssue::ReadError);
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                let child_group = if group == 0 && directory == root {
                    groups.push((
                        entry.file_name().to_string_lossy().into_owned(),
                        [0; 3],
                        [0; 3],
                    ));
                    groups.len() - 1
                } else {
                    group
                };
                let child_category = if category != 0 {
                    category
                } else {
                    match entry.file_name().to_string_lossy().as_ref() {
                        "target" | "build" | "dist" | ".next" | "obj" | "bin" => 1,
                        "vendor" | "node_modules" | ".venv" => 2,
                        _ => 0,
                    }
                };
                stack.push((entry.path(), child_group, child_category));
            } else if kind.is_file() {
                match entry.metadata() {
                    Ok(metadata) => {
                        out.files += 1;
                        out.bytes = out.bytes.saturating_add(metadata.len());
                        match category {
                            1 => {
                                out.artifact_bytes =
                                    out.artifact_bytes.saturating_add(metadata.len())
                            }
                            2 => {
                                out.dependency_bytes =
                                    out.dependency_bytes.saturating_add(metadata.len())
                            }
                            _ => {}
                        }
                        let c = category as usize;
                        groups[group].1[c] = groups[group].1[c].saturating_add(metadata.len());
                        groups[group].2[c] += 1;
                    }
                    Err(_) => issue(out, false, ScanIssue::ReadError),
                }
            }
        }
    }
    out.directories = groups
        .iter()
        .filter(|(_, _, files)| files[0] > 0)
        .map(|(name, bytes, _)| (name.clone(), bytes[0]))
        .collect();
    out.directories
        .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out.artifacts = groups
        .into_iter()
        .filter(|(_, _, files)| files[1] + files[2] > 0)
        .map(|(name, bytes, _)| (name, bytes[1].saturating_add(bytes[2])))
        .collect();
    out.artifacts
        .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out.artifacts.truncate(6);
    out.directories.truncate(6);
    generation.load(Ordering::Relaxed) == ticket
}
