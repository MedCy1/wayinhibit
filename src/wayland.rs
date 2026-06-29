use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    globals::GlobalListContents,
    globals::registry_queue_init,
    protocol::{wl_compositor::WlCompositor, wl_registry, wl_surface::WlSurface},
};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};

struct App;

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

pub struct IdleInhibitor {
    app: App,
    event_queue: EventQueue<App>,
    surface: WlSurface,
    inhibitor: ZwpIdleInhibitorV1,
    timeout: Timespec,
}

impl IdleInhibitor {
    pub fn connect(timeout: Duration) -> Result<Self, String> {
        let conn = Connection::connect_to_env()
            .map_err(|err| format!("failed to connect to the Wayland compositor: {err}"))?;

        let (globals, mut event_queue) = registry_queue_init::<App>(&conn)
            .map_err(|err| format!("failed to initialize the Wayland registry: {err}"))?;
        let qh = event_queue.handle();
        let mut app = App;

        let compositor: WlCompositor = globals
            .bind(&qh, 1..=1, ())
            .map_err(|err| format!("failed to bind wl_compositor: {err}"))?;
        let inhibitor_manager: ZwpIdleInhibitManagerV1 =
            globals.bind(&qh, 1..=1, ()).map_err(|_| {
                "your compositor does not support idle inhibition (zwp_idle_inhibit_manager_v1).\n\
                 Supported: Sway, Hyprland, KWin, Mutter (GNOME), Muffin (Cinnamon), COSMIC, \
                 niri, river, Wayfire, labwc, Cage, Jay, Mir, Louvre, phoc, Treeland."
                    .to_owned()
            })?;

        let surface = compositor.create_surface(&qh, ());
        let inhibitor = inhibitor_manager.create_inhibitor(&surface, &qh, ());

        event_queue
            .roundtrip(&mut app)
            .map_err(|err| format!("failed to complete Wayland setup: {err}"))?;

        let timeout = Timespec::try_from(timeout)
            .map_err(|err| format!("failed to build poll timeout: {err}"))?;

        Ok(Self {
            app,
            event_queue,
            surface,
            inhibitor,
            timeout,
        })
    }

    pub fn tick(&mut self) -> Result<(), String> {
        self.event_queue
            .dispatch_pending(&mut self.app)
            .map_err(|err| format!("Wayland event dispatch failed: {err}"))?;

        let Some(read_guard) = self.event_queue.prepare_read() else {
            return Ok(());
        };

        self.event_queue
            .flush()
            .map_err(|err| format!("failed to flush the Wayland connection: {err}"))?;

        let mut fds = [PollFd::from_borrowed_fd(
            read_guard.connection_fd(),
            PollFlags::IN,
        )];

        match poll(&mut fds, Some(&self.timeout)) {
            Ok(0) => {
                drop(read_guard);
                Ok(())
            }
            Ok(_) => {
                let ready = fds[0].revents();
                if ready.intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP) {
                    read_guard
                        .read()
                        .map_err(|err| format!("failed to read Wayland events: {err}"))?;
                    self.event_queue
                        .dispatch_pending(&mut self.app)
                        .map_err(|err| format!("Wayland event dispatch failed: {err}"))?;
                } else {
                    drop(read_guard);
                }

                Ok(())
            }
            Err(rustix::io::Errno::INTR) => {
                drop(read_guard);
                Ok(())
            }
            Err(err) => {
                drop(read_guard);
                Err(format!("failed to poll the Wayland connection: {err}"))
            }
        }
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        self.inhibitor.destroy();
        self.surface.destroy();
        self.event_queue
            .flush()
            .map_err(|err| format!("failed to flush shutdown requests: {err}"))
    }
}
