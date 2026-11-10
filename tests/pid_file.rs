//! Cross-process behavior of `--pid-file` and `--toggle`, against the real binary.
//!
//! Everything runs with `--dry-run`, so no Wayland compositor is needed. POSIX locks never
//! conflict inside one process, which is why this cannot live in the unit tests.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_wayinhibit");
const PATIENCE: Duration = Duration::from_secs(10);

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("wayinhibit-it-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn pid_file(&self) -> PathBuf {
        self.path("w.pid")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A wayinhibit process that is killed if the test panics before it has been collected.
struct Instance(Option<Child>);

impl Instance {
    fn id(&self) -> u32 {
        self.0.as_ref().unwrap().id()
    }

    fn signal(&self, signal: libc::c_int) {
        let pid = i32::try_from(self.id()).unwrap();
        assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
    }

    fn try_wait(&mut self) -> Option<ExitStatus> {
        self.0.as_mut().unwrap().try_wait().unwrap()
    }

    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(status) = self.try_wait() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "process {} did not exit",
                self.id()
            );
            sleep(Duration::from_millis(10));
        }
    }

    fn finish(mut self) -> Output {
        self.0.take().unwrap().wait_with_output().unwrap()
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn command(pid_file: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.args(["--dry-run", "--quiet", "--pid-file"])
        .arg(pid_file)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    cmd
}

fn spawn(pid_file: &Path, args: &[&str]) -> Instance {
    Instance(Some(command(pid_file, args).spawn().unwrap()))
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        sleep(Duration::from_millis(10));
    }
}

