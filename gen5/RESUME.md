# Gen5 再開ガイド (fresh lineage, dacelo製・gen0無改変)

## issue #1 残件の追い込み（本ターン）

- **非ASCII**：CJKコメントは元から可（`//`と書くミスに注意、正は`--`）。CJK文字列は
  `chr` mojibakeでformat再parse不一致＋実行時文字化け→`dcc.dc`の`scan_string`を
  バイト保持化（substring方式、ASCII同一性維持）し両系譜で修正。Gen0(Rust)は
  元からバイト正確。`unicode_span.dc`＋Eテスト＋`--at`列=scalar仕様。
- **why多箇所**：`Warn5`に`sites`追加、app衝突で[app/fn-use/arg-use/束縛decl]を
  JSON限定記録（text不変、38維持）。兄弟適用の列挙は履歴不保持のため将来。
- **hole capped**：`HoleE5`にcapped追加、JSON＋textに表示（trial/rollback本体は既存）。
- **doc comment**：`---`行のquote-aware収集→types binds `"doc"`＋.dci `-- |` 行。
  sem hashはval行のみで不変（doc編集で差分にしない）。stdinは組込みがなく見送り。
- **エラー継続**：fv-walk＋failed集合で独立部を継続解析・依存部をlacking化。
  textは先頭エラーのみ不変。`check_files`のinvalid時sums破棄も修正。
  吸収バグが自作に的中した例あり（case直下case＋後続`|`は括弧化）。
- **§5.2実バグ**：行順・文字割当が表示順依存でdiff誤検出→表示側sort＋canon_order。
  既存`g5_sort_fields`を再利用（重複実装を作って衝突させた失敗あり）。
  prepend収集の順序バグはconcat版で修正。文字割当は意味が変わる箇所
  （tuple順等）は保存される（o1/o2の罠に注意）。
- **GC**：`examples/gc_stress.dc`（20000生＋churn、厳密和で検証）をB組込。
- **hatch警告**：`--backend-only`時に警告行をstdoutへ追加（Daceloにstderr出力組込みなし）。
- **⑧不可確定**：自ソースへの`sig`はlegacy parserがand-group結合を壊す
  （`unbound variable 'sig'`）ためrevert。11.3(3)はseed切替後の課題。
- **box都合と決着**：llama-server常駐で自己検査(要~25GB)がOOM死することがある。
  p1再測＋backend不動点で回帰なしを確定後、静かな時間帯に再走しexit 0を確認。
  test.sh Cはそのまま（要約：boxが重い時はCだけ後で）。

## 達成状態 (./gen5/test.sh が ALL PASSED)

| 検証 | 結果 |
|---|---|
| A. `gen5check check` vs Gen0 `--types` (examples 5 + gen4 tests 33) | 38 agree, 0 differ（終了コード＋メッセージ完全一致） |
| B. `dcc_6` vs `dcc_1` の `.s` (hello/fib/list_ops/closures/tree) | バイト同一＋実行一致 |
| B. Gen5機能 | record→`Alice,Bob`、`{with}`更新→`11`、空レコードOK、moduleリンクOK、sig受諾、holeはexit 2、sig_wrong・unknown field・ill-typed拒否 |
| C. `gen5check check gen5/g5cc_full.dc` (7030行) | exit 0 |
| C. 不動点 `dcc_6.s == dcc_7.s` (4,483,043B) | 一致。dcc_7でhello/record実行OK |
| D. 全15ソース `format` 再parse一致 | clean |

## Layout

- `build.sh`: 一段完結ブートストラップ。seed(dcc_1)がstage1を一回ビルド→
  stage1が両ソースを自己検査→stage1がstage2をビルド→不動点＋自作checkerの
  38一致＋smoke→`./gen5check`・`./dcc_6` にpromote。完走で
  `GEN5 BUILD COMPLETE`。所要約25分（自己検査が大半）。
- Rust-free化：build.shはcargo/rustc/gen0/gen2を一切呼ばない。seedは
  `./dcc_6→./dcc_7→./dcc_1` の順でDacelo製のみ、gateは既存gen5check優先、
  oracleは `gen5/oracle/` golden比較に fallback（`GEN5_NO_RUST=1` で強制も可）。
  goldenは `gen5/regen_goldens.sh` で生成・Gen0と全件照合済みであること
  （regen時のみGen0必須）。残る非Dacelo依存はzsh・cc・rt.c(Cソース)のみ。
- 自己検査はRSS ~25GBで34GB共有機では一過性OOMがあり得る→1回だけリトライ
  （exit 0のみ通過なので誤検出なし）。実例あり（再実行でexit 0確認）。
- `test.sh`: 検証用（build.shとほぼ同工程＋例題マトリクス＋format。約15分）。

- `g5_front.dc`: span lexer (UTF-8 byte半開区間 `Span`)、位置付き `PTok`、AST5
  (`E*5/T*5/P*5` 改名でlegacy衝突回避)、supersetパーサ。Gen4サブセット構文のみ使用。
