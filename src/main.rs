use std::env;
use std::error::Error;
use std::process::ExitCode;

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

fn run() -> Result<(), Box<dyn Error>> {
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

    let _surface = compositor.create_surface(&qh, ());
    let _inhibitor = inhibitor_manager.create_inhibitor(&_surface, &qh, ());

    event_queue
        .roundtrip(&mut app)
        .map_err(|err| format!("failed to complete Wayland setup: {err}"))?;

    println!("Inhibiting idle. Press Ctrl-C to stop.");

    loop {
        event_queue
            .blocking_dispatch(&mut app)
            .map_err(|err| format!("Wayland event loop failed: {err}"))?;
    }
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
