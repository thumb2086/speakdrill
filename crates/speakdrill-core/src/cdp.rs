//! 最小 CDP（Chrome DevTools Protocol）client。
//!
//! 只做我們需要的事：
//!   - 從 `http://127.0.0.1:9222/json/list` 挑一個 page target
//!   - 建 WebSocket，送出 method 請求、等回應
//!   - 把 `Runtime.consoleAPICalled` 事件轉出來
//!
//! 刻意不用 chromiumoxide：它會假設自己想開瀏覽器，而我們是接既有的 MyET。

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;

/// 一個 console 訊息（只留字串內容）
#[derive(Debug, Clone)]
pub struct ConsoleEvent {
    pub kind: String,
    pub text: String,
}

struct Pending {
    map: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
}

pub struct Cdp {
    tx: mpsc::UnboundedSender<Message>,
    pending: Arc<Pending>,
    next_id: AtomicU64,
    pub target_url: String,
    /// 由 reader task 轉出的 console 事件
    pub console_rx: Mutex<mpsc::UnboundedReceiver<ConsoleEvent>>,
    _reader: tokio::task::JoinHandle<()>,
}

fn http_get(host: &str, port: u16, path: &str) -> Result<String> {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream, ToSocketAddrs};

    // Chrome 的 devtools 可能只綁其中一個 loopback
    let hosts: Vec<&str> = if host == "127.0.0.1" || host == "localhost" {
        vec!["127.0.0.1", "::1"]
    } else {
        vec![host]
    };

    let mut tried = Vec::new();
    for h in hosts {
        let addrs: Vec<SocketAddr> = match (h, port).to_socket_addrs() {
            Ok(a) => a.collect(),
            Err(e) => {
                tried.push(format!("{h}: {e}"));
                continue;
            }
        };
        for addr in addrs {
            let mut s = match TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(1500)) {
                Ok(s) => s,
                Err(e) => {
                    tried.push(format!("{addr}: {e}"));
                    continue;
                }
            };
            let _ = s.set_read_timeout(Some(std::time::Duration::from_millis(1500)));
            let _ = s.set_write_timeout(Some(std::time::Duration::from_millis(1500)));
            if write!(
                s,
                "GET {path} HTTP/1.1\r\nHost: {h}:{port}\r\nConnection: close\r\n\r\n"
            )
            .is_err()
            {
                tried.push(format!("{addr}: write failed"));
                continue;
            }

            // 讀到連線關閉或讀取逾時為止（devtools 回完就關）
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                match s.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    Err(_) => break,
                }
            }

            let text = String::from_utf8_lossy(&buf);
            if let Some((_, body)) = text.split_once("\r\n\r\n") {
                if !body.trim().is_empty() {
                    return Ok(body.to_string());
                }
            }
            tried.push(format!("{addr}: empty response"));
        }
    }

    Err(anyhow!(
        "連不上 debug port（{}）。MyET 有加 --remote-debugging-port={port} 嗎？",
        tried.join("; ")
    ))
}

impl Cdp {
    /// 連上既有的 debug port，挑第一個 `type == "page"` 的 target。
    pub async fn connect(host: &str, port: u16) -> Result<Self> {
        let body = tokio::task::spawn_blocking({
            let host = host.to_string();
            move || http_get(&host, port, "/json/list")
        })
        .await??;

        let list: Value = serde_json::from_str(&body).context("debug port 回應不是 JSON")?;
        let arr = list.as_array().ok_or_else(|| anyhow!("debug port 回應不是陣列"))?;
        let page = arr
            .iter()
            .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
            .ok_or_else(|| anyhow!("找不到 page target，MyET 可能還沒開好"))?;

        let ws_url = page
            .get("webSocketDebuggerUrl")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("target 沒有 webSocketDebuggerUrl"))?
            .to_string();
        let target_url = page
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let (ws, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .with_context(|| format!("WebSocket 連線失敗：{ws_url}"))?;

        let (mut write, mut read) = ws.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
        tokio::spawn(async move {
            while let Some(m) = out_rx.recv().await {
                if write.send(m).await.is_err() {
                    break;
                }
            }
        });

        let pending = Arc::new(Pending { map: Mutex::new(HashMap::new()) });
        let (con_tx, con_rx) = mpsc::unbounded_channel::<ConsoleEvent>();
        let pending2 = pending.clone();

        let reader = tokio::spawn(async move {
            while let Some(Ok(msg)) = read.next().await {
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                    Message::Close(_) => break,
                    _ => continue,
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };

                if let Some(id) = v.get("id").and_then(|x| x.as_u64()) {
                    let mut map = pending2.map.lock().await;
                    if let Some(tx) = map.remove(&id) {
                        let _ = tx.send(v);
                    }
                    continue;
                }

                if v.get("method").and_then(|m| m.as_str()) == Some("Runtime.consoleAPICalled") {
                    let p = &v["params"];
                    let kind = p.get("type").and_then(|t| t.as_str()).unwrap_or("log").to_string();
                    let mut s = String::new();
                    if let Some(args) = p.get("args").and_then(|a| a.as_array()) {
                        for a in args {
                            if let Some(vs) = a.get("value") {
                                match vs {
                                    Value::String(x) => s.push_str(x),
                                    other => s.push_str(&other.to_string()),
                                }
                            } else if let Some(d) = a.get("description").and_then(|d| d.as_str()) {
                                s.push_str(d);
                            }
                        }
                    }
                    let _ = con_tx.send(ConsoleEvent { kind, text: s });
                }
            }
        });

