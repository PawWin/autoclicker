//! Own the global shortcut for the lifetime of the window.
use global_hotkey::{
    GlobalHotKeyManager,
    hotkey::{Code, HotKey},
};

pub struct Shortcuts {
    native: Option<GlobalHotKeyManager>,
    #[cfg(target_os = "linux")]
    compositor: Option<crate::compositor::Compositor>,
}

pub fn hotkey(key: &str) -> Result<HotKey, String> {
    let code = match key {
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        _ => return Err("Choose a function key from F1 to F12.".into()),
    };
    Ok(HotKey::new(None, code))
}

impl Shortcuts {
    pub fn new(wayland: bool) -> Result<Self, String> {
        if wayland {
            #[cfg(target_os = "linux")]
            return Ok(Self {
                native: None,
                compositor: Some(crate::compositor::Compositor::detect()?),
            });
            #[cfg(not(target_os = "linux"))]
            return Err("Global shortcuts are unavailable in this session.".into());
        }
        Ok(Self {
            native: Some(
                GlobalHotKeyManager::new()
                    .map_err(|e| format!("Cannot access global shortcuts: {e}"))?,
            ),
            #[cfg(target_os = "linux")]
            compositor: None,
        })
    }

    pub fn register(&mut self, key: &str) -> Result<HotKey, String> {
        let hotkey = hotkey(key)?;
        if let Some(manager) = &self.native {
            manager
                .register(hotkey)
                .map_err(|_| format!("{key} is unavailable. Choose another key."))?;
        }
        #[cfg(target_os = "linux")]
        if let Some(compositor) = &mut self.compositor {
            compositor.register(key)?;
        }
        Ok(hotkey)
    }
}
