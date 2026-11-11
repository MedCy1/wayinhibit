//! Does wayinhibit really keep the compositor from considering the session idle?
//!
//! This is the one thing the tool exists for, and no other test checks it: the others only
//! look at exit codes, pid files and hooks. A headless compositor is started and asked, through
//! `ext-idle-notify` (the protocol idle daemons such as swayidle and hypridle rely on), to
//! report inactivity. The same question is asked with and without wayinhibit running.
//!
//! * Sway needs `sway` on the PATH. Without it the test is skipped, unless
//!   WAYINHIBIT_REQUIRE_SWAY is set (as in CI), in which case a missing Sway is a failure.
//! * KDE needs `kwin_wayland` and `dbus-run-session`; the others (labwc, cage, wayfire) are
//!   plain programs. Each has its own WAYINHIBIT_REQUIRE_<NAME> variable.
//! * river 0.3 (`river-classic`) is tested here. river 0.4 and later needs a separate window
//!   manager and refuses layer surfaces without one, so it cannot be used for this test; it has
//!   its own test in behavior.rs.

mod common;

use std::process::Child;
use std::thread::sleep;

use common::{Compositor, Kind, SETTLE};

/// A wayinhibit process that is killed if the test fails before it is stopped.
struct Running(Child);

impl Running {
    /// Gives wayinhibit time to connect and establish its inhibitors before anything is
    /// measured.
    fn settled(child: Child) -> Self {
        sleep(SETTLE);
        Self(child)
    }

    /// Stops it with SIGTERM and checks the exit code: 0 in foreground mode, 143 (128 plus
    /// SIGTERM) in command mode, where the wrapped command's own status is passed on.
    fn stop(mut self, expected_code: i32) {
        let pid = i32::try_from(self.0.id()).unwrap();
        assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
        assert_eq!(self.0.wait().unwrap().code(), Some(expected_code));
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn the_compositor_stays_awake_exactly_while_wayinhibit_runs(kind: Kind) {
    if !kind.available() {
        assert!(
            std::env::var_os(kind.required_variable()).is_none(),
            "{} is required here but was not found",
            kind.name()
        );
        eprintln!("skipped: {} not found", kind.name());
        return;
    }
    let compositor = Compositor::start(kind);

    // The control: this proves the harness can see idleness at all, so that the checks below
    // cannot pass just because nothing is ever reported.
    assert!(
        compositor.goes_idle(),
        "control: an idle session was not reported idle"
    );

    let foreground = Running::settled(compositor.wayinhibit(&[]));
    assert!(
        !compositor.goes_idle(),
        "the compositor went idle while wayinhibit (foreground mode) was running"
    );
    foreground.stop(0);

    let wrapper = Running::settled(compositor.wayinhibit(&["--", "sleep", "30"]));
    assert!(
        !compositor.goes_idle(),
        "the compositor went idle while wayinhibit (command mode) was running"
    );
    wrapper.stop(143);

    assert!(
        compositor.goes_idle(),
        "the compositor stayed awake after wayinhibit stopped"
    );
}

#[test]
fn sway_stays_awake_exactly_while_wayinhibit_runs() {
    the_compositor_stays_awake_exactly_while_wayinhibit_runs(Kind::Sway);
}

#[test]
fn kwin_stays_awake_exactly_while_wayinhibit_runs() {
    the_compositor_stays_awake_exactly_while_wayinhibit_runs(Kind::Kwin);
}

#[test]
fn labwc_stays_awake_exactly_while_wayinhibit_runs() {
    the_compositor_stays_awake_exactly_while_wayinhibit_runs(Kind::Labwc);
}

#[test]
fn cage_stays_awake_exactly_while_wayinhibit_runs() {
    the_compositor_stays_awake_exactly_while_wayinhibit_runs(Kind::Cage);
}

#[test]
fn wayfire_stays_awake_exactly_while_wayinhibit_runs() {
    the_compositor_stays_awake_exactly_while_wayinhibit_runs(Kind::Wayfire);
}

#[test]
fn river_classic_stays_awake_exactly_while_wayinhibit_runs() {
    the_compositor_stays_awake_exactly_while_wayinhibit_runs(Kind::River);
}
