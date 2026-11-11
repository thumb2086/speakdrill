//! 音訊播放：用 cpal 指定裝置、零延遲餵樣本。
//!
//! 為什麼要自己寫：MyET 的錄音視窗只有 ~2.7 秒，
//! 每次啟動外部播放器（Python/ffplay）都會吃掉 0.5~1 秒。
//! 這裡把 stream 開好常駐，播歌只是把樣本塞進環形緩衝。
//!
//! 注意：cpal 的 `Stream` 是 `!Send`，所以裝置列舉與 stream 建立
//! 必須全部在音訊執行緒裡完成。

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

enum Cmd {
    Play(Vec<f32>),
    Stop,
    Quit,
}

pub struct AudioPlayer {
    tx: mpsc::Sender<Cmd>,
    playing: Arc<Mutex<bool>>,
    pub device_name: String,
    pub sample_rate: u32,
    _handle: JoinHandle<()>,
}

impl AudioPlayer {
    /// 依名稱片段找輸出裝置。`needle` 例如 "Voicemeeter Input"。
    pub fn new(needle: &str) -> Result<Self> {
        let (info_tx, info_rx) = mpsc::channel::<Result<(String, u32)>>();
        let (tx, rx) = mpsc::channel::<Cmd>();
        let playing = Arc::new(Mutex::new(false));

        let needle = needle.to_string();
        let buf: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let buf2 = buf.clone();
        let playing2 = playing.clone();
        let playing3 = playing.clone();

        let handle = std::thread::Builder::new()
            .name("myet-audio".into())
            .spawn(move || {
                let host = cpal::default_host();
                let device = host
                    .output_devices()
                    .ok()
                    .and_then(|mut it| {
                        it.find(|d| {
                            d.name()
                                .map(|n| n.to_lowercase().contains(&needle.to_lowercase()))
                                .unwrap_or(false)
                        })
                    });

                let Some(device) = device else {
                    let _ = info_tx.send(Err(anyhow!(
                        "找不到輸出裝置「{needle}」。請開 VoiceMeeter，或用 --device 指定。"
                    )));
                    return;
                };

                let name = device.name().unwrap_or_default();
                let cfg = match device.default_output_config() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = info_tx.send(Err(anyhow!("取裝置預設格式失敗：{e}")));
                        return;
                    }
                };
                let rate = cfg.sample_rate().0;
                let channels = cfg.channels() as usize;
                let stream_cfg: cpal::StreamConfig = cfg.config();

                let stream = match device.build_output_stream(
                    &stream_cfg,
                    move |data: &mut [f32], _| {
                        let mut b = buf2.lock().unwrap();
                        let mut any = false;
                        for frame in data.chunks_mut(channels) {
                            let s = b.pop_front().unwrap_or(0.0);
                            if s != 0.0 {
                                any = true;
                            }
                            for out in frame.iter_mut() {
                                *out = s;
                            }
                        }
                        let mut p = playing3.lock().unwrap();
                        if any {
                            *p = true;
                        } else if b.is_empty() {
                            *p = false;
                        }
                    },
                    move |err| eprintln!("[audio] {err}"),
                    None,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = info_tx.send(Err(anyhow!("建立輸出串流失敗：{e}")));
                        return;
                    }
                };

                if let Err(e) = stream.play() {
                    let _ = info_tx.send(Err(anyhow!("啟動串流失敗：{e}")));
                    return;
                }
                let _ = info_tx.send(Ok((name, rate)));

                // stream 必須活在這個 thread 裡
                let _stream = stream;
                loop {
                    match rx.recv() {
                        Ok(Cmd::Play(samples)) => {
                            let mut b = buf.lock().unwrap();
                            b.clear();
                            b.extend(samples);
                            *playing2.lock().unwrap() = true;
                        }
                        Ok(Cmd::Stop) => {
                            buf.lock().unwrap().clear();
                            *playing2.lock().unwrap() = false;
                        }
                        Ok(Cmd::Quit) | Err(_) => break,
                    }
                }
            })
            .context("建立音訊執行緒失敗")?;

        let (device_name, sample_rate) = info_rx
            .recv()
            .map_err(|_| anyhow!("音訊執行緒沒有回報"))??;

        Ok(Self {
            tx,
            playing,
            device_name,
            sample_rate,
            _handle: handle,
        })
    }

    pub fn play(&self, samples: Vec<f32>) -> Result<()> {
        self.tx
            .send(Cmd::Play(samples))
            .map_err(|_| anyhow!("音訊執行緒已結束"))
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }

    pub fn is_playing(&self) -> bool {
        *self.playing.lock().unwrap()
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
    }
}

/// 讀 wav → (單聲道 f32, 原始取樣率)
pub fn load_wav(path: &std::path::Path) -> Result<(Vec<f32>, u32)> {
    let mut r = hound::WavReader::open(path).with_context(|| format!("開不了 {}", path.display()))?;
    let spec = r.spec();
    let sr = spec.sample_rate;
    let ch = spec.channels as usize;

    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().filter_map(|s| s.ok()).collect(),
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / max)
                .collect()
        }
    };

    let mono: Vec<f32> = if ch <= 1 {
        raw
    } else {
        raw.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect()
    };
    Ok((mono, sr))
}

/// 線性插值重取樣（夠用，且比拉整個 rubato 輕）
pub fn resample(x: &[f32], src: u32, dst: u32) -> Vec<f32> {
    if src == dst || x.is_empty() {
        return x.to_vec();
    }
    let n = ((x.len() as f64) * (dst as f64) / (src as f64)).round() as usize;
    let mut out = Vec::with_capacity(n);
    let step = (src as f64) / (dst as f64);
    for i in 0..n {
        let pos = i as f64 * step;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(x.len() - 1);
        let t = (pos - i0 as f64) as f32;
        out.push(x[i0] * (1.0 - t) + x[i1] * t);
    }
    out
}

/// 正規化到 0.95 峰值
pub fn normalize(mut x: Vec<f32>) -> Vec<f32> {
    let peak = x.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    if peak > 0.0 {
        let g = 0.95 / peak;
        for s in x.iter_mut() {
            *s *= g;
        }
    }
    x
}

/// 列出所有輸出裝置名稱（給 doctor 用）
pub fn list_outputs() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}
