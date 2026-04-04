# Fix: tugboat-runtime のエラー時 `todo!()` パニックを正常終了に置き換える

## 概要

`tugboat-runtime/src/lib.rs` の `run()` 関数内でエラーが発生した際に `todo!()` マクロを呼んでいる。
`todo!()` はパニックを引き起こすため、プロセスがスタックトレースを出力して異常終了する。
正常な終了コードで終了するよう修正する必要がある。

## 対象ファイル

- `tugboat-runtime/src/lib.rs`

## 現状のコード（97–100行目付近）

```rust
error!("Runtime error: {e}");
todo!();
```

## 修正内容

`todo!()` を削除し、ログ出力後に非ゼロ終了コードでプロセスを終了する。

```rust
error!("Runtime error: {e}");
std::process::exit(1);
```

## 完了条件

- `todo!()` が `std::process::exit(1)` に置き換えられている
- `cargo clippy` および `cargo test` が通ること
