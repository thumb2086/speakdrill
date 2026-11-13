//! 課程資料：目錄清單、單課句子。

use anyhow::{anyhow, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::cdp::Cdp;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonRef {
    pub lesson_id: String,
    pub course_id: String,
    /// 課程名稱，抓不到就等於 lesson_id
    #[serde(default)]
    pub title: String,
    /// 已有成績（代表練過）→ 最佳分數
    #[serde(default)]
    pub best_score: Option<f64>,
}

impl LessonRef {
    pub fn done(&self) -> bool {
        self.best_score.is_some()
    }

    /// 給 UI 顯示用：名稱優先，沒有就退回代碼；已練過加 ✓ 與分數
    pub fn label(&self) -> String {
        let mut s = if self.title.is_empty() || self.title == self.lesson_id {
            self.lesson_id.clone()
        } else {
            format!("{}  ({})", self.title, self.lesson_id)
        };
        match self.best_score {
            Some(sc) => s.push_str(&format!("  ✓ {sc:.1}")),
            None => s.push_str("  · 未練"),
        }
        s
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Line {
    pub index: usize,
    pub text: String,
}

/// 取課程清單（含名稱與是否已練過）。
///
/// 效能重點：
///   - **不導覽頁面**，直接在現有頁面內 `fetch()`，避免整頁重新載入
///   - **在 JS 裡用 DOMParser 解析**，只回傳必要欄位（~3 KB，而不是 200 KB HTML）
///   - 有磁碟快取（預設 10 分鐘），重開 TUI/GUI 幾乎瞬間
///
/// 主要來源 `CategoryLessonScores.aspx`（一頁同時有課名、LessonID、CourseID、最佳分數），
/// 抓不到才退回 `CatalogPage.aspx`（只有名稱）。
pub async fn fetch_catalog(cdp: &Cdp, server: &str, code: &str, content_type: &str) -> Result<Vec<LessonRef>> {
    fetch_catalog_opts(cdp, server, code, content_type, false).await
}

pub async fn fetch_catalog_opts(
    cdp: &Cdp,
    server: &str,
    code: &str,
    content_type: &str,
    refresh: bool,
) -> Result<Vec<LessonRef>> {
    let cache = crate::config::cache_dir().join(format!("catalog_{code}_{content_type}.json"));

    if !refresh {
        if let Ok(s) = std::fs::read_to_string(&cache) {
            if let Ok(c) = serde_json::from_str::<CatalogCache>(&s) {
                let age = c.fetched_at.elapsed_secs();
                if age < 600 && !c.lessons.is_empty() {
                    return Ok(c.lessons);
                }
            }
        }
    }

    let lessons = fetch_catalog_live(cdp, server, code, content_type).await?;

    if let Ok(s) = serde_json::to_string(&CatalogCache {
        fetched_at: CacheStamp::now(),
        lessons: lessons.clone(),
    }) {
        let _ = std::fs::create_dir_all(cache.parent().unwrap_or(std::path::Path::new(".")));
        let _ = std::fs::write(&cache, s);
    }

    Ok(lessons)
}

/// 取得所有課程（跨所有已知 CodeName）
pub async fn fetch_all_catalogs(
    cdp: &Cdp,
    server: &str,
    content_type: &str,
    refresh: bool,
) -> Result<Vec<LessonRef>> {
    use crate::config::KNOWN_CODENAMES;
    let mut all_lessons = Vec::new();
    
    for (code, name) in KNOWN_CODENAMES {
        match fetch_catalog_opts(cdp, server, code, content_type, refresh).await {
            Ok(mut lessons) => {
                if !lessons.is_empty() {
                    // 為每個課程加上 CodeName 標記
                    for lesson in &mut lessons {
                        lesson.title = format!("{}  [{}]", lesson.title, name);
                    }
                    all_lessons.extend(lessons);
                }
            }
            Err(_) => {
                // 忽略無法存取的 CodeName（使用者可能無權限）
            }
        }
    }
    
    Ok(all_lessons)
}

#[derive(Serialize, Deserialize)]
struct CatalogCache {
    fetched_at: CacheStamp,
    lessons: Vec<LessonRef>,
}

#[derive(Serialize, Deserialize)]
struct CacheStamp {
    unix: u64,
}

impl CacheStamp {
    fn now() -> Self {
        Self {
            unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }
    fn elapsed_secs(&self) -> u64 {
        Self::now().unix.saturating_sub(self.unix)
    }
}

/// 單次 JS 呼叫完成「抓頁面 + 解析 + 回傳精簡 JSON」
async fn fetch_catalog_live(cdp: &Cdp, _server: &str, code: &str, content_type: &str) -> Result<Vec<LessonRef>> {
    let js = format!(
        r#"(async () => {{
  const parse = (html, withScore) => {{
    const doc = new DOMParser().parseFromString(html, 'text/html');
    const out = [];
    if (withScore) {{
      for (const tr of doc.querySelectorAll('.data-tr')) {{
        const a = tr.querySelector('[data-lessonid]');
        if (!a) continue;
        const t = (tr.querySelector('.lesson-title-td')?.textContent || '').replace(/\s+/g, ' ').trim();
        const sEl = tr.querySelector('.score');
        const s = sEl ? parseFloat(sEl.textContent) : NaN;
        out.push({{ id: a.getAttribute('data-lessonid'), cid: a.getAttribute('data-courseid'), title: t, score: Number.isFinite(s) ? s : null }});
      }}
    }}
    if (!out.length) {{
      const seen = new Set();
      for (const a of doc.querySelectorAll('a[href*="GoToLesson"]')) {{
        const m = /GoToLesson\('([^']+)','([^']+)'/.exec(a.getAttribute('href') || '');
        if (!m || seen.has(m[1])) continue;
        seen.add(m[1]);
        const t = (a.querySelector('.L2')?.textContent || '').replace(/\s+/g, ' ').trim();
        out.push({{ id: m[1], cid: m[2], title: t, score: null }});
      }}
    }}
    return out;
  }};
  const get = async (u) => {{ try {{ const r = await fetch(u, {{credentials:'include'}}); return r.ok ? await r.text() : ''; }} catch(e) {{ return ''; }} }};
  let rows = parse(await get('CategoryLessonScores.aspx?CodeName={code}&ContentType={content_type}'), true);
  if (!rows.length) rows = parse(await get('CatalogPage.aspx?CodeName={code}&ContentType={content_type}'), false);
  return JSON.stringify(rows);
}})()"#
    );

    // Retry logic for "Inspected target navigated or closed" errors
    let mut last_err = None;
    for _attempt in 1..=3 {
        match cdp.eval_string(&js).await {
            Ok(json) => {
                let rows: Vec<RawRow> = serde_json::from_str(&json).unwrap_or_default();
                if rows.is_empty() {
                    last_err = Some(anyhow!("抓不到課程，可能沒登入或 CodeName 不對（試試 --refresh 或先登入）"));
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    continue;
                }
                return Ok(rows
                    .into_iter()
                    .map(|r| LessonRef {
                        lesson_id: r.id,
                        course_id: r.cid,
                        title: unescape(&r.title),
                        best_score: r.score,
                    })
                    .collect());
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("Inspected target navigated or closed") || err_str.contains("target closed") {
                    last_err = Some(e);
                    // Wait a bit before retry
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    continue;
                }
                last_err = Some(e);
                break;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("抓取課程失敗，已重試 3 次")))
}

#[derive(serde::Deserialize)]
struct RawRow {
    id: String,
    cid: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    score: Option<f64>,
} 

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// 取單一課程的句子（在頁面內 fetch，自動帶登入 cookie）。
pub async fn fetch_lines(cdp: &Cdp, scoring_query: &str) -> Result<Vec<Line>> {
    let q = serde_json::to_string(scoring_query)?;
    let js = format!(
        r#"(async () => {{
            const r = await fetch('LessonMainPageServices.aspx?op=GetLessonInfo&' + {q}, {{credentials:'include'}});
            return await r.text();
        }})()"#
    );
    let xml = cdp.eval_string(&js).await?;

    let re = Regex::new(r"(?s)<Line\b([^>]*)>(.*?)</Line>").unwrap();
    let en_re = Regex::new(r"(?s)<English>(.*?)</English>").unwrap();
    let idx_re = Regex::new(r#"IndexNo="(\d+)""#).unwrap();

    let mut lines = Vec::new();
    for c in re.captures_iter(&xml) {
        let attrs = &c[1];
        let body = &c[2];
        let Some(idx) = idx_re.captures(attrs) else { continue };
        let Some(en) = en_re.captures(body) else { continue };
        lines.push(Line {
            index: idx[1].parse().unwrap_or(0),
            text: unescape(en[1].trim()),
        });
    }
    if lines.is_empty() {
        return Err(anyhow!("課程 XML 解析不到句子（LessonID 錯或沒登入）"));
    }
    lines.sort_by_key(|l| l.index);
    Ok(lines)
}

/// 組出 ScoringPage 的 query string
pub fn scoring_query(lesson: &str, code: &str, content_type: &str, mode: u32, course: &str) -> String {
    format!(
        "PracticeMode={mode}&LessonID={lesson}&CodeName={code}&ContentType={content_type}\
         &From=MENU&TurnPage=AUTO&CourseID={course}"
    )
}

pub fn scoring_url(server: &str, query: &str) -> String {
    format!("{server}/AudreyWeb/ScoringPage.aspx?{query}")
}
