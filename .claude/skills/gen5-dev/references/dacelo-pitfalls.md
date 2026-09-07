# Dacelo でコンパイラソースを書くときの落とし穴

Gen5 の自ソース（gen5/*.dc）は Gen0（Rust インタプリタ）、dcc_1（Gen3）、gen5check 自身の
三者に受理される必要がある。以下は実際に踏んだものだけを集めている（出典: gen5/RESUME.md）。

## 構文

- **予約語**: `sig` `module` `exposing` `import` `with` `forall` を識別子にしない。
  変数に使うと次の行頭の同語が適用引数に吸収され、`unbound variable 'sig'` などになる。
  過去に `forall`→`fall`、`exposing`→`exps` へ改名して解決した。
- **case 吸収**: 多腕 `case` の arm 直下に内側の `case` を置くと、後続の `|` を内側が吸収する
  （インデント無関係、内側が多腕でも起きる）。後続 `|` がある arm 直下の case は括弧で囲む。
  自作コード側にも同じ癖があるので、lowering/formatter の出力も同様に括弧化する。
- **コメントは `--`**。`//` は不可（CJK コメントは可）。
- **負数リテラル**は `(0 - 1)` と書く。
- **`[]` パターンを prelude/lowering に使わない**: legacy backend は式位置の `[]` にしか
  `_ctorinst_Nil` を出さないため `| [] ->` だけだとリンクエラー。`| _ ->` で受ける。
- **nullary ctor を引数位置に出さない**: `(Cons h Nil)` は再parse不能。`::`/`[]` 糖衣で印刷する。
- **型適用引数は括弧**: `TTTup (List TyAst)` を `TTTup List TyAst` にしない。

## 型検査（Gen0 の癖）

- **`let ... and ...` グループの束縛は複数の型で使えない**（グループ内で単相化される）。
  多相 helper は必ず standalone の `let rec` にする。
- **文字列リテラル腕の後に変数腕**を置くと Gen0 が型検査を誤る。`if` に書き換える。
- **自ソースに Gen5 構文を入れない**（`sig` を書くと legacy parser が and-group を壊す）。
  Gen4 サブセットのみ。

## 推論器の実装規約（g5_infer.dc）

- 状態 `TIS5` は 16-tuple。必ず `g5s_*` / `g5s_with_*` accessor 経由で触る。
  位置を増やすときは accessor 約 35 箇所を一括で直し、`GEN5_SKIP=C` を合否条件にする。
- `g5_snapshot` は各リストの長さ＋カウンタ、`g5_rollback` は **newest-first のリストから
  新しい方を drop** する（history を残す）。逆向きにすると trial のゴミが残り、hole 再試行と
  エラー継続が壊れる。
- row 単一化: flex/flex は fresh な共有 tail を作り **両側**を束縛する（片側だけは unsound）。
  `g5_row_occurs` は chase 後の tail も検査する。rigid tail は負の id。
- 文字列を再度ソースに出すときは `g5_dc_str` で再エスケープ（式・パターンとも）。
- record helper 名 `g5_rec_get` / `g5_rec_with` は予約名。record を使うファイルでは
  全 bind site で拒否する（legacy-only のファイルでは互換のため許容）。

## モジュール（v1 flat namespace）

- 型の同一性は名前文字列（TypeDefId 未導入）なので、**型名の重複は program-wide で拒否**する
  （非公開型も含む）。同一ファイルは一度しか訪問しないので diamond は誤検出しない。
- 各ファイルの初期環境は builtins＋明示 import の export 表のみ（`(name, scheme, sid)`）。
  型名・ctor・SymId も同じ境界で受け渡す。
- lowering の連結順は依存先優先（entry-first だと未初期化のグローバルを読む）。

## Gen0 との一致

- `gen5check check` の text 出力は Gen0 `--types` の **終了コード＋メッセージと byte 一致**が
  38 ケースの oracle 条件。warning は JSON 限定にし、text 側に出さない。
