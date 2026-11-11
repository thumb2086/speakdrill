//! TTS：把句子合成 wav。
//!
//! 兩種引擎：
//!   - `piper`：單一執行檔 + onnx 聲音檔，最好帶（Rust 專案首選）
//!   - `command`：自訂命令，適合已經有 Kokoro / 其他 TTS 的人
//!
//! 一律輸出 16-bit PCM wav 到快取資料夾。

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TtsEngine {
    /// piper --model <model> --output_file <out>
    Piper { exe: String, model: String },
    /// 自訂命令；支援 {textfile} 與 {out} 佔位符
    Command { template: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsConfig {
    pub engine: TtsEngine,
    /// 語速（piper 用 --length_scale，值越大越慢）
    #[serde(default = "default_speed")]
    pub length_scale: f32,
}

fn default_speed() -> f32 {
    1.0
}

impl TtsConfig {
    pub fn synthesize(&self, text: &str, out: &Path) -> Result<()> {
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        match &self.engine {
            TtsEngine::Piper { exe, model } => {
                let mut child = Command::new(exe)
                    .arg("--model")
                    .arg(model)
                    .arg("--output_file")
                    .arg(out)
                    .arg("--length_scale")
                    .arg(self.length_scale.to_string())
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .with_context(|| format!("啟動 piper 失敗（{exe}）"))?;

                {
                    use std::io::Write;
                    let stdin = child.stdin.as_mut().ok_or_else(|| anyhow!("piper stdin 不可用"))?;
                    stdin.write_all(text.as_bytes())?;
                }
                let out_res = child.wait_with_output()?;
                if !out_res.status.success() {
                    return Err(anyhow!(
                        "piper 失敗：{}",
                        String::from_utf8_lossy(&out_res.stderr).trim()
                    ));
                }
            }
            TtsEngine::Command { template } => {
                let textfile = out.with_extension("txt");
                std::fs::write(&textfile, text)?;
                let cmd = template
                    .replace("{textfile}", &textfile.display().to_string())
                    .replace("{out}", &out.display().to_string())
                    .replace("{text}", text);
                let status = Command::new("cmd")
                    .arg("/C")
                    .arg(&cmd)
                    .stdout(std::process::Stdio::null())
                    .status()
                    .with_context(|| format!("執行 TTS 命令失敗：{cmd}"))?;
                if !status.success() {
                    return Err(anyhow!("TTS 命令回傳失敗：{cmd}"));
                }
                std::fs::remove_file(&textfile).ok();
            }
        }

        if !out.exists() {
            return Err(anyhow!("TTS 沒有產出檔案：{}", out.display()));
        }
        Ok(())
    }
}

/// 找出 piper 執行檔（PATH 或常見位置）
pub fn find_piper() -> Option<PathBuf> {
    if let Ok(p) = which("piper") {
        return Some(p);
    }
    let candidates = [
        dirs::data_dir().map(|d| d.join("speakdrill").join("piper").join("piper.exe")),
        dirs::config_dir().map(|d| d.join("speakdrill").join("piper").join("piper.exe")),
    ];
    for c in candidates.into_iter().flatten() {
        if c.exists() {
            return Some(c);
        }
    }
    None
}

fn which(name: &str) -> Result<PathBuf> {
    let out = Command::new("where").arg(name).output()?;
    if !out.status.success() {
        return Err(anyhow!("not found"));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let first = s.lines().next().unwrap_or("").trim().to_string();
    if first.is_empty() {
        Err(anyhow!("not found"))
    } else {
        Ok(PathBuf::from(first))
    }
}
