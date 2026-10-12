//! The fingerprint-chromium executable: how a profile becomes its command line, and how the
//! resulting browser process is found, run and stopped.

pub mod args;
pub mod lock;
pub mod probe;
pub mod process;
pub mod shell;
