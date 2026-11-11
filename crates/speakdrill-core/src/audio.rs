//! 音訊播放：用 cpal 指定裝置、零延遲餵樣本。
//!
//! 為什麼要自己寫：MyET 的錄音視窗只有 ~2.7 秒，
//! 每次啟動外部播放器（Python/ffplay）都會吃掉 0.5~1 秒。
//! 這裡把 stream 開好常駐，播放只是把樣本塞進環形緩衝。
//!
//! 三個實作要點（都踩過）：
//!   1. cpal 的 `Stream` 是 `!Send` → 裝置列舉與建立都要在音訊執行緒內完成
//!   2. 虛擬裝置（VB-CABLE）常拒絕硬塞的格式 → 先問 `supported_output_configs()`
//!   3. 舊式虛擬驅動只吃 16-bit 整數 → 需要 f32 / i16 / u16 三種寫法

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

type Buf = Arc<Mutex<VecDeque<f32>>>;
type Flag = Arc<Mutex<bool>>;

/// 找輸出裝置：名稱不分大小寫比對；同名的取「聲道數最少」那顆
/// （VB-CABLE 會同時掛 16ch 與 2ch 兩個端點，16ch 那顆常常開不起來）
fn find_output_device(host: &cpal::Host, needle: &str) -> Option<cpal::Device> {
    let needle = needle.to_lowercase();
    let mut best: Option<(u16, cpal::Device)> = None;
    for d in host.output_devices().ok()? {
        let name = d.name().unwrap_or_default().to_lowercase();
        if !name.contains(&needle) {
            continue;
        }
        let ch = d
            .default_output_config()
            .map(|c| c.channels())
            .unwrap_or(u16::MAX);
        match &best {
            Some((bch, _)) if *bch <= ch => {}
            _ => best = Some((ch, d)),
        }
    }
    best.map(|(_, d)| d)
}

fn stream_cfg(rate: u32, channels: u16) -> cpal::StreamConfig {
    cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(rate),
        buffer_size: cpal::BufferSize::Default,
    }
}

fn build_f32(dev: &cpal::Device, rate: u32, ch: u16, buf: Buf, playing: Flag) -> Result<cpal::Stream> {
    let n = ch as usize;
    let s = dev
        .build_output_stream(
            &stream_cfg(rate, ch),
            move |data: &mut [f32], _| {
                let mut b = buf.lock().unwrap();
                let mut any = false;
                for frame in data.chunks_mut(n) {
                    let v = b.pop_front().unwrap_or(0.0);
                    if v != 0.0 {
                        any = true;
                    }
                    for o in frame.iter_mut() {
                        *o = v;
                    }
                }
                let mut p = playing.lock().unwrap();
                if any {
                    *p = true;
                } else if b.is_empty() {
                    *p = false;
                }
            },
            |e| eprintln!("[audio] {e}"),
            None,
        )
        .map_err(|e| anyhow!("{e}"))?;
    s.play().map_err(|e| anyhow!("{e}"))?;
    Ok(s)
}

fn build_i16(dev: &cpal::Device, rate: u32, ch: u16, buf: Buf, playing: Flag) -> Result<cpal::Stream> {
    let n = ch as usize;
    let s = dev
        .build_output_stream(
            &stream_cfg(rate, ch),
            move |data: &mut [i16], _| {
                let mut b = buf.lock().unwrap();
                let mut any = false;
                for frame in data.chunks_mut(n) {
                    let v = b.pop_front().unwrap_or(0.0);
                    if v != 0.0 {
                        any = true;
                    }
                    let x = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
                    for o in frame.iter_mut() {
                        *o = x;
                    }
                }
                let mut p = playing.lock().unwrap();
                if any {
                    *p = true;
                } else if b.is_empty() {
                    *p = false;
                }
            },
            |e| eprintln!("[audio] {e}"),
            None,
        )
        .map_err(|e| anyhow!("{e}"))?;
    s.play().map_err(|e| anyhow!("{e}"))?;
    Ok(s)
}

