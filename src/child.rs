use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use std::os::unix::process::{CommandExt, ExitStatusExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    pub fn spawn(&self) -> Result<ManagedChild, String> {
        let child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            // Run in its own process group so terminate_and_wait can signal the whole
            // group (e.g. a shell script's own background children), not just this PID.
            .process_group(0)
            .spawn()
            .map_err(|err| format!("failed to start '{}': {err}", self.program))?;

        Ok(ManagedChild { child })
    }
}

pub struct ManagedChild {
    child: Child,
}

impl ManagedChild {
    pub fn try_wait(&mut self) -> Result<Option<u8>, String> {
        self.child
            .try_wait()
            .map_err(|err| format!("failed to wait for child process: {err}"))
            .map(|status| status.map(exit_code_from_status))
    }

    pub fn terminate_and_wait(&mut self, grace_period: Duration) -> Result<u8, String> {
        if let Some(code) = self.try_wait()? {
            return Ok(code);
        }

        send_signal_to_group(self.child.id(), libc::SIGTERM)?;

        let deadline = Instant::now() + grace_period;
        loop {
            if let Some(code) = self.try_wait()? {
                return Ok(code);
            }

            if Instant::now() >= deadline {
                break;
            }

            thread::sleep(Duration::from_millis(50));
        }

        send_signal_to_group(self.child.id(), libc::SIGKILL)?;

        self.child
            .wait()
            .map(exit_code_from_status)
            .map_err(|err| format!("failed to wait for child process after SIGKILL: {err}"))
    }
}

fn exit_code_from_status(status: ExitStatus) -> u8 {
    if let Some(code) = status.code() {
        return u8::try_from(code).unwrap_or(1);
    }

    if let Some(signal) = status.signal() {
        return 128u8.saturating_add(u8::try_from(signal).unwrap_or(0));
    }

    1
}

fn kill(target: i32, signal: libc::c_int) -> Result<(), String> {
    if unsafe { libc::kill(target, signal) } == -1 {
        return Err(format!(
            "failed to send signal {signal} to {target}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

pub(crate) fn send_signal(pid: u32, signal: libc::c_int) -> Result<(), String> {
    let pid = i32::try_from(pid).map_err(|_| "process id is out of range".to_string())?;
    kill(pid, signal)
}

/// Sends `signal` to every process in the group led by `pgid` (a `process_group(0)`
/// child's PID doubles as its own pgid). A negative PID targets the whole group.
fn send_signal_to_group(pgid: u32, signal: libc::c_int) -> Result<(), String> {
    let pgid = i32::try_from(pgid).map_err(|_| "process id is out of range".to_string())?;
    kill(-pgid, signal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> ManagedChild {
        CommandSpec {
            program: "sh".into(),
            args: vec!["-c".into(), script.into()],
        }
        .spawn()
        .unwrap()
    }

    fn status(script: &str) -> ExitStatus {
        Command::new("sh").args(["-c", script]).status().unwrap()
    }

    fn wait_for(mut child: ManagedChild) -> u8 {
        loop {
            if let Some(code) = child.try_wait().unwrap() {
                return code;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[test]
    fn exit_code_is_passed_through() {
        assert_eq!(exit_code_from_status(status("exit 0")), 0);
        assert_eq!(exit_code_from_status(status("exit 3")), 3);
    }

    #[test]
    fn signal_death_maps_to_128_plus_signal() {
        assert_eq!(exit_code_from_status(status("kill -TERM $$")), 143);
        assert_eq!(exit_code_from_status(status("kill -KILL $$")), 137);
    }

    #[test]
    fn spawn_failure_is_reported() {
        let spec = CommandSpec {
            program: "/nonexistent/wayinhibit-test".into(),
            args: vec![],
        };
        assert!(spec.spawn().is_err());
    }

    #[test]
    fn try_wait_reports_exit_code() {
        assert_eq!(wait_for(sh("exit 7")), 7);
    }

    #[test]
    fn terminate_stops_a_cooperative_child() {
        let mut child = sh("sleep 30");
        assert_eq!(
            child.terminate_and_wait(Duration::from_secs(2)).unwrap(),
            143
        );
    }

    #[test]
    fn terminate_on_finished_child_returns_its_code() {
        let mut child = sh("exit 5");
        thread::sleep(Duration::from_millis(200));
        assert_eq!(child.terminate_and_wait(Duration::from_secs(1)).unwrap(), 5);
    }

    #[test]
    fn terminate_kills_the_whole_process_group() {
        let pidfile =
            std::env::temp_dir().join(format!("wayinhibit-grandchild-{}", std::process::id()));
        let mut child = sh(&format!("sleep 30 & echo $! > {}; wait", pidfile.display()));
        let grandchild = loop {
            if let Some(pid) = std::fs::read_to_string(&pidfile)
                .ok()
                .and_then(|s| s.trim().parse::<i32>().ok())
            {
                break pid;
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert!(alive(grandchild));
        child.terminate_and_wait(Duration::from_secs(2)).unwrap();
        thread::sleep(Duration::from_millis(100));
        // A killed-but-unreaped grandchild would still answer kill(0); sh was killed, so init reaps it.
        let gone = (0..50).any(|_| {
            thread::sleep(Duration::from_millis(20));
            !alive(grandchild)
        });
        let _ = std::fs::remove_file(&pidfile);
        assert!(gone, "grandchild {grandchild} survived");
    }

    #[test]
    fn stubborn_child_is_sigkilled_after_grace_period() {
        let mut child = sh("trap '' TERM; while :; do sleep 0.1; done");
        thread::sleep(Duration::from_millis(200));
        let start = Instant::now();
        let code = child
            .terminate_and_wait(Duration::from_millis(300))
            .unwrap();
        assert_eq!(code, 137);
        assert!(start.elapsed() >= Duration::from_millis(300));
    }
}
