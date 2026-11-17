#!/usr/bin/env node
/**
 * speakdrill 終端機入口：呼叫原生 speakdrill.exe
 *
 * 原生檔由 scripts/install.js 從 GitHub Releases 下載並驗證。
 */
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const exe = process.platform === 'win32' ? 'speakdrill.exe' : 'speakdrill';
const dir = process.platform === 'win32' ? 'win32-x64' : 'linux-x64';
const candidates = [
  path.join(__dirname, 'binaries', dir, exe),
  path.join(__dirname, '..', '..', 'target', 'release', exe),
  path.join(__dirname, '..', '..', 'target', 'debug', exe),
];

const bin = candidates.find((p) => fs.existsSync(p));

if (!bin) {
  console.error(
    `[speakdrill] 找不到原生執行檔（${dir}）。\n` +
      `      重新安裝：npm i -g speakdrill@latest\n` +
      `      或從 https://github.com/thumb2086/speakdrill/releases 下載`
  );
  process.exit(1);
}

const child = spawn(bin, process.argv.slice(2), { stdio: 'inherit' });
child.on('error', (err) => {
  // Windows 上最常見：exe 被「網路下載」標記封鎖 → spawn UNKNOWN
  if (err.errno === -4094 || String(err).includes('UNKNOWN')) {
    console.error(`[speakdrill] 無法啟動 ${bin}`);
    console.error('  原因：Windows 封鎖了從網路下載的檔案（Mark-of-the-Web）');
    console.error('  解除封鎖（PowerShell）：');
    console.error(`    Unblock-File -LiteralPath "${bin}"`);
    console.error('    或重新安裝：npm i -g speakdrill@latest（install.js 會自動 Unblock）');
    process.exit(1);
  }
  console.error(`[speakdrill] 啟動失敗：${err.message}`);
  process.exit(1);
});
child.on('exit', (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exit(code ?? 1);
});