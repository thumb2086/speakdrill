//! speakdrill core：把 MyET 口說練習自動化的所有零件。

pub mod audio;
pub mod cdp;
pub mod devices;
pub mod config;
pub mod lesson;
pub mod pipeline;
pub mod piper;
pub mod tts;
pub mod vm;

#[cfg(windows)]
pub mod winvol;

pub use audio::AudioPlayer;
pub use cdp::Cdp;
pub use config::Config;
pub use lesson::{fetch_catalog, fetch_lines, scoring_query, scoring_url, LessonRef, Line};
pub use pipeline::{
    ensure_logged_in, ensure_tts, run_lesson, save_report, LessonReport, LineScore, Progress,
    RunOptions, SilentProgress,
};
pub use tts::{TtsConfig, TtsEngine};
pub use vm::Vm;

/// 版本字串
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
