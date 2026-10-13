# License server

The license API the launcher talks to (see the doc comment in `src/main.rs` for the endpoints). **Production deployment: see [deploy/README.md](deploy/README.md).** This page covers local development with the stub key check.

Stub keys only work with `--dev`: `LF-TEST-<anything>` is valid, `LF-REVOKED-<anything>` is refused as refunded, everything else is unknown. Without `--dev` the server refuses to start unless `GUMROAD_PRODUCT_ID` is set, so test keys can never be accepted in production.

## Try it locally

```bash
# 1. build the sample pro lab pack (license-server/dev-labs/demo-pro)
python scripts/build_lab_packs.py --labs-dir license-server/dev-labs --tier pro \
  --out dist-pro --base-url http://127.0.0.1:8787/v1/packs

# 2. run the server (release mode: key handling is slow in debug builds)
cd license-server && cargo run --release -- --dev --packs ../dist-pro
```

The server prints `LABFORGE_LICENSE_URL` and `LABFORGE_LICENSE_PUBKEY`. Set both and start a **debug** build of the launcher (`cargo tauri dev`); release builds ignore them. Then **System → LabForge Pro**: enter `LF-TEST-ABCD-1234`, and the sample pro lab becomes installable from **Labs → Check for new labs**.

The automated version of that walkthrough:

```bash
cd src-tauri
LABFORGE_LICENSE_URL=... LABFORGE_LICENSE_PUBKEY=... cargo test --lib license::live -- --ignored
```

`--ttl-secs 60` makes tokens expire quickly to try renewal and the expired state.

## Going live

See [deploy/README.md](deploy/README.md): Docker Compose + Caddy on a VPS, key generation and backup, publishing pro packs, and a refund test with a real Gumroad purchase.

## What a license does and doesn't do

A token only gates **downloading** pro labs and their updates. Installed labs keep running if a token expires or the server disappears, so a licensing outage never locks a learner out. Tokens are bound to a random per-install machine id and expire (30 days by default); the launcher renews them quietly. This is not copy protection: once a pro lab is on disk it is just files.
