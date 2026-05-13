# Ship

`Ship` は Tugboat における最小のデプロイ単位であり、単一の仮想マシン（VM）を表します。

## マニフェスト例

```yaml
apiVersion: core/v1
kind: Ship
metadata:
  name: example-ship
  namespace: default
spec:
  shipClassName: standard
  serviceAccountName: my-service-account
  automountServiceAccountToken: true
```

## Service Account トークンの自動投影

デフォルトでは、Tugboat は Service Account トークンを Ship 内部に自動的にマウントします。このトークンを使用して、VM 内のアプリケーションから Tugboat API サーバーに対して認証を行うことができます。

- **マウントパス**: `/var/run/secrets/tugboat.cloud/serviceaccount/`
- **ファイル**:
    - `token`: 署名済み JWT トークン
    - `ca.crt`: API サーバーの CA 証明書
    - `namespace`: Ship の名前空間

トークンはノードエージェントによって自動的に更新（ローテーション）されます。
