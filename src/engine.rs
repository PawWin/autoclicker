use std::{sync::mpsc, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub interval: Duration,
    pub button: usize,
    pub limit: Option<u64>,
}

impl Config {
    pub fn parse(interval: &str, button: i32, limited: bool, limit: i32) -> Result<Self, String> {
        let ms = interval
            .trim()
            .parse::<u64>()
            .map_err(|_| "Enter a whole number for the interval.")?;
        if !(1..=60_000).contains(&ms) {
            return Err("Choose an interval between 1 and 60,000 ms.".into());
        }
        if !(0..=2).contains(&button) || (limited && limit < 1) {
            return Err("Choose a valid mouse button and click count.".into());
        }
        Ok(Self {
            interval: Duration::from_millis(ms),
            button: button as usize,
            limit: limited.then_some(limit as u64),
        })
    }
}

pub enum Command {
    Start(u64, Config),
    Stop,
    Shutdown,
}
pub enum Event {
    Countdown(u64, i32),
    Clicks(u64, u64),
    Finished(u64),
    Error(u64, String),
}

// Every wait is interruptible, including long intervals and the initial countdown.
// Returning the received command preserves a new start or shutdown for the outer loop.
fn wait(rx: &mpsc::Receiver<Command>, duration: Duration) -> Result<(), Command> {
    match rx.recv_timeout(duration) {
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Command::Shutdown),
        Ok(command) => Err(command),
    }
}

fn session(
    id: u64,
    config: Config,
    rx: &mpsc::Receiver<Command>,
    tx: &mpsc::Sender<Event>,
    mut click: impl FnMut(usize) -> Result<(), String>,
    countdown_tick: Duration,
) -> Result<(), Command> {
    for seconds in (1..=3).rev() {
        let _ = tx.send(Event::Countdown(id, seconds));
        wait(rx, countdown_tick)?;
    }
    let _ = tx.send(Event::Countdown(id, 0));
    let mut count = 0;
    loop {
        // Check cancellation before injecting input, including immediately after countdown.
        if let Ok(command) = rx.try_recv() {
            return Err(command);
        }
        if let Err(error) = click(config.button) {
            let _ = tx.send(Event::Error(id, error));
            return Ok(());
        }
        count += 1;
        let _ = tx.send(Event::Clicks(id, count));
        if config.limit.is_some_and(|limit| count >= limit) {
            let _ = tx.send(Event::Finished(id));
            return Ok(());
        }
        wait(rx, config.interval)?;
    }
}

pub fn worker(rx: mpsc::Receiver<Command>, tx: mpsc::Sender<Event>) {
    use crate::input::Input;
    let mut pending = None;
    loop {
        let command = pending.take().or_else(|| rx.recv().ok());
        match command {
            Some(Command::Start(id, config)) => {
                let mut input = match Input::new() {
                    Ok(input) => input,
                    Err(error) => {
                        let _ = tx.send(Event::Error(id, error));
                        continue;
                    }
                };
                pending = session(
                    id,
                    config,
                    &rx,
                    &tx,
                    |button| input.click(button),
                    Duration::from_secs(1),
                )
                .err();
            }
            Some(Command::Stop) => {}
            Some(Command::Shutdown) | None => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_intervals_and_counts() {
        for interval in ["", "abc", "-1", "0", "60001", "1.5"] {
            assert!(Config::parse(interval, 0, false, 1).is_err());
        }
        assert!(Config::parse("10", 0, true, 0).is_err());
        assert!(Config::parse("60000", 2, true, 1).is_ok());
    }

    #[test]
    fn accepts_intervals_below_ten_milliseconds() {
        for ms in 1..=10 {
            let config = Config::parse(&ms.to_string(), 0, false, 1).unwrap();
            assert_eq!(config.interval, Duration::from_millis(ms));
        }
    }

    #[test]
    fn stops_at_exact_click_limit() {
        let (_commands, rx) = mpsc::channel();
        let (tx, events) = mpsc::channel();
        let mut clicks = 0;
        let config = Config {
            interval: Duration::ZERO,
            button: 1,
            limit: Some(3),
        };
        assert!(
            session(
                7,
                config,
                &rx,
                &tx,
                |button| {
                    assert_eq!(button, 1);
                    clicks += 1;
                    Ok(())
                },
                Duration::ZERO
            )
            .is_ok()
        );
        assert_eq!(clicks, 3);
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Finished(7)))
        );
    }

    #[test]
    fn cancellation_during_countdown_injects_no_clicks() {
        let (commands, rx) = mpsc::channel();
        let (tx, _) = mpsc::channel();
        commands.send(Command::Stop).unwrap();
        let config = Config::parse("60000", 0, false, 1).unwrap();
        assert!(matches!(
            session(
                1,
                config,
                &rx,
                &tx,
                |_| panic!("must not click"),
                Duration::from_secs(1)
            ),
            Err(Command::Stop)
        ));
    }

    #[test]
    fn cancellation_interrupts_a_long_interval() {
        let (commands, rx) = mpsc::channel();
        let (tx, _) = mpsc::channel();
        let config = Config::parse("60000", 0, false, 1).unwrap();
        let mut clicks = 0;
        assert!(matches!(
            session(
                1,
                config,
                &rx,
                &tx,
                |_| {
                    clicks += 1;
                    commands.send(Command::Stop).unwrap();
                    Ok(())
                },
                Duration::ZERO
            ),
            Err(Command::Stop)
        ));
        assert_eq!(clicks, 1);
    }

    #[test]
    fn input_errors_end_the_session() {
        let (_commands, rx) = mpsc::channel();
        let (tx, events) = mpsc::channel();
        let config = Config::parse("100", 0, false, 1).unwrap();
        assert!(
            session(
                9,
                config,
                &rx,
                &tx,
                |_| Err("denied".into()),
                Duration::ZERO
            )
            .is_ok()
        );
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Error(9, _)))
        );
    }
}
