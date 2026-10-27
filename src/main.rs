#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod compositor;
#[cfg(target_os = "linux")]
mod control;
mod engine;
mod input;
mod shortcuts;

use engine::{Command, Config, Event};
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::mpsc,
    time::Duration,
};

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        match args.as_slice() {
            [flag] if flag == "--help" || flag == "-h" => {
                println!(
                    "Tap autoclicker\nUsage: tap-autoclicker [--toggle | --stop | --help]\n\nOn Linux/Wayland, --toggle and --stop control the running app."
                );
                return Ok(());
            }
            #[cfg(target_os = "linux")]
            [flag] if flag == "--toggle" || flag == "--stop" => {
                let action = if flag == "--toggle" {
                    control::Action::Toggle
                } else {
                    control::Action::Stop
                };
                control::send(action).map_err(|error| {
                    format!("Cannot control Tap. Open Tap in this Wayland session first: {error}")
                })?;
                return Ok(());
            }
            _ => return Err("Unknown arguments. Use --help for usage.".into()),
        }
    }
    let wayland = input::is_wayland();
    #[cfg(target_os = "linux")]
    let control = if wayland {
        Some(control::Control::bind()?)
    } else {
        None
    };
    let ui = AppWindow::new()?;
    let (commands, command_rx) = mpsc::channel();
    let (events, event_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || engine::worker(command_rx, events));
    let generation = Rc::new(Cell::new(0u64));

    let shortcuts = Rc::new(RefCell::new(shortcuts::Shortcuts::new(wayland)));
    let active_hotkey = Rc::new(Cell::new(None));
    let registration = match &mut *shortcuts.borrow_mut() {
        Ok(shortcuts) => shortcuts.register("F6"),
        Err(error) => Err(error.clone()),
    };
    match registration {
        Ok(hotkey) => {
            active_hotkey.set(Some(hotkey));
            ui.set_hotkey_available(true);
            ui.set_hotkey_label("F6".into());
        }
        Err(error) => {
            ui.set_shortcut_prompt(true);
            ui.set_shortcut_error(error.into());
        }
    }
    let weak = ui.as_weak();
    let backend = shortcuts.clone();
    let active = active_hotkey.clone();
    ui.on_set_shortcut(move |key| {
        let Some(ui) = weak.upgrade() else { return };
        let result = {
            let mut backend = backend.borrow_mut();
            // Retry detection after a missing compositor tool or session error.
            if backend.is_err() {
                *backend = shortcuts::Shortcuts::new(wayland);
            }
            match &mut *backend {
                Ok(shortcuts) => shortcuts.register(&key),
                Err(error) => Err(error.clone()),
            }
        };
        match result {
            Ok(hotkey) => {
                active.set(Some(hotkey));
                ui.set_hotkey_available(true);
                ui.set_hotkey_label(key);
                ui.set_shortcut_prompt(false);
                ui.set_shortcut_error("".into());
            }
            Err(error) => ui.set_shortcut_error(error.into()),
        }
    });

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
        ui.set_notice("".into());
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
            #[cfg(target_os = "linux")]
            if let Some(control) = &control {
                // Process one command per tick; a stop never starts an idle session.
                if let Some(action) = control.receive() {
                    match action {
                        control::Action::Toggle => ui.invoke_toggle(),
                        control::Action::Stop if ui.get_running() => ui.invoke_toggle(),
                        control::Action::Stop => {}
                    }
                }
            }
            for event in GlobalHotKeyEvent::receiver().try_iter() {
                if active_hotkey
                    .get()
                    .is_some_and(|hotkey| event.id == hotkey.id())
                    && event.state == HotKeyState::Pressed
                {
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
    // The timer no longer owns any shortcut state when compositor bindings are removed.
    drop(ui);
    drop(shortcuts);
    result.map_err(Into::into)
}
