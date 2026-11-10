//! Isolated allocation evidence for the actual atomic-save path, not just the encoder.
use devtabs::{
    editor::{self, Save},
    preview::{self, Encoding},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Instant,
};
struct Allocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn added(bytes: usize) {
    let current = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(current, Relaxed);
    TOTAL.fetch_add(bytes, Relaxed);
}
// SAFETY: Every operation delegates the identical pointer/layout to System. Counters
// use only lock-free atomics and never allocate or change allocator behavior.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            added(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            added(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(p, layout) }
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let out = unsafe { System.realloc(p, layout, size) };
        if !out.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            added(size);
        }
        out
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

#[test]
fn atomic_save_avoids_multiple_full_encoded_copies() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("DevTabs-save-budget-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("file.txt");
    let source = "中文😀 abcdefghijklmnopqrstuvwxyz\r\n".repeat(6000);
    std::fs::write(&path, editor::encode(&source, Encoding::Utf16Le)).unwrap();
    let request = Save {
        snapshot: Arc::new(preview::read(&path).unwrap()),
        source,
        revision: 1,
    };
    let before = LIVE.load(Relaxed);
    PEAK.store(before, Relaxed);
    let result = editor::save(&request).unwrap();
    let peak = PEAK.load(Relaxed).saturating_sub(before);
    assert_eq!(
        result.bytes,
        editor::encode(&request.source, Encoding::Utf16Le)
    );
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        peak < 2_000_000,
        "atomic save retained {peak} additional heap bytes for a {}-byte UTF16 file",
        result.bytes.len()
    );
}

#[test]
fn streaming_encoder_handles_chunk_boundaries_short_writes_and_errors() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    use std::io::{self, Write};
    struct Sink {
        bytes: Vec<u8>,
        largest: usize,
        interrupt: bool,
        fail_at: Option<usize>,
    }
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.largest = self.largest.max(bytes.len());
            if self.interrupt {
                self.interrupt = false;
                return Err(io::ErrorKind::Interrupted.into());
            }
            if self.fail_at.is_some_and(|at| self.bytes.len() >= at) {
                return Err(io::Error::from_raw_os_error(112));
            }
            let n = bytes.len().min(7);
            self.bytes.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let text = format!("{}😀中文\r\n{}", "a".repeat(4095), "😀中\n".repeat(2000));
    for encoding in [
        Encoding::Utf8,
        Encoding::Utf8Bom,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for text in ["", text.as_str()] {
            let mut sink = Sink {
                bytes: Vec::new(),
                largest: 0,
                interrupt: true,
                fail_at: None,
            };
            let count = editor::write_encoded(text, encoding, &mut sink).unwrap();
            assert_eq!(sink.bytes, editor::encode(text, encoding));
            assert_eq!(count, sink.bytes.len());
            assert!(sink.largest <= 8192);
        }
        let mut sink = Sink {
            bytes: Vec::new(),
            largest: 0,
            interrupt: false,
            fail_at: Some(100),
        };
        assert_eq!(
            editor::write_encoded(&text, encoding, &mut sink)
                .unwrap_err()
                .raw_os_error(),
            Some(112)
        );
    }
    struct Zero;
    impl Write for Zero {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Ok(0)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    assert_eq!(
        editor::write_encoded("text", Encoding::Utf8, &mut Zero)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WriteZero
    );
}

#[test]
#[ignore = "isolated Release atomic save before/after allocation and latency workload"]
fn atomic_save_workload() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let root = std::env::temp_dir().join(format!("DevTabs-stream-save-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("file.txt");
    let mut cases = Vec::new();
    for encoding in [
        Encoding::Utf8,
        Encoding::Utf8Bom,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        let original = "中文😀 abcdefghijklmnopqrstuvwxyz\r\n".repeat(6000);
        // UTF-16 case is below the 512 KiB encoded edit limit, including BOM.
        let source = original.replacen("abc", "ABC", 1);
        for iteration in 0..5 {
            std::fs::write(&path, editor::encode(&original, encoding)).unwrap();
            let request = Save {
                snapshot: Arc::new(preview::read(&path).unwrap()),
                source: source.clone(),
                revision: 1,
            };
            let before = LIVE.load(Relaxed);
            PEAK.store(before, Relaxed);
            let total = TOTAL.load(Relaxed);
            let start = Instant::now();
            let result = editor::save(&request).unwrap();
            let save_ms = start.elapsed().as_secs_f64() * 1000.0;
            let peak = PEAK.load(Relaxed).saturating_sub(before);
            let allocated = TOTAL.load(Relaxed) - total;
            assert_eq!(result.bytes, editor::encode(&source, encoding));
            assert_eq!(result.source.as_ref(), source);
            cases.push(serde_json::json!({"encoding":format!("{encoding:?}"),"iteration":iteration,"source_bytes":source.len(),"encoded_bytes":result.bytes.len(),"save_ms":save_ms,"peak_additional_live_bytes":peak,"allocated_bytes":allocated}));
        }
    }
    std::fs::write(std::env::var_os("DEVTABS_SAVE_REPORT").expect("report path"),serde_json::to_vec_pretty(&serde_json::json!({"cases":cases,"scope":"Actual editor::save, including conflict check, atomic replacement, backup verification and reload; request source/snapshot allocations precede measurement; global System allocation counts, serial process"})).unwrap()).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
