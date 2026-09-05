# RSA test fixtures

These files contain the same 2048-bit RSA private key as base64-encoded DER,
in PKCS#8 and PKCS#1 formats. They are public test data used by API server
unit tests and integration tests. Never use this key outside tests.

The fixtures were generated with OpenSSL using the commands below. OpenSSL
is only needed to regenerate them, not to build or run the tests.

```sh
openssl genrsa 2048 | openssl pkcs8 -topk8 -nocrypt -outform DER | openssl base64 -A > testdata/rsa-private-key.pkcs8.b64
openssl base64 -d -A -in testdata/rsa-private-key.pkcs8.b64 | openssl rsa -inform DER -traditional -outform DER | openssl base64 -A > testdata/rsa-private-key.pkcs1.b64
```