        let cdp = Self {
            tx: out_tx,
            pending,
            next_id: AtomicU64::new(1),
            target_url,
            console_rx: Mutex::new(con_rx),
            _reader: reader,
        };
        cdp.call("Runtime.enable", json!({})).await?;
        cdp.call("Page.enable", json!({})).await.ok();
        Ok(cdp)
    }

    /// 檢查是否有可用的 page target 正在運行（例如 MyET 主視窗）
    pub async fn check_myet_running(host: &str, port: u16) -> bool {
        match tokio::task::spawn_blocking({
            let host = host.to_string();
            move || http_get(&host, port, "/json/list")
        })
        .await
        {
            Ok(Ok(body)) => {
                if let Ok(list) = serde_json::from_str::<Value>(&body) {
                    if let Some(arr) = list.as_array() {
                        return arr.iter().any(|t| {
                            t.get("type").and_then(|v| v.as_str()) == Some("page")
                        });
                    }
                }
                false
            }
            _ => false,
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.map.lock().await.insert(id, tx);

        let payload = json!({ "id": id, "method": method, "params": params }).to_string();
        self.tx
            .send(Message::Text(payload.into()))
            .map_err(|_| anyhow!("WebSocket 已關閉"))?;

        let v = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| anyhow!("CDP {method} 逾時"))?
            .map_err(|_| anyhow!("CDP {method} 無回應"))?;

        if let Some(err) = v.get("error") {
            bail!("CDP {method} 失敗：{err}");
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    /// 檢查 WebSocket 連線是否仍然存活
    pub async fn is_connected(&self) -> bool {
        // 嘗試發送一個簡單的命令來檢查連線
        self.call("Runtime.evaluate", json!({
            "expression": "1+1",
            "returnByValue": true
        })).await.is_ok()
    }

    /// 重新連線到 CDP
    pub async fn reconnect(&mut self, host: &str, port: u16) -> Result<()> {
        *self = Self::connect(host, port).await?;
        Ok(())
    }

    /// 在頁面裡 eval 一段 JS，等 Promise 完成，回傳字串。
    pub async fn eval_string(&self, expr: &str) -> Result<String> {
        let r = self
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expr,
                    "awaitPromise": true,
                    "returnByValue": true,
                    "userGesture": true,
                }),
            )
            .await?;

        if let Some(ex) = r.get("exceptionDetails") {
            let msg = ex
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(|d| d.as_str())
                .unwrap_or("unknown");
            bail!("JS 例外：{msg}");
        }
        let v = &r["result"]["value"];
        Ok(match v {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        })
    }

    /// 導覽到指定網址並等載入完成（用輪詢，容忍 context 被銷毀）。
    pub async fn navigate(&self, url: &str) -> Result<()> {
        let js = format!("location.href = {}; 'ok'", serde_json::to_string(url)?);
        let _ = self.eval_string(&js).await;

        for _ in 0..80 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if let Ok(state) = self
                .eval_string("document.readyState + '|' + location.href")
                .await
            {
                if let Some((rs, href)) = state.split_once('|') {
                    if rs == "complete" && href.contains(url.split('?').next().unwrap_or(url)) {
                        return Ok(());
                    }
                }
            }
        }
        bail!("導覽逾時：{url} 超過 40 秒未完成載入")
    }

    /// 取目前頁面完整 HTML（透過 DOM，避免額外網路請求）
    pub async fn html(&self) -> Result<String> {
        self.eval_string("document.documentElement.outerHTML").await
    }

    pub async fn body_text(&self) -> Result<String> {
        self.eval_string("document.body ? document.body.innerText : ''").await
    }
}
