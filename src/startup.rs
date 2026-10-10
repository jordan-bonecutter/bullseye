//! Opt-in startup timing, with no logging during normal use.
use std::{sync::OnceLock, time::Instant};

static START: OnceLock<Option<Instant>> = OnceLock::new();

pub fn start() {
    START.get_or_init(|| std::env::var_os("BULLSEYE_PROFILE_STARTUP").map(|_| Instant::now()));
}

pub fn stage(name: &str) {
    if let Some(Some(start)) = START.get() {
        eprintln!(
            "Bullseye startup {name}: {:.2} ms",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
}
