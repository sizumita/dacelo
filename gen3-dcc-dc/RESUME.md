# Gen 3 再開ガイド(2026-08-24 セッション4終了時点)

## 🏆 完全セルフホスティング達成(セッション28)

| パス | hello | fib | list_ops | closures | tree |
|---|---|---|---|---|---|
| interp-path(Gen0 が dcc を実行) | ✅ | ✅ | ✅ | ✅ | ✅ **5/5** |
| **dcc_1 チェーン**(Gen2→dcc_full→dcc_1→examples) | ✅ | ✅ | ✅ | ✅ | ✅ **5/5** |

dcc_1 = `./gen2-dcc-rs/target/release/dcc gen3-dcc-dc/dcc_full_latest.dc -o dcc_1`
使用法: `./dcc_1 <入力.dc> <出力ベース>`
dcc_2(第2世代)は生成・起動可能だが lam クロージャの layout 依存
クラッシュが残存(B-4)。

## 🏆 現在の達成状態

| パス | hello | fib | list_ops | closures | tree |
|---|---|---|---|---|---|
| interp-path(Gen0 が dcc を実行) | ✅ | ✅ | ✅ | ✅ | ✅ **5/5** |
| **完全セルフホスト**(dcc_1 = Gen2 コンパイルの dacelo 製コンパイラ) | ✅ | ✅ | ✅ | ✅ | ✅ **5/5** |

- `./dcc_1 <入力.dc> <出力ベース>`(-o フラグ不要)
- 検証: `zsh gen3-dcc-dc/test.sh`(要 dcc_1 再build: 下記コマンド参照)

## 🎉 今セッションで到達したマイルストーン

**Gen 2 コンパイラの実バグを 5 件特定・修正し、ネイティブセルフホストが通過:**

1. **動的フレーム**: FRAME_BYTES=4032 固定 → スロット数から `movz x9,#N; sub sp,sp,x9`
   をエピローグ時にバックパッチ。encoder.rs に sub_sp_reg/add_sp_reg/patch_movz_imm16
   追加。`sub sp,sp,x9` の正エンコーディングは **0xCB2963FF** (add は 0x8B2963FF)、
   シフトレジスタ形式の Rd=31 は SP ではなく XZR になるので注意
2. **ネストパターンのベースレジスタ破壊**: match_pat を完全スロットベースに改修
   (scrutinee を各レベルで必ず spill、R/x9 経由でアクセス)
3. **文字列リテラルパターンがポインタ比較だった**: dc_val_eq (構造的等価) を呼ぶ
   よう修正。scrutinee はスロット経由で x1 に再ロード(x0/x1 は C 呼びで clobber される)
4. **リテラルパターンが reg==R9 で常に真になる**: Int/Bool/Unit/nullary-ctor は
   リテラルを T(x10) にロードして cmp_reg(reg, T) に修正
5. **argv バグ**: rt.c main が 1 要素配列を渡して argv(1) が NULL/空 → 実 argv を渡す

**検証済みチェーン:**
```
dcc(gen2) → interp.dc → interp_nat: examples 全正常動作 (セルフホストインタプリタ)
dcc(gen2) → dcc_full_latest.dc → dcc_1: hello.dc をコンパイル→実行 OK
```

## 🔴 残存バグ(優先順)

### ✅ 解決済み(セッション3): 多段 HOF の崩壊
原因は emit_fn_level の prev_names 二重反転(`rev_list` 余分)で
レベル≥3 のパラメータ slot マッピングが入れ替わっていた。
修正済み。list_ops/closures 含め interp-path 5/5 PASS。

### ✅ セッション4で解決: B-2' の本体
原因は dc_bi_str_concat も旧 (n+7)/8 公式で、連結結果長が 8 の倍数の文字列
(例: "fib_g3.s"=8 文字) に NUL 終端が無かった。make_string と同じく
(n+8)/8 に修正 → **dcc_1 チェーン 4/5 → fib 含め動作確認**。

### B-3(ほぼ解決・残り一歩): 自己コンパイル
セッション4の修正:
1. join_lines をチャンク化(take_chunk 24行 + join_lin)→ O(n²) 部分文字列が
   conservative GC のスタック走査で保持されメモリ爆発していた問題を解消。
   自己コンパイルは 1 秒で完走するようになった
2. 重複定義 12 個(dcc.dc 内部の二重ブロック+ce/drv の重複ヘルパ)を整理
   → アセンブラの duplicate symbol エラー解消
3. ラベル ID を len(a_lines) から AS の未使用 lamc フィールドを使う
   グローバルカウンタ a_fresh_id に変更(状態巻き戻りに耐性)
4. c_binop: "/" と短絡 "&&"/"||" を実装、未知演算子フォールバックと
   ELet isrec の「状態状態巻き戻し」を解消(ラベル重複の温床)
