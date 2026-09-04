# dacelo

Dacelo(ワライカワセミ属)— ML 系静的型付け関数型言語。自分自身で自分自身をコンパイルする(セルフホスト)まで世代を重ねて育てる。

## 特徴(設計方針)

- ML/Haskell 系構文:`let` / `case-of` / カリー化 / ADT + パターンマッチ
- Hindley-Milner 型推論による完全な静的型付け
- 最終形態は **ARM64 機械語の直接生成**(macOS Apple Silicon)
- mark-sweep GC によるメモリ管理

詳細は [DESIGN.md](./DESIGN.md) を参照。

## ロードマップ

| 世代 | 成果物 | 実装言語 | 状態 |
|---|---|---|---|
| Gen 0 | `gen0-interp-rs` — Rust 製ツリーウォーク インタプリタ | Rust | **完了** ✅ |
| Gen 1 | `gen1-interp-dc` — dacelo 製インタプリタ(dacelo 上で動作)| dacelo | **完了** ✅ |
| Gen 2 | `gen2-dcc-rs` — Rust 製コンパイラ(ARM64 直接コード生成)| Rust | **完了** ✅ |
| Gen 3 | `gen3-dcc-dc` — dacelo 製コンパイラ(自己コンパイル)| dacelo | **完了** ✅ |

## Gen 0 の現状

- 字句解析 → 再帰下降パーサ → Hindley-Milner 型推論 → ツリーウォーク評価
- 対応: `let` / `let rec`(相互再帰 `and`)/ `if` / `case-of`(ADT + パターンマッチ)/
  `fun` ラムダ / カリー化 / リスト・タプル / パターン let 束縛(`let (a, b) = e in`)/
  型注釈 / let 多相
- 組み込み: `print_int`, `print_string`, `int_to_string`, `bool_to_string`,
  `string_length`, `str_concat`, `read_file`, `write_file`, `exit`, `chr`, `ord`,
  `string_get`, `substring`, `string_to_int`, `error`, `show`, `argv`
- 実行: `cargo run -- examples/fib.dc`(gen0-interp-rs 内)
- テスト: `cargo test`(26 tests)

## Gen 2 の現状

**ARM64 ネイティブコード生成コンパイラ完成。**

```
gen2-dcc-rs/target/release/dcc examples/fib.dc -o fib && ./fib
# fib 20 = 6765
# fib 30 = 832040        (インタプリタの約 60 倍速)
```

- 構成: フロントエンド(Gen 0 の lexer/parser/HM 推論を再利用)→ ARM64 エンコーダ →
  Mach-O オブジェクト直接出力 → `cc` で rt.o とリンク
- 値表現: Int/Bool はタグ付き即値(`n<<2|1` / `3,7`)、その他はヒープブロック
  `[hdr: size|tag][payload]`(String/Tuple/ADT/Closure)
- GC: **mark-sweep**。ルートはミューテータスタックの保守的走査(即値が奇数なため
  誤マークなし)+ グローバルテーブルの精密走査
- 呼出規約: 全関数アリティ 1 のクロージャ(x0=クロージャ、x1=引数)。カリー化は
  クロージャ連鎖で、コンストラクタ部分適用は rt 側 trampoline で処理
- シンボル参照はすべて ADRP+LDR の GOT 形式(GOT_LOAD_PAGE21/PAGEOFF12 リロケーション)
- プログラムは 1 GiB スタックのスレッド上で実行(深い再帰に対応)
- 検証: `zsh gen2-dcc-rs/test.sh`(全サンプルがインタプリタと出力一致 + GC ストレス)

## Gen 1 の現状

**言語レベルのセルフホスト達成。**

```
dacelo gen1-interp-dc/interp.dc examples/hello.dc            # 単段
dacelo gen1-interp-dc/interp.dc gen1-interp-dc/interp.dc examples/hello.dc   # 二段!
```

- `interp.dc`(:900 行): 字句解析・パーサ・評価器をすべて dacelo で実装
  (内部に型推論は持たない動的評価器。interp.dc 自身は Gen 0 の HM 検査を通る)
- トップレベルの相互再帰は「最終グローバル環境の遅延解決」で、ローカル `let rec`
  は自己束縛クロージャ(VRecLam)で実現 — 純関数型のままノットタイ
- ネスト時の引数受け渡しは argv シフト方式(各インタプリタは自分を argv 0、
  ターゲットを argv 1 として見せ、子には 1 ずらした argv を見せる)
- 検証: `zsh gen1-interp-dc/test.sh`

### ゴール(Gen 3 完了条件) — 達成 ✅

```
dcc-rs -o dcc_1 dcc.dc     # Rust 製コンパイラで dacelo 製コンパイラをビルド
./dcc_1 -o dcc_2 dcc.dc    # dacelo 製コンパイラが自分自身をコンパイル
# dcc_1 と dcc_2 が同一挙動 → セルフホスト達成
```

達成内容: `dcc_1` が自分自身をコンパイルした `dcc_2` が完全動作し、
`dcc_2` が生成した `dcc_3` とバイト一致(`diff dcc_2.s dcc_3.s` 差分 0 行)
= 不動点に到達。全サンプルがインタプリタと出力一致。検証: `zsh gen3-dcc-dc/test.sh`。

## ステータス

Gen 0〜2 完了(ネイティブコンパイラ動作、全サンプル出力がインタプリタと一致)。

**Gen 3 完了**: `gen3-dcc-dc/` — dacelo 自身で書かれたコンパイラが
自分自身をコンパイルして完全動作する `dcc_2` を生成し、`dcc_2` の
自己コンパイル出力とバイト一致する不動点に到達。完成後のゴール:

```
dcc-rs  gen3-dcc-dc/dcc.dc -o dcc_1        # ホストが dacelo 製コンパイラをビルド
./dcc_1 gen3-dcc-dc/dcc.dc rt/rt.c dcc_2   # dcc_1 が自分自身をコンパイル → セルフホスト
```
