use enigo::{Button, Direction, Enigo, Mouse, Settings};

#[cfg(target_os = "linux")]
mod wayland;

pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value == "wayland"))
}

pub enum Input {
    Native(Box<Enigo>),
    #[cfg(target_os = "linux")]
    Wayland(wayland::Pointer),
}

impl Input {
    pub fn new() -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        if is_wayland() {
            // Never fall back to XWayland: that can report success without reaching
            // native applications, or duplicate clicks when both backends are used.
            return wayland::Pointer::new().map(Self::Wayland);
        }
        Enigo::new(&Settings::default())
            .map(|input| Self::Native(Box::new(input)))
            .map_err(|error| format!("Input unavailable: {error}"))
    }

    pub fn click(&mut self, button: usize) -> Result<(), String> {
        match self {
            Self::Native(input) => input
                .button(
                    [Button::Left, Button::Right, Button::Middle][button],
                    Direction::Click,
                )
                .map_err(|error| format!("Click failed: {error}")),
            #[cfg(target_os = "linux")]
            Self::Wayland(input) => input.click(button),
        }
    }
}