fn build_u16(dev: &cpal::Device, rate: u32, ch: u16, buf: Buf, playing: Flag) -> Result<cpal::Stream> {
    let n = ch as usize;
    let s = dev
        .build_output_stream(
            &stream_cfg(rate, ch),
            move |data: &mut [u16], _| {
                let mut b = buf.lock().unwrap();
                let mut any = false;
                for frame in data.chunks_mut(n) {
                    let v = b.pop_front().unwrap_or(0.0);
                    if v != 0.0 {
                        any = true;
                    }
                    let x = (((v.clamp(-1.0, 1.0) * 0.5) + 0.5) * 65535.0) as u16;
                    for o in frame.iter_mut() {
                        *o = x;
                    }
                }
                let mut p = playing.lock().unwrap();
                if any {
                    *p = true;
                } else if b.is_empty() {
                    *p = false;
                }
            },
            |e| eprintln!("[audio] {e}"),
            None,
        )
        .map_err(|e| anyhow!("{e}"))?;
    s.play().map_err(|e| anyhow!("{e}"))?;
    Ok(s)
}

fn try_build(
    dev: &cpal::Device,
    fmt: cpal::SampleFormat,
    rate: u32,
    ch: u16,
    buf: &Buf,
    playing: &Flag,
) -> Result<cpal::Stream> {
    match fmt {
        cpal::SampleFormat::F32 => build_f32(dev, rate, ch, buf.clone(), playing.clone()),
        cpal::SampleFormat::I16 => build_i16(dev, rate, ch, buf.clone(), playing.clone()),
        cpal::SampleFormat::U16 => build_u16(dev, rate, ch, buf.clone(), playing.clone()),
        other => Err(anyhow!("不支援的樣本格式 {other:?}")),
    }
}

/// 依裝置真正支援的格式開 stream。回傳 (stream, 實際取樣率, 錯誤訊息)
fn open_stream(dev: &cpal::Device, buf: &Buf, playing: &Flag) -> (Option<(cpal::Stream, u32)>, String) {
    let mut last = String::new();

    if let Ok(ranges) = dev.supported_output_configs() {
        for r in ranges {
            let lo = r.min_sample_rate().0;
            let hi = r.max_sample_rate().0;
            let rate = 48000.clamp(lo, hi);
            let ch = r.channels();
            let fmt = r.sample_format();
            match try_build(dev, fmt, rate, ch, buf, playing) {
                Ok(s) => return (Some((s, rate)), last),
                Err(e) => last = format!("{fmt:?} {rate}Hz/{ch}ch: {e}"),
            }
        }
    }

    // 後備：硬塞常見組合
    for fmt in [cpal::SampleFormat::F32, cpal::SampleFormat::I16] {
        for (rate, ch) in [(48000u32, 2u16), (44100, 2), (48000, 1), (44100, 1)] {
            match try_build(dev, fmt, rate, ch, buf, playing) {
                Ok(s) => return (Some((s, rate)), last),
                Err(e) => last = format!("{fmt:?} {rate}Hz/{ch}ch: {e}"),
            }
        }
    }

    (None, last)
}

impl AudioPlayer {
    /// 依名稱片段找輸出裝置。`needle` 例如 "Voicemeeter Input" 或 "CABLE Input"。
    pub fn new(needle: &str) -> Result<Self> {
        let (info_tx, info_rx) = mpsc::channel::<Result<(String, u32)>>();
        let (tx, rx) = mpsc::channel::<Cmd>();
        let playing: Flag = Arc::new(Mutex::new(false));

        let needle = needle.to_string();
        let buf: Buf = Arc::new(Mutex::new(VecDeque::new()));
        let buf2 = buf.clone();
        let playing2 = playing.clone();
        let playing3 = playing.clone();

        let handle = std::thread::Builder::new()
            .name("speakdrill-audio".into())
            .spawn(move || {
                let host = cpal::default_host();
                let Some(device) = find_output_device(&host, &needle) else {
                    let _ = info_tx.send(Err(anyhow!(
                        "找不到輸出裝置「{needle}」。請確認虛擬音效裝置已安裝並啟用。"
                    )));
                    return;
                };
                let name = device.name().unwrap_or_default();

                let (opened, last_err) = open_stream(&device, &buf2, &playing3);
                let Some((stream, rate)) = opened else {
                    let _ = info_tx.send(Err(anyhow!(
                        "裝置「{name}」開不起來（{last_err}）。\n\
                         若剛安裝 VB-CABLE，請重開機後再試。"
                    )));
                    return;
                };

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

/// 線性插值重取樣
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
