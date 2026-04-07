# タスク: Node と RuntimeClass の対応付け設計と実装

## 前提

タスク `01-proto-definition.md` が完了済みであること。

## 概要

スケジューラが「あるノードがどの RuntimeClass に対応しているか」を判断するための仕組みを決定・実装する。

現状 `Node` リソースには RuntimeClass を指す専用フィールドが存在しない。
本タスクではノードと RuntimeClass の対応付け方法として **Node のラベルを使う方式** を採用する。

## 設計

ノード登録時（または `tugboat-agent` 起動時）に、Node のラベルに以下を設定する:

```
tugboat.cloud/runtime-class: <runtimeclass-name>
```

スケジューラはこのラベルを読み取って対応する RuntimeClass を特定する。

## 実装内容

### 1. ラベルキーの定数定義

`tugboat-resources` に定数を追加するか、スケジューラ内でリテラル定数として定義する:

```rust
// tugboat-scheduler/src/plugins/runtime_class_fit.rs 等
const RUNTIME_CLASS_LABEL_KEY: &str = "tugboat.cloud/runtime-class";
```

### 2. agent の Node 登録処理の確認・更新

`tugboat-agent` がノード起動時に Node リソースを作成/更新する箇所を確認し、
RuntimeClass 名をラベルとして設定するオプション（設定ファイル経由）を追加することを検討する。

具体的には `tugboat-agent` の設定ファイル (`config.toml`) に `runtime_class` フィールドを追加し、
Node 登録時のラベルに反映する。

### 3. sample-configs の更新

`sample-configs/agent/config.toml` に `runtime_class` の記述例をコメントで追加する。

## 備考

- この設計はノード1台につき RuntimeClass が1つという想定（todo に記載の設計方針）に合致する
- ラベルが設定されていないノードはすべての RuntimeClass 制約をもつ Ship のスケジュール対象外となる
  （`04-scheduler-runtimeclass-plugin.md` のフィルタープラグインが Reject する）

## 確認コマンド

```bash
cargo build --package tugboat-agent
cargo clippy --package tugboat-agent
```
