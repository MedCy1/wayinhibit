use std::env;
use std::error::Error;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::GlobalListContents,
    globals::registry_queue_init,
    protocol::{wl_compositor::WlCompositor, wl_registry, wl_surface::WlSurface},
};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BIN_NAME: &str = env!("CARGO_PKG_NAME");
static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

struct App;

fn print_help() {
    println!(
        "\
{BIN_NAME} {VERSION}

A small Wayland idle inhibitor written in Rust.

Usage:
  {BIN_NAME} [OPTIONS]

Options:
  -h, --help       Print help
  -V, --version    Print version
"
    );
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(App: WlCompositor);
delegate_noop!(App: ignore WlSurface);
delegate_noop!(App: ZwpIdleInhibitManagerV1);
delegate_noop!(App: ZwpIdleInhibitorV1);

extern "C" fn handle_shutdown_signal(_: libc::c_int) {
    SHOULD_STOP.store(true, Ordering::Relaxed);
}

fn install_signal_handlers() -> Result<(), Box<dyn Error>> {
    unsafe {
        let handler = handle_shutdown_signal as *const () as libc::sighandler_t;

        if libc::signal(libc::SIGINT, handler) == libc::SIG_ERR {
            return Err("failed to install SIGINT handler".into());
        }

        if libc::signal(libc::SIGTERM, handler) == libc::SIG_ERR {
            return Err("failed to install SIGTERM handler".into());
        }
    }

    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    SHOULD_STOP.store(false, Ordering::Relaxed);
    install_signal_handlers()?;

    let conn = Connection::connect_to_env()
        .map_err(|err| format!("failed to connect to the Wayland compositor: {err}"))?;

    let (globals, mut event_queue) = registry_queue_init::<App>(&conn)
        .map_err(|err| format!("failed to initialize the Wayland registry: {err}"))?;
    let qh = event_queue.handle();
    let mut app = App;

    let compositor: WlCompositor = globals
        .bind(&qh, 1..=1, ())
        .map_err(|err| format!("failed to bind wl_compositor: {err}"))?;
    let inhibitor_manager: ZwpIdleInhibitManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|err| format!("idle inhibit protocol is not available: {err}"))?;

    let surface = compositor.create_surface(&qh, ());
    let inhibitor = inhibitor_manager.create_inhibitor(&surface, &qh, ());

    event_queue
        .roundtrip(&mut app)
        .map_err(|err| format!("failed to complete Wayland setup: {err}"))?;

    println!("Inhibiting idle. Press Ctrl-C to stop.");

    let timeout = Timespec::try_from(Duration::from_millis(250))
        .map_err(|err| format!("failed to build poll timeout: {err}"))?;

    loop {
        if SHOULD_STOP.load(Ordering::Relaxed) {
            break;
        }

        event_queue
            .dispatch_pending(&mut app)
            .map_err(|err| format!("Wayland event dispatch failed: {err}"))?;

        let Some(read_guard) = event_queue.prepare_read() else {
            continue;
        };

        event_queue
            .flush()
            .map_err(|err| format!("failed to flush the Wayland connection: {err}"))?;

        let mut fds = [PollFd::from_borrowed_fd(
            read_guard.connection_fd(),
            PollFlags::IN,
        )];

        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => {
                drop(read_guard);
            }
            Ok(_) => {
                if SHOULD_STOP.load(Ordering::Relaxed) {
                    drop(read_guard);
                    break;
                }

                let ready = fds[0].revents();
                if ready.intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP) {
                    read_guard
                        .read()
                        .map_err(|err| format!("failed to read Wayland events: {err}"))?;
                    event_queue
                        .dispatch_pending(&mut app)
                        .map_err(|err| format!("Wayland event dispatch failed: {err}"))?;
                } else {
                    drop(read_guard);
                }
            }
            Err(rustix::io::Errno::INTR) => {
                drop(read_guard);
            }
            Err(err) => {
                drop(read_guard);
                return Err(format!("failed to poll the Wayland connection: {err}").into());
            }
        }
    }

    inhibitor.destroy();
    surface.destroy();
    event_queue
        .flush()
        .map_err(|err| format!("failed to flush shutdown requests: {err}"))?;

    println!("Stopped idle inhibition.");

    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    match args.as_slice() {
        [] => match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{BIN_NAME}: {err}");
                ExitCode::from(1)
            }
        },
        [flag] if flag == "-h" || flag == "--help" => {
            print_help();
            ExitCode::SUCCESS
        }
        [flag] if flag == "-V" || flag == "--version" => {
            println!("{VERSION}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{BIN_NAME}: unsupported arguments: {}", args.join(" "));
            eprintln!("Run '{BIN_NAME} --help' for usage.");
            ExitCode::from(2)
        }
    }
}
