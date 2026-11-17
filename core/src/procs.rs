//! Process control, behind a trait so the switching logic can be tested
//! without touching real processes.

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

pub trait Procs: Send + Sync {
    /// Which of `names` are running right now (case-insensitive match).
    fn running(&self, names: &[String]) -> Vec<String>;
    /// Ends every process whose name is in `names`.
    fn kill(&self, names: &[String]);
    /// Starts `exe` with `args` and returns immediately.
    fn launch(&self, exe: &Path, args: &[String]) -> io::Result<()>;

    /// Kills and waits until the processes are really gone, because their
    /// files stay locked until then. Returns false on timeout.
    fn kill_and_wait(&self, names: &[String], timeout: Duration) -> bool {
        if self.running(names).is_empty() {
            return true;
        }
        let start = Instant::now();
        loop {
            self.kill(names);
            std::thread::sleep(Duration::from_millis(150));
            if self.running(names).is_empty() {
                return true;
            }
            if start.elapsed() > timeout {
                return false;
            }
        }
    }
}

/// Real implementation.
#[derive(Default)]
pub struct SysProcs;

/// One fresh look at the process table. `None` if the OS refuses.
fn process_table() -> Option<sysinfo::System> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new().ok()?;
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    Some(sys)
}

fn is_listed(p: &sysinfo::Process, names: &[String]) -> bool {
    let name = p.name().to_string_lossy();
    names.iter().any(|n| n.eq_ignore_ascii_case(&name))
}

impl Procs for SysProcs {
    fn running(&self, names: &[String]) -> Vec<String> {
        let Some(sys) = process_table() else { return Vec::new() };
        let mut found: Vec<String> = sys
            .processes()
            .values()
            .filter(|p| is_listed(p, names))
            .map(|p| p.name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        found.dedup();
        found
    }

    fn kill(&self, names: &[String]) {
        let Some(sys) = process_table() else { return };
        for p in sys.processes().values().filter(|p| is_listed(p, names)) {
            p.kill();
        }
    }

    fn launch(&self, exe: &Path, args: &[String]) -> io::Result<()> {
        let mut cmd = std::process::Command::new(exe);
        cmd.args(args);
        if let Some(dir) = exe.parent() {
            cmd.current_dir(dir);
        }
        cmd.spawn().map(|_| ())
    }
}
