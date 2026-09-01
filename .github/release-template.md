<!--
Release notes template for Translatus.

Not part of the automated pipeline: `.github/workflows/release.yml` still
pulls the GitHub Release body straight from the matching `## [x.y.z]` section
of CHANGELOG.md, and that stays the single source of truth for what shipped.
Use this file when you also want a bilingual, reader-facing writeup: paste
the version's CHANGELOG entries in here, rewrite each line, then post the
result wherever it is useful (the release description, a Zenn/V2EX/note.com
post, a devlog entry).

The one rule: write what changed **for someone using Translatus**, not what
the commit did. If a category has nothing in it, delete that heading.

  Bad  (commit-message voice): "Refactor doctor.rs check ordering"
  Good (user-consequence voice): "`translatus doctor` now checks the sidecar
        port before the binary, so a stuck sidecar shows up first instead of
        being masked by an unrelated binary warning."

不是自動化管線的一部分：`.github/workflows/release.yml` 仍然直接從
CHANGELOG.md 對應版本的 `## [x.y.z]` 段落取出 Release 內文，那裡才是「這版出了
什麼」的唯一真相。這份模板是給你想額外寫一份雙語、給讀者看的版本說明時用：把
該版的 CHANGELOG 條目貼進來、逐條改寫，再貼到用得上的地方（Release 說明欄、
Zenn／V2EX／note.com 貼文、開發日誌）。

唯一的規則：寫「對使用 Translatus 的人來說改變了什麼」，不要寫 commit 做了
什麼。哪個分類沒東西，就把那個標題整段刪掉。

  差（commit 訊息口吻）：「重構 doctor.rs 的檢查順序」
  好（使用者後果口吻）：「`translatus doctor` 現在會先查 sidecar 的埠，
      再查執行檔本身，卡住的 sidecar 不會被無關的執行檔警告蓋過去。」
-->

## vX.Y.Z - YYYY-MM-DD

### English

#### Added

-

#### Improved

-

#### Fixed

-

#### Contributors

Thanks to @handle, @handle for this release.
<!-- find them: git log vPREV..vX.Y.Z --format='%aN <%aE>' | sort -u -->

---

### 繁體中文

#### 新增

-

#### 改進

-

#### 修復

-

#### 貢獻者致謝

感謝 @handle、@handle 對這個版本的貢獻。
