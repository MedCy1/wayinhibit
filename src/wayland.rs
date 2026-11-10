use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{MemfdFlags, ftruncate, memfd_create};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    globals::GlobalListContents,
    globals::registry_queue_init,
    protocol::{
        wl_buffer::WlBuffer,
        wl_compositor::WlCompositor,
        wl_region::WlRegion,
        wl_registry,
        wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
    },
};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

/// How long the layer surface is left alone before its inhibitor is created. Hyprland fades a
/// new layer in from fully transparent and only counts an inhibitor whose surface is already
/// visible when the inhibitor is created, without checking again later.
const LAYER_SETTLE: Duration = Duration::from_millis(300);

/// How many round trips to wait for the compositor to answer our first commit with a
/// `configure`. One is enough in practice; the rest is slack for a busy compositor.
const CONFIGURE_ROUNDTRIPS: usize = 10;

#[derive(Default)]
struct App {
    /// The compositor sent the first `configure` of the layer surface.
    configured: bool,
    /// The compositor closed the layer surface (for instance because its output went away).
    closed: bool,
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

impl Dispatch<ZwlrLayerSurfaceV1, ()> for App {
    fn event(
        state: &mut Self,
        proxy: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                proxy.ack_configure(serial);
                state.configured = true;
            }
            zwlr_layer_surface_v1::Event::Closed => state.closed = true,
            _ => {}
        }
    }
}

delegate_noop!(App: WlCompositor);
delegate_noop!(App: ignore WlSurface);
delegate_noop!(App: WlRegion);
delegate_noop!(App: ignore WlShm);
delegate_noop!(App: WlShmPool);
delegate_noop!(App: ignore WlBuffer);
delegate_noop!(App: ZwlrLayerShellV1);
delegate_noop!(App: ZwpIdleInhibitManagerV1);
delegate_noop!(App: ZwpIdleInhibitorV1);

/// The on-screen surface that makes the inhibitor count. Compositors only honor an idle
/// inhibitor whose surface is actually shown: since 2025 Sway, among others, ignores one
/// attached to a surface that has no role or no content.
struct MappedSurface {
    layer_surface: ZwlrLayerSurfaceV1,
    buffer: WlBuffer,
}

pub struct IdleInhibitor {
    app: App,
    event_queue: EventQueue<App>,
    qh: QueueHandle<App>,
    manager: ZwpIdleInhibitManagerV1,
    /// A surface with no role. Some compositors (Hyprland, labwc, Wayfire, Cage, older Sway)
    /// still honor an inhibitor on it, and it needs no timing care. Others ignore it.
    bare: WlSurface,
    bare_inhibitor: ZwpIdleInhibitorV1,
    /// A transparent surface really shown in the background layer, which compositors that
    /// ignore the bare one (Sway 1.11+, river, KWin, niri) do honor. `None` without
    /// wlr-layer-shell (GNOME).
    layer: Option<LayerInhibitor>,
    /// Something worth telling the user, handed out once by `take_notice`.
    notice: Option<String>,
    timeout: Timespec,
}

struct LayerInhibitor {
    surface: WlSurface,
    mapped: MappedSurface,
    /// Created once `LAYER_SETTLE` has passed.
    inhibitor: Option<ZwpIdleInhibitorV1>,
    settle_until: Instant,
}

impl IdleInhibitor {
    pub fn connect(timeout: Duration) -> Result<Self, String> {
        let conn = Connection::connect_to_env()
            .map_err(|err| format!("failed to connect to the Wayland compositor: {err}"))?;

        let (globals, mut event_queue) = registry_queue_init::<App>(&conn)
            .map_err(|err| format!("failed to initialize the Wayland registry: {err}"))?;
        let qh = event_queue.handle();
        let mut app = App::default();

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

        // The bare surface is inhibited at once: it has no role to wait for.
        let bare = compositor.create_surface(&qh, ());
        let bare_inhibitor = inhibitor_manager.create_inhibitor(&bare, &qh, ());

        // A transparent 1x1 pixel really shown in the background layer. Without layer-shell
        // and shm there is nothing better to do.
        let shell = globals.bind::<ZwlrLayerShellV1, _, _>(&qh, 1..=1, ());
        let shm = globals.bind::<WlShm, _, _>(&qh, 1..=1, ());
        let notice = inhibition_warning(shell.is_ok(), shm.is_ok());
        let layer = match (shell, shm) {
            (Ok(shell), Ok(shm)) => {
                let surface = compositor.create_surface(&qh, ());
                let mapped = map_surface(
                    &compositor,
                    &shell,
                    &shm,
                    &surface,
                    &mut event_queue,
                    &mut app,
                )
                .map_err(|err| format!("failed to show the inhibitor surface: {err}"))?;
                Some(LayerInhibitor {
                    surface,
                    mapped,
                    inhibitor: None,
                    settle_until: Instant::now() + LAYER_SETTLE,
                })
            }
            _ => None,
        };

        event_queue
            .roundtrip(&mut app)
            .map_err(|err| format!("failed to complete Wayland setup: {err}"))?;

        let timeout = Timespec::try_from(timeout)
            .map_err(|err| format!("failed to build poll timeout: {err}"))?;

        Ok(Self {
            app,
            event_queue,
            qh,
            manager: inhibitor_manager,
            bare,
            bare_inhibitor,
            layer,
            notice,
            timeout,
        })
    }

