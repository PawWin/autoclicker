//! Compositor keybindings send commands to the running Wayland instance.
use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::{fs::OpenOptionsExt, net::UnixDatagram},
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Stop,
}

impl Action {
    fn parse(bytes: &[u8]) -> Option<Self> {
        match bytes {
            b"toggle" => Some(Self::Toggle),
            b"stop" => Some(Self::Stop),
            _ => None,
        }
    }
}

fn socket_path() -> io::Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| io::Error::other("XDG_RUNTIME_DIR is required for compositor shortcuts"))?;
    // Use the display's basename so absolute WAYLAND_DISPLAY paths cannot escape
    // the runtime directory. Separate nested compositor sessions get separate sockets.
    let display = std::env::var_os("WAYLAND_DISPLAY").unwrap_or_else(|| "wayland-0".into());
    let name = Path::new(&display)
        .file_name()
        .ok_or_else(|| io::Error::other("Invalid WAYLAND_DISPLAY"))?;
    let mut path = PathBuf::from(runtime);
    let mut filename = std::ffi::OsString::from("tap-autoclicker-");
    filename.push(name);
    filename.push(".sock");
    path.push(filename);
    Ok(path)
}

pub fn send(action: Action) -> io::Result<()> {
    let socket = UnixDatagram::unbound()?;
    socket.set_write_timeout(Some(std::time::Duration::from_secs(1)))?;
    socket.send_to(
        match action {
            Action::Toggle => b"toggle" as &[u8],
            Action::Stop => b"stop",
        },
        socket_path()?,
    )?;
    Ok(())
}

pub struct Control {
    socket: UnixDatagram,
    path: PathBuf,
    // Keep the lock file open until after the socket is removed. The lock file
    // stays on disk so concurrent launches never lock different inodes.
    _lock: File,
}

impl Control {
    pub fn bind() -> io::Result<Self> {
        Self::bind_at(socket_path()?)
    }

    fn bind_at(path: PathBuf) -> io::Result<Self> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path.with_extension("lock"))?;
        lock.try_lock().map_err(|error| {
            io::Error::other(format!(
                "Tap is already running, or its control lock is unavailable: {error}"
            ))
        })?;
        // Owning the lock proves no live instance owns a leftover socket.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let socket = UnixDatagram::bind(&path)?;
        let control = Self {
            socket,
            path,
            _lock: lock,
        };
        control.socket.set_nonblocking(true)?;
        Ok(control)
    }

    pub fn receive(&self) -> Option<Action> {
        let mut bytes = [0; 32];
        // Bound work per UI tick even if a client floods the socket.
        for _ in 0..32 {
            match self.socket.recv(&mut bytes) {
                Ok(size) => {
                    if let Some(action) = Action::parse(&bytes[..size]) {
                        return Some(action);
                    }
                }
                Err(_) => return None,
            }
        }
        None
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_instance_lock_and_stale_socket_recovery() {
        let path =
            std::env::temp_dir().join(format!("tap-control-test-{}.sock", std::process::id()));
        let server = Control::bind_at(path.clone()).unwrap();
        assert!(Control::bind_at(path.clone()).is_err());
        let client = UnixDatagram::unbound().unwrap();
        client.send_to(b"unknown", &path).unwrap();
        client.send_to(b"stop", &path).unwrap();
        client.send_to(b"toggle", &path).unwrap();
        assert_eq!(server.receive(), Some(Action::Stop));
        assert_eq!(server.receive(), Some(Action::Toggle));
        assert_eq!(server.receive(), None);
        drop(server);
        assert!(!path.exists());
        // Simulate a process that exited without removing its socket.
        drop(UnixDatagram::bind(&path).unwrap());
        drop(Control::bind_at(path.clone()).unwrap());
        std::fs::remove_file(path.with_extension("lock")).unwrap();
    }
}
