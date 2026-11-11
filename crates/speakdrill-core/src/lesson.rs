//! 課程資料：目錄清單、單課句子。

use anyhow::{anyhow, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::cdp::Cdp;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonRef {
    pub lesson_id: String,
    pub course_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Line {
    pub index: usize,
    pub text: String,
}

/// 從目錄頁抓出所有課程（`GoToLesson('<LessonID>','<CourseID>',…)`）。
pub async fn fetch_catalog(cdp: &Cdp, server: &str, code: &str, content_type: &str) -> Result<Vec<LessonRef>> {
    let url = format!(
        "{server}/AudreyWeb/CatalogPage.aspx?CodeName={code}&ContentType={content_type}"
    );
    cdp.navigate(&url).await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let html = cdp.html().await?;

    let re = Regex::new(r"GoToLesson\('([^']+)','([^']+)'").unwrap();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for c in re.captures_iter(&html) {
        let lesson_id = c[1].to_string();
        let course_id = c[2].to_string();
        if seen.insert(lesson_id.clone()) {
            out.push(LessonRef { lesson_id, course_id });
        }
    }
    if out.is_empty() {
        return Err(anyhow!("目錄頁抓不到課程，可能沒登入或 CodeName 不對"));
    }
    Ok(out)
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