- `g5_infer.dc`: HM + row多相 + rigid sig + hole partial。状態 `TIS5` 16-tupleを
  関数的にthread。`Ckpt5` 蓄積は停止済み（下流で完全未使用だったO(n²)メモリ）。
- `g5_query.dc`: JSON/focus/why/holes/interface/diff/formatter（再parse等価検査付き）。
- `g5_lower.dc`: テキストlowering＋record prelude（alist＋`g5_rec_get/with`）。
- `g5_driver.dc`: loader（明示import・循環検出）＋check/types/focus/why/holes/interface/format CLI。
- `g5cc_driver.dc`: check-then-compile。`--backend-only` hatchあり（下記）。
- `g5_main.dc`、`test.sh`、`g5check_full.dc`/`g5cc_full.dc`（生成物、gitignored）。
- `../gen5-examples/`: 構文例11件＋README（全コマンドはrepo rootから逐語実行可能）。

## 重要な設計決定

1. **新キーワードは予約語** (`sig/module/exposing/import/with/forall`)。
   変数に使うと後続行頭の同語が適用引数・型適用引数に吸収される
   （例：`let id x = x` の次行 `sig ...` が `x sig ...` と誤parse）。
   `fall`/`exps` に改名して解決。以後これらを識別子に使わないこと。
2. **loweringはテキスト経由＋再parse**（AST直構築を避け、legacy backendを無改変で再利用）。
3. **二段階不動点**：codegenはparse木のみ消費し検査結果を読まない
   （`g5cc_legacy/lowered` が証拠）ため、検査通過済み入力では
   `--backend-only` 出力は完全実行と同一。34GB機で検査＋codegen同プロセスが
   OOM死するため（各相は単独で収まる）、`gen5check check` (exit 0) と
   `dcc_6 --backend-only` に分離。default動作はcheck-then-compileのまま。
4. **preludeは `[]` パターンを使わない**：legacy backendは式位置の `[]` にしか
   ctor instanceを出さない癖があり、`| [] ->` だけだと `_ctorinst_Nil` の
   リンクエラーになる。`| _ ->` フォールバックで回避（`::` パターンはCons
   instanceが出ることを確認済み）。
5. **case吸収バグの回避**：多腕 `case` のarm直下に単腕内側 `case` を置く形は
   括弧化/hoistする（Gen0/Dacelo共通の癖）。`let/and` グループ規則も厳格。

## 自己ソースの制約：Gen4サブセット厳守（11.3(3)は延期）

自ソースにGen5構文（`sig`等）を入れるとseed鎖が壊れる。Gen0/dcc_1の
legacy parserは `sig` itemを知らず、and-group内に落ちた `sig` 行を
メンバー名と誤読して `unbound variable 'sig'` で死ぬ（実測）。
`sig` 移行は、seedをGen5以降のみに切った後の時代の作業として延期する。
自ソースで使ってよいのはGen4サブセットのみ（`fall`/`exps`改名と同根の制約）。

## 既知の性能特性（将来工作）

- 検査は二次時間：dcc+front (2600行) 44s → +infer (4500行) 174s →
  g5cc_full (7030行) 約7～10分。ピークRSS ~26GB（`g5s_nodes` の型コピー等。
  `Ckpt5` 停止前はOOM死していた）。
- 内訳の主犯候補：①アイテム毎 `g5_snapshot` の3×`len`（subst/rowsubstは
  run-wideにgrow-only）、②boundary毎 `g5_norm_env`（全envの型正規化コピー）、
  ③assoc-list envの線形探索。①の `len` キャッシュは `TIS5` 16-tuple surgeryが
  必要（accessor約35箇所の位置パターン）で回帰リスク大のため未着手。
  着手時は `./gen5/test.sh` 全完走（約15分）を合否条件にすること。

## セッション記録（バグ修正の要点）

- `g5_lc_off_line` 0固定、`g5_drop_brace` の-1/-2、`g5_hash_bytes` の負剰余。
- formatterが `(Cons h Nil)` を出すと再parse不能（arg位置のnullary ctorは
  legacy parityで非対応）→ Cons/Nilは `::`/`[]` 糖衣で印刷（lower側も同一修正）。
- 型適用引数の括弧落ち（`TTTup (List TyAst)` → `TTTup List TyAst`）→ 引数あり
  `TTCon` は括弧化。
- 真偽リテラル隠蔽：`ppat_atom` のTKw-soft腕が `"true"/"false"` 腕より前に
  入り `ELet false ...` が死ぬ→腕を末尾へ（後にsoft機構ごと撤去）。
- `forall` キーワード化と自ソースの変数 `forall`/`exposing` の衝突→予約語化＋改名。
- リネーマ誤作動で7件の `"` を剥離→連結物から復旧＋quote-strip等価で全件検証。
- `sig` 2件で「unbound variable `sig`」に見えた件の真因は上記キーワード衝突。
- `gen5-examples/module.dc` のリンクエラー真因はprelude欠落ではなく
  `_ctorinst_Nil` 未生成（上記4）。
- `test.sh` の `set -e` と hole exit 2 の衝突→`set +e` ガード。
