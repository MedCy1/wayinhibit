use std::ffi::OsString;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
}

impl CommandSpec {
    pub fn spawn(&self) -> Result<ManagedChild, String> {
        // The terminal is handed to the command only when wayinhibit owns it right now, i.e.
        // when it was started in the foreground of a terminal. Otherwise (script, service,
        // background job) there is nothing to hand over.
        let terminal_owner = foreground_group();

        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            // Run in its own process group so terminate_and_wait can signal the whole
            // group (e.g. a shell script's own background children), not just this PID.
            .process_group(0);
        if terminal_owner.is_some() {
            // SAFETY: the closure only calls async-signal-safe functions (setpgid, getpid,
            // sigprocmask, tcsetpgrp) and does not allocate.
            unsafe {
                command.pre_exec(|| {
                    // Done here, before the command starts, so it can read the terminal from
                    // its very first instruction: a process in a background group is stopped
                    // by SIGTTIN as soon as it reads. Errors are ignored on purpose: without
                    // the terminal the command still runs, as it did before.
                    libc::setpgid(0, 0);
                    set_foreground(libc::getpid());
                    Ok(())
                });
            }
        }

        let child = command.spawn().map_err(|err| {
            format!(
                "failed to start '{}': {err}",
                self.program.to_string_lossy()
            )
        })?;

        Ok(ManagedChild {
            pid: child.id(),
            terminal_owner,
            exited: None,
        })
    }
}

/// What a child is doing right now.
pub enum ChildState {
    Running,
    /// Stopped by a job-control signal (Ctrl-Z).
    Stopped,
    Exited(u8),
}

pub struct ManagedChild {
    pid: u32,
    /// Our own process group when we handed the terminal to the child; the terminal goes back
    /// to it when the child ends or stops.
    terminal_owner: Option<libc::pid_t>,
    exited: Option<u8>,
}

impl ManagedChild {
    fn pgid(&self) -> libc::pid_t {
        // A `process_group(0)` child's PID doubles as its own process group.
        self.pid as libc::pid_t
    }

    /// Looks at the child without waiting. Stops are reported too, because a command that
    /// owns the terminal can be suspended with Ctrl-Z without wayinhibit hearing about it.
    pub fn poll(&mut self) -> Result<ChildState, String> {
        if let Some(code) = self.exited {
            return Ok(ChildState::Exited(code));
        }

        let mut raw_status: libc::c_int = 0;
        loop {
            // SAFETY: `raw_status` outlives the call.
            let result = unsafe {
                libc::waitpid(
                    self.pid as libc::pid_t,
                    &mut raw_status,
                    libc::WNOHANG | libc::WUNTRACED,
                )
            };
            if result == -1 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(format!("failed to wait for child process: {err}"));
            }
            if result == 0 {
                return Ok(ChildState::Running);
            }
            break;
        }

        if libc::WIFSTOPPED(raw_status) {
            return Ok(ChildState::Stopped);
        }

