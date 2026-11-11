#!/usr/bin/env node
/**
 * GUI 啟動器：把 `speakdrill-gui` 轉給原生 egui 視窗。
 */
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const exe = process.platform === 'win32' ? 'speakdrill-gui.exe' : 'speakdrill-gui';
const dir = `${process.platform}-${process.arch}`;
const candidates = [
  path.join(__dirname, 'binaries', dir, exe),
  path.join(__dirname, '..', '..', 'target', 'release', exe),
  path.join(__dirname, '..', '..', 'target', 'debug', exe),
];

const bin = candidates.find((p) => fs.existsSync(p));

if (!bin) {
  console.error(
    `[speakdrill] 找不到 GUI 執行檔（${dir}）。\n` +
      `      請重新安裝：npm rebuild speakdrill\n` +
      `      或從 https://github.com/thumb2086/speakdrill/releases 下載`
  );
  process.exit(1);
}

const child = spawn(bin, process.argv.slice(2), { stdio: 'inherit', detached: false });
child.on('exit', (code) => process.exit(code ?? 0));
