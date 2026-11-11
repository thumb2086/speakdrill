#!/usr/bin/env node
/**
 * npm 包裝器：把 `speakdrill ...` 轉給對應平台的原生二進位。
 *
 * 二進位由 scripts/install.js 從 GitHub Releases 下載到
 *   <package>/bin/binaries/<platform>-<arch>/speakdrill[.exe]
 */
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const exe = process.platform === 'win32' ? 'speakdrill.exe' : 'speakdrill';
const dir = `${process.platform}-${process.arch}`;
const candidates = [
  path.join(__dirname, 'binaries', dir, exe),
  path.join(__dirname, '..', '..', 'target', 'release', exe),
  path.join(__dirname, '..', '..', 'target', 'debug', exe),
];

const bin = candidates.find((p) => fs.existsSync(p));

if (!bin) {
  console.error(
    `[speakdrill] 找不到可執行檔（${dir}）。\n` +
      `      請重新安裝：npm rebuild speakdrill\n` +
      `      或從 https://github.com/thumb2086/speakdrill/releases 下載`
  );
  process.exit(1);
}

const child = spawn(bin, process.argv.slice(2), { stdio: 'inherit' });
child.on('exit', (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exit(code ?? 1);
});
