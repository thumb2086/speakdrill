#!/usr/bin/env node
import { readFileSync, writeFileSync } from "fs";
import { execSync } from "child_process";

const newVersion = process.argv[2];
if (!newVersion || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(newVersion)) {
  console.error("用法：node scripts/bump-version.mjs <版本>  （例如 0.2.11）");
  process.exit(1);
}

const cargo = readFileSync("Cargo.toml", "utf8");
const pkg = JSON.parse(readFileSync("npm/package.json", "utf8"));
const oldVersion = pkg.version;

// Cargo.toml: 更新 [workspace.package] 的 version（只取第一個完全相同的）
const wsRe = new RegExp(`^(version\\s*=\\s*")${oldVersion}(")`, "m");
const cargo2 = cargo.replace(wsRe, `$1${newVersion}$2`);
if (cargo2 === cargo) {
  console.error(`Cargo.toml 找不到 workspace version ${oldVersion}`);
  process.exit(1);
}

// npm/package.json: 直接寫入新版本
pkg.version = newVersion;

writeFileSync("Cargo.toml", cargo2);
writeFileSync("npm/package.json", JSON.stringify(pkg, null, 2) + "\n");

// 讓 Cargo.lock 與 Cargo.toml 對齊
console.log(`版本 ${oldVersion} -> ${newVersion}，正在更新 Cargo.lock…`);
execSync("cargo check --workspace", { stdio: "inherit" });

console.log("完成。請再 commit / tag / push：");
console.log(`  git add Cargo.toml Cargo.lock npm/package.json scripts/bump-version.mjs`);
console.log(`  git commit -m "chore: bump to ${newVersion}"`);
console.log(`  git tag v${newVersion}`);
console.log(`  git push origin v${newVersion}`);