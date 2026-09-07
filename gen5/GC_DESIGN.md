# Gen5 メモリ管理・所有権型システム再設計（Gen5-RC）

作成 2026-09-08。対象: `gen5/`（dacelo 製チェッカ＋コンパイラ）。破壊的変更。
Gen3/Gen4 系譜（`gen3-dcc-dc/`, `gen2-dcc-rs/rt/rt.c`, `dcc_1`）は seed として**無改変**で残す。

## 0. 結論（何を採用するか）

完全型推論（HM＋let多相、注釈なし）の言語に合う「最先端」として、
**Perceus 系の精密参照カウント（garbage-free RC）＋ 注釈なしの借用推論 ＋ drop-guided reuse（FBIP）**
を採用し、mark-sweep（保守的スタック走査）を**廃止**する。

| 研究 | 採用する部分 | 採用しない/延期 |
|---|---|---|
| Tolmach 1994 tag-free GC | 「型で走査を誘導」の代わりに、ヘッダの tag／レイアウト表で精密に子を辿る（保守的走査ゼロ）。型パラメータの実行時受け渡しは不要（値は既に odd=即値／even=ポインタで自己記述） | 型記述子の受け渡し |
| Goldberg–Gloger 1992 | — | GC 時の型再構築（tracing 自体を捨てるため不要） |
| Tofte–Talpin / MLKit / Elsman 2023 | 「寿命を推論で決める」発想は借用推論として取り込む | region 推論（寿命の粗い併合、GC 併用時の安全性問題） |
| **Perceus 2021** | dup/drop の自動挿入、所有権渡し（owned by default）、早期 drop、**drop-guided reuse**（`case` で消費した unique セルを同サイズ生成に再利用＝関数型のまま in-place 更新） | 循環参照の回収（本言語は不変データ＋局所 `let rec` の自己参照のみ。自己参照はランタイムで non-owning edge として扱う） |
| Lean 4 “Counting Immutable Beans” 2019 / Koka | **借用パラメータ推論**（一階のトップレベル関数について、消費されない引数を borrowed にし、呼出側の dup と被呼出側の drop を省く） | — |
| **Morphic 2026** | 「注釈なし・拒否せず RC にフォールバック」という設計原則。借用の対象を直接呼出（既知関数の飽和適用）に限定し、それ以外は所有権渡し | 寿命変数つき借用型、lambda set specialization（高階の一階化）は後続 |

併せて **既知トップレベル関数への飽和直接呼出（uncurrying）** を導入する。現行バックエンドは全関数を
arity-1 クロージャ連鎖で呼ぶため（1 回の `f a b` で中間クロージャを確保）、これがないと
「引数を借用する」余地が存在しない。

## 1. 現状（Gen3 バックエンド＋rt.c）の事実

- 値: Int `(n<<2)|1`、Bool 3/7（odd）。それ以外は 8 バイト整列ポインタ（even）。
- ブロック: `[hdr][payload]`、`hdr = (size_words<<8) | (mark<<7) | tag`。
  tag: 1 String `[hdr][len][bytes…]`、2 Tuple `[hdr][e1..en]`、3 ADT `[hdr][ctor_id][f1..fn]`、
  4 Closure `[hdr][code][env_size][env…]`、5 Unit（静的 `dc_unit_block`）。
- 静的データ: 文字列リテラル（`_astrN`）、nullary ctor（`_ctorinst_C`）は `.data` にコンパイラが直書き。
- 呼出規約: x0=クロージャ, x1=引数, `ldr x9,[x0,#8]; blr x9`、結果 x0。フレームは固定 4032B、
  すべての中間値を `[sp,#8*slot]` に spill。評価順: 関数式→引数。
- 多引数関数 `let f p1..pk = body`: `_fn_f_i`（i<k）は x1 を env に入れた次レベルのクロージャを返し、
  `_fn_f_k` が本体。局所 `let rec` は frame slot ＋ env への自己ポインタ backpatch（**自己参照サイクル**）。
- rt.c: `dacelo_alloc`（bump＋first-fit free list、閾値で `gc()`）、mark-sweep、根＝グローバル表（精密）＋
  スタック保守的走査（`hdr_plausible` で誤マーク緩和）。組込みは `dc_bi_*`、部分適用は
  `dc_builtin_step`/`dc_ctor_step` トランポリン、グローバルは `dc_gset/dc_gget`、初期化は `_dc_init`
  （ctor 登録→組込みクロージャ→部分 ctor→thunk 順呼出）→`_dc_user_main`。