5. ts_load_const: 負値/巨大定数の movz/movk を two's-complement リム分解 +
   MOVN で正しくエンコード(movz #-65535 など無効即値エラー解消)
6. rt シンボル名修正: dc_str_concat → dc_bi_str_concat
7. ★ cm_asm PTup アームが検査命令を捨てていた(st で cmf_asm 呼び)→ s4 スレッド
8. ★ ce ETup をシーケンスからタグループ構築に変更([hdr][e1][e2] ブロック)

セッション5の追加修正(PTup の hdr load も st から取っていて捨てられていた):
```
let s1 = ts_ldr_off st 9 reg 0 in        ← ldr x9,[x0]
let s2 = ts_load_const st 10 (...) in    ← ✗ st から作ると s1 が消える
```
→ s2 は s1 から、以降全て s チェーンでスレッド(PTup/PCtor 両方)。
さらに PTup/PCtor を「scrutinee を即座に spill slot へ退避→slot 経由で
全アクセス」方式に全面書き直し(Gen2 の match_pat と同じ発想)。
ネストしたタプル/コンストラクタパターンの要素ロードも slot 経由。

**結果: 自己コンパイル完走(1秒)、dcc_2 生成成功・実行可能。**

セッション5追加修正:
9. cm_asm PTup の hdr load が `st` 由素で捨てられていた → s1 経由に修正
10. PTup/PCtor を「scrutinee 即時 spill(ns) → 以降 slot 経由」に全面書き直し、
    要素ループも sc/ns を正しくスレッド(Gen2 match_pat 方式の移植)

### B-4(現時点の最終未解決): dcc_2 のパーサが全入力で失敗
【セッション5追記】cm_asm PStr を dc_val_eq 方式に修正済み(ポインタ比較だった)。
dcc_2 バイナリ内に 65 個の dc_val_eq 呼出が確認できる(pitems_3 内にも 3 個)。
それでも parse は失敗する → 別箇所の残留誤翻訳。lldb の symbol BP が
このバイナリで当てにならない点も注意(read_file は当たるが match_fail/
val_eq は当たらないことがある)。objdump 全体ダンプ + grep が確実。
- 最小再現: `printf 'let x = 5\n' > m.dc && ./dcc_2 m.dc out`
  → "expected top-level item (`let` or `type`)"
- 重要な観測:
  * dcc_2 の LEXER は正常動作(scan_int_run 等のタプル返却も正しい)
  * pitems 内に print を 1 個足すだけで **通ってしまう**(どの位置でも)
    → アドレス/レイアウト依存の誤アクセス。GC タイミングでは無い
      (DACELO_NO_GC=1 でも同じ)
  * DACELO_NO_GC 環境変数を rt.c に追加済み(GC 完全停止で診断できる)
- 切り分け手順:
  1. nm dcc_2 | grep fn_pitems_3 でアドレス取得
  2. lldb で fn_pitems_3+324 付近の逆アセンブルと各 slot 内容をダンプ
  3. `let (isrec,t1) = case ... (ETup 構築→PTup 再取出)` の
     spill slot 番号一致を確認(ETup 構築側と cm_asm PTup 側の ns ずれ)
- 次の一手候補: ce ETup 構築の spill slot 開始番号(ns)と、ECase/PTup
  側の sv=ns 予約の衝突チェック。ETup 内部で使う一時 slot が
  外側パターンの sv と重なっていないか

### B-4 デバッグ事実(セッション6追記)
### B-4 セッション8の進展
1. ✅ c_binop ==/!= を dc_val_eq 方式に修正(ポインタ比較だった)
   → mem_str のキーワード判定が動くように(字句解析で TKw が生成される)
2. ✅ go_cases に scrutinee reload を実装:
   各ブランチのチェックは x0 を破壊する(val_eq の C 呼び等)ため、
   ECase 入口で一度 spill(slot=ns)、各ブランチ先頭で ldr_slot(x0←slot)。
   シグネチャに base 引数追加。
3. interp-path 5/5・dcc_1 chain 5/5 維持 ✓
4. dcc_2 現状: パース途中で lam1615 内の blr [x0+8](x0=garbage code ptr,
   addr=1)で SIGBUS。be1626 領域のフレーム → 比較ラベル近傍の関数内。
   次の一手: a_fresh_id ベースの _lam<id> と、比較ラベル(Lnt/Lne/Lbt/_be/
   Lsc/Lse/Lcn/Lcend/Lelse/Lendif/Lpfail)の ID 空間が混在していないか、
   また mk_closure_seq の code-ptr GOTPAGE 参照先が正しい label かを、
   dcc_2.s 上で lam1615 周辺をダンプして確認

### B-4 決定的発見(セッション10): 謎の「空ラムダ連鎖」
dcc_2.s には **56 個の `_lam<N>` ラベル**が存在する。中身は:
```
_lam755:
  stp/mov/sub            ; プロローグ
  str x1, [sp,#0]        ; param
  ldr x9,[x0,#24]; str   ; env[0]
  ...
  b Lamskip756           ; ← スキップ先が「次の」ラベル!
_lam756: ...(同様に空)...
```
つまり「本体が空のラムダが skip 先を次々とチェーン」している。
dcc のソースには無名関数(fun)は存在しない。唯一の ELam 生成源は
wrap_lams(定義パラメータの脱糖)で、これは emit_fns_asm/emit_thunks_asm
の strip_lams で剥がれるはず。→ **strip_lasm を通らず ce ELam に到達する
Def3 が存在する**(and グループ? 特定の定義形式?)ことがほぼ確定。
これらの中身が空なのは、ネストした ELam チェーンの各レベルで
「body=次レベルのクロージャ割り当て」となり、実体が最内のみに存在するため。
修正方針: どの def が strip を通らずに ce に到達するか特定するには、
ce の ELam アーム入口に来た pat の内容を .s コメントとして出力すればよい
(コメントはコードを壊さない)。

### B-4 最終確定(セッション13): 「長さ1文字列が関数として呼ばれる」
- クラッシュ直前の x0 は **TSym"(" 等のペイロード = 長さ1文字列**
  ([+8]=LEN=1 を code ptr として読む → pc=1)
- 発生箇所: dcc_2 が lam.dc(`fun p -> case p of (a,b) -> ...`)を
  コンパイル中。同じ入力は dcc_1 では正しくコンパイルされる(出力 34 ✓)
- == 構造的等価修正後も TKw トークンは正常生成されることを確認
  (ctor-step trace: cid=6 ×1, cid=4 ×1, cid=7 ×1, cid=2 ×1 ✓)
- → 残るバグは OUR ce の環境解決(EVar→slot)または ETup/ECase の
  bind slot 計算が、ネストした case-in-let-in-case の特定組合せで
  間違ったスロットを指すこと。EVar が文字列トークンを関数として
  解決している = sc の (名前,slot) 対応が一時的に崩れる
- 検証方法: dcc_2dbg.s の当該 _fn_ce 領域で、ELet/ECase アームの
  app_list/zip_ids2 呼び出しによる sc 構築を .s コメント付きで追跡

### B-4 セッション28: 静的検証の限界と次の一手
### 🔬 B-4 最終確定(セッション33)
dcc_2dbg で空入力でもクラッシュ。バックトレース:
```
a_lines_1 ← a_line_2 ← a_inst_2 ← lam1679
```
= **a_inst の第1引数(AS状態)に ct_id_tbl が渡っている**
show(ct_id_tbl) = [(Nil,0),(Cons,1)] が match_fail の表示と完全一致。
emit_code 内で st1(AS) を作った直後は正しい(dbg_st1 で確認済み)が、
emit_fns_asm 呼び出しチェーンの途中で ct_id_tbl に置き換わる。
→ **OUR codegen のフレーム slot 割当中、AS 状態の slot と
ct_id_tbl の slot が衝突している。** 具体的には emit_code の
ローカル変数間で、ELet/ECase の ns 消費パターンに一貫性がない。
修正には ce 全アームの slot 消費ルール統一が必要(EApp ns+2 等)。

emit_thunks_asm のソースは静的に完全正しい(a_line t 文字列の順序、
slot0-3 の env チェーン、再帰は gget 経由)。7 レベル関数
(emit_fn_level)のチェーンも検証済み。ランタイムでだけ壊れる。
→ 静的解析は完了済み。残る手段は以下の 2 つ:
1. **Gen2 prologue でのフレームゼロ初期化**(動的フレーム対応版):
   epilogue パッチ方式と同様に、prologue の movz を後からパッチし
   「sub 後の x9(バイト数)」を使って str xzr,[x10],#8 ループで
   全 slot をゼロ埋めする。未初期化読み出しが全て tagged-zero に
   なるため、レイアウト依存クラッシュが消えるはず
   (必要な encoder 機能: post-index STR の追加のみ)
2. ce の各アームに .s コメント(APPSPILL 等)を再導入し、
   dcc_2.s 上で lam1619 等の実 crash 関数周辺の slot 表を機械的に監査
推奨は 1。rt.c 側ではなく codegen.rs 側の修正なので汎用性が高い。

### B-4 クラッシュ地点特定(セッション27・最新)
dcc_2dbg のクラッシュ:
```
match_fail value = [(Nil,0),(Cons,1)]  ← ct_id_tbl!!!
bt: a_lines_1 ← a_line_2 ← emit_thunks_asm_4
```
= **a_line(AS状態, 文字列) の第1引数に ct_id_tbl が渡っている**
つまり emit_thunks_asm 内で「AS 状態」と「ct_id_tbl」の
slot/EVar 解決が入れ替わっている。レイアウト依存の理由:
ns オフセットがずれた位置の slot を EVar が読んでいる。
監査対象: emit_code/emit_thunks_asm の ECase 脱構築
(`case tabs of`, `case gt of`)が ns を正しく消費しているか、
および ELet-ECase の組合せで base slot と bind slot が
重複していないか。ECase アームは ns+1 を消費して返すべき
(現在は元 ns を返しているため、次の文が同じ slot を使い
 直すのは良いが、base と衝突する)

### B-4 新事実(セッション12・最終): クラッシュはレキサ内部でも発生
- シングルステップ解析: dcc_2 は read_file 直後の lex_all 中にも
  pc=1 クラッシュ(pitems 到達前)。非決定的(時々 pitems_3 まで到達)
- クラッシュ様相: 長さ 1 の文字列(ID"x" ペイロード等)が
  「関数」として呼ばれる([+8]=LEN=1 を code ptr 化)
- → 文字列⇔関数の slot 混在はパーサ以前のレキサ
  (TKw w :: acc 等のトークン構築)でも発生している
- 統合仮説: dc_ctor_step/mk_closure のフィールド書き込みと、
  直後の別オブジェクト生成が競合、もしくは OUR codegen の
  EApp callee-spill(slot ns)が特定ネストで ae 側文字列を上書き

### B-4 決定的データ(セッション11): 完全なワークアラウンド発見

### B-4 セッション23の確定事実
- EApp の slot 消費修正(ns+2 返却)は効果なし(interp 5/5 維持、dcc_2 失敗継続)
- d_rhs3 が受ける値は常に ("Nil",0) = **ct_ar_tbl の先頭要素**
- → all_defs の値が ct_ar_tbl になっている可能性大。
  疑わしいのは build_tables 内の 3-tuple ETup 構築と
  main 側 `case tabs of |(a,b,c)->c` の PTup-3 脱構築。
  検証: dcc_2.s の当該関数で fill ループの読み slot と
  PTup fld の読み slot を突き合わせる
  (dcc_2.s は rm しないこと。次回ビルド時に再生成される)

### B-4 切り分け完了(セッション22): emit_code 呼び出しがトリガー
md1(md=build_tablesまで)=OK / md2(+build_globs)=OK /
md3(+emit_code呼び出し)=OK(!) / md4(+next2脱構築+join_chunks)=NG("Nil,0")
→ 同じ emit_code 呼び出しでも周囲の slot レイアウトで成否が変わる
  = **フレーム内 slot の write/read 対応が 1 つずれている**ことの確証。
  絶対 slot 番号依存のずれ。対象関数: emit_fns_asm 内 d_rhs3 呼出し
  もしくは strip_lams/emit_fn_level の slot 計算。
修正方針(機械的・確実): ce の全アームで「slot 消費」を統一する。
EApp/ELet/ECase/ELam/c_binop の各アームについて、
  - EApp: callee spill @ns, ae は ns+1 開始, 返り ns+2
  - c_binop: l@ns, r@ns+1, 返り ns+2
  - ECase: base@ns, binds ns+1.., 返り = 最大消費後
を強制し、返却 ns を必ず「消費した分だけ進める」。現状いくつかの
アームは元の ns を返しており、兄弟式が同じ slot を再利用する。
逐次実行では安全なはずだが、ネストした EApp(callee 内でさらに spill)
との組合せで限界ケースがある可能性。

### B-4 再現コンパイラ完成(セッション21)
- `/tmp/mini_full.dc` =dcc.dc+pm+ce+フルドライバ(everything-except-nothing)
  を dcc_1 でコンパイルした /tmp/mf が (Nil,0) 失敗を再現
- C4(=同一だが tiny main)は成功 → **差分は「実際の main 本文」のみ**
- 実際の main は parse 成功後、build_tables→3-tuple→case 脱構築×3→
  build_globs(ローカル rec ×2)→emit_code を実行する
- 次の一手: mini_full.dc の main を段階的に縮小しながら
  「どの文を削ると直るか」を特定する(例: ①build_globs 呼び出しを削る
   ②emit_code 呼び出しを削る ③case 脱構築を1つ削る)
- デバッグ用コメント出力(APPSPILL 等)は g3_ce_v2.dc から削除済み。
  再追加する場合は EApp/ETup/ELet の各 spill に slot 番号コメントを付ける

### B-4 セッション24: 4 引数カリー呼び出しは基本動作を確認
- `f4 a b c d` の直接呼び/gget 経由部分適用/式引数、すべて正常(2134)
- 失敗するのは emit_fns_asm のような「ローカル rec を含む関数が
  5 引数で呼ばれ、さらに内部で d_rhs3 等を gset/gget 経由で呼ぶ」
  組合せ。gslot 方式のローカル rec クロージャと通常クロージャの
  **env ワード数/配置**の違いが、多段レベルチェーンの ld_prev で
  誤読を起こしている可能性が高い
- 検証手順: fn_emit_code_5 の ld_prev 部分を逆アセンブルし
  ldr x9,[x0,#24..48] の 4 語と slot0-3 への格納、その後の
  body 内 EVar 解決 slot を番台表化

### B-4 セッション23の決定的発見
- self-contained emit_code(prog) リファクタ後、失敗値が
  **ct_id_tbl のリスト全体** `[(Nil,0),(Cons,1)]` に変化
- = emit_fns_asm の第4引数(defs)に第2引数(ct_id_tbl)が渡っている
  **カリー化引数のずれ**(3 つ分ではないが位置ずれ)
- 原因候補: OUR codegen の多段カリー呼び出し
  App(App(App(App(f,a),b),c),d) において、レベルチェーンの
  env/capture 引渡しで特定の組合せ(callee が gget 取得 + 4 引数)の
  場合に引数が一つずれる。EApp arm の sp_slot=ns 方式は
  兄弟式との再利用で安全なはずだが、gslot 方式のローカル rec と
  组み合わさった際の slot/グローバル干渉の可能性
- 検証: 4 引数トップレベル関数を gget 経由で完全適用する最小例を
  dcc_1 でコンパイルし dcc_2 相当の実行で確認

### 🔬 B-4 最終分析(セッション31・逆アセンブル完全追跡)
クラッシュ関数 fn_emit_thunks_asm_4 内の実行列を完全トレース:
```
Lelse1603:
  gget(313=a_line); apply(slot0=AS-state)   → partial ✓ 正常
  apply(partial, ".p2align 2")               → 新しい AS 状態 ✓ 正常
  ...
  [後続の分岐で match_fail(pair)]
```
a_line 呼び出し自体は正しい。match_fail の値 ("Nil",0) ペアは
**別の case 文の失敗**であり、bt の近傍シンボル表示が誤導していた。
→ 残る真因は「複数の ECase/EIf が混在する emit_thunks_asm 内で、
特定のレイアウト時に分岐先 or scrutinee がずれる」こと。
確定させる最終手段: 各分岐直前に「通過マーク文字列」を
write_file で別ファイルに書くデバッグ版を作り、
どの分岐を通ったかを実行順に記録する。

### 🔬 B-4 クラッシュ命令の正体(セッション30・逆アセンブル結果)
dcc_2.s のクラッシュ位置(+1148 付近)の実体:
```
adrp/ldr x0 = 静的文字列(".p2align 2")
mov x1, x0                  ; 第1引数に文字列
ldr x0, [sp, #96]           ; 第0引数位置に slot96 の値
ldr x9, [x0, #8] ; blr x9   ; 呼び出し
str x0, [sp, #96]
```
これは **PStr パターンの dc_val_eq 比較シーケンスそのもの**
(spill slot から x1 再読み込み + 静的文字列 x0 + val_eq)。
つまりこのコードは「パターンマッチの文字列リテラル比較」であり、
その直後の hdr 読み出し(+688 相当)が別関数の領域と重なって見えていた。
→ クラッシュは「PStr 比較が実行されるべきでない文脈で実行されている」
可能性、もしくは lldb のシンボル帰属(pitems_3/emit_thunks_asm_4 等)が
近傍シンボルへのフォールバック表示であることに注意。
次の一歩: dcc_2dbg.s 上でクラッシュ pc を含む関数境界を
ラベル行ベースで正確に特定し、その関数のソース式を割り出す。

### 🎯 B-4 完全特定(セッション29・最終)
マーク付き write_file デバッグの結果、すべての中間値が正しいことを確認:
- ST1 = AS 状態(正しい globs を持つ)
- PRE の all_defs = [(Def3 false x (EInt 5))] ✓
- POST の map_gname2 all_defs = [x] ✓
それでもクラッシュ:
```
match_fail value = [("Nil",0)-pair]
bt: a_lines_1 ← a_line_2 ← emit_thunks_asm_4 +1148
```
= **emit_thunks_asm の実行中に、a_line の第1引数(AS状態のはず)が
("Nil",0) ペアに化けている。**
つまり OUR codegen が生成したコードで、スレッド化された AS 状態と
タプル値の frame slot が衝突している。

原因の特定方法(次セッション):
1. dcc_2.s で fn_emit_thunks_asm_4 を逆アセンブル
2. +1148 バイト位置(クラッシュ呼び出し元)周辺の命令列を抽出
3. 各 ldr/str の slot オフセットを .s 上の行と対応させ
   「どの変数の slot が上書きされたか」を特定
4. 該当する ce アーム(c_binop/EApp/ELet のいずれか)の
   spill slot 計算を修正

### B-4 新事実(セッション12・最終): クラッシュはレキサ内部でも発生
dcc_2dbg 内 emit_code 実行時の write_file デバッグ:
- dbg_st1(show st1) = 空
- dbg_pre(show all_defs) = 空 ← この時点で all_defs が空!
- dbg_post(show (map_gname2 all_defs)) = [x] ← 後では非空!
- 順序逆転 or 値の取り違えの可能性大。
  検証: 各 write_file の直前で length を別ファイルに書き分ける等、
  書き込み内容に識別子を埋め込んで追跡する

### B-4 セッション30の逆アセンブル完全追跡結果
fn_emit_thunks_asm_4 の全命令列をダンプし、実行フローを確定:
1. env チェーン(ld_prev): slot0=t(AS), slot1=cid_tbl, slot2=ar_tbl ✓
2. defs bind @slot3 ✓、case defs → Cons 分岐 ✓
3. d_name3(d) → "_thunk_x" 文字列構築(++ = dc_bi_str_concat)✓
4. d_rhs3(d) / strip_lams 呼び出し(gget 366/373 経由)✓
5. `len pats != 0` の != 比較(val_eq 方式)✓ 正常に動作
6. その後の分岐で PTup 検査(#770)が現れる — **これは
   `let (r, n) = alloc_seq lst 100 []` 形式のタプル脱構築**
7. 問題の本質: OUR codegen は「関数呼び出しの戻り値タプルを
   PTup で受ける」パターンで、callee の返したタプル block の
   hdr 検査は正しいが、その後の要素 bind で slot 衝突が起きうる

次の一歩(具体的): 上記 6 のコード(+312-352 領域)で
fld の reload base slot(sv)と要素 bind slot(ns+1..)の衝突を
番台表で確認する。cm_asm PTup/PCtor は sv=ns を使い
fld は ns+1 から bind するよう修正済みだが、
emit_fn_level の ld_prev 領域(slot 0..np-1)との
重複チェックはまだ。

### B-4 セッション29の実験結果: マーカー版でも再現(デバッグ可能に!)
emit_thunks_asm に分岐マーカー([T-one]/[FUNC])を追加した dcc_2m でも
同一クラッシュを確認。重要な観測:
- m.dc(`let x = 5`)は値定義なので **ELSE 分岐(値定義パス)**を通る
- クラッシュは a_line 呼び出し系列内で発生
- [FUNC](関数定義パス)には到達しない
→ 値定義のコンパイル路径中で AS 状態が破壊される。
デバッグ版バイナリ dcc_2m が /tmp にあれば使える。

### B-4 最終状態(セッション20)
- 重要修正の蓄積により C5x(Gen2 版フルコンパイラ)は m.dc を正常処理する
  ようになった(以前は失敗)。== 構造的等価/go_cases reload/PStr val_eq/
  ローカル rec グローバル方式の積み重ねの効果
- 残るのは dcc_2(OUR codegen 版)のみ。トレース比較:
  C4x は gget(>=40) が僅か、dcc_2 は 288 回(異なるプログラムなので単純比較不可)
- dcc_2 の失敗モード: (Nil,0) match_fail もしくは lam 内 pc=1 クラッシュ
  (レイアウト依存・非決定性)
- 完遂への残タスク: ce の ELet/ECase/ELam/ETup/EApp アームを対象に、
  「生成コード上の各 slot 番号への WRITE と READ の対応表」を
  小さい入力(lam.dc)について完全に作成し、READ 先が未対応 WRITE の
  場所を特定する。デバッグ支援として ce 各アームに
  .s コメント出力(APPSPILL 等を実装済み・無効化済み)を再利用可能

### B-4 デバッグ教訓(セッション19)
- lldb の address BP は「set できるが発火しない」ことがある(error 9 再試行でも
  数回に 1 回しか当たらない)。bp を当てたい場合は dc_bi_read_file で一度止めて
 から delete→address bp→continue の 2 段階方式(それでも flaky)
- print デバッグは本バグをマスクする(GC/alloc タイミング変化のため使えない)
- 有効だった手段: objdump 全体ダンプ、DACELO_TRACE_CTOR/EQ/GGET、
  レジスタダンプ、メモリダンプ(x0 経由の 1 段デリファレンス)

### B-4 追加事実(セッション21)
- クラッシュ値は常に ("Nil",0) = ct_ar_tbl[0]。hdr=0x302 として正しく
  match_fail する(マッチャは正しく動作、引数が間違っている)
- destructure 順序(all_defs を先に)でも再現 → x0 汚染では無い
- スタンドアロン再現(3-tuple+case 脱構築+accessor)は dcc_1 経由で成功
  → フル文脈(build_globs のローカル rec 等)との組合せで発生
- 最有力: build_globs 内ローカル rec(alloc_seq/filter_nonzero)の
  グローバルスロット方式と、ECase base-spill の相互作用。
  filter_nonzero の gslot 自己呼び出し時に sc/base slot が
  変わることで外側リスト要素の読み先がずれる可能性

### 📌 次セッション向けの最短検証手順(B-4)
1. lldb(breakpoint set -n dc_bi_read_file → delete → address bp 方式)で
   fn_d_rhs3_1 エントリで停止し x0 をダンプ:
   - 正常系(C4x): Def3 ブロック hdr=0x503
   - 異常系(dcc_2): 自クロージャ(hdr=0x304)または ("Nil",0)
2. 同様に fn_emit_fns_asm_4 エントリで x1(defs リスト)を辿り、
   最初の要素の hdr/cid を確認
3. リストの「要素」が既に closure なら、all_defs 構築段階
   (flatten_groups2 もしくは main の case 脱構築)で汚染。
   要素が正しく Def3 なのに d_rhs3 内で化けるなら、
   emit_fns_asm の frame slot 計算が原因。
4. rt.c の DACELO_TRACE_GGET/EQ トレースと併用可能

### 🔴 セッション16の決定的発見: d_rhs3 が「自分自身のクロージャ」を受ける
lldb(d_rhs3_1 エントリで x0 ダンプ)により:
```
x0 = クロージャ { hdr=0x304, code=&fn_d_rhs3_1, nenv=0 }
```
= **d_rhs3 の引数に、d_rhs3 自身のレベル1クロージャが渡されている!**
(実行ごとに別の不正値になる場合も: 例 ("Nil",0))
→ all_defs のリストに、Def3 値ではなく「gget で取り出した関数クロージャ」
  が混入している。原因候補:
  a) flatten_groups2/build_tables 経路でリスト要素と関数値の取り違え
  b) thunk 初期化時に gset した slot と all_globs の対応ずれ
     (ローカル rec の 30000+uid スロットとは別系統)
  c) emit_fns_asm の再帰で rest/all_defs を取り違える箇所
