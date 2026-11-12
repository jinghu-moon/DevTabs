//! Complete bounded-parallel traversal. Only directory state is retained.
use super::*;
use std::sync::{Mutex, MutexGuard, TryLockError};

static SCAN: Mutex<()> = Mutex::new(());

fn permit(generation: &AtomicU64, ticket: u64) -> Option<MutexGuard<'static, ()>> {
    loop {
        if generation.load(Ordering::Relaxed) != ticket {
            return None;
        }
        match SCAN.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(error)) => return Some(error.into_inner()),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn category(name: &std::ffi::OsStr, inherited: u8) -> u8 {
    if inherited != 0 {
        return inherited;
    }
    match name.to_str() {
        Some("target" | "build" | "dist" | ".next" | "obj" | "bin") => 1,
        Some("vendor" | "node_modules" | ".venv") => 2,
        _ => 0,
    }
}

#[derive(Default)]
struct Group {
    name: String,
    bytes: [u64; 3],
    files: [u64; 3],
}

fn rankings(out: &mut Stats, groups: &[Group]) {
    out.directories = groups
        .iter()
        .filter(|g| g.files[0] > 0)
        .map(|g| (g.name.clone(), g.bytes[0]))
        .collect();
    out.artifacts = groups
        .iter()
        .filter(|g| g.files[1] + g.files[2] > 0)
        .map(|g| (g.name.clone(), g.bytes[1].saturating_add(g.bytes[2])))
        .collect();
    for rows in [&mut out.directories, &mut out.artifacts] {
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        rows.truncate(6);
    }
}

pub(super) fn scan(
    root: &Path,
    generation: &Arc<AtomicU64>,
    ticket: u64,
    out: &mut Stats,
    threads: usize,
    progress: &mut dyn FnMut(&Stats),
) -> bool {
    let Some(_permit) = permit(generation, ticket) else {
        return false;
    };
    let _perf = crate::perf::span(crate::perf::Metric::OverviewDiskScan);
    out.files = 0;
    out.bytes = 0;
    out.artifact_bytes = 0;
    out.dependency_bytes = 0;
    out.scanned_entries = 0;
    out.disk_issues.clear();
    out.directories.clear();
    out.artifacts.clear();
    out.disk_ready = false;
    out.disk_scanning = true;
    progress(out);
    if generation.load(Ordering::Relaxed) != ticket {
        return false;
    }
    let cancel = generation.clone();
    let mut walk = dua_core::walk(
        root,
        threads.clamp(1, 4),
        dua_core::Order::ParentFirst,
        dua_core::Options::default().queued_batches_per_worker(2),
        move |_| cancel.load(Ordering::Relaxed) == ticket,
    );
    let mut groups = vec![Group {
        name: ".".into(),
        ..Default::default()
    }];
    // ParentFirst guarantees a parent's small aggregation state is present.
    // No full paths or per-file tree nodes are stored in this table.
    let mut directories = Vec::<(usize, u8)>::new();
    let mut published = Instant::now();
    for item in &mut walk {
        if generation.load(Ordering::Relaxed) != ticket {
            return false;
        }
        let entry = match item {
            Ok(entry) => entry,
            Err(_) => {
                issue(out, false, ScanIssue::ReadError);
                continue;
            }
        };
        if entry.depth == 0 && !entry.file_type.is_dir() {
            issue(out, false, ScanIssue::ReadError);
            continue;
        }
        if entry.depth != 0 {
            out.scanned_entries += 1;
        }
        let (mut group, inherited) = entry
            .parent_directory_id
            .map_or((0, 0), |id| directories[id.index()]);
        if entry.file_type.is_dir() {
            if entry.depth == 1 {
                group = groups.len();
                groups.push(Group {
                    name: entry.file_name.to_string_lossy().into_owned(),
                    ..Default::default()
                });
            }
            // A project named "target" is still a project; classify descendants.
            let kind = if entry.depth == 0 {
                0
            } else {
                category(&entry.file_name, inherited)
            };
            let index = entry
                .directory_id
                .expect("walker directory identity")
                .index();
            if directories.len() <= index {
                directories.resize(index + 1, (0, 0));
            }
            directories[index] = (group, kind);
        } else if entry.file_type.is_file() {
            match entry.metadata {
                Some(Ok(metadata)) => {
                    let bytes = metadata.len();
                    out.files += 1;
                    out.bytes = out.bytes.saturating_add(bytes);
                    match inherited {
                        1 => out.artifact_bytes = out.artifact_bytes.saturating_add(bytes),
                        2 => out.dependency_bytes = out.dependency_bytes.saturating_add(bytes),
                        _ => {}
                    }
                    let g = &mut groups[group];
                    g.bytes[inherited as usize] = g.bytes[inherited as usize].saturating_add(bytes);
                    g.files[inherited as usize] += 1;
                }
                _ => issue(out, false, ScanIssue::ReadError),
            }
        }
        if out.scanned_entries.is_multiple_of(256)
            && published.elapsed() >= Duration::from_millis(250)
        {
            rankings(out, &groups);
            progress(out);
            published = Instant::now();
        }
    }
    if generation.load(Ordering::Relaxed) != ticket {
        return false;
    }
    rankings(out, &groups);
    out.disk_scanning = false;
    out.disk_ready = true;
    out.partial = !out.source_issues.is_empty() || !out.disk_issues.is_empty();
    crate::perf::add(
        crate::perf::Count::StatisticsDiskEntries,
        out.scanned_entries,
    );
    crate::perf::add(crate::perf::Count::StatisticsDiskFiles, out.files);
    true
}
