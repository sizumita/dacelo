---
name: gen5-dev
description: Dacelo Gen5（gen5/ の dacelo 製チェッカ gen5check とコンパイラ dcc_6）の変更・検証・レビュー対応の手順。gen5/*.dc を編集する、gen5/test.sh や build.sh を回す、issue #1 のレビューコメントに対応する、probe_*.dc でソルバの単体回帰を書く、自己検査（C 節）が OOM/exit 137 で止まる、Dacelo で書いたコンパイラソースが Gen0 や dcc_1 で不可解なエラーになる、focus/why/holes などのクエリ結果を検証する、といった場面では作業前に必ず読むこと。"Gen5" "gen5check" "dcc_6" "row" "hole" "sig" "module" "RESUME.md" が話題に出たら該当。
---

# Gen5 開発ガイド

Gen5 は `gen5/` にある「dacelo で書かれた」チェッカ（`./gen5check`）と
check-then-compile コンパイラ（`./dcc_6`）。ソース連結 → seed コンパイラでビルド →
自己検査 → 自己ビルド不動点、という鎖の上に乗っているため、**どの段階を触ったら
どの検証を回す必要があるか**を先に決めてから手を動かす。

## 1. まず把握するもの

- 設計の根拠は issue #1（RFC）とそのレビューコメント。読むときは
  `gh issue view 1 --repo sizumita/dacelo --comments`（本文 40KB＋コメント 20KB 級なので
  ファイルに落として分割して読む）。
- 現状・既知の制約・過去のバグの要点は `gen5/RESUME.md`。**これが正なので、作業後は必ず更新する。**
- 連結順（この順以外はリンク/型検査が壊れる）:
  - checker: `gen3-dcc-dc/dcc.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_driver.dc gen5/g5_main.dc` → `gen5/g5check_full.dc`
  - compiler: `gen3-dcc-dc/dcc.dc gen3-dcc-dc/g3_pm_v2.dc gen3-dcc-dc/g3_ce_v2.dc gen3-dcc-dc/g3_driver_v2.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_driver.dc gen5/g5cc_driver.dc` → `gen5/g5cc_full.dc`
  - `*_full.dc` は生成物（gitignore）。test.sh/build.sh が毎回作り直す。
- 各ファイルの役割: `g5_front.dc`（span lexer/AST5/parser）、`g5_infer.dc`（HM＋row＋rigid sig＋hole、状態は `TIS5` 16-tuple）、`g5_query.dc`（JSON/focus/why/holes/interface/format）、`g5_lower.dc`（record/module のテキスト lowering）、`g5_driver.dc`（loader＋CLI）、`g5cc_driver.dc`（compile driver）。
- 自ソースは **Gen4 サブセット構文のみ**（sig/record/module を自ソースに使うと seed 鎖の legacy parser が死ぬ）。

## 2. 変更 → 検証の最短ループ

型エラーを数秒で拾う（Gen0 は Rust 製で軽い）:

```bash
cat gen3-dcc-dc/dcc.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_driver.dc gen5/g5_main.dc > gen5/g5check_full.dc && ./gen0-interp-rs/target/release/dacelo gen5/g5check_full.dc --types
```

バイナリを作り直す（`dcc_1` は検査なしの Gen3 コンパイラなので速く、RAM も食わない）:

```bash
./dcc_1 gen5/g5check_full.dc gen5check
```

検証はセクション選択つきの `gen5/test.sh`（A: 38 ケース oracle、B: dcc_6 の .s 同一＋実行、
C: 自己検査＋不動点、D: format 再parse、E: RFC 受入、F: issue #1 レビュー回帰）:

```bash
GEN5_SKIP=C zsh gen5/test.sh      # A/B/D/E/F 全部で約 4 分。ソース変更後はまずこれ
GEN5_ONLY=F zsh gen5/test.sh      # 既存バイナリがソースと一致している前提で F だけ（約 1 分）
```

- A/B が `./gen5check` `./dcc_6` を作り直す。A/B を飛ばすときは、直前に同じソースから
  ビルド済みであること（`cat <連結> | cmp - gen5/g5check_full.dc` で確認できる）。
- test.sh は各節を `sec_X()` 関数にしてある。**実行中の test.sh を同じ inode に上書きしない**
  （zsh はスクリプトを逐次読みする）。走らせながら直すときは別名に書いて `mv` する。

## 3. C 節（自己検査＋不動点）は別扱い

- `./gen5check check gen5/g5cc_full.dc`（7000 行）は約 7〜10 分、**ピーク RSS 約 27GB**。
  この box は 34GB なので、llama-server などの常駐を止め、他の重い処理と同時に走らせない。
  `exit 137` は OOM kill であってバグではない（test.sh は 1 回だけリトライする）。
- 起動は切り離して行い、ログを監視する:

```bash
nohup zsh -c 'GEN5_ONLY=C zsh gen5/test.sh; echo "EXIT=$?"' > /tmp/g5_sectionC.log 2>&1 &
```

- 二段階不動点の理由: codegen は parse 木しか読まないので、`gen5check check` が exit 0 なら
  `dcc_6 --backend-only` の出力は完全実行と同一。検査＋codegen を 1 プロセスでやると OOM する。
- **C を回していない commit を「ALL PASSED」「完治」と書かない。** RESUME には commit ごとに
  「何が green で何が未実行か」を書く（レビューで指摘済みの事項）。

## 4. ソルバの単体回帰（probe_*.dc）

CLI 例だけでは「通常の occurs-check でも弾ける例」になりがちなので、ソルバの不変条件は
`gen5/probe_row_occurs.dc` / `gen5/probe_row_unify.dc` の形で直接ピン留めする:

- `dcc.dc + g5_front.dc + g5_infer.dc + probe` を連結し **Gen0 で実行**（0.5 秒、RAM ほぼゼロ）。
- `main` で `true/false` や `g5_show s t` の文字列を 1 行ずつ出し、期待出力を test.sh F に
  `printf ... > exp; diff -q` で置く。
- 状態は `g5s_new` から作り、手書きの var/row id と衝突しないよう
  `g5s_with_rowctr (g5s_with_ctr g5s_new 10) 10` のようにカウンタを上げる。
- 「単一化成功 ⇒ `g5_show s a == g5_show s b`」「引数を入れ替えても同じ正規形」を必ず含める。

## 5. レビューコメント対応の流れ

1. コメントを項目ごと（P1/P2/R1…）に分け、各項目に対して **F 節の回帰テスト**を先に書く
   （レビューに載っている最小例をそのまま使う。JSON は `grep -q` で具体値まで検証し、
   「コマンドが成功した」だけを合格にしない）。
2. 直す → `GEN5_SKIP=C` → C 単独 → `gen5/RESUME.md` に「項目 → 直し方 → 検証」と
   commit 別の結果表を書く。
3. commit メッセージは `Gen5: ...` で始める（既存の `git log` に合わせる）。
4. issue への返信は user が投稿する。返信案には「修正確認できた項目 / 部分 / 未完」を
   レビュー側の表現に合わせて書き、未実行の検証（C など）を明記する。

## 6. Dacelo で書くときの落とし穴

自ソースを書き換えるときは必ず `references/dacelo-pitfalls.md` を読む。特に
「予約語」「case 吸収」「`and` グループの単相化」「文字列リテラル腕の後の変数腕」は
症状が意味不明なエラー（`unbound variable 'sig'` 等）になる。

## 7. クエリ CLI 早見

```bash
./gen5check check|types|focus|why|holes|interface|format <file> [--format=json] [--at=L:C] [--max-bytes=N] [--write=P] [--check=P]
./dcc_6 <file.dc> <out> [--backend-only]
```

- `--at` は 1 始まりの行・列、列は **スカラ（文字）単位**、JSON の span は UTF-8 byte 半開区間。
- exit code: 0 checked / 1 invalid / 2 partial（hole あり）/ 3 interface diff。
- `focus` は `binding.scheme`（定義スキーム）と `occurrence`（使用箇所の単相型）を分けて返す。
  `local_env` は解決済みスコープ由来で、「宣言が前にある」だけでは載らない。