/// Starts a foreground instance and waits until it owns the pid file.
fn start_owner(pid_file: &Path, args: &[&str]) -> Instance {
    let instance = spawn(pid_file, args);
    wait_for("the instance to own the pid file", || {
        read_pid(pid_file) == Some(instance.id())
    });
    instance
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_second_instance_still_does_its_job_and_leaves_the_pid_file_to_the_first() {
    let sandbox = Sandbox::new("second");
    let pid_file = sandbox.pid_file();
    let mut first = start_owner(&pid_file, &[]);

    // A command wrapped by a second instance must run: the pid file is only bookkeeping.
    let marker = sandbox.path("ran");
    let script = format!("echo ran > '{}'", marker.display());
    let second = command(&pid_file, &["--", "sh", "-c", &script])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
    let message = stderr(&second);
    assert!(message.contains("warning"), "{message}");
    assert!(
        message.contains(&format!("in use by PID {}", first.id())),
        "{message}"
    );
    assert!(marker.exists(), "the wrapped command must have run");

    // The second instance exiting must not take the first one's file with it.
    assert!(first.try_wait().is_none(), "first instance must survive");
    assert_eq!(read_pid(&pid_file), Some(first.id()));

    first.signal(libc::SIGTERM);
    assert_eq!(first.wait().code(), Some(0));
    assert!(!pid_file.exists(), "pid file removed on exit");
}

#[test]
fn toggle_stops_the_running_instance() {
    let sandbox = Sandbox::new("toggle-stop");
    let pid_file = sandbox.pid_file();
    let mut running = start_owner(&pid_file, &[]);

    let toggle = command(&pid_file, &["--toggle"]).output().unwrap();
    assert_eq!(toggle.status.code(), Some(0), "{}", stderr(&toggle));
    assert!(toggle.stderr.is_empty(), "{}", stderr(&toggle));

    assert_eq!(running.wait().code(), Some(0));
    assert!(!pid_file.exists());
}

#[test]
fn toggle_starts_an_instance_when_none_is_running() {
    let sandbox = Sandbox::new("toggle-start");
    let pid_file = sandbox.pid_file();
    let mut started = start_owner(&pid_file, &["--toggle", "--timeout", "1s"]);

    assert_eq!(started.wait().code(), Some(0));
    assert!(!pid_file.exists());
}

#[test]
fn toggling_twice_stops_then_starts_again() {
    let sandbox = Sandbox::new("toggle-twice");
    let pid_file = sandbox.pid_file();
    let mut first = start_owner(&pid_file, &["--toggle"]);

    assert!(
        command(&pid_file, &["--toggle"])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(first.wait().code(), Some(0));
    wait_for("the pid file to disappear", || !pid_file.exists());

    let mut second = start_owner(&pid_file, &["--toggle", "--timeout", "1s"]);
    assert_ne!(second.id(), first.id());
    assert_eq!(second.wait().code(), Some(0));
}

#[test]
fn a_crashed_instance_leaves_a_file_that_does_not_block_the_next_one() {
    let sandbox = Sandbox::new("crash");
    let pid_file = sandbox.pid_file();
    let mut crashed = start_owner(&pid_file, &[]);
    let crashed_pid = crashed.id();

    crashed.signal(libc::SIGKILL); // no chance to clean up
    crashed.wait();
    assert_eq!(
        read_pid(&pid_file),
        Some(crashed_pid),
        "the stale file is left behind"
    );

    let mut next = start_owner(&pid_file, &["--toggle", "--timeout", "1s"]);
    assert_eq!(next.wait().code(), Some(0));
    assert!(!pid_file.exists());
}

#[test]
fn toggle_never_signals_the_unrelated_process_named_in_a_stale_file() {
    let sandbox = Sandbox::new("unrelated");
    let pid_file = sandbox.pid_file();
    let mut bystander = Instance(Some(Command::new("sleep").arg("30").spawn().unwrap()));
    std::fs::write(&pid_file, format!("{}\n", bystander.id())).unwrap();

    let mut started = start_owner(&pid_file, &["--toggle", "--timeout", "1s"]);

    assert!(
        bystander.try_wait().is_none(),
        "the bystander must still be running"
    );
    assert_eq!(started.wait().code(), Some(0));
}

#[test]
fn unreadable_pid_file_content_is_simply_overwritten() {
    let sandbox = Sandbox::new("garbage");
    let pid_file = sandbox.pid_file();
    for content in ["", "abc\n", "-1", "0\n", "4294967296", "1 2\n", "\0\0\0"] {
        std::fs::write(&pid_file, content).unwrap();
        let mut started = start_owner(&pid_file, &["--toggle", "--timeout", "30s"]);
        started.signal(libc::SIGTERM);
        assert_eq!(started.wait().code(), Some(0), "{content:?}");
        assert!(!pid_file.exists(), "{content:?}");
    }
}

#[test]
fn a_file_replaced_under_a_running_instance_is_not_deleted_by_it() {
    let sandbox = Sandbox::new("replaced");
    let pid_file = sandbox.pid_file();
    let mut old = start_owner(&pid_file, &[]);

    // Someone clears the file (a tmp cleaner, a careless `rm`) and a new instance takes over.
    std::fs::remove_file(&pid_file).unwrap();
    let mut new = start_owner(&pid_file, &[]);

    old.signal(libc::SIGTERM);
    assert_eq!(old.wait().code(), Some(0));
    assert_eq!(
        read_pid(&pid_file),
        Some(new.id()),
        "the new instance's file must survive"
    );

    new.signal(libc::SIGTERM);
    assert_eq!(new.wait().code(), Some(0));
    assert!(!pid_file.exists());
}

#[test]
fn a_start_failure_does_not_leave_a_pid_file_behind() {
    let sandbox = Sandbox::new("startfail");
    let pid_file = sandbox.pid_file();
    let output = command(&pid_file, &["--", "/nonexistent/wayinhibit-test"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(!pid_file.exists());
}

#[test]
fn a_failed_wayland_connection_does_not_leave_a_pid_file_behind() {
    let sandbox = Sandbox::new("noconnect");
    let pid_file = sandbox.pid_file();
    // The file is created before connecting (so that starting is atomic); it must not survive
    // the failure. No --dry-run here, and a runtime dir with no compositor socket in it.
    let output = Command::new(BIN)
        .args(["--quiet", "--pid-file"])
        .arg(&pid_file)
        .env("XDG_RUNTIME_DIR", &sandbox.0)
        .env("WAYLAND_DISPLAY", "wayinhibit-test-nonexistent")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("Wayland"), "{}", stderr(&output));
    assert!(!pid_file.exists());
}

/// The core guarantee: however many instances start at the same instant, the pid file never
/// has two owners. Each instance's command checks, once at its start and once at its end,
/// whether the file names its parent (i.e. whether that instance owns it) and logs only if so.
/// If two owners ever overlapped, the log would show `start start`.
#[test]
fn simultaneous_starts_never_give_the_pid_file_two_owners() {
    const ROUNDS: usize = 15;
    const CONTENDERS: usize = 16;

    let sandbox = Sandbox::new("race");
    let pid_file = sandbox.pid_file();
    let log = sandbox.path("log");
    let owns = format!("[ \"$(cat '{}')\" = \"$PPID\" ]", pid_file.display());
    let script = format!(
        "{owns} && echo start >> '{log}'; sleep 0.2; {owns} && echo end >> '{log}'; true",
        log = log.display()
    );

    for round in 0..ROUNDS {
        let contenders: Vec<Instance> = (0..CONTENDERS)
            .map(|_| spawn(&pid_file, &["--", "sh", "-c", &script]))
            .collect();

        let mut without_pid_file = 0;
        for contender in contenders {
            let output = contender.finish();
            assert_eq!(
                output.status.code(),
                Some(0),
                "round {round}: {}",
                stderr(&output)
            );
            let message = stderr(&output);
            if message.is_empty() {
                continue; // an owner
            }
            assert!(
                message.contains("already in use"),
                "round {round}: {message}"
            );
            without_pid_file += 1;
        }

        let log_text = std::fs::read_to_string(&log).unwrap();
        let events: Vec<&str> = log_text.lines().collect();
        let owners = CONTENDERS - without_pid_file;
        assert!(owners >= 1, "round {round}: nobody owned the file");
        assert_eq!(events.len(), owners * 2, "round {round}: {events:?}");
        for pair in events.chunks(2) {
            assert_eq!(
                pair,
                ["start", "end"],
                "round {round}: two owners at once: {events:?}"
            );
        }
        assert!(!pid_file.exists(), "round {round}: pid file left behind");
        std::fs::remove_file(&log).unwrap();
    }
}
