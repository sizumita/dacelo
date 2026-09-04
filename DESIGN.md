# dacelo 設計書 v0

Dacelo(ワライカワセミ属)— 笑うように高速にセルフホストまで駆け上がる、ML 系静的型付け関数型言語。

## 1. 言語アイデンティティ

- **ML/Haskell 系構文**: `let` / `case-of` / カリー化 / ADT + パターンマッチ
- **最初から静的型**: Hindley-Milner 型推論(Algorithm J)。注釈ほぼ不要
- **最終形態: 直接機械語生成**: dacelo 製コンパイラが ARM64 機械語を直接吐く
- **メモリ管理: mark-sweep GC**
- **目標: セルフホスト**。コンパイラが自分自身をコンパイルできるまで世代を重ねる

## 2. 言語仕様 v0(Gen 0 スコープ)

### 2.1 構文スケッチ

```ocaml
-- コメントは -- 行末まで

let add x y = x + y                 -- カリー化関数

let compose f g = fun x -> f (g x)  -- 無名関数

let rec fib n =
  if n < 2 then n else fib (n - 1) + fib (n - 2)

type Tree =
  | Leaf                            -- 代数的データ型
  | Node Int Tree Tree

let rec sum t = case t of
    Leaf       -> 0
  | Node v l r -> v + sum l + sum r  -- パターンマッチ

let main () =
  print_int (sum (Node 1 Leaf (Node 2 Leaf Leaf))) ;
  print_string ("fib 10 = " ++ int_to_string (fib 10))
```

### 2.2 基本型

| 型 | 表現 |
|---|---|
| `Int` | 64bit 符号付き整数 |
| `Bool` | true / false |
| `String` | UTF-8 バイト列(不変) |
| `Unit` | () |
| `[a]`(リスト) | 不連結リスト |
| `(a, b)`(タプル) | 固定長組 |
| ADT | ユーザー定義代数的データ型 |

### 2.3 式と文

- `let x = e` / `let f x y = e` / `let rec f ... = ...`(再帰は明示 `rec`)
- `if cond then e1 else e2`
- `case e of | pat -> e ...`
- パターン: 変数 / `_` / リテラル(Int, Bool, ()) / タプル / コンストラクタ適用
- `e ; e'` 順次合成(Unit を捨てる)
- 関数適用は隣接(`f x y`)、演算子は中置記法
- 型注釈: `e : T`(省略可能)

### 2.4 型システム

- **Hindley-Milner 推論**(Algorithm J + 単一化)
- **let 多相**。参照を持たないため value restriction 不要(フル多相)
- ADT 定義: `type Name t1 t2 = | Con1 ... | Con2 ...`
- 組み込み型コンストラクタ: `Int Bool String Unit List Fun Tuple`

### 2.5 組み込み関数(Gen 0 最小セット)

```
print_string  : String -> Unit
print_int     : Int -> Unit
str_concat    : String -> String -> String   -- 中置 ++
int_to_string : Int -> String
string_length : String -> Int
read_file     : String -> String             -- Gen 1 のインタプリタが
                                             -- ソースを読むのに必須
write_file    : String -> String -> Unit
exit          : Int -> Unit
```

## 3. 世代計画(セルフホストロードマップ)

| 世代 | 成果物 | 実装言語 | 形態 | マイルストーン |
|---|---|---|---|---|
| Gen 0 | `interp-rs` | Rust | ツリーウォーク インタプリタ | 字句解析 → パーサ → HM 推論 → 評価が動く |
| Gen 1 | `interp.dc` | dacelo | dacelo 製インタプリタ | **言語レベルのセルフホスト** |
| Gen 2 | `dcc-rs` | Rust | ARM64 直接コード生成コンパイラ | ネイティブバイナリ誕生 |
| Gen 3 | `dcc.dc` | dacelo | dacelo 製コンパイラ | **セルフホスト完全達成** |

### 完了条件

- **Gen 0**: fib と ADT+パターンマッチを含むプログラムが正しく実行される
- **Gen 1**: `interp-rs interp.dc` が動き、さらに `interp-rs interp.dc interp.dc` まで通る(二段セルフホスト)
- **Gen 2**: `dcc-rs prog.dc -o prog` が Mach-O 実行ファイルを生成し、正しく動作する
- **Gen 3**:
  ```
  dcc-rs -o dcc_1 dcc.dc
  ./dcc_1 -o dcc_2 dcc.dc
  # dcc_1 と dcc_2 が同一挙動 = 勝利
  ```

## 4. ネイティブバックエンド設計(Gen 2 以降)

- **ターゲット**: macOS Apple Silicon(arm64)、AAPCS64 呼出規約
- **出力**: Mach-O オブジェクトファイルを直接エンコード → `ld` でリンク
- **メモリ管理**: mark-sweep GC
  - オブジェクトヘッダ: mark bit + 型タグ + サイズ
  - ルート探索: 初期案は shadow stack(callee-saved レジスタのみ使用し、局所変数はすべて shadow stack に置く)= 実装が単純
  - アロケータ: free list + 必要に応じ OS から拡張確保
- **コード生成**: まず素直なスタック多用コードから開始。レジスタ割付けは Gen 4+ で改良
- **GC ランタイム**: Gen 2 では小さな C ランタイム(rt.c)を静的リンク。Gen 4+ で dacelo 化を目指す

## 5. リポジトリ構成(予定)

```
dacelo/
├── README.md          # 概要とステータス
├── DESIGN.md          # 本書
├── gen0-interp-rs/    # Gen 0: Rust 製インタプリタ
├── gen1-interp-dc/    # Gen 1: interp.dc(dacelo 製インタプリタ)
├── gen2-dcc-rs/       # Gen 2: Rust 製コンパイラ
├── gen3-dcc-dc/       # Gen 3: dcc.dc(dacelo 製コンパイラ)
└── examples/          # .dc サンプル
```

拡張子は `.dc`。
