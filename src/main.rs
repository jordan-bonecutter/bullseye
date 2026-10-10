mod selection;
use wayland_client::{
    delegate_noop,
    protocol::{
        wl_compositor::WlCompositor,
        wl_pointer::{Event as PointerEvent, WlPointer},
        wl_seat::WlSeat,
        wl_surface::WlSurface,
    },
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::wp::layer_shell::client::{
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
    zwlr_layer_surface_v1::{Event as LayerSurfaceEvent, ZwlrLayerSurfaceV1},
};

struct State {
    compositor: Option<WlCompositor>,
    surface: Option<WlSurface>,
    layer_shell: Option<ZwlrLayerShellV1>,
    layer_surface: Option<ZwlrLayerSurfaceV1>,
    seat: Option<WlSeat>,
    pointer: Option<WlPointer>,
    exit: bool,
}

impl State {
    fn new() -> Self {
        Self {
            compositor: None,
            surface: None,
            layer_shell: None,
            layer_surface: None,
            seat: None,
            pointer: None,
            exit: false,
        }
    }
}

impl Dispatch<WlCompositor, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: <WlSeat as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_seat::Event::Capabilities { capabilities } = event {
            if let WEnum::Value(caps) = capabilities {
                if caps.contains(wayland_client::protocol::wl_seat::Capability::Pointer) {
                    let pointer = seat.get_pointer(qh, ());
                    state.pointer = Some(pointer);
                }
            }
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: PointerEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let PointerEvent::Button { state: btn_state, .. } = event {
            if let WEnum::Value(btn_state) = btn_state {
                if btn_state == wayland_client::protocol::wl_pointer::ButtonState::Pressed {
                    println!("click -> exiting");
                    state.exit = true;
                }
            }
        }
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: <ZwlrLayerShellV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: LayerSurfaceEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let LayerSurfaceEvent::Configure { serial, .. } = event {
            layer_surface.ack_configure(serial);
        }
        if let LayerSurfaceEvent::Closed = event {
            state.exit = true;
        }
    }
}

delegate_noop!(State: ignore WlSurface);

fn main() {
    let conn = Connection::connect_to_env().unwrap();
    let display = conn.display();

    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut state = State::new();

    // Get globals
    let globals = wayland_client::globals::registry_queue_init(&conn).unwrap();
    let globals = globals.0;

    state.compositor = globals.bind(&qh, 1..=4, ()).ok();
    state.layer_shell = globals.bind(&qh, 1..=4, ()).ok();
    state.seat = globals.bind(&qh, 1..=7, ()).ok();

    // Create surface
    let surface = state
        .compositor
        .as_ref()
        .unwrap()
        .create_surface(&qh, ());
    state.surface = Some(surface.clone());

    // Create layer surface
    let layer_surface = state
        .layer_shell
        .as_ref()
        .unwrap();
        //.get_layer_surface(
        //    &surface,
        //    None,
        //    wayland_protocols::wp::layer_shell::client::zwlr_layer_shell_v1::Layer::Overlay,
        //    "mini-slurp".into(),
        //    &qh,
        //    (),
        //);

    layer_surface.set_size(0, 0); // fullscreen
    layer_surface.set_anchor(
        wayland_protocols::wp::layer_shell::client::zwlr_layer_surface_v1::Anchor::all(),
    );

    surface.commit();

    state.layer_surface = Some(layer_surface);

    // Event loop
    while !state.exit {
        event_queue.blocking_dispatch(&mut state).unwrap();
    }
}
