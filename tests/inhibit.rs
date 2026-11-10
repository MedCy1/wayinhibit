//! Does wayinhibit really keep the compositor from considering the session idle?
//!
//! This is the one thing the tool exists for, and no other test checks it: the others only
//! look at exit codes, pid files and hooks. A headless compositor is started and asked, through
//! `ext-idle-notify` (the protocol idle daemons such as swayidle and hypridle rely on), to
//! report inactivity. The same question is asked with and without wayinhibit running.
//!
//! * Sway needs `sway` on the PATH. Without it the test is skipped, unless
//!   WAYINHIBIT_REQUIRE_SWAY is set (as in CI), in which case a missing Sway is a failure.
//! * KDE needs `kwin_wayland` and `dbus-run-session`; WAYINHIBIT_REQUIRE_KWIN works the same way.

use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_seat::WlSeat},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

const BIN: &str = env!("CARGO_BIN_EXE_wayinhibit");
/// Inactivity after which the compositor reports the session idle.
const IDLE_AFTER: Duration = Duration::from_millis(600);
/// How long to watch for that report.
const WINDOW: Duration = Duration::from_millis(2500);
/// Time for wayinhibit to connect, show its surface and create its inhibitors.
const SETTLE: Duration = Duration::from_millis(1200);

#[derive(Clone, Copy)]
enum Kind {
    Sway,
    Kwin,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Sway => "sway",
            Kind::Kwin => "kwin",
        }
    }

    fn required_variable(self) -> &'static str {
        match self {
            Kind::Sway => "WAYINHIBIT_REQUIRE_SWAY",
            Kind::Kwin => "WAYINHIBIT_REQUIRE_KWIN",
        }
    }

    fn available(self) -> bool {
        let on_path = |program: &str| {
            std::env::var_os("PATH").is_some_and(|paths| {
                std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
            })
        };
        match self {
            Kind::Sway => on_path("sway"),
            Kind::Kwin => on_path("kwin_wayland") && on_path("dbus-run-session"),
        }
    }
}

struct Compositor {
    child: Child,
    runtime_dir: PathBuf,
    display: String,
}

impl Compositor {
    fn start(kind: Kind) -> Self {
        let runtime_dir =
            std::env::temp_dir().join(format!("wayinhibit-{}-{}", kind.name(), std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime_dir);
        std::fs::create_dir_all(&runtime_dir).unwrap();

        let mut command = match kind {
            Kind::Sway => {
                let mut command = Command::new("sway");
                command.args(["-c", "/dev/null"]);
                command
            }
            Kind::Kwin => {
                let mut command = Command::new("dbus-run-session");
                command.args([
                    "--",
                    "kwin_wayland",
                    "--virtual",
                    "--no-lockscreen",
                    "--no-global-shortcuts",
                    "--socket",
                    "wayinhibit-kwin",
                    "--width",
                    "800",
                    "--height",
                    "600",
                ]);
                command
            }
        };
        let child = command
            .env("XDG_RUNTIME_DIR", &runtime_dir)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_RENDERER", "pixman")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("KWIN_COMPOSE", "Q")
            .env("QT_QPA_PLATFORM", "offscreen")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            // Its own group, so that dropping kills the compositor and whatever it spawned.
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut compositor = Self {
            child,
            runtime_dir,
            display: String::new(),
        };

        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let socket = std::fs::read_dir(&compositor.runtime_dir)
                .unwrap()
                .flatten()
                .find(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    (name.starts_with("wayland-") || name.starts_with("wayinhibit-kwin"))
                        && !name.ends_with(".lock")
                });
            if let Some(entry) = socket {
                compositor.display = entry.file_name().to_string_lossy().into_owned();
                // Give it a moment to finish starting after the socket appears.
                sleep(Duration::from_millis(500));
                return compositor;
            }
            sleep(Duration::from_millis(100));
        }
        panic!("{} did not create its socket", kind.name());
    }

    fn wayinhibit(&self, args: &[&str]) -> Child {
        Command::new(BIN)
            .arg("--quiet")
            .args(args)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("WAYLAND_DISPLAY", &self.display)
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Asks the compositor to report inactivity and says whether it did within `WINDOW`.
    fn goes_idle(&self) -> bool {
        struct Watcher {
            idled: bool,
        }
        impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Watcher {
            fn event(
                _: &mut Self,
                _: &wl_registry::WlRegistry,
                _: wl_registry::Event,
                _: &GlobalListContents,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
        impl Dispatch<WlSeat, ()> for Watcher {
            fn event(
                _: &mut Self,
                _: &WlSeat,
                _: <WlSeat as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
        impl Dispatch<ExtIdleNotifierV1, ()> for Watcher {
            fn event(
                _: &mut Self,
                _: &ExtIdleNotifierV1,
                _: <ExtIdleNotifierV1 as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
        impl Dispatch<ExtIdleNotificationV1, ()> for Watcher {
            fn event(
                state: &mut Self,
                _: &ExtIdleNotificationV1,
                event: ext_idle_notification_v1::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                if matches!(event, ext_idle_notification_v1::Event::Idled) {
                    state.idled = true;
                }
            }
        }

        let stream = UnixStream::connect(self.runtime_dir.join(&self.display)).unwrap();
        let conn = Connection::from_socket(stream).unwrap();
        let (globals, mut queue) = registry_queue_init::<Watcher>(&conn).unwrap();
        let qh = queue.handle();
        let seat: WlSeat = globals.bind(&qh, 1..=1, ()).unwrap();
        let notifier: ExtIdleNotifierV1 = globals.bind(&qh, 1..=1, ()).unwrap();
        let _notification =
            notifier.get_idle_notification(IDLE_AFTER.as_millis() as u32, &seat, &qh, ());

        let mut watcher = Watcher { idled: false };
        let deadline = Instant::now() + WINDOW;
        while Instant::now() < deadline && !watcher.idled {
            queue.roundtrip(&mut watcher).unwrap();
            sleep(Duration::from_millis(50));
        }
        watcher.idled
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let group = -i32::try_from(self.child.id()).unwrap();
        unsafe { libc::kill(group, libc::SIGKILL) };
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.runtime_dir);
    }
}

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
