# Test keys

Signing keys for the stand-in identity provider in `tests/sign_in.rs`, generated with OpenSSL on
2026-10-22 for these tests and used nowhere else. They are public by being here: never configure
a real provider, or anything else, with them.

| File          | What                                                 |
| ------------- | ---------------------------------------------------- |
| `rsa-a.der`   | RSA 2048, PKCS#1 DER: the provider's key             |
| `rsa-b.der`   | RSA 2048, PKCS#1 DER: a rotated or rogue key         |
| `ec-p256.der` | P-256, PKCS#8 DER                                    |
| `*.jwk.json`  | Each one's public half, as the provider publishes it |
