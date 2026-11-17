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
///
/// One process table is kept and brought up to date on each look, instead
/// of building a new one every time: closing the client asks many times in
/// a row.
#[derive(Default)]
pub struct SysProcs {
    table: std::sync::Mutex<Option<sysinfo::System>>,
}

impl SysProcs {
    /// Runs `f` on a fresh look at the process table. `None` if the OS refuses.
    fn with_table<R>(&self, f: impl FnOnce(&sysinfo::System) -> R) -> Option<R> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
        let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        if table.is_none() {
            *table = System::new().ok();
        }
        let sys = table.as_mut()?;
        // `true` drops processes that have ended, so none is reported twice.
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
        Some(f(sys))
    }
}

fn is_listed(p: &sysinfo::Process, names: &[String]) -> bool {
    let name = p.name().to_string_lossy();
    names.iter().any(|n| n.eq_ignore_ascii_case(&name))
}

impl Procs for SysProcs {
    fn running(&self, names: &[String]) -> Vec<String> {
        self.with_table(|sys| {
            let mut found: Vec<String> = sys
                .processes()
                .values()
                .filter(|p| is_listed(p, names))
                .map(|p| p.name().to_string_lossy().into_owned())
                .collect();
            found.sort();
            found.dedup();
            found
        })
        .unwrap_or_default()
    }

    fn kill(&self, names: &[String]) {
        self.with_table(|sys| {
            for p in sys.processes().values().filter(|p| is_listed(p, names)) {
                p.kill();
            }
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_processes_come_and_go_with_one_kept_table() {
        let procs = SysProcs::default();
        let name_of = |pid: u32| {
            let pid = sysinfo::Pid::from_u32(pid);
            procs.with_table(|sys| sys.process(pid).map(|p| p.name().to_string_lossy().into_owned())).flatten()
        };
        assert!(procs.running(&["no-such-process-qsw".to_string()]).is_empty());

        // Something that just waits; only this one process is ended below.
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = std::process::Command::new("sleep");
            c.arg("30");
            c
        };
        let Ok(mut child) = cmd.stdout(std::process::Stdio::null()).spawn() else { return };
        let own = name_of(child.id()).expect("a started process is in the table");
        assert_eq!(procs.running(&[own.to_uppercase()]), vec![own.clone()], "found, whatever the case");
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(name_of(child.id()).is_none(), "gone from the kept table once it has ended");
    }
}