- **現行 GC は実質一度も走らない**: `live_bytes` は sweep でしか更新されず（初期 0）、トリガ条件
  `live_bytes + nbytes > next_gc` は 8MB 超の単一要求でしか成立しない（rt.c `dacelo_alloc`）。
  したがって全プログラムはヒープを 8MB chunk で伸ばし続けるだけで、`gc_stress.dc` も GC なしで通っている。
  自己検査（7000 行）の RSS 25GB 超は「総確保量」そのもの。RC 化で初めて実際にメモリが回収される。

## 2. 新しい値表現とランタイム ABI（`gen5/rt5.c`）

### 2.1 ヘッダ

```
hdr = (rc << 40) | (size_words << 8) | tag        rc: bits 40..59 (20 bit), bits 60..63 = 0
RC_IMMORTAL = 0xFFFFF   (静的データ／飽和カウント: dup/drop とも何もしない)
tag: 1 String, 2 Tuple, 3 ADT, 4 Closure, 5 Unit, 0x7E Freed(デバッグ検出用)
SIZE_OF(h) = (h >> 8) & 0xFFFFFFFF, TAG_OF(h) = h & 0x7F, RC_OF(h) = (h >> 40) & 0xFFFFF
```

- Dacelo の Int は 62 bit なので、コンパイラが生成する定数は 2^61 未満に収める（rc=1: `1<<40`、
  immortal: `0xFFFFF<<40 = 1152920405095219200`）。
- パターン照合のヘッダ比較は下位 40 bit をマスクしてから比較する（`and x9, x9, #0xffffffffff`）。
- 即値（odd）と immortal は参照カウント対象外。オーバーフローは immortal に飽和（sticky）。

### 2.2 エクスポート関数（生成コードが呼ぶ）

| C シグネチャ | 意味 |
|---|---|
| `value dc_alloc_rc(uint64_t nbytes)` | 新ブロック。ヘッダは**呼出側が書く**（`(1<<40)|(words<<8)|tag`） |
| `void dc_dup(value v)` | rc++（即値・immortal は no-op、Freed なら fatal） |
| `void dc_drop(value v)` | rc--、0 で子を drop して解放。**反復（worklist）**でスタックを使わない |
| `value dc_drop_reuse(value v)` | v がヒープで rc==1 なら子だけ drop しブロックを返す（ヘッダは size/tag/rc=1 のまま）。それ以外は `dc_drop` して 0 |
| `value dc_reuse_or_alloc(value tok, uint64_t nbytes)` | tok≠0 かつサイズ一致なら tok、さもなくば（tok を解放して）`dc_alloc_rc` |
| `void dc_gset(uint64_t i, value v)` / `value dc_gget(uint64_t i)` | 表が所有権を取る／借用で返す |
| `dc_register_ctor`, `dc_mk_partial_builtin`, `dc_mk_partial_ctor`, `dc_match_fail`, `dc_val_eq`, `dc_show`, `dc_argv`, `dc_div_check`, `dc_unit_block` | 現行と同名・同意味 |
| `value dc_bi_<name>(value …)` | 18 組込み全部を同一命名で直接呼出可能に（`error`/`exit`/`show`/`argv`/`system` も `dc_bi_` 別名を用意）。**引数は全て借用**、結果は新規 owned か即値か immortal |
| `void dc_rt_shutdown(void)` | `_dc_user_main` 後にランタイムが呼ぶ。`DACELO_RC_CHECK=1` なら全グローバルを drop し live blocks が 0 でなければ `dacelo: LEAK n blocks` を stderr に出し exit 3。`DACELO_RC_STATS=1` なら allocs/frees/reuses/peak を stderr に出す |

### 2.3 所有権規約（ランタイム側）

- クロージャ呼出 `code(clo, arg)`: `clo` は**借用**（呼出側が呼出中も参照を保持）、`arg` は**被呼出側が所有**。
- `dc_ctor_step` / `dc_builtin_step`: `arg` を所有。env から複製する既存引数は `dc_dup`。
  `dc_builtin_step` は組込み本体（借用）を呼んだ後 `arg` を drop。