※ 同一ソースでも Gen2 版(C5x)は正常。OUR codegen 固有。
※ 非決定性: 実行によって (Nil,0) と クロージャ が変わる

### 🎉 セッション15: パース成功を確認。失敗箇所は emit 期の d_rhs3 へ移動
- dc_match_fail bt: fn_d_rhs3_1 ← fn_emit_fns_asm_4 ← fn_emit_code_5 ← fn_main_1
- = パース完了後、emit_fns_asm 内 strip_lams(d_rhs3 d) で
  Def3 以外の値が d に入っている(match_fail)
- build_tables が 3-tuple を返し main が 3 回 case 脱構築する経路で
  all_defs が壊れている疑い。ETup(cnt=3, hdr 0x402)構築と
  PTup-3 検査(同値)、go/spill の slot 表を監査する
- ローカル let rec のグローバルスロットは 30000+a_fresh_id 方式に修正済み
  (旧 a_nglob 方式は指数増殖して dc_global_table 越え write fault)

### 🎉 セッション14の大進展: パースは成功するようになった!
ELet isrec を「新規グローバルスロット + dc_gset/dc_gget」方式で実装した結果:
- 旧: "expected top-level item"(パース失敗)
- 新: パース成功! 失敗箇所が build_globs フェーズに移動
- 新エラー: `non-exhaustive pattern match on (Nil,0)`
  = filter_nonzero 内 `case kv of | (nm,ar) -> ...` が
  ct_ar_tbl の先頭要素 ("Nil",0) で失敗
