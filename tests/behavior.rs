//! How wayinhibit behaves around the command it wraps, its hooks, its signals and its output.
//!
//! Everything here runs the real binary, with `--dry-run` unless a compositor is the point.
//! Set `WAYINHIBIT_TEST_BIN` to run the same tests against another build.

mod common;

use std::ffi::OsStr;
use std::os::fd::FromRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use common::{Compositor, Kind, bin};

const PATIENCE: Duration = Duration::from_secs(10);

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("wayinhibit-bh-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A wayinhibit process that is killed if the test fails before it has been collected.
struct Instance(Child);

impl Instance {
    fn signal(&self, signal: libc::c_int) {
        let pid = i32::try_from(self.0.id()).unwrap();
        assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
    }

    fn running(&mut self) -> bool {
        self.0.try_wait().unwrap().is_none()
    }

    /// Waits for the exit, up to `limit`.
    fn wait_within(&mut self, limit: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts a dry-run instance and waits until it is ready to be signalled: its pid file only
/// exists once the signal handlers are installed, so a fixed delay is not needed.
fn ready_instance(
    sandbox: &Sandbox,
    name: &str,
    args: &[&str],
    setup: impl FnOnce(&mut Command),
) -> Instance {
    let pid_file = sandbox.path(name);
    let mut all: Vec<&str> = vec!["--pid-file", pid_file.to_str().unwrap()];
    all.extend_from_slice(args);
    let mut command = dry_run(&all);
    setup(&mut command);
    let instance = spawn(command);
    wait_for("the instance to be ready", || pid_file.exists());
    instance
}

fn dry_run(args: &[&str]) -> Command {
    let mut command = Command::new(bin());
    command.args(["--dry-run", "--quiet"]).args(args);
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    command
}

fn spawn(mut command: Command) -> Instance {
    Instance(command.spawn().unwrap())
}

fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        sleep(Duration::from_millis(10));
    }
}

fn alive(pid: u32) -> bool {
    // A zombie still answers kill(0); read the state instead.
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit(')')
                .next()
                .map(|rest| rest.trim_start().starts_with(|c| c != 'Z'))
        })
        .unwrap_or(false)
}

// --- arguments and output ---------------------------------------------------------------

#[test]
fn a_non_utf8_argument_reaches_the_command_untouched() {
    // 0xff is not valid UTF-8, and is a legal byte in a Linux file name.
    let output = dry_run(&["--", "sh", "-c", r#"test "$1" = "$(printf '\377')""#, "sh"])
        .arg(OsStr::from_bytes(&[0xff]))
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_closed_output_does_not_crash_or_change_the_commands_status() {
    let mut ends = [0; 2];
    assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
    // Nobody will ever read this pipe: every write fails with EPIPE.
    unsafe { libc::close(ends[0]) };
    let write_end = unsafe { Stdio::from_raw_fd(ends[1]) };

    let output = Command::new(bin())
        .args(["--dry-run", "--", "sh", "-c", "exit 7"])
        .stdout(write_end)
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(7), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn help_and_version_work_next_to_other_options() {
    for args in [["-q", "--help"], ["--dry-run", "--help"], ["-q", "-h"]] {
        let output = Command::new(bin()).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("Usage"),
            "{args:?}"
        );
    }
    for args in [["-q", "--version"], ["--dry-run", "-V"]] {
        let output = Command::new(bin()).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            env!("CARGO_PKG_VERSION"),
            "{args:?}"
        );
    }
}

#[test]
fn a_usage_error_prints_the_hint_once() {
    let output = Command::new(bin()).arg("--bogus").output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stderr.matches("--help").count(), 1, "{stderr}");
}

// --- signals ----------------------------------------------------------------------------

#[test]
fn a_hangup_that_was_ignored_at_startup_stays_ignored() {
    let sandbox = Sandbox::new("sighup");
    let mut ignoring = ready_instance(&sandbox, "ignoring.pid", &[], |command| unsafe {
        command.pre_exec(|| {
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
            Ok(())
        });
    });
    ignoring.signal(libc::SIGHUP);
    sleep(Duration::from_millis(600));
    assert!(
        ignoring.running(),
        "SIGHUP must not stop an instance that started with it ignored"
    );
    ignoring.signal(libc::SIGTERM);
    assert_eq!(
        ignoring.wait_within(PATIENCE).and_then(|s| s.code()),
        Some(0)
    );

    // Control: without the inheritance, the same signal is a normal request to stop.
    let mut normal = ready_instance(&sandbox, "normal.pid", &[], |_| {});
    normal.signal(libc::SIGHUP);
    assert_eq!(
        normal
            .wait_within(Duration::from_secs(3))
            .and_then(|s| s.code()),
        Some(0)
    );
}

// --- hooks ------------------------------------------------------------------------------

fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_release_hook_follows_the_start_hook_when_the_command_cannot_start() {
    let sandbox = Sandbox::new("pairing");
    let log = sandbox.path("log");
    let start = format!("echo start >> '{}'", log.display());
    let release = format!("echo release >> '{}'", log.display());

    let output = dry_run(&[
        "--on-inhibit",
        &start,
        "--on-release",
        &release,
        "--",
        "/nonexistent/wayinhibit-test",
    ])
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(lines(&log), ["start", "release"]);
}

#[test]
fn a_slow_start_hook_does_not_delay_a_stop() {
    let sandbox = Sandbox::new("slowhook");
    let pids = sandbox.path("pids");
    let hook = format!("sleep 30 & echo $$ $! > '{}'; wait", pids.display());
    let mut instance = spawn(dry_run(&["--on-inhibit", &hook]));
    wait_for("the hook to start", || {
        pids.exists() && !std::fs::read_to_string(&pids).unwrap().trim().is_empty()
    });

    let asked = Instant::now();
    instance.signal(libc::SIGTERM);
    let status = instance.wait_within(Duration::from_secs(5));
    assert_eq!(status.and_then(|s| s.code()), Some(0));
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "stop took {:?}",
        asked.elapsed()
    );

    // The hook and what it started are gone.
    let text = std::fs::read_to_string(&pids).unwrap();
    for pid in text.split_whitespace().map(|p| p.parse::<u32>().unwrap()) {
        wait_for("the hook process to end", || !alive(pid));
    }
}

