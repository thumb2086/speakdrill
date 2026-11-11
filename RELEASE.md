# 發佈流程

## 一句話

**推一個 `v*` tag，CI 就會同時更新 CLI + GUI 的 GitHub Release，並發佈 npm 套件。**

```bash
git tag v0.2.0
git push origin v0.2.0
```

---

## CI 檔案

| 項目 | 值 |
|---|---|
| 檔案路徑 | `.github/workflows/release.yml` |
| **npm Trusted Publisher 要填的檔名** | **`release.yml`**（只填檔名，不要路徑） |

---

## npm Trusted Publisher 設定

在 npmjs.com → 你的套件 → **Settings** → **Trusted Publisher** → **GitHub Actions**：

| 欄位 | 值 |
|---|---|
| Organization or user | `thumb2086` |
| Repository | `speakdrill` |
| Workflow filename | `release.yml` |
| Environment name | （留空） |
| Allowed actions | **要勾 `npm publish`** |

> ⚠️ 2026-09-03 之後建立的設定，預設**只允許 `npm stage publish`**（需人工 2FA 核准）。
> 我們用的是直接 `npm publish`，所以一定要額外勾選它。

### 前置條件（不滿足會 ENEEDAUTH）

1. npm CLI **≥ 11.5.1**、Node **≥ 22.14.0** → workflow 已處理
2. workflow 要有 `permissions: id-token: write` → 已設定
3. **不要**設 `NODE_AUTH_TOKEN` → 已移除
4. `npm/package.json` 的 `repository.url` 必須和 GitHub repo 完全一致
   （目前是 `https://github.com/thumb2086/speakdrill.git`）
5. 必須用 GitHub-hosted runner（`windows-latest`）→ 已設定
6. **設定後 2 天內要完成第一次成功發佈**，否則設定過期要重設

### 第一次發佈的順序（重要）

Trusted Publisher 的設定頁在「已存在的套件」底下，所以**全新的套件名無法先設定**：

1. 先用 token 或 `npm login` 手動發一次 `0.1.0`
2. 到 npmjs.com 設定 Trusted Publisher
3. 之後所有版本都靠 tag 自動發，token 可以撤掉
4. （建議）Settings → Publishing access → **Require 2FA and disallow tokens**

---

## 產出物

| 產物 | 內容 |
|---|---|
| `speakdrill-v0.2.0-win32-x64.zip` | `speakdrill.exe`、`speakdrill-gui.exe`、`kokoro_tts.py` |
| npm `speakdrill@0.2.0` | 包裝器；`postinstall` 從上面那個 Release 抓 zip |

npm 的 `install.js` 會組出：

```
https://github.com/thumb2086/speakdrill/releases/download/v<版本>/speakdrill-v<版本>-win32-x64.zip
```

所以 **tag 名稱必須等於 `v` + package.json 的 version**。

---

## 版本號要同步的地方

只有一處：`npm/package.json` 的 `version`。
Rust 的版本在 `Cargo.toml` 的 `[workspace.package]`，兩邊要一致。

```bash
# 建議做法
npm version 0.2.0 --no-git-tag-version -w npm   # 或手改
# 改 Cargo.toml 的 version
git commit -am "chore: v0.2.0"
git tag v0.2.0 && git push origin v0.2.0
```

---

## 手動觸發

workflow 也支援 `workflow_dispatch`：

```bash
gh workflow run release.yml
```

但這種情況下 `github.ref_name` 是分支名，zip 命名會不對，
所以**正式發佈請用 tag**。
