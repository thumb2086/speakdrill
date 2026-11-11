#!/usr/bin/env node
/**
 * 安裝時下載對應平台的原生二進位。
 *
 * 環境變數：
 *   MYET_SKIP_DOWNLOAD=1   跳過下載（本機開發用，會直接用 target/ 的執行檔）
 *   MYET_VERSION           指定版本（預設讀 package.json）
 */
const fs = require('fs');
const path = require('path');
const https = require('https');
const { execFileSync } = require('child_process');

const pkg = require('../package.json');
const VERSION = process.env.MYET_VERSION || pkg.version;
const REPO = process.env.MYET_REPO || 'yourname/myet-practice';

if (process.env.MYET_SKIP_DOWNLOAD === '1') {
  console.log('[myet] MYET_SKIP_DOWNLOAD=1，跳過下載');
  process.exit(0);
}

const isWin = process.platform === 'win32';
const exe = isWin ? 'myet.exe' : 'myet';
const guiExe = isWin ? 'myet-gui.exe' : 'myet-gui';
const dir = `${process.platform}-${process.arch}`;
const outDir = path.join(__dirname, '..', 'bin', 'binaries', dir);

const ext = isWin ? 'zip' : 'tar.gz';
const asset = `myet-${VERSION}-${dir}.${ext}`;
const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${asset}`;

function download(u, dest) {
  return new Promise((resolve, reject) => {
    const go = (u2, depth = 0) => {
      if (depth > 5) return reject(new Error('太多重新導向'));
      https
        .get(u2, { headers: { 'User-Agent': 'myet-practice-installer' } }, (res) => {
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

(async () => {
  fs.mkdirSync(outDir, { recursive: true });
  const archive = path.join(outDir, asset);

  const existing = path.join(outDir, exe);
  if (fs.existsSync(existing)) {
    console.log(`[myet] 已存在 ${existing}`);
    return;
  }

  console.log(`[myet] 下載 ${url}`);
  try {
    await download(url, archive);

    if (ext === 'zip') {
      execFileSync('powershell', [
        '-NoProfile',
        '-Command',
        `Expand-Archive -LiteralPath '${archive}' -DestinationPath '${outDir}' -Force`,
      ], { stdio: 'inherit' });
    } else {
      execFileSync('tar', ['-xzf', archive, '-C', outDir], { stdio: 'inherit' });
    }
    fs.unlinkSync(archive);

    for (const f of [exe, guiExe]) {
      const p = path.join(outDir, f);
      if (fs.existsSync(p) && !isWin) fs.chmodSync(p, 0o755);
    }
    console.log(`[myet] 安裝完成 → ${outDir}`);
  } catch (e) {
    console.error(`[myet] 下載失敗：${e.message}`);
    console.error('[myet] 可改用手動安裝，或設定 MYET_SKIP_DOWNLOAD=1 使用本機建置');
    process.exit(0); // 不讓 npm install 整個失敗
  }
})();
