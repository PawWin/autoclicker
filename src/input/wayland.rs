use std::time::Instant;
use wayland_client::{
    Connection, EventQueue, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_pointer::ButtonState, wl_registry},
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

struct State;

impl wayland_client::Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore ZwlrVirtualPointerV1);

pub struct Pointer {
    queue: EventQueue<State>,
    pointer: ZwlrVirtualPointerV1,
    manager: ZwlrVirtualPointerManagerV1,
    started: Instant,
}

impl Pointer {
    pub fn new() -> Result<Self, String> {
        let connection = Connection::connect_to_env()
            .map_err(|error| format!("Cannot connect to Wayland: {error}"))?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, String> {
        let (globals, mut queue) = registry_queue_init::<State>(&connection)
            .map_err(|error| format!("Cannot query Wayland protocols: {error}"))?;
        let handle = queue.handle();
        let manager: ZwlrVirtualPointerManagerV1 = globals.bind(&handle, 1..=2, ())
            .map_err(|_| "This compositor does not expose wlr-virtual-pointer. Use niri 25.02+ or Hyprland with virtual pointer support.".to_string())?;
        // A null seat lets the compositor choose its default seat. No keyboard,
        // input-method, output geometry, or privileged helper is needed.
        let pointer = manager.create_virtual_pointer(None, &handle, ());
        queue
            .roundtrip(&mut State)
            .map_err(|error| format!("Wayland pointer unavailable: {error}"))?;
        Ok(Self {
            queue,
            pointer,
            manager,
            started: Instant::now(),
        })
    }

    pub fn click(&mut self, button: usize) -> Result<(), String> {
        // Linux input-event codes: BTN_LEFT, BTN_RIGHT, BTN_MIDDLE.
        let code = [0x110, 0x111, 0x112][button];
        let time = self.started.elapsed().as_millis() as u32;
        self.pointer.button(time, code, ButtonState::Pressed);
        self.pointer.frame();
        self.pointer.button(time, code, ButtonState::Released);
        self.pointer.frame();
        self.queue
            .roundtrip(&mut State)
            .map_err(|error| format!("Wayland click failed: {error}"))?;
        Ok(())
    }
}

impl Drop for Pointer {
    fn drop(&mut self) {
        self.pointer.destroy();
        self.manager.destroy();
        let _ = self.queue.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::net::UnixStream,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };
    use wayland_protocols_wlr::virtual_pointer::v1::server::{
        zwlr_virtual_pointer_manager_v1::{
            self as manager, ZwlrVirtualPointerManagerV1 as Manager,
        },
        zwlr_virtual_pointer_v1::{self as pointer, ZwlrVirtualPointerV1 as VirtualPointer},
    };
    use wayland_server::{Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New};

    struct ServerState(mpsc::Sender<String>);

    impl GlobalDispatch<Manager, ()> for ServerState {
        fn bind(
            _: &mut Self,
            _: &DisplayHandle,
            _: &Client,
            resource: New<Manager>,
            _: &(),
            init: &mut DataInit<'_, Self>,
        ) {
            init.init(resource, ());
        }
    }

    impl Dispatch<Manager, ()> for ServerState {
        fn request(
            state: &mut Self,
            _: &Client,
            _: &Manager,
            request: manager::Request,
            _: &(),
            _: &DisplayHandle,
            init: &mut DataInit<'_, Self>,
        ) {
            if let manager::Request::CreateVirtualPointer { id, seat } = request {
                assert!(seat.is_none());
                init.init(id, ());
                state.0.send("created".into()).unwrap();
            }
        }
    }

    impl Dispatch<VirtualPointer, ()> for ServerState {
        fn request(
            state: &mut Self,
            _: &Client,
            _: &VirtualPointer,
            request: pointer::Request,
            _: &(),
            _: &DisplayHandle,
            _: &mut DataInit<'_, Self>,
        ) {
            let event = match request {
                pointer::Request::Button { button, state, .. } => format!("{button}:{state:?}"),
                pointer::Request::Frame => "frame".into(),
                pointer::Request::Destroy => return,
                _ => panic!("unexpected pointer request"),
            };
            let _ = state.0.send(event);
        }
    }

    struct Server {
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
        events: mpsc::Receiver<String>,
    }

    impl Server {
        fn start(version: Option<u32>) -> (Self, Connection) {
            let (client_socket, server_socket) = UnixStream::pair().unwrap();
            let mut display = Display::<ServerState>::new().unwrap();
            let mut handle = display.handle();
            if let Some(version) = version {
                handle.create_global::<ServerState, Manager, _>(version, ());
            }
            handle.insert_client(server_socket, Arc::new(())).unwrap();
            let (tx, events) = mpsc::channel();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let thread = thread::spawn(move || {
                let mut state = ServerState(tx);
                let deadline = Instant::now() + Duration::from_secs(5);
                while !stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
                    display.dispatch_clients(&mut state).unwrap();
                    display.flush_clients().unwrap();
                    thread::sleep(Duration::from_millis(1));
                }
            });
            (
                Self {
                    stop,
                    thread: Some(thread),
                    events,
                },
                Connection::from_socket(client_socket).unwrap(),
            )
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    #[test]
    fn pointer_only_compositor_receives_exact_clicks_for_all_buttons() {
        // No keyboard or X11 globals exist; test both supported manager versions.
        for version in [1, 2] {
            let (server, connection) = Server::start(Some(version));
            let mut input = Pointer::from_connection(connection).unwrap();
            assert_eq!(server.events.try_recv().unwrap(), "created");
            for (button, code) in [272, 273, 274].into_iter().enumerate() {
                input.click(button).unwrap();
                let events: Vec<_> = server.events.try_iter().collect();
                assert_eq!(
                    events,
                    [
                        format!("{code}:Value(Pressed)"),
                        "frame".into(),
                        format!("{code}:Value(Released)"),
                        "frame".into()
                    ]
                );
            }
        }
    }

    #[test]
    fn disconnected_compositor_reports_click_failure() {
        let (server, connection) = Server::start(Some(2));
        let mut input = Pointer::from_connection(connection).unwrap();
        drop(server);
        assert!(input.click(0).unwrap_err().contains("Wayland click failed"));
    }

    #[test]
    fn missing_virtual_pointer_is_an_actionable_error() {
        let (_server, connection) = Server::start(None);
        let error = Pointer::from_connection(connection).err().unwrap();
        assert!(error.contains("wlr-virtual-pointer"));
        assert!(error.contains("niri"));
    }
}
