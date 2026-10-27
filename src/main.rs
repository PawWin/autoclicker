#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod engine;

use engine::{Command, Config, Event};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey},
};
use slint::ComponentHandle;
use std::{cell::Cell, rc::Rc, sync::mpsc, time::Duration};

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;
    let (commands, command_rx) = mpsc::channel();
    let (events, event_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || engine::worker(command_rx, events));
    let generation = Rc::new(Cell::new(0u64));

    // X11 shortcuts do not work across native Wayland applications. Don't advertise them.
    let wayland = cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some();
    let hotkey = HotKey::new(None, Code::F6);
    let manager = if wayland {
        None
    } else {
        GlobalHotKeyManager::new()
            .ok()
            .filter(|manager| manager.register(hotkey).is_ok())
    };
    ui.set_hotkey_available(manager.is_some());
    let default_notice: slint::SharedString = if wayland {
        "Wayland: global F6 is unavailable. Use Stop in this window. Input requires compositor support.".into()
    } else if manager.is_none() {
        "F6 is unavailable. Use the Start / Stop button in this window.".into()
    } else {
        "".into()
    };
    ui.set_notice(default_notice.clone());

    let weak = ui.as_weak();
    let tx = commands.clone();
    let current = generation.clone();
    ui.on_toggle(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_running() {
            current.set(current.get().wrapping_add(1));
            let _ = tx.send(Command::Stop);
            ui.set_running(false);
            ui.set_countdown(0);
            ui.set_status("Stopped".into());
            return;
        }
        let config = match Config::parse(
            &ui.get_interval_text(),
            ui.get_mouse_button(),
            ui.get_limited(),
            ui.get_limit(),
        ) {
            Ok(config) => config,
            Err(error) => {
                ui.set_notice(error.into());
                return;
            }
        };
        current.set(current.get().wrapping_add(1));
        ui.set_notice(default_notice.clone());
        ui.set_clicks("0".into());
        ui.set_countdown(3);
        ui.set_status("Move your cursor to the target".into());
        ui.set_running(true);
        if tx.send(Command::Start(current.get(), config)).is_err() {
            ui.set_running(false);
            ui.set_status("Click worker unavailable".into());
        }
    });

    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(30),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            for event in GlobalHotKeyEvent::receiver().try_iter() {
                if event.id == hotkey.id() && event.state == HotKeyState::Pressed {
                    ui.invoke_toggle();
                }
            }
            for event in event_rx.try_iter() {
                match event {
                    Event::Countdown(id, seconds) if id == generation.get() => {
                        ui.set_countdown(seconds);
                        if seconds == 0 {
                            ui.set_status("Clicking at cursor".into());
                        }
                    }
                    Event::Clicks(id, count) if id == generation.get() => {
                        ui.set_clicks(count.to_string().into())
                    }
                    Event::Finished(id) if id == generation.get() => {
                        ui.set_running(false);
                        ui.set_status("Completed".into());
                    }
                    Event::Error(id, error) if id == generation.get() => {
                        ui.set_running(false);
                        ui.set_countdown(0);
                        ui.set_status("Couldn't start clicking".into());
                        ui.set_notice(error.into());
                    }
                    _ => {}
                }
            }
        },
    );

    let result = ui.run();
    timer.stop();
    let _ = commands.send(Command::Shutdown);
    let _ = worker.join();
    drop(manager);
    result.map_err(Into::into)
}
