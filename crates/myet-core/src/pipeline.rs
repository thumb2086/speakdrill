//! 練習流程：把「等句子 → 播 TTS → 收分數」串起來。

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::audio::{load_wav, normalize, resample, AudioPlayer};
use crate::cdp::Cdp;
use crate::config::{audio_cache, Config};
use crate::lesson::{fetch_lines, scoring_query, scoring_url, Line};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineScore {
    pub index: usize,
    pub text: String,
    pub total: i32,
    pub pronunciation: i32,
    pub pitch: i32,
    pub fluency: i32,
    pub stress: i32,
    pub attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonReport {
    pub lesson_id: String,
    pub total_lines: usize,
    pub title: String,
    pub lines: Vec<LineScore>,
    pub avg: f64,
    pub elapsed_secs: u64,
}

/// 進度回呼（給 TUI / GUI 顯示）
pub trait Progress: Send {
    fn log(&mut self, msg: &str);
    fn line_done(&mut self, s: &LineScore);
    fn should_stop(&self) -> bool {
        false
    }
}

pub struct SilentProgress;
impl Progress for SilentProgress {
    fn log(&mut self, msg: &str) {
        println!("{msg}");
    }
    fn line_done(&mut self, s: &LineScore) {
        println!(
            "  #{} {} 分（發音 {} 語調 {} 流利 {} 重音 {}）",
            s.index, s.total, s.pronunciation, s.pitch, s.fluency, s.stress
        );
    }
}

fn wav_path(lesson: &str, idx: usize) -> PathBuf {
    audio_cache(lesson).join(format!("{idx:03}.wav"))
}

/// 確保每句都有 TTS wav（缺的才合成）
pub fn ensure_tts(cfg: &Config, lesson: &str, lines: &[Line], p: &mut dyn Progress) -> Result<()> {
    let mut need = Vec::new();
    for l in lines {
        if !wav_path(lesson, l.index).exists() {
            need.push(l);
        }
    }
    if need.is_empty() {
        p.log(&format!("TTS 快取完整（{} 句）", lines.len()));
        return Ok(());
    }
    p.log(&format!("合成 TTS：{} / {} 句…", need.len(), lines.len()));
    for l in need {
        let out = wav_path(lesson, l.index);
        cfg.tts
            .synthesize(&l.text, &out)
            .map_err(|e| anyhow!("句子 #{} 合成失敗：{e}", l.index))?;
    }
    Ok(())
}

/// 讀畫面上的五個分數。
/// 注意：`0` 是**合法分數**（代表沒收到音），不能當成「還沒更新」而丟棄，
/// 否則分數表永遠是空的、迴圈會空轉。這裡只跳過空白與 `-`。
async fn read_scores(cdp: &Cdp) -> Option<[i32; 5]> {
    let js = r#"(() => {
        const g = (id) => { const e = document.querySelector('#'+id); return e ? e.textContent.trim() : ''; };
        return JSON.stringify([g('total_score'), g('pr_score'), g('pi_score'), g('ti_score'), g('in_score')]);
    })()"#;
    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let Ok(s) = cdp.eval_string(js).await else { continue };
        let Ok(v) = serde_json::from_str::<Vec<String>>(&s) else { continue };
        if v.len() < 5 {
            continue;
        }
        if v[0].is_empty() || v[0] == "-" {
            continue;
        }
        let mut out = [0i32; 5];
        for i in 0..5 {
            out[i] = v[i].parse().unwrap_or(0);
        }
        return Some(out);
    }
    None
}

/// 課程是否已經結束（成績單出現）
async fn lesson_finished(cdp: &Cdp) -> bool {
    let js = r#"(() => {
        const body = document.body ? document.body.innerText : '';
        const report = document.querySelector('#ScoreReportZone');
        const visible = report && report.style.display !== 'none' && report.offsetParent !== null;
        return (body.includes('成績單') || body.includes('查看成績') || body.includes('本課完成') || visible) ? '1' : '';
    })()"#;
    cdp.eval_string(js).await.map(|s| s == "1").unwrap_or(false)
}