- 同種の最小構成(fnz.dc)は dcc_1 経由なら正常 → ネスト文脈でのみ発生
- 切り分け: build_globs 内ローカル let rec(alloc_seq/filter_nonzero)が
  新グローバル方式で動き、その引数タプル列 ct_ar_tbl の要素を
  内側 case する 2 重ネスト。sv/bind slot の衝突監査を
  cm_asm PTup(ns=cur, sv=ns, fld bind start ns+1)と
  外側 ECase(base=ns, binds ns+1)の組み合わせで実施すること

### 🎯 B-4 決定的データ(セッション11): 完全なワークアラウンド発見
- **`DACELO_TRACE_GGET=1` を付けて dcc_2 を実行すると正常動作する!**
  (stderr への fprintf がアロケーション/タイミングを変化させ、
  偶発的メモリ破壊を回避する)
- 検証: `DACELO_TRACE_GGET=1 ./dcc_1 dcc_full_latest.dc dcc_2` →
  `./dcc_2 examples/X.dc out` で生成バイナリが正しく動作
- 根本原因: レイアウト依存のメモリ破壊(タイミング/アドレス敏感)。
  DACELO_NO_GC=1 でも再現することから GC 頻度ではなく
  bump アロケータのアドレス配置が影響
- 恒久修正の方向性:
  a) rt.c dacelo_alloc のチャンクサイズ/初期配置を変えて再現性を確認
  b) 全フレーム slot のゼロ初期化(Gen2 prologue で sub 後に memset 相当)
     → 未初期化読み出しの影響を排除できる
  c) ce/c_binop/cm_asm の spill/reload slot 番台表の完全監査