- `dc_gset(i, v)` は v の所有権を表に移す（グローバルは不滅）。thunk の結果はそのまま渡す。
- 解放時、クロージャ env 内の**自分自身へのポインタは飛ばす**（局所 `let rec` の自己参照は non-owning）。
- 解放したブロックの tag を `0x7E` にし、dup/drop/drop_reuse で検出したら `dacelo: use after free` で abort。
- free list はサイズ別（2..64 words は配列、超過は一般リスト）。chunk（8MB）から bump 確保。GC は存在しない。

## 3. 所有権 IR（`gen5/g5_oir.dc`、共有定義）

legacy `Expr`（dcc.dc）→ ANF 形式の `OExpr`。全ての中間値は変数（frame slot）に束縛される。
定義は `gen5/g5_oir.dc` を正とする（下は要約）。

```
OAtom : AVar x | AInt n | ABool b | AStr s | AUnit | AGlob name | ANull ctor
OExpr : -- prim 系（1 回の確保または呼出、結果は owned）
        PAtom atom | PApp f arg | PCall fname [atoms] | PBi bname [atoms]
      | PMkCtor cname [atoms] tok | PMkTup [atoms] tok | PLam pat body [caps]
      | PBin op atom atom | PBlock e
        -- 式系
      | ORet atom | OLet x prim e | OLetRec x prim e | OIf atom e1 e2
      | OCase atom [(Pat, e)] | ODup x e | ODrop x e | ODropReuse x tok e
```
（Gen0 は `type … and …` も前方参照する型宣言も受理しないため prim と式は 1 つの型に統合。
`PCtor`/`PTup` は dcc.dc の `Pat` コンストラクタと衝突するので `PMkCtor`/`PMkTup`。）

### 3.1 位置ごとの所有権モード

| 位置 | モード | 生成コードの責務 |
|---|---|---|
| `ORet (AVar x)` | 消費（owned） | x は owned でなければ事前に `ODup` |
| `PAtom (AVar y)` | 消費 | 同上 |
| `PApp f a` | f: 借用、a: 消費 | f が owned で最後の使用なら呼出**後**に `ODrop f` |
| `PCall g args` | g のシグネチャ（`List Bool`, true=owned）に従う | 借用引数が owned 最終使用なら呼出後に drop |
| `PBi name args` | 全て借用 | 同上 |
| `PMkCtor`/`PMkTup` フィールド、`PLam` キャプチャ | 消費 | 同一 prim 内で同じ変数が複数回消費されるなら 1 回を除き `ODup` |
| `PBin` オペランド、`OIf` 条件、`OCase` 被検査値 | 借用 | `OCase` の被検査値 x が owned なら分岐内で `ODrop x`（未使用時、分岐先頭）または `ODropReuse` |
| `OCase` のパターン変数 | x が owned: 分岐先頭で使用分を `ODup` して owned 化 / x が借用: 借用のまま | 束縛は field load のみ（カウント操作なし） |
| `PLam` 本体 | パラメータ owned、キャプチャ変数は借用（x0 クロージャから）、グローバルは `AGlob` | |
| `AGlob`（グローバル） | 借用（表が 1 参照を保持） | **消費位置**（`ORet`/`PAtom`/`PApp` 引数/owned `PCall` 引数/フィールド）では codegen が `dc_gget`＋`dc_dup` を出す。借用位置は `dc_gget` のみ。immortal 静的（`AStr`/`ANull`/`AUnit`）は dup 不要 |
| `OLetRec f (PLam …)` | 自己キャプチャは dup しない・backpatch | ランタイムが解放時に自己ポインタを無視 |

### 3.2 借用パラメータ推論（Lean/Koka 流、一階）

対象: トップレベル関数 `let f p1..pk = body`、1 ≤ k ≤ 8（直接呼出可能）。k>8 の 8 関数と局所関数は
全 owned（クロージャ経路のみ）。

1. 全パラメータを borrowed で初期化。
2. body（ANF）中で p が**消費位置**に現れれば owned: `ORet`、`PAtom`、`PApp` の引数、`PCall` の owned 位置
   （現在の推定値）、`PMkCtor`/`PMkTup` フィールド、`PLam` キャプチャ。加えて、p が `OCase` の被検査値で
   分岐の直線部に同サイズ生成があるなら owned（reuse を可能にするため）。
   パターン変数の消費は元パラメータの owned 化を要求しない（dup で済む）。
