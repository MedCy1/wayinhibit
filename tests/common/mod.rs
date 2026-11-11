//! Shared test harness: a headless compositor to run wayinhibit against, and a way to ask it
//! whether the session is idle.
#![allow(dead_code)]

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

/// The binary under test. `WAYINHIBIT_TEST_BIN` points the tests at another build, which is how
/// a test is shown to fail on the code it was written to fix.
pub fn bin() -> String {
    std::env::var("WAYINHIBIT_TEST_BIN")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_wayinhibit").to_owned())
}
/// Inactivity after which the compositor reports the session idle.
pub const IDLE_AFTER: Duration = Duration::from_millis(600);
/// How long to watch for that report.
pub const WINDOW: Duration = Duration::from_millis(2500);
/// Time for wayinhibit to connect, show its surface and create its inhibitors.
pub const SETTLE: Duration = Duration::from_millis(1200);

#[derive(Clone, Copy)]
pub enum Kind {
    Sway,
    Kwin,
    Labwc,
    Cage,
    Wayfire,
    /// river 0.3 and earlier: a window manager and a compositor in one program.
    River,
    /// river 0.4 and later: the window manager is a separate program. Without one it closes
    /// every layer surface.
    RiverModular,
}

/// The minor version of the installed `river` (0.3.17 gives 3), if there is one.
fn river_minor_version() -> Option<u32> {
    let output = Command::new("river").arg("-version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text
        .split_whitespace()
        .find(|word| word.starts_with("0."))?;
    version.split('.').nth(1)?.parse().ok()
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Sway => "sway",
            Kind::Kwin => "kwin",
            Kind::Labwc => "labwc",
            Kind::Cage => "cage",
            Kind::Wayfire => "wayfire",
            Kind::River => "river",
            Kind::RiverModular => "river-modular",
        }
    }

    /// Setting this variable turns "not installed" from a skip into a failure (as in CI).
    pub fn required_variable(self) -> String {
        format!("WAYINHIBIT_REQUIRE_{}", self.name().to_uppercase())
    }

    pub fn available(self) -> bool {
        match self {
            Kind::Kwin => on_path("kwin_wayland") && on_path("dbus-run-session"),
            Kind::River => on_path("river") && river_minor_version().is_some_and(|m| m < 4),
            Kind::RiverModular => on_path("river") && river_minor_version().is_some_and(|m| m >= 4),
            other => on_path(other.name()),
        }
    }
}

pub struct Compositor {
    child: Child,
    runtime_dir: PathBuf,
    display: String,
}

impl Compositor {
    pub fn start(kind: Kind) -> Self {
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
            Kind::Labwc => {
                // An empty configuration directory: no autostart, no panel.
                let config = runtime_dir.join("labwc-config");
                std::fs::create_dir_all(&config).unwrap();
                let mut command = Command::new("labwc");
                command.arg("-C").arg(config);
                command
            }
            Kind::Cage => {
                // Cage shows one program full screen and quits with it.
                let mut command = Command::new("cage");
                command.args(["--", "sleep", "300"]);
                command
            }
            Kind::Wayfire => {
                let config = runtime_dir.join("wayfire.ini");
                std::fs::write(&config, "[core]\nplugins = \n").unwrap();
                let mut command = Command::new("wayfire");
                command.arg("-c").arg(config);
                command
            }
            Kind::River => {
                // `-c true`: the init program, so that no user configuration is read.
                let mut command = Command::new("river");
                command.args(["-c", "true"]);
                command
            }
            Kind::RiverModular => Command::new("river"),
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

    pub fn wayinhibit(&self, args: &[&str]) -> Child {
        Command::new(bin())
            .arg("--quiet")
            .args(args)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("WAYLAND_DISPLAY", &self.display)
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Asks the compositor to report inactivity and says whether it did within `WINDOW`.
    pub fn goes_idle(&self) -> bool {
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

impl Compositor {
    pub fn runtime_dir(&self) -> &std::path::Path {
        &self.runtime_dir
    }

    pub fn display(&self) -> &str {
        &self.display
    }

    /// Takes the compositor away abruptly, as a crash would.
    pub fn crash(&mut self) {
        let group = -i32::try_from(self.child.id()).unwrap();
        unsafe { libc::kill(group, libc::SIGKILL) };
        let _ = self.child.wait();
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