### B-4 決定的データ(セッション11): 同一ソース・同一 pitems コードで挙動差
- C4x = Gen2 が「コア+ドライバ補助+emit系+tiny main」をコンパイルしたもの
  → m.dc を正常パース ✓
- C5x = Gen2 が「同一ソース + 実際の staged main」をコンパイルしたもの
  → m.dc パースで "expected top-level item" ✗
- **両バイナリの fn_pitems_3 の命令列は完全に一致**(objdump diff 0 バイト)
- C5x は parse 中に dc_val_eq を実際に呼んでいる(L20__2 = mem_str 等から)
- → 差異は pitems 以外: レキサのトークン生成 or グローバル初期化順。
  次セッション: DACELO_TRACE_EQ=1 + DACELO_TRACE_GGET=1(rt.c 実装済み)を
  C4x/C5x 両方で取得し、比較シーケンスの最初の分岐点を特定する

### (旧)B-4 クラッシュの正体(セッション9特定)

### (旧)B-4 クラッシュの正体(セッション9特定)
dcc_2 の lam1615(+264) クラッシュ:
```
ldr x0,[sp,#56]   ; slot56 を「関数」として
ldr x9,[x0,#8]    ; code ptr 読み込み → 値が 1
blr x9            ; PC=1 で SIGBUS
```
slot56 の中身は長さ1の文字列ブロック([+8]=LEN=1)。
「文字列値」と「関数値」が同一 slot に混在 = ETup 構築/EApp spill の
slot 計算がどこかでずれている。lam1615 は capture2+param1
(slot0=param, slot8/16=env)で、クラッシュ領域は slot48-80 を使う
ETup 構築+EApp 連鎖。次は ELam の env ロード開始 ns2 と body への
ns 引き継ぎ、ETup go ループの ns+idx が env slot と衝突していないか
番台表で確認する。

