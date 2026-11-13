#!/usr/bin/env node
/**
 * speakdrill 安裝腳本：從 GitHub Releases 下載預編譯執行檔。
 *
 * 停用下載：
 *   SPEAKDRILL_SKIP_DOWNLOAD=1   跳過下載（可改讀 target/ 的本機 build）
 *   SPEAKDRILL_VERSION           覆寫版本（預設讀 package.json）
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
  console.log('[speakdrill] SPEAKDRILL_SKIP_DOWNLOAD=1；跳過下載');
  process.exit(0);
}

if (process.platform !== 'win32') {
  console.log(`[speakdrill] 目前僅支援 Windows（VB-CABLE / VoiceMeeter）；本平台為 ${process.platform}，跳過安裝`);
  process.exit(0);
}

const exes = ['speakdrill.exe', 'speakdrill-gui.exe'];
const dir = `${process.platform}-${process.arch}`;
const outDir = path.join(__dirname, '..', 'bin', 'binaries', dir);

const tag = `v${VERSION}`;
const asset = `speakdrill-${tag}-${dir}.zip`;
const url = `https://github.com/${REPO}/releases/download/${tag}/${asset}`;

function download(u, dest) {
  return new Promise((resolve, reject) => {
    const go = (u2, depth = 0) => {
      if (depth > 5) return reject(new Error('too many redirects'));
      https
        .get(u2, { headers: { 'User-Agent': 'speakdrill-installer' } }, (res) => {
          if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
            res.resume();
            return go(res.headers.location, depth + 1);
          }
          if (res.statusCode !== 200) {
            res.resume();
            return reject(new Error(`HTTP ${res.statusCode} ${u2}`));
          }
          const f = fs.createWriteStream(dest);
          res.pipe(f);
          f.on('finish', () => f.close(() => resolve()));
        })
        .on('error', reject);
    };
    go(u);
  });
}

function ensure(outDir) {
  for (const exe of exes) {
    const p = path.join(outDir, exe);
    if (!fs.existsSync(p)) {
      throw new Error(`安裝後找不到 ${exe}（解壓或下載失敗）`);
    }
  }
}

function rmrf(p) {
  if (fs.existsSync(p)) fs.rmSync(p, { recursive: true, force: true });
}

(async () => {
  // 每次安裝都重新下載並解壓，避免沿用舊 exe 造成版本錯位
  rmrf(outDir);
  fs.mkdirSync(outDir, { recursive: true });
  const archive = path.join(outDir, asset);

  console.log(`[speakdrill] 下載 ${url}`);
  try {
    await download(url, archive);
    execFileSync(
      'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe',
      ['-NoProfile', '-Command',
        `Expand-Archive -LiteralPath '${archive}' -DestinationPath '${outDir}' -Force`],
      { stdio: 'inherit' }
    );
    fs.unlinkSync(archive);
    ensure(outDir);
    console.log(`[speakdrill] 安裝完成：${outDir}`);
  } catch (e) {
    console.error(`[speakdrill] 下載/解壓失敗：${e.message}`);
    console.error('[speakdrill] 可改用 SPEAKDRILL_SKIP_DOWNLOAD=1 略過下載');
    process.exit(1);
  }
})();