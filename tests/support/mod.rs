use std::{
    fs,
    path::PathBuf,
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

pub struct Fixture(pub PathBuf);
impl Fixture {
    pub fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs parallel {}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct ChildGuard(pub Child);
impl ChildGuard {
    pub fn spawn(command: &mut Command) -> Self {
        Self(command.spawn().unwrap())
    }
    pub fn wait(&mut self, timeout: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
        let until = Instant::now() + timeout;
        loop {
            if let Some(status) = self.0.try_wait()? {
                return Ok(Some(status));
            }
            if Instant::now() >= until {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