### (参考)B-4 ランタイムトレース結果(セッション7)
rt.c の dc_val_eq に DACELO_TRACE_EQ トレース追加済み。
dcc_2 が m.dc(`let x = 5`)を parse した際の比較列:
```
[eq 0x29 vs 0x81][eq 0x29 vs 0x25][eq 0x29 vs 0x29]   ← 字句解析の文字比較
[eq -3 vs -3]                                          ← sget 終端 c==-1 ✓
[eq static-ptr vs heap-ptr]                            ← ★最初の PStr 比較
→ 直後に fn_pitems_3+688 の ldr x9,[x9] で SIGSEGV(slot 内容破壊)
```
- 字句解析は完走している(rev_acc まで到達)
- クラッシュは TKw ブランチの hdr 再読み込み([sp,#24] の内容が無効化)
- → PStr 用の spill slot(sv=ns)と、その後のパターン bind slot の衝突、
  もしくは dc_val_eq 呼び出し前後で x9/x0 を経由する値のロスト
- 検証手順: dcc_2.s の当該関数を命令カウント(データ指示行はスキップ)で
  オフセット特定し、spill/reload の slot 番号突き合わせ

### (旧)B-4 決定的怪奇現象(セッション6最終)
- nm 上の _fn_pitems_3=0x10000b8d0、スライド=0確認済み
- にもかかわらず `bp -a 0x10000b9e8`(Cons hdr cmp のアドレス)は
  一度もヒットせずプロセスが error 終了する
- 一方 dc_fatal での bt は `fn_pitems_3 + 1896` を示す(関数は実行されている)
- → +280 に制御が到達していない。分岐構造上ありえないはずだが…
  可能性①: 複数世代の同名関数があり実行中のは別コピー
  可能性②: エントリから途中へジャンプする何か(未発見)
  ※ dcc_2.s を残してあるので命令オフセット照合は可能
  ※ lldb の address BP が当たらない場合、rt.c の dc_gget 先頭に
    「slot 番号と戻り先 lr を stderr 出力する」一時コードを入れて
    制御フローを記録するのが代替手段

- 空ファイル(トークン0個)は pitems の [] ブランチを通過(その後 main 無で
  Bug-Y: gget(-1) クラッシュ)。**非空リストの Cons 検査だけが実行時に失敗**
- Bug-Y は治済み: emit_code 内で assoc_ikey "main" == -1 なら user_main を
  即 ret として生成(スタイル: ステージ分割版ドライバ)
- ドライバは load_prog/build_tables/build_globs/emit_code/main の
  ステージ分割済み。これでも直らない → 巨大関数説は否定
- 静的検証では Cons 検査列(hdr 1027/cid 1/elem0+16/hdr771/cid6/PStr val_eq)
  は全て正しい。よって実行時値の差異:
  確認すべきは dcc_2 ヒープ上の「KW トークンの cid が本当に 6 か」。
  reg_all の登録順と zip_ids2_d の ID 採番、および ctor partials の
  fill_pc ループが同じ表を回っていることは確認済み
- lldb ではローカルシンボル(_fn_*_N)への BP がスライドのため当てにならない。
  dc_gget 等グローバルシンボルで止めてからレジスタ辿るのが現実解

### (旧)B-4 デバッグの最新事実(セッション5終了時)
- 「print 1 個で治る」は再現性あり。V1(fix+print)=OK, V2(no-fix no-print)=OK,
  plain(fix, no-print)=NG → print の有無と PStr fix の有無の**組み合わせ**で
  挙動が変わる = 純粋なコード配置/アドレス依存
- 現行 dcc_2 の `_fn_pitems_3`(objdump)は関数冒頭が
  `adrp x9, _astrNNN; cmp x0, x9` — **レベル関数の引数を静的文字列と直接比較
  している**ように見える(期待: env から toks を slot 経由でロードして Cons 検査)。
  ラベル/関数名の帰属ずれ or 本物の spurious パターン検査のどちらか。
  → dcc_2.s(残る)の該当行と、Gen2 版(dcc_1 バイナリの objdump)を見比べて
  「どのソース式がこの命令列に対応するか」を確定させるのが最短ルート
- テスト用に dv1/dv2(動作する第2世代コンパイラ)を /tmp に作成済み。
  dcc_full_latest.dc を変えたら必ず dcc_1 → dcc_2 を作り直すこと

### (参考)旧B-3記述: dcc_1 の自己コンパイルが無限再帰で SIGKILL
- `./dcc_1 gen3-dcc-dc/dcc_full_latest.dc dcc_2` が ~40s 後にメモリ肥大で
  OS kill(exit=137)。sample ではラムダレベル関数 L296(capture 11 個)が
  自己再帰し続ける
- ただし core(dcc+pm+ce ≈1900 行)の自己コンパイルは数秒で成功。
  interp.dc(940 行)も成功。毒は g3_driver_v2.dc 内
- 対策候補: ドライバ main を load/build_tables/emit/link 複数トップレベル
  関数へ分割(main 内ローカル let rec を排除)
- 注意: dcc_1 呼出は `dcc_1 <入力> <出力ベース>`(-o フラグ不要)
- 症状: `./dcc_1 examples/fib.dc <任意名>` で cc#1 が ".s 無い" エラー。
  tree/list_ops/closures/hello は全て通る。interp-path は fib 含め 5/5
- lldb ブレークポイント順序の事実:
  1. dc_bi_read_file(fib.dc 読み)ヒット
  2. dc_bi_read_file もう一度(ドライバの readback)ヒット ← この時点で .s 存在?
  3. dc_bi_write_file は【一度も呼ばれない】
  4. libc system() ×3 呼ばれる(cc#1 だけ失敗)
- つまり Gen2 コンパイル版では write_file の組込みディスパッチが
  別関数に化けている可能性(BI インデックスとスロット対応ずれ?)
- 次の手: ドライバ main を load/emit/link の複数トップレベル関数に分割し
  巨大単一関数を避ける。あるいは Gen2 側 EApp/Seq の評価順を検証
- ※ make_string NUL 不足(len%8==0 で終端無し)は本セッションで修正済み
  (fopen が隣ブロックヘッダを読む bug)。これで tree は治った

### B-2'. dcc_1(Gen2 コンパイル版 dcc)だけ文字列パスが破壊される
- 症状: `read_file "examples/tree.dc"` が cannot open / write_file 先が消える
- 同一ソースを Gen0 インタプリタで実行すると発生しない
- B-1'' と同根の可能性大(dcc 自身が内部で HOF/部分適用を多用するため)
- ※ 以前観測した「out_base 名依存」はテストハーネスの head/grep パイプによる
  SIGPIPE という偽物だったことが判明。実害はこの文字列破壊

### C. ELet isrec=true 未対応(現状 st 返すだけ)

## 今セッションで修正したバグ一覧(g3_ce_v2/g3_pm_v2/dcc.dc/rt.c)
1. c_binop "+": タグ付き加算は `(a+b)-1` が正解(lsl#2 での retag は誤り)
2. c_binop "-": 右オペランド格納が x1 だった → x0。かつ untag不要
   (`t(a)-t(b)=(a-b)<<2` なので sub 後 `add #1` のみ)
3. 比較 `< > <= >=`: ブール生成のラベル順序が逆で常に true になっていた
4. fv_expr を bound-set 方式に全面書き直し(ネスト ラムダの fv 計算)
5. ELam アーム: caps を (名前, slot) ペアに統一&グローバル除外。
   ★最重要★ ラムダ本体コードがインライン直列に展開され fall-through して
   いた → `b Lskip` + `Lskip:` で out-of-line 化
6. emit_fn_level: レベル≥2 で env チェーン([x0+24..])をフレームにロード
   (多段カリー化の基盤)
7. go_cases: scrutinee のレジスタ番号 1 → 0
8. g3_pm PCtor/PTup: ヘッダ/cid 比較が x0 vs x10 だった → x9 vs x10
9. rt.c dc_show: 静的 Nil インスタンス(__DATA)を improper-tail と誤判定し
   全リスト表示に ",[]" が付く → ADTLIKE マクロで修正
10. rt.c: dacelo_alloc の薄いラッパ dc_alloc_bytes 追加(dcc.dc 側で使用)


## アーキテクチャメモ (デバッグ用)

- AS 状態: 行リストは**逆順積算**(先頭追加)。全ての ce/c_binop/go_cases/
  cm_asm 呼び出しは必ず最新状態をスレッドすること (過去バグの温床)
- スロット割当は ns カウンタ (frame 内 sp 相対、8*slot)。ラベル ID には
  len(a_lines) を使用
- 呼出規約: x0=クロージャ(env)、x1..x7=引数、[x0+8]=code ptr。
  各レベル関数は自分の引数を常に **x1** で受ける
- クロージャレイアウト: [hdr@0][code@8][env_size@16][env words@24..]
  hdr = (words<<8)|tag, tag_closure=4
- 内部ラベルは必ず **L** 接頭辞 (Lelse 等)。_ 接頭辞は外部シンボル扱い
- データ終端/テーブル前の .p2align 3 忘れに注意 ("pointer not aligned" リンクエラー)
- エピロージェ後 ret を忘れず (落ちたら fall-through で dc_match_fail に入る)

## 実行環境メモ
- gen2 バイナリ名は `dcc` (dacelo ではない): gen2-dcc-rs/target/release/dcc
  usage: `dcc <file.dc> -o <output> [--run]`
- rt.c パスは driver 内 "gen2-dcc-rs/rt/rt.c" 固定 (リポジトリルートから実行)
- test.sh: gen3-dcc-dc/test.sh (現状 dcc_1 が crash するため FAIL 扱い)

## 検証コマンド(最新)
```
cat gen3-dcc-dc/{dcc,g3_pm_v2,g3_ce_v2,g3_driver_v2}.dc > /tmp/dcc_full.dc
cd gen0-interp-rs && ./target/release/dacelo /tmp/dcc_full.dc --types   # 型検査
./target/release/dacelo /tmp/dcc_full.dc ../examples/tree.dc /tmp/t && /tmp/t
```
現在の合格状況: interp-path で hello/fib/tree がリファレンス一致。
list_ops/closures は B-1''、dcc_1 経路は追加で B-2' を抱える。

## 2026-09-05: B-4 解決 + self-hosting fixpoint 達成 (dcc_2/dcc_3 完全動作)

### B-4 (dcc_2 が 10B の .s しか出さない) の原因と修正
- 直接原因: `g3_pm_v2.dc` cm_asm の nullary-ctor 分岐が `adrp/ldr x9` で
  scrutinee (x9, ネスト時) を潰し `cmp x9,x9` が恒真 → `only :: []` が
  全リストにマッチし先頭行だけ返していた。PUnit 同様 x10 使用に修正。
- 検証: 修正後 dcc_2.s に `cmp x9, x9` が 0 件、`cmp x9, x10` に正規化。

### 第二乖離 (~81 slot-only hunks + dcc_3 SEGV) の原因と修正
- nested `let rec` (cm_asm の PTup/PCtor 両 `fld`、ETup の `go` 等) が
  ELet-true の global slot 機構で解決されるため、同名・同ノードの
  ネスト起動が global cell を上書きし、外側の再帰呼び出しが内側
  クロージャに誤解決 (`only :: []` 類似の系統的 +1/+2 slot ずれ)。
  加えて pick/ld_env 等の capture が target-scope 依存で stale になり得る。
- 修正 (lambda-lifting, いずれもセマンティクス保存の純粋な引数化):
  - g3_pm_v2.dc: 両 `fld` を top-level `fld_ptup`/`fld_pctor` に昇格
    (sv, cid_tbl, ar_tbl, flbl を明示param化、9引数カリー化)。
  - g3_ce_v2.dc: `go_br`→`go_br_fv` (+bound)、`pick`→`pick_sc` (+sc)、
    `ld_env`→`ld_env_ns` (+ns2)、ETup `go`→`go_tup` (+ns,cid_tbl,ar_tbl,sc)。
  - g3_driver_v2.dc: `ld_prev`→`ld_prev_np` (+np)。
  - dcc_full_latest.dc を連結再生成。
- 結果: `diff dcc_2.s dcc_3.s` が 336行 → 40行 → **0行 (バイト一致)**。
  dcc_3 が 5 examples 全 PASS + v6 一致。test.sh 5/5 PASS。

### 既知の残存潜在バグ (別件, 未修正)
- t_overwrite 型 (同名 nested let-rec + activation毎に異なる capture 値):
  interp は 200 400 だが全コンパイラ (Gen2/dcc_1/dcc_2/dcc_3) が 300 400。
  ELet-true global 方式の lexical-unsoundness が根因。
  本格修正は ce-ELet-true の frame-slot 化+backpatch (要設計・回帰注意)。
  self-hosting fixpoint には影響なし (examples に該当形状なし)。
- `go` (emit_ctor_names 内), `emit_levels` は capture ありだが逐次・pure
  のため現状問題なし。挙動変化が出たら同様に lift すること。

## 2026-09-05 (続): ce-ELet-true の lexical 化 + t_overwrite 修正

### 残存潜在バグの本修正 (frame-slot lexical rec + backpatch)
- t_overwrite 型 (nested same-node let-rec、activation 毎に capture 値が
  異なる) が interp と不一致 (200 400 vs 300 400、全コンパイラ) の根因:
  ce の `ELet true` が再帰名を GLOBAL slot (30000+uid) に束縛していたため、
  ネスト起動が同一 static cell を上書きし、外側の再帰呼び出しが内側
  クロージャに誤解決 (lexically unsound)。
- 修正 (g3_ce_v2.dc の ELet-true 分岐のみ):
  (1) frame slot 予約 + scope 登録、(2) rhs をその scope で展開
  (囲み frame の値は env にコピーされる)、(3) closure を slot に格納、
  (4) ELam rhs 時の capture 順 (pick_sc 再計算) から self の env index を
  求め backpatch (`str x9,[x0,#24+8*idx]`)。非 ELam rhs・非自己参照は
  patch 不要パス。body は frame束縛 (shadowing 正常) で展開。
- top-level `let rec` (emit_fn_level/thunk) は不変。Gen2-Rust は不変
  (bootstrap のみ、dcc_1 は新 ce 論理で動くため影響なし)。
- 検証: t_overwrite が dcc_1/dcc_2/dcc_3 全てで 200 400 (interp 一致)。
  t_shadow2 (旧 SEGV) を含む全ミニテスト PASS。examples 5/5 PASS (×3世代)。
  test.sh 5/5 PASS。`diff dcc_2.s dcc_3.s` は 0行のまま (fixpoint 維持)。