/// 處理彈窗：回傳做了什麼
async fn handle_dialog(cdp: &Cdp, resume: bool, allow_retry: bool) -> Option<String> {
    let js = format!(
        r#"(() => {{
            const byText = (t) => Array.from(document.querySelectorAll('a,button')).find(e => (e.innerText||'').trim() === t);
            const body = document.body ? document.body.innerText : '';
            if (body.includes('繼續進行未完成的句子')) {{
                const b = byText({resume});
                if (b) {{ b.click(); return 'start'; }}
            }}
            if ({allow_retry}) {{
                const r = byText('再試一次');
                if (r && r.offsetParent !== null) {{ r.click(); return 'retry'; }}
            }}
            const s = byText('跳過這一句');
            if (s && s.offsetParent !== null) {{ s.click(); return 'skip'; }}
            return '';
        }})()"#,
        resume = if resume { "'是'" } else { "'否'" },
        allow_retry = if allow_retry { "true" } else { "false" }
    );
    let r = cdp.eval_string(&js).await.ok()?;
    if r.is_empty() { None } else { Some(r) }
}

/// 抓課程標題（頁面第一行通常就是）
async fn page_title(cdp: &Cdp) -> String {
    cdp.eval_string("document.title + '|' + ((document.querySelector('.course-title')||{}).innerText||'')")
        .await
        .unwrap_or_default()
        .split('|')
        .nth(1)
        .unwrap_or("")
        .trim()
        .to_string()
}

pub struct RunOptions {
    pub resume: bool,
    pub timeout: Duration,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self { resume: false, timeout: Duration::from_secs(40 * 60) }
    }
}