3. 全関数で不動点まで反復（単調: borrowed→owned のみ）。

推論結果 `g5o_sigs : List (String, List Bool)` はコード生成の呼出規約であり、`gen5check types --format=json` の
`"ownership"` フィールドとしても公開する（RFC「型が見える」の延長: 注釈なしで推論した所有権を読める）。

### 3.3 Perceus 挿入（要点）

Δ=このスコープで**必ず 1 回だけ消費すべき** owned 変数集合、Γ=借用変数集合。
- `OLet x prim e`: Δ のうち prim にも e にも現れない変数は prim の前で `ODrop`。prim と e の両方に
  現れる変数は prim では借用扱い（消費位置なら `ODup`）、e が消費。x は e で owned。
- `OIf`/`OCase`: 各分岐で未使用の owned 変数を分岐先頭で drop。
- `PLam`: キャプチャは消費位置。本体は Δ=パターン変数、Γ=キャプチャ。
- 評価順は **ANF の束縛順＝実行順**（関数式→引数、左→右）なので、「最後の使用」はこの順で決める。

### 3.4 Reuse（drop-guided）

分岐先頭の `ODrop x`（x=被検査値）と、その分岐本体の**直線部**（`OLet`/`ODup`/`ODrop` を辿り、
`OIf`/`OCase`/`PLam` に入らない）にある最初の同サイズ `PMkCtor`/`PMkTup`（tok 未設定）を対にし、
`ODropReuse x tok` ＋ `PMkCtor … tok` に書き換える。直線部限定なので token が消費されない経路は無い。
サイズ: `PMkCtor` は 2+arity words、`PMkTup` は 1+n words。static/即値は `dc_drop_reuse` が 0 を返すため安全。

## 4. コード生成（`gen5/g5_cg.dc`, `gen5/g5_cgdriver.dc`）

- 関数 `f`（k=1..8）: n-ary エントリ `_fnn_f`: 引数 x0..x(k-1) をまずスロット 0..k-1 に保存し、
  各パラメータパターンを束縛（`cm_asm5`）、本体をコンパイル。結果 x0。
  カリー化レベル `_fn_f_i`（i<k）: 現行同様に x1 を env に積んだ次レベルクロージャを返す（env は**生の引数値**
  v1..vi、クロージャが所有）。最終レベル `_fn_f_k`: env から v1..v(k-1)（借用）、x1=vk（owned）を集め、
  owned 位置の env 値は `dc_dup`、`_fnn_f` を直接呼出、vk が借用位置なら呼出後 `dc_drop`。
  k>8: 最終レベルで env 値（借用）＋x1（owned）を束縛して本体を直接コンパイル（`_fnn_` なし）。
- 直接呼出 `PCall f [a1..ak]`: 各 atom をレジスタ x0..x(k-1) に載せ `bl _fnn_f`。
- `PApp f a`: `x0=f, x1=a, ldr x9,[x0,#8], blr x9`。
- `PBi name args`: `bl _dc_bi_<name>`（x0.. に引数）。
- `PMkCtor`/`PMkTup`/`PLam` 生成: `dc_reuse_or_alloc(tok|0, nbytes)` または `dc_alloc_rc`、ヘッダ `(1<<40)|(words<<8)|tag`。
- `ODup x`/`ODrop x`: `ldr x0,[sp,#slot]; bl _dc_dup|_dc_drop`。`ODropReuse x tok`: `bl _dc_drop_reuse; str x0,[tok]`。
- 静的データ（文字列・nullary ctor・unit）はヘッダ rc=immortal で出力。
- パターン照合のヘッダ比較は 40 bit マスク。ctor_id（blk[1]）比較は現行通り。
- フレーム 4032B（504 スロット）は据置き。ANF の一時変数で超過したらコンパイル時エラー（後で可変フレーム化）。
- `_dc_init`/`_dc_user_main`/thunk/ctor 名テーブルは現行と同じ手順（`dc_gset` は所有権移転）。
  `_dc_user_main` の後にランタイムが `dc_rt_shutdown` を呼ぶ。
