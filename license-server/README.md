# Dev license server

Reference implementation of the license API the launcher talks to, with a **stub** key check so it runs without a Gumroad account. See the doc comment in `src/main.rs` for the three endpoints.

Stub keys: `LF-TEST-<anything>` is valid, `LF-REVOKED-<anything>` is refused as refunded, everything else is unknown.

## Try it locally

```bash
# 1. build the sample pro lab pack (license-server/dev-labs/demo-pro)
python scripts/build_lab_packs.py --labs-dir license-server/dev-labs --tier pro \
  --out dist-pro --base-url http://127.0.0.1:8787/v1/packs

# 2. run the server (release mode: key handling is slow in debug builds)
cd license-server && cargo run --release -- --packs ../dist-pro
```

The server prints `LABFORGE_LICENSE_URL` and `LABFORGE_LICENSE_PUBKEY`. Set both and start a **debug** build of the launcher (`cargo tauri dev`); release builds ignore them. Then **System → LabForge Pro**: enter `LF-TEST-ABCD-1234`, and the sample pro lab becomes installable from **Labs → Check for new labs**.

The automated version of that walkthrough:

```bash
cd src-tauri
LABFORGE_LICENSE_URL=... LABFORGE_LICENSE_PUBKEY=... cargo test --lib license::live -- --ignored
```

`--ttl-secs 60` makes tokens expire quickly to try renewal and the expired state.

## Going live

1. Replace `check_key` with a call to Gumroad's `POST https://api.gumroad.com/v2/licenses/verify` (`product_id`, `license_key`, `increment_uses_count=false`; treat refunded, chargebacked or disputed purchases as invalid).
2. Host the three endpoints over HTTPS (nothing is stateful, so a Cloudflare Worker, Fly.io or any small VPS works). Serve pack bytes from private storage, ideally by redirecting to a short-lived signed URL.
3. Generate a production signing key pair, keep the secret key on the server only, and set `SERVER` and `PUBKEY` in `src-tauri/src/license.rs`. Until both are set, licensing stays hidden in release builds.
4. Build pro packs with `scripts/build_lab_packs.py --tier pro` and upload them (and `index.json`) to the license server's storage. Never publish pro packs to the public `labs` release.

## What a license does and doesn't do

A token only gates **downloading** pro labs and their updates. Installed labs keep running if a token expires or the server disappears, so a licensing outage never locks a learner out. Tokens are bound to a random per-install machine id and expire (30 days by default); the launcher renews them quietly. This is not copy protection: once a pro lab is on disk it is just files.