/// 跑一課
pub async fn run_lesson(
    cdp: &Cdp,
    cfg: &Config,
    player: &AudioPlayer,
    lesson_id: &str,
    course_id: &str,
    opts: &RunOptions,
    p: &mut dyn Progress,
) -> Result<LessonReport> {
    let started = Instant::now();
    let query = scoring_query(lesson_id, &cfg.code, &cfg.content_type, cfg.mode, course_id);
    let lines = fetch_lines(cdp, &query).await?;
    p.log(&format!("課程 {lesson_id}：{} 句", lines.len()));

    ensure_tts(cfg, lesson_id, &lines, p)?;

    // 進練習頁
    let url = scoring_url(&cfg.server, &query);
    p.log("進入練習頁…");
    cdp.navigate(&url).await?;
    tokio::time::sleep(Duration::from_secs(10)).await;

    for _ in 0..6 {
        if handle_dialog(cdp, opts.resume, true).await.is_some() {
            tokio::time::sleep(Duration::from_millis(1500)).await;
        } else {
            break;
        }
    }

    let title = page_title(cdp).await;

    let mut cur_idx: Option<usize> = None;
    let mut attempts: std::collections::HashMap<usize, u32> = Default::default();
    let mut done: std::collections::HashMap<usize, LineScore> = Default::default();
    let mut pending: Option<(usize, Instant)> = None;
    let mut last_event = Instant::now();
    let mut tick: u32 = 0;

    while started.elapsed() < opts.timeout {
        if p.should_stop() {
            p.log("使用者中止");
            break;
        }

        // 收 console 事件（非阻塞）
        loop {
            let ev = {
                let mut rx = cdp.console_rx.lock().await;
                match rx.try_recv() {
                    Ok(e) => Some(e),
                    Err(_) => None,
                }
            };
            let Some(e) = ev else { break };
            let t = &e.text;
            last_event = Instant::now();

            if let Some(cap) = regex::Regex::new(r#"RENEW"\s+SentenceIdxArray="(\d+)""#)
                .unwrap()
                .captures(t)
            {
                cur_idx = cap[1].parse().ok();
                continue;
            }

            if t.contains("STARTREC") {
                if let Some(idx) = cur_idx {
                    let wav = wav_path(lesson_id, idx);
                    if wav.exists() {
                        let (mono, sr) = load_wav(&wav)?;
                        let scaled = resample(&mono, sr, player.sample_rate);
                        player.play(normalize(scaled))?;
                        p.log(&format!("▶ #{idx}"));
                    } else {
                        p.log(&format!("⚠ 缺少 {}", wav.display()));
                    }
                }
                continue;
            }

            if t.contains("GETSCORE") {
                pending = Some((cur_idx.unwrap_or(0), Instant::now()));
            }
        }

        // GETSCORE 後 ~900ms 讀 DOM（讀不到也要記一筆，否則會無限重試）
        if let Some((idx, at)) = pending {
            if at.elapsed() >= Duration::from_millis(900) {
                pending = None;
                let s = read_scores(cdp).await.unwrap_or([0, 0, 0, 0, 0]);
                let attempts_n = attempts.entry(idx).or_insert(0);
                *attempts_n += 1;
                let text = lines
                    .iter()
                    .find(|l| l.index == idx)
                    .map(|l| l.text.clone())
                    .unwrap_or_default();
                let ls = LineScore {
                    index: idx,
                    text,
                    total: s[0],
                    pronunciation: s[1],
                    pitch: s[2],
                    fluency: s[3],
                    stress: s[4],
                    attempts: *attempts_n,
                };
                let ok = ls.total >= cfg.pass_score as i32;
                if ok {
                    done.insert(idx, ls.clone());
                    p.line_done(&ls);
                } else {
                    p.log(&format!("✗ #{idx} {} 分（重試 {}）", ls.total, attempts_n));
                }
            }
        }

        if done.len() >= lines.len() {
            p.log("全部句子完成");
            break;
        }

        // 成績單出現 → 結束
        tick = tick.wrapping_add(1);
        if tick % 5 == 0 && lesson_finished(cdp).await {
            // 最後一句常常還沒更新 DOM 就被成績單蓋掉，補讀一次
            if let Some(i) = cur_idx {
                if !done.contains_key(&i) {
                    tokio::time::sleep(Duration::from_millis(1200)).await;
                    if let Some(s) = read_scores(cdp).await {
                        if s[0] > 0 {
                            let text = lines
                                .iter()
                                .find(|l| l.index == i)
                                .map(|l| l.text.clone())
                                .unwrap_or_default();
                            let ls = LineScore {
                                index: i,
                                text,
                                total: s[0],
                                pronunciation: s[1],
                                pitch: s[2],
                                fluency: s[3],
                                stress: s[4],
                                attempts: attempts.get(&i).copied().unwrap_or(1),
                            };
                            p.line_done(&ls);
                            done.insert(i, ls);
                        }
                    }
                }
            }
            p.log("已出現成績單，結束");
            break;
        }

        // 閒置太久（超過 50 秒沒有任何事件）→ 視為卡住
        if last_event.elapsed() > Duration::from_secs(50) {
            p.log("⚠ 閒置過久，結束本課");
            break;
        }

        let allow_retry = cur_idx
            .map(|i| attempts.get(&i).copied().unwrap_or(0) < cfg.max_retry)
            .unwrap_or(true);
        if let Some(d) = handle_dialog(cdp, opts.resume, allow_retry).await {
            p.log(&format!("對話框：{d}"));
            // 跳過這一句 → 記 0 分，否則這一句永遠不會被算進完成數
            if d == "skip" {
                if let Some(i) = cur_idx {
                    if !done.contains_key(&i) {
                        let text = lines
                            .iter()
                            .find(|l| l.index == i)
                            .map(|l| l.text.clone())
                            .unwrap_or_default();
                        done.insert(
                            i,
                            LineScore {
                                index: i,
                                text,
                                total: 0,
                                pronunciation: 0,
                                pitch: 0,
                                fluency: 0,
                                stress: 0,
                                attempts: attempts.get(&i).copied().unwrap_or(0),
                            },
                        );
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }

        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    let mut scores: Vec<LineScore> = done.into_values().collect();
    scores.sort_by_key(|s| s.index);
    let avg = if scores.is_empty() {
        0.0
    } else {
        scores.iter().map(|s| s.total as f64).sum::<f64>() / scores.len() as f64
    };

    Ok(LessonReport {
        lesson_id: lesson_id.to_string(),
        total_lines: lines.len(),
        title,
        lines: scores,
        avg: (avg * 10.0).round() / 10.0,
        elapsed_secs: started.elapsed().as_secs(),
    })
}

/// 寫報告
pub fn save_report(r: &LessonReport) -> Result<PathBuf> {
    let dir = crate::config::reports_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", r.lesson_id));
    std::fs::write(&path, serde_json::to_string_pretty(r)?)?;
    Ok(path)
}

/// 確保登入（導到 Home，失敗就提示）
pub async fn ensure_logged_in(cdp: &Cdp, server: &str) -> Result<()> {
    cdp.navigate(&format!("{server}/AudreyWeb/Home.aspx")).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let acc = cdp
        .eval_string("(typeof My !== 'undefined' && My.AccID) ? String(My.AccID) : '0'")
        .await
        .unwrap_or_else(|_| "0".into());
    if acc == "0" || acc.is_empty() {
        return Err(anyhow!("MyET 尚未登入，請先在手動登入後再跑"));
    }
    Ok(())
}

pub fn _unused_path_marker(_p: &Path) {}