- リンク: `gen5/rt5.c`。旧 `rt.c` は seed（dcc_1）専用のまま。

## 5. ブートストラップと検証

```
stage0: dcc_1 (Gen3, 旧ランタイム上で動く) が新ソースから gen5check / dcc_6 を生成   ← 生成物は RC コード
stage1: dcc_6 が同ソースから dcc_7 / gen5check_rc を生成（RC 上で動くコンパイラ／チェッカ）
stage2: dcc_7 が dcc_8 を生成、 dcc_7.s == dcc_8.s（不動点）
```

`gen5/test.sh`:
- A: 変更なし（38 ケース oracle。ただしチェッカ concat に g5_oir/g5_own を含め `types` の ownership を出す）。
- B: `dcc_6` で examples（hello/fib/list_ops/closures/tree/gc_stress）＋ gen5-examples を実行し **Gen0 と出力一致**
  （旧 B の「dcc_1 と .s バイト同一」は廃止）。全例を `DACELO_RC_CHECK=1` で **leak 0**、
  `DACELO_RC_STATS=1` で record_with / list-map 例に **reuses > 0**。use-after-free 検出が発火しないこと。
- C: 自己検査（gen5check）→ stage1/stage2 → 不動点。`gen5check_rc` の 38 ケース一致。
  自己検査の時間・peak RSS を mark-sweep 版と比較して記録。
- G（新設）: `probe_own.dc`（Gen0 実行）で ANF/借用推論/dup-drop/reuse の単体出力を固定。
  `types --format=json` の `"ownership"`。負例（借用引数を返す関数は owned と推論される等）。

## 6. 作業分割（並列）

| 担当 | 成果物 | 依存 |
|---|---|---|
| RT | `gen5/rt5.c`（§2） | なし |
| OWN | `gen5/g5_own.dc`（§3.2–3.4）、`gen5/probe_own.dc`、IR プリンタ | `gen5/g5_oir.dc`, `dcc.dc` の `Expr` |
| CG | `gen5/g5_cg.dc`, `gen5/g5_cgdriver.dc`、`g5cc_driver.dc` の接続 | `gen5/g5_oir.dc`, §2, §4 |
| 統合 | concat 順序・`test.sh`・ブートストラップ・RESUME/README | 全部 |

concat（compiler）: `dcc.dc g5_front g5_infer g5_query g5_lower g5_oir g5_own g5_driver g5_cg g5_cgdriver g5cc_driver`
concat（checker）: `dcc.dc g5_front g5_infer g5_query g5_lower g5_oir g5_own g5_driver g5_main`
（Gen0 は前方参照を許さないため、`g5_driver` が `g5o_sigs_of` を呼ぶ都合で g5_oir/g5_own は g5_driver より前）

自ソースは引き続き **Gen4 サブセット構文のみ**（seed 鎖のため）。`gen5/RESUME.md` の落とし穴に従う。

## 7. 延期事項

- Morphic 流の寿命変数つき借用・lambda set specialization（局所関数の直接呼出・借用）。
- 可変長フレーム、レジスタ割付、直接呼出の引数 9 個以上（現状 8 関数はクロージャ経路）。
- `.dci` への ownership 出力（ABI 互換性の意味論を決めてから）。

## 8. 結果（2026-09-08、branch `gen5-gc`）

| 検証 | 結果 |
|---|---|
| A. `gen5check check` vs Gen0 `--types`（38 ケース） | 38 agree（旧チェッカ・RC 版チェッカとも） |
| B. examples 6 本＋gen5-examples | 全て Gen0 と出力一致、`DACELO_RC_CHECK=1` でリークゼロ、`reuse` 例 reuses=1000 |
| C. 不動点 | `dcc_7.s == dcc_8.s`（5,853,382 B） |
| C. 自己検査（8,500 行） | RC 版: exit 0 / 46.7 s / 最大 RSS 1.84 GB。mark-sweep 版: 272 s 後 OOM kill（23.7 GB、footprint 140 GB） |
| D/E/F/G | green（G: `types` の ownership、`probe_own.dc` の IR dump 固定） |
| 即席性能 | fib 30: 0.46 s → 0.01 s（直接呼出＋クロージャ確保消滅） |

