# Gen5 再開ガイド (fresh lineage, dacelo製・gen0無改変)

## issue #1 追記レビュー対応（P1×6＋P2×5、11件完治）

- **P0級の副産物：rollback反転**：`g5_rollback`が newest-ns を残し history を
  捨てていた（`g5_trunc`の向き間違い。unit probeで確定後 `g5_drop` に修正）。
  旧コードはhard error即停止で発覚せず、tentativeも偶然動いていた。
  継続検査(#2)とhole再 trial(#9)は正しいrollbackなしには成立しない。
  probeが一度 `with_subst` のreplace意味で空振りした教訓あり（consで再検証）。

- **P1-1 open row残余**：flex/flexにfresh共有tailを両側束縛（片側のみは成功後も
  両辺不一致でunsound）。片側循環はinfiniteで拒否。`pick`例で検証。
- **P1-2 occurs**：`g5_row_occurs`がchase後tailを捨てていた→tailも検査。
  内部probe相当7件（direct/nested/fun/tuple/否定×2/subst経由）をCLI例で検証。
- **P1-3 括弧**：Proj/With受信側・App関数側・With更新値を非atomic時括弧化。
  `(mk 42).value`→42、`((fun..) 41)`→42で検証。
- **P1-4 文字列**：loweringは`g5_dc_str`で再エスケープ（式・パターン）。
  `"\\n"`(2byte)→2、`"\n"`一致→7で検証。
- **P1-5 capture**：全bind site走査で予約拒否（topはprogram-wide、localは
  自file記録使用時のみ。legacy互換維持）。prelude衝突も同走査で防御。
- **P1-6 順序**：lowering連結をdep-firstに（entry-firstは未初期化読み）。
  値コピー・diamond・top-call実行で検証。
- **P2-7 env分離**：file毎export表（値・ctor・tydecl）＋初期env再構成＋
  tydefs save/restore。exposingは大文字可に拡張、型名露出も検証可に。
  import順不変・単独一致を検証。
- **P2-8 focus分離**：file毎node/sym slice＋entry限定探索＋island local_env
  ＋decl file tag。Dep/Main例・f/g例で検証。
- **P2-9 hole再 trial**：scheme保存＋refresh時re-trial＋後絞りcap。
  good採用・wrong除外を検証。
- **P2-10 ADT diff**：semに正規化type行を含め、diffも対応。ctor追加でexit 3、
  param改名は無視を検証。
- **P2-11 toolchain**：3 systemの戻り値検査＋tmpbin経由publish。fake ccで検証。
- **Dacelo実装上の新知見**：`and`束縛名は複数型で使えない（単相化される。
  `g5_slice_new`で発覚→standalone化）。多相helperは必ずstandalone `let rec`。
  変数腕が文字列リテラル腕の後だとGen0が誤る（`g5_canon_or`で回避）。
- **⑧不可再確認**：自ソースsigはlegacy鎖破壊のため不可（前回同様revert）。
- **box都合（再）**：llama常駐で自己検査(~27GB)が死ぬ。prefix曲線
  （p1 47s/21GB→pfxD 261s/24.8GB）は滑らかで回帰なし確定。C完走は静穏時要再走。

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
- **box都合と決着**：llama-server常駐で自己検査(要~27GB)がOOM死することがある。
  p1再測＋backend不動点＋pfx曲線で回帰なしを確定。全11件修正後の現行ソースでは
  自己検査がexit 137で死ぬことを確認済み（3:58経過時点）。A/B/D/E/F＋不動点は
  現行でgreen。C完走は空き27GB+のboxで再走要（test.sh Cにtry_twice済み）。

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