        let code = exit_code_from_status(ExitStatus::from_raw(raw_status));
        self.exited = Some(code);
        self.give_back_terminal();
        Ok(ChildState::Exited(code))
    }

    pub fn try_wait(&mut self) -> Result<Option<u8>, String> {
        match self.poll()? {
            ChildState::Exited(code) => Ok(Some(code)),
            ChildState::Running | ChildState::Stopped => Ok(None),
        }
    }

    pub fn terminate_and_wait(&mut self, grace_period: Duration) -> Result<u8, String> {
        if let Some(code) = self.try_wait()? {
            return Ok(code);
        }

        send_signal_to_group(self.pid, libc::SIGTERM)?;
        // A suspended command only sees SIGTERM once it is running again.
        let _ = send_signal_to_group(self.pid, libc::SIGCONT);

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

        send_signal_to_group(self.pid, libc::SIGKILL)?;

        loop {
            // A killed process cannot stay stopped, so this ends with the exit.
            match self.poll() {
                Ok(ChildState::Exited(code)) => return Ok(code),
                Ok(_) => thread::sleep(Duration::from_millis(10)),
                Err(err) => return Err(format!("{err} after SIGKILL")),
            }
        }
    }

    /// The command was suspended (Ctrl-Z) while it owned the terminal. Do what a shell expects
    /// of a job: give the terminal back, suspend too, and when resumed wake the command up and
    /// take the terminal again if we are back in the foreground.
    pub fn relay_stop(&mut self) {
        let Some(own_group) = self.terminal_owner else {
            return;
        };

        set_foreground(own_group);
        // SAFETY: plain signal to ourselves; returns once something sends SIGCONT.
        unsafe { libc::kill(libc::getpid(), libc::SIGSTOP) };

        let _ = send_signal_to_group(self.pid, libc::SIGCONT);
        // Resumed with `fg` the terminal is ours again and goes to the command; resumed with
        // `bg` it belongs to the shell or another job, and must stay there.
        if current_foreground_group() == Some(own_group) {
            set_foreground(self.pgid());
        }
    }

    /// Returns the terminal to wayinhibit once the command no longer needs it.
    fn give_back_terminal(&self) {
        if let Some(own_group) = self.terminal_owner {
            if current_foreground_group() == Some(self.pgid()) {
                set_foreground(own_group);
            }
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.give_back_terminal();
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

fn kill(target: i32, signal: libc::c_int) -> std::io::Result<()> {
    if unsafe { libc::kill(target, signal) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Sends `signal` to a single process. The raw error is returned so callers can tell
/// "no such process" (`ESRCH`) from a real failure.
pub(crate) fn send_signal(pid: u32, signal: libc::c_int) -> std::io::Result<()> {
    let pid = i32::try_from(pid).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "process id is out of range",
        )
    })?;
    kill(pid, signal)
}

/// Sends `signal` to every process in the group led by `pgid` (a `process_group(0)`
/// child's PID doubles as its own pgid). A negative PID targets the whole group.
pub(crate) fn send_signal_to_group(pgid: u32, signal: libc::c_int) -> Result<(), String> {
    let pgid = i32::try_from(pgid).map_err(|_| "process id is out of range".to_string())?;
    kill(-pgid, signal)
        .map_err(|err| format!("failed to send signal {signal} to process group {pgid}: {err}"))
}

/// Runs `action` with SIGTTOU blocked. A process outside the foreground group that changes the
/// terminal's foreground group, or writes to a terminal with TOSTOP set, is stopped by SIGTTOU
/// unless it blocks or ignores it, in which case the operation simply goes through.
pub(crate) fn with_sigttou_blocked<T>(action: impl FnOnce() -> T) -> T {
    // SAFETY: `sigset_t` is a plain C type initialised by `sigemptyset` before use.
    unsafe {
        let mut blocked: libc::sigset_t = std::mem::zeroed();
        let mut previous: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut blocked);
        libc::sigaddset(&mut blocked, libc::SIGTTOU);
        libc::sigprocmask(libc::SIG_BLOCK, &blocked, &mut previous);
        let result = action();
        libc::sigprocmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
        result
    }
}

fn current_foreground_group() -> Option<libc::pid_t> {
    // SAFETY: plain system call; fails (-1) when stdin is not our controlling terminal.
    let group = unsafe { libc::tcgetpgrp(0) };
    (group > 0).then_some(group)
}

/// Our own process group, if stdin is our terminal and we are in its foreground.
fn foreground_group() -> Option<libc::pid_t> {
    // SAFETY: plain system call.
    let own = unsafe { libc::getpgrp() };
    (current_foreground_group() == Some(own)).then_some(own)
}

fn set_foreground(group: libc::pid_t) -> bool {
    // SAFETY: plain system call on stdin.
    with_sigttou_blocked(|| unsafe { libc::tcsetpgrp(0, group) } == 0)
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
