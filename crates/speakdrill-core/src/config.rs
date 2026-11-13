//! 設定檔（JSON）與路徑。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::tts::{TtsConfig, TtsEngine};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: String,
    pub code: String,
    pub content_type: String,
    pub mode: u32,
    /// TTS 要播到哪個輸出裝置（名稱片段）
    pub play_device: String,
    /// MyET 的 debug port
    pub cdp_port: u16,
    pub pass_score: u32,
    pub max_retry: u32,
    /// VoiceMeeter strip（TTS 落點）；None = 自動
    pub vm_strip: Option<usize>,
    /// VoiceMeeter bus（B1）；None = 自動
    pub vm_bus: Option<usize>,
    /// 是否在開跑前自動呼叫 VM 路由設定
    pub auto_vm_setup: bool,
    /// 目標 App 的執行檔名稱（用於比對 per-app 麥克風綁定）
    #[serde(default = "default_app_exe")]
    pub app_exe: String,
    pub tts: TtsConfig,
}


impl Default for Config {
    fn default() -> Self {
        Self {
            server: "https://140.125.168.213".into(),
            code: "BKM_1A".into(),
            content_type: "GEN".into(),
            mode: 6,
            play_device: "Voicemeeter Input".into(),
            cdp_port: 9222,
            pass_score: 80,
            max_retry: 3,
            vm_strip: None,
            vm_bus: None,
            auto_vm_setup: true,
            app_exe: default_app_exe(),
            tts: TtsConfig {
                engine: TtsEngine::Command {
                    template: "python tts_kokoro.py {textfile} {out}".into(),
                },
                length_scale: 1.0,
            },
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("speakdrill")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("speakdrill")
}

pub fn audio_cache(lesson: &str) -> PathBuf {
    cache_dir().join("audio").join(lesson)
}

pub fn reports_dir() -> PathBuf {
    config_dir().join("reports")
}

impl Config {
    pub fn load_or_default() -> Self {
        match std::fs::read_to_string(config_path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::write(config_path(), serde_json::to_string_pretty(self)?)
            .with_context(|| format!("寫入 {} 失敗", config_path().display()))?;
        Ok(())
    }
}

/// 所有可用的 CodeName 對應表
pub static KNOWN_CODENAMES: &[(&str, &str)] = &[
    ("BKM_1A", "書林美語走天下-1A"),
    ("BKM_1B", "書林美語走天下-1B"),
    ("BKM_2A", "書林美語走天下-2A"),
    ("BKM_2B", "書林美語走天下-2B"),
    ("BKM_3A", "書林美語走天下-3A"),
    ("BKM_3B", "書林美語走天下-3B"),
    ("BKM_GAR_1A", "Getting Around in American English 1A"),
    ("BKM_GAR_2A", "Getting Around in American English 2A"),
    ("BKM_AME_1A", "American English 1A"),
    ("BKM_MBA_SALES", "MBA 英語-銷售管理"),
    ("BKM_MBA_PROD", "MBA 英語-生產管理"),
    ("BKM_MBA_FIN", "MBA 英語-財務管理"),
    ("BKM_MBA_HR", "MBA 英語-人資管理"),
    ("BKM_MBA_MKT", "MBA 英語-行銷管理"),
    ("BKM_MBA_RD", "MBA 英語-研發管理"),
    ("BKM_MICE", "MICE 英語"),
    ("BKM_TOEIC_1", "多益口說測驗攻略 Part 1"),
    ("BKM_TOEIC_2", "多益口說測驗攻略 Part 2"),
    ("BKM_TOEIC_3", "多益口說測驗攻略 Part 3"),
    ("BKM_TOEIC_SP", "TOEIC Speaking"),
];

/// 取得所有已知的 CodeName
pub fn all_codenames() -> Vec<(&'static str, &'static str)> {
    KNOWN_CODENAMES.to_vec()
}

pub fn default_app_exe() -> String {
    "MyET10".to_string()
}