#[test]
fn toggle_stops_an_instance_that_is_busy_with_a_slow_hook() {
    let sandbox = Sandbox::new("toggle");
    let pid_file = sandbox.path("w.pid");
    let started = sandbox.path("started");
    let hook = format!("touch '{}'; sleep 30", started.display());
    let mut instance = spawn(dry_run(&[
        "--pid-file",
        pid_file.to_str().unwrap(),
        "--on-inhibit",
        &hook,
    ]));
    wait_for("the hook to start", || started.exists());

    let asked = Instant::now();
    let toggle = dry_run(&["--pid-file", pid_file.to_str().unwrap(), "--toggle"])
        .status()
        .unwrap();
    assert!(toggle.success());
    assert_eq!(
        instance
            .wait_within(Duration::from_secs(5))
            .and_then(|s| s.code()),
        Some(0)
    );
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "stop took {:?}",
        asked.elapsed()
    );
}

#[test]
fn the_release_hook_is_not_cut_short_by_the_stop_that_caused_it_but_by_a_second_one() {
    let sandbox = Sandbox::new("release");
    let done = sandbox.path("done");
    let hook = format!("sleep 1; touch '{}'", done.display());
    let mut patient = ready_instance(&sandbox, "patient.pid", &["--on-release", &hook], |_| {});
    patient.signal(libc::SIGTERM);
    assert_eq!(
        patient.wait_within(PATIENCE).and_then(|s| s.code()),
        Some(0)
    );
    assert!(
        done.exists(),
        "the release hook must run to completion after a single stop request"
    );

    let mut impatient = ready_instance(
        &sandbox,
        "impatient.pid",
        &["--on-release", "sleep 30"],
        |_| {},
    );
    impatient.signal(libc::SIGTERM);
    sleep(Duration::from_millis(500));
    assert!(impatient.running(), "still waiting for its release hook");
    let asked = Instant::now();
    impatient.signal(libc::SIGTERM);
    assert_eq!(
        impatient
            .wait_within(Duration::from_secs(5))
            .and_then(|s| s.code()),
        Some(0)
    );
    assert!(
        asked.elapsed() < Duration::from_secs(3),
        "{:?}",
        asked.elapsed()
    );
}

// --- the compositor going away ----------------------------------------------------------

fn start(kind: Kind) -> Option<Compositor> {
    if !kind.available() {
        assert!(
            std::env::var_os(kind.required_variable()).is_none(),
            "{} is required here but was not found",
            kind.name()
        );
        eprintln!("skipped: {} not found", kind.name());
        return None;
    }
    Some(Compositor::start(kind))
}

fn sway() -> Option<Compositor> {
    start(Kind::Sway)
}

#[test]
fn foreground_mode_gives_up_when_the_compositor_goes_away() {
    let Some(mut compositor) = sway() else { return };
    let sandbox = Sandbox::new("lost-fg");
    let released = sandbox.path("released");
    let hook = format!("touch '{}'", released.display());
    let mut instance = Instance(compositor.wayinhibit(&["--on-release", &hook]));
    sleep(common::SETTLE);

    compositor.crash();

    let status = instance.wait_within(Duration::from_secs(5));
    assert_eq!(
        status.and_then(|s| s.code()),
        Some(1),
        "it must not keep running without inhibiting"
    );
    assert!(released.exists(), "the release hook must still run");
}

#[test]
fn command_mode_keeps_the_command_and_its_status_when_the_compositor_goes_away() {
    let Some(mut compositor) = sway() else { return };
    let mut command = Command::new(bin());
    command
        .arg("--quiet")
        .args(["--", "sh", "-c", "sleep 2; exit 7"])
        .env("XDG_RUNTIME_DIR", compositor.runtime_dir())
        .env("WAYLAND_DISPLAY", compositor.display())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut instance = spawn(command);
    sleep(common::SETTLE);

    compositor.crash();

    let status = instance.wait_within(Duration::from_secs(8));
    assert_eq!(status.and_then(|s| s.code()), Some(7));
    let mut stderr = String::new();
    std::io::Read::read_to_string(instance.0.stderr.as_mut().unwrap(), &mut stderr).unwrap();
    assert!(stderr.contains("keeps running without it"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

/// river 0.4 and later has no window manager of its own and closes layer surfaces until one is
/// running (this test only runs where that river is installed),
/// so the displayed surface is refused. The command must run all the same.
#[test]
fn a_compositor_that_refuses_the_displayed_surface_does_not_stop_the_command() {
    let Some(compositor) = start(Kind::RiverModular) else {
        return;
    };
    let mut command = Command::new(bin());
    command
        .args(["--", "sh", "-c", "exit 7"])
        .env("XDG_RUNTIME_DIR", compositor.runtime_dir())
        .env("WAYLAND_DISPLAY", compositor.display())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let output = command.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(7),
        "the command must have run: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "{stderr}");
}