    /// Something the caller should tell the user, at most once per occurrence.
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    pub fn tick(&mut self) -> Result<(), String> {
        self.dispatch_once()?;
        if self.app.closed {
            // The compositor took the displayed surface away (its output went away, for
            // instance). The bare inhibitor is unaffected, so keep running with it alone.
            self.app.closed = false;
            if let Some(layer) = self.layer.take() {
                layer.destroy();
                self.notice = Some(
                    "the compositor closed the inhibitor surface; some compositors may no \
                     longer honor the inhibition"
                        .to_owned(),
                );
            }
        }
        if let Some(layer) = self.layer.as_mut() {
            // Created only now that the surface is mapped and has finished fading in: Sway and
            // Hyprland judge an inhibitor when it appears and do not look again when its
            // surface becomes visible later.
            if layer.inhibitor.is_none() && Instant::now() >= layer.settle_until {
                layer.inhibitor = Some(self.manager.create_inhibitor(&layer.surface, &self.qh, ()));
                self.event_queue
                    .flush()
                    .map_err(|err| format!("failed to flush the Wayland connection: {err}"))?;
            }
        }
        Ok(())
    }

    fn dispatch_once(&mut self) -> Result<(), String> {
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
        self.bare_inhibitor.destroy();
        self.bare.destroy();
        if let Some(layer) = self.layer.take() {
            layer.destroy();
        }
        self.event_queue
            .flush()
            .map_err(|err| format!("failed to flush shutdown requests: {err}"))
    }
}

impl LayerInhibitor {
    fn destroy(self) {
        if let Some(inhibitor) = self.inhibitor {
            inhibitor.destroy();
        }
        self.mapped.layer_surface.destroy();
        self.mapped.buffer.destroy();
        self.surface.destroy();
    }
}

/// What to tell the user when the compositor cannot show a surface for us. The bare inhibitor
/// is still created, but several compositors (GNOME above all) only honor inhibitors on real
/// windows, so the user must not be left believing the session is protected.
fn inhibition_warning(has_layer_shell: bool, has_shm: bool) -> Option<String> {
    if has_layer_shell && has_shm {
        return None;
    }
    let missing = if has_layer_shell {
        "wl_shm"
    } else {
        "wlr-layer-shell"
    };
    Some(format!(
        "this compositor does not offer {missing}, so wayinhibit can only use a surface without \
         a role; compositors such as GNOME ignore it and the session may still go idle"
    ))
}

/// Gives `surface` the layer-shell role, waits for the compositor to configure it, then
/// attaches a transparent pixel so that it is mapped.
fn map_surface(
    compositor: &WlCompositor,
    shell: &ZwlrLayerShellV1,
    shm: &WlShm,
    surface: &WlSurface,
    queue: &mut EventQueue<App>,
    app: &mut App,
) -> Result<MappedSurface, String> {
    let qh = queue.handle();

    let layer_surface = shell.get_layer_surface(
        surface,
        None, // let the compositor pick the output
        Layer::Background,
        "wayinhibit".to_owned(),
        &qh,
        (),
    );
    layer_surface.set_size(1, 1);
    layer_surface.set_exclusive_zone(-1); // do not move other surfaces around
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);

    // An empty input region: the pixel must never catch a click or a hover.
    let region = compositor.create_region(&qh, ());
    surface.set_input_region(Some(&region));
    region.destroy();

    // The first commit carries no buffer; the compositor answers with a `configure`, and only
    // after acknowledging it may a buffer be attached.
    surface.commit();
    for _ in 0..CONFIGURE_ROUNDTRIPS {
        if app.configured || app.closed {
            break;
        }
        queue
            .roundtrip(app)
            .map_err(|err| format!("Wayland round trip failed: {err}"))?;
    }
    if app.closed {
        return Err("the compositor refused the surface".to_owned());
    }
    if !app.configured {
        return Err("the compositor never configured the surface".to_owned());
    }

    let buffer = transparent_pixel(shm, &qh)?;
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    queue
        .roundtrip(app)
        .map_err(|err| format!("Wayland round trip failed: {err}"))?;
    if app.closed {
        return Err("the compositor closed the surface".to_owned());
    }

    Ok(MappedSurface {
        layer_surface,
        buffer,
    })
}

/// A 1x1 fully transparent ARGB pixel. A new memory file is zero-filled, which in this
/// format is exactly that, so nothing needs to be written or mapped.
fn transparent_pixel(shm: &WlShm, qh: &QueueHandle<App>) -> Result<WlBuffer, String> {
    const BYTES: u32 = 4;
    let file = memfd_create("wayinhibit", MemfdFlags::CLOEXEC)
        .map_err(|err| format!("failed to create shared memory: {err}"))?;
    ftruncate(&file, u64::from(BYTES))
        .map_err(|err| format!("failed to size shared memory: {err}"))?;
    let pool = shm.create_pool(std::os::fd::AsFd::as_fd(&file), BYTES as i32, qh, ());
    let buffer = pool.create_buffer(0, 1, 1, BYTES as i32, wl_shm::Format::Argb8888, qh, ());
    pool.destroy(); // the buffer keeps the memory alive
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::inhibition_warning;

    #[test]
    fn no_warning_when_both_globals_exist() {
        assert_eq!(inhibition_warning(true, true), None);
    }

    #[test]
    fn warns_naming_the_missing_global() {
        let no_shell = inhibition_warning(false, true).unwrap();
        assert!(no_shell.contains("wlr-layer-shell"), "{no_shell}");
        assert!(no_shell.contains("GNOME"), "{no_shell}");
        let no_shm = inhibition_warning(true, false).unwrap();
        assert!(no_shm.contains("wl_shm"), "{no_shm}");
        assert!(inhibition_warning(false, false).is_some());
    }
}
