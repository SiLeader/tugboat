# 02: ServiceAccount Protobuf定義

## 概要

`core/v1` APIグループにServiceAccountリソースのProtobufメッセージを定義する。
ServiceAccountはnamespaced resourceで、Ship等がAPIサーバーに対して認証するためのIDを提供する。

## 対象リソース

### ServiceAccount (namespaced)

```protobuf
message ServiceAccount {
  TypeMeta type_meta = 1;
  ObjectMeta object_meta = 2;
  repeated ObjectReference secrets = 3;           // 関連するSecretの参照
  optional bool automount_service_account_token = 4;
}

message ObjectReference {
  string kind = 1;
  string namespace = 2;
  string name = 3;
  string uid = 4;
  string api_version = 5;
}
```

## 作業内容

1. `tugboat-resources/proto/core/v1/service_account.proto` を作成
2. 必要であれば `object_reference.proto` を `proto/meta/v1/` に作成
3. `tugboat-resources/build.rs` にprotoファイルを追加
4. コンパイルが通ることを確認 (`cargo build --package tugboat-resources`)

## 参考

- 既存のcore/v1リソース: `tugboat-resources/proto/core/v1/secret.proto` など
- KubernetesのServiceAccountは `core/v1` に属する
