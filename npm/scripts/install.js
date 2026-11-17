#!/usr/bin/env node
/**
 * speakdrill 安裝程式：從 GitHub Releases 下載 TUI 原生二進位（GUI 暫不提供 npm 安裝）
 *
 * 環境變數：
 *   SPEAKDRILL_SKIP_DOWNLOAD=1   跳過下載（開發 / 用 target/ 直接跑）
 *   SPEAKDRILL_VERSION           覆寫要下載的版本（預設為 package.json 的 version）
 *   SPEAKDRILL_REPO              覆寫 GitHub repo（預設 thumb2086/speakdrill）
 */
const fs = require('fs');
const path = require('path');
const https = require('https');
const { execFileSync } = require('child_process');

const pkg = require('../package.json');
const VERSION = process.env.SPEAKDRILL_VERSION || pkg.version;
const REPO = process.env.SPEAKDRILL_REPO || 'thumb2086/speakdrill';

if (process.env.SPEAKDRILL_SKIP_DOWNLOAD === '1') {
  console.log('[speakdrill] SPEAKDRILL_SKIP_DOWNLOAD=1，跳過下載');
  process.exit(0);
}

if (process.platform !== 'win32') {
  console.log(`[speakdrill] 目前僅支援 Windows（VB-CABLE / VoiceMeeter）；本平台為 ${process.platform}，跳過下載`);
  process.exit(0);
}

const exes = ['speakdrill.exe'];
const dir = process.platform === 'win32' ? 'win32-x64' : 'win32-x64';
const outDir = path.join(__dirname, '..', 'bin', 'binaries', dir);

const tag = `v${VERSION}`;
const asset = `speakdrill-${tag}-${dir}.zip`;
const url = `https://github.com/${REPO}/releases/download/${tag}/${asset}`;
const exePath = (name) => path.join(outDir, name);

function download(u, dest) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    const request = (current, redirectsLeft) => {
      if (redirectsLeft <= 0) return reject(new Error('too many redirects'));
      https
        .get(current, { headers: { 'User-Agent': 'speakdrill-installer' } }, (res) => {
          if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
            res.resume();
            return request(res.headers.location, redirectsLeft - 1);
          }
          if (res.statusCode !== 200) {
            res.resume();
            return reject(new Error(`HTTP ${res.statusCode}`));
          }
          res.pipe(file);
          file.on('finish', () => file.close(() => resolve()));
          file.on('error', (err) => { fs.unlink(dest, () => {}); reject(err); });
        })
        .on('error', reject);
    };
    request(u, 5);
  });
}

function extractZip(archive, dest) {
  const psCmd = `
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory('${archive.replace(/'/g, "''")}', '${dest.replace(/'/g, "''")}')
  `;
  execFileSync('powershell.exe', ['-NoProfile', '-Command', psCmd], { stdio: 'inherit' });
}

// 移除 Mark-of-the-Web（從網路下載的 .exe 會被 Windows 擋，導致 spawn UNKNOWN）
function unblock(target) {
  try {
    const psCmd = `Get-ChildItem -LiteralPath '${target.replace(/'/g, "''")}' -Filter *.exe | Unblock-File`;
    execFileSync('powershell.exe', ['-NoProfile', '-Command', psCmd], { stdio: 'inherit' });
    console.log('[speakdrill] 已移除 Mark-of-the-Web（.exe 可正常啟動）');
  } catch {
    console.log('[speakdrill] ⚠ 無法自動移除 MOTW（不影響安裝，只是 exe 被 Windows 擋時需手動 Unblock）');
  }
}

(async () => {
  const archive = path.join(outDir, asset);

  try {
    fs.mkdirSync(outDir, { recursive: true });
    console.log(`[speakdrill] 下載 ${url}`);
    await download(url, archive);
    extractZip(archive, outDir);
    fs.unlinkSync(archive);

    // 驗證
    for (const name of exes) {
      const p = exePath(name);
      if (!fs.existsSync(p)) throw new Error(`解壓縮後找不到 ${name}`);
      const size = fs.statSync(p).size;
      if (size < 1024 * 512) throw new Error(`${name} 大小異常（${size} bytes），下載可能損壞`);
      console.log(`[speakdrill] ${name}  ${(size / 1024 / 1024).toFixed(1)} MB ✓`);
    }

    // 移除 MOTW（關鍵：避免 spawn UNKNOWN）
    unblock(outDir);

    console.log(`[speakdrill] 安裝完成 → ${outDir}`);
  } catch (e) {
    console.error(`[speakdrill] 安裝失敗：${e.message}`);
    console.error('[speakdrill] 請改用手動安裝：');
    console.error(`  https://github.com/${REPO}/releases/download/${tag}/${asset}`);
    console.error('  下載後解壓縮，對 speakdrill.exe 執行：');
    console.error('  右鍵 → 內容 → 勾選「解除封鎖」；或 powershell 執行 Unblock-File -Path <路徑>');
    process.exit(1);
  }
})();