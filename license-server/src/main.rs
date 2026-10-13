//! License server for LabForge pro labs.
//!
//! This is the contract the launcher talks to. Run it with `--dev` for the stub key check, or set
//! `GUMROAD_PRODUCT_ID` to verify keys against Gumroad's `POST /v2/licenses/verify`. It speaks plain
//! HTTP and is meant to sit behind a TLS-terminating proxy (see `deploy/`).
//!
//!   POST /v1/license   {"key","machine"} -> {"token","signature"}   signed, expiring token
//!   GET  /v1/index                       -> pro index.json + X-Signature header (public metadata)
//!   GET  /v1/packs/<file>  X-License-Token / X-License-Signature -> pack bytes (needs a valid token)
//!   GET  /healthz                        -> ok
//!
//! Commands: `serve` (default), `keygen` (create the signing key once), `pubkey` (print the public key
//! to paste into the launcher). Dev stub keys: `LF-TEST-*` valid, `LF-REVOKED-*` refunded.

use base64::Engine;
use minisign::{KeyPair, SecretKey};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Request, Response, Server};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Requests handled at once; anything beyond this gets a 503 instead of piling up threads.
const MAX_IN_FLIGHT: usize = 64;
/// Per client IP, per window.
const LICENSE_LIMIT: usize = 10;
const PACK_LIMIT: usize = 60;
const WINDOW: Duration = Duration::from_secs(600);

struct Config {
    bind: String,
    port: u16,
    packs: PathBuf,
    keys: PathBuf,
    ttl: u64,
    /// Allow stub keys and generating a signing key on the fly. Never use in production.
    dev: bool,
    /// Take the client address from X-Forwarded-For (only behind a proxy you control).
    trust_proxy: bool,
    gumroad_product: Option<String>,
}

struct Signer {
    sk: SecretKey,
    pk: minisign::PublicKey,
}

impl Signer {
    fn create(dir: &Path) -> Result<(), String> {
        let (sk_path, pk_path) = (dir.join("license.key"), dir.join("license.pub"));
        if sk_path.exists() {
            return Err(format!("{} already exists; refusing to overwrite the signing key", sk_path.display()));
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let KeyPair { pk, sk } = KeyPair::generate_unencrypted_keypair().map_err(|e| e.to_string())?;
        write_private(&sk_path, &hex::encode(sk.to_bytes()))?;
        std::fs::write(&pk_path, hex::encode(pk.to_bytes())).map_err(|e| e.to_string())
    }

    /// Loads the key pair. Raw key bytes are stored as hex: minisign's own key-file format wants a
    /// terminal password prompt, which a server can't answer.
    fn load(dir: &Path, create_if_missing: bool) -> Result<Signer, String> {
        let (sk_path, pk_path) = (dir.join("license.key"), dir.join("license.pub"));
        if !sk_path.exists() {
            if !create_if_missing {
                return Err(format!("No signing key in {}. Run `keygen` first.", dir.display()));
            }
            Signer::create(dir)?;
        }
        let read = |p: &Path| -> Result<Vec<u8>, String> {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            hex::decode(text.trim()).map_err(|e| format!("{}: {e}", p.display()))
        };
        let sk = SecretKey::from_bytes(&read(&sk_path)?).map_err(|e| e.to_string())?;
        let pk = minisign::PublicKey::from_bytes(&read(&pk_path)?).map_err(|e| e.to_string())?;
        Ok(Signer { sk, pk })
    }

    /// The public key in the launcher's format: base64 of the minisign public key file.
    fn public_key(&self) -> String {
        B64.encode(self.pk.to_box().unwrap().to_string())
    }

    /// Signature in the launcher's format: base64 of the minisign signature file.
    fn sign(&self, data: &[u8]) -> String {
        let sig = minisign::sign(Some(&self.pk), &self.sk, Cursor::new(data), Some("labforge license"), Some("signature from labforge license server"))
            .expect("sign");
        B64.encode(sig.to_string())
    }

    fn verify(&self, data: &[u8], sig_b64: &str) -> bool {
        let Ok(text) = B64.decode(sig_b64.trim()).map(|b| String::from_utf8_lossy(&b).to_string()) else { return false };
        let Ok(sig) = minisign::SignatureBox::from_string(&text) else { return false };
        minisign::verify(&self.pk, &sig, Cursor::new(data), true, false, false).is_ok()
    }
}

fn write_private(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

// -------------------------------------------------------- rate limiting ----

/// Sliding-window counter per client, in memory (the server is a single process).
struct Limiter {
    hits: Mutex<HashMap<String, Vec<Instant>>>,
}

impl Limiter {
    fn new() -> Limiter {
        Limiter { hits: Mutex::new(HashMap::new()) }
    }

    /// Records a request and says whether it is within `limit` per `WINDOW`.
    fn allow(&self, bucket: &str, client: &str, limit: usize) -> bool {
        let mut map = self.hits.lock().unwrap();
        if map.len() > 20_000 {
            map.clear(); // bounded memory under a flood of distinct addresses
        }
        let now = Instant::now();
        let times = map.entry(format!("{bucket}:{client}")).or_default();
        times.retain(|t| now.duration_since(*t) < WINDOW);
        if times.len() >= limit {
            return false;
        }
        times.push(now);
        true
    }
}

fn client_ip(req: &Request, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(xff) = header(req, "X-Forwarded-For") {
            if let Some(first) = xff.split(',').next().map(str::trim).filter(|s| !s.is_empty()) {
                return first.to_string();
            }
        }
    }
    req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_else(|| "unknown".into())
}

// ------------------------------------------------------------ key check ----

type KeyError = (u16, String);

/// Verifies a license key with Gumroad when a product is configured, else (dev only) the stub.
fn check_key(cfg: &Config, key: &str) -> Result<(), KeyError> {
    match &cfg.gumroad_product {
        Some(product) => check_gumroad(product, key),
        None if cfg.dev => check_stub(key),
        None => Err((503, "Licensing isn't configured on this server.".into())),
    }
}

fn check_stub(key: &str) -> Result<(), KeyError> {
    if key.starts_with("LF-TEST-") && key.len() > 8 {
        Ok(())
    } else if key.starts_with("LF-REVOKED-") {
        Err((403, "This license key was refunded or revoked.".into()))
    } else {
        Err((404, "That license key wasn't found.".into()))
    }
}

/// Gumroad's license API. `increment_uses_count=false` so checking a key never burns its use count.
fn check_gumroad(product_id: &str, key: &str) -> Result<(), KeyError> {
    let reply = ureq::post("https://api.gumroad.com/v2/licenses/verify")
        .timeout(Duration::from_secs(10))
        .send_form(&[("product_id", product_id), ("license_key", key), ("increment_uses_count", "false")]);
    let body: serde_json::Value = match reply {
        Ok(r) => r.into_json().map_err(|_| (502, "The license service sent an unreadable reply.".to_string()))?,
        // Gumroad answers unknown keys with a 404 and a JSON body
        Err(ureq::Error::Status(404, r)) => r.into_json().unwrap_or_default(),
        Err(ureq::Error::Status(code, _)) => return Err((502, format!("The license service returned {code}. Try again later."))),
        Err(_) => return Err((502, "Couldn't reach the license service. Try again later.".into())),
    };
    interpret_gumroad(&body)
}

/// Turns a Gumroad verify response into allow / refuse. Refunded, charged-back or lost-dispute
/// purchases and ended subscriptions don't count as licensed.
fn interpret_gumroad(body: &serde_json::Value) -> Result<(), KeyError> {
    if body["success"].as_bool() != Some(true) {
        return Err((404, "That license key wasn't found.".into()));
    }
    let p = &body["purchase"];
    let refused = p["refunded"].as_bool() == Some(true)
        || p["chargebacked"].as_bool() == Some(true)
        || (p["disputed"].as_bool() == Some(true) && p["dispute_won"].as_bool() != Some(true));
    if refused {
        return Err((403, "This license key was refunded or revoked.".into()));
    }
    if ["subscription_ended_at", "subscription_cancelled_at", "subscription_failed_at"].iter().any(|f| p[*f].is_string()) {
        return Err((403, "This subscription is no longer active.".into()));
    }
    Ok(())
}

// ------------------------------------------------------------- handlers ----

fn json(code: u16, body: serde_json::Value) -> Response<Cursor<Vec<u8>>> {
    Response::from_string(body.to_string())
        .with_status_code(code)
        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap())
}

fn header(req: &Request, name: &str) -> Option<String> {
    req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string())
}

/// True when the request carries an unexpired token this server signed, with the `pro` entitlement.
fn authorized(req: &Request, signer: &Signer) -> bool {
    let (Some(tok), Some(sig)) = (header(req, "X-License-Token"), header(req, "X-License-Signature")) else { return false };
    let Ok(raw) = B64.decode(tok.trim()) else { return false };
    if !signer.verify(&raw, &sig) {
        return false;
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&raw) else { return false };
    v["expires_at"].as_u64().is_some_and(|e| e > now()) && v["entitlements"].as_array().is_some_and(|a| a.iter().any(|x| x == "pro"))
}

struct Ctx {
    cfg: Config,
    signer: Signer,
    limiter: Limiter,
}

fn too_many() -> Response<Cursor<Vec<u8>>> {
    json(429, serde_json::json!({ "error": "Too many attempts. Wait a few minutes and try again." }))
        .with_header(Header::from_bytes(&b"Retry-After"[..], &b"600"[..]).unwrap())
}

fn route(req: &mut Request, ctx: &Ctx, ip: &str) -> Response<Cursor<Vec<u8>>> {
    let url = req.url().split('?').next().unwrap_or("").to_string();
    let method = req.method().clone();
    match (&method, url.as_str()) {
        (Method::Get, "/healthz") => Response::from_string("ok").with_status_code(200),
        (Method::Post, "/v1/license") => {
            if !ctx.limiter.allow("license", ip, LICENSE_LIMIT) {
                return too_many();
            }
            let mut body = String::new();
            let _ = req.as_reader().take(4096).read_to_string(&mut body);
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let (key, machine) = (v["key"].as_str().unwrap_or(""), v["machine"].as_str().unwrap_or(""));
            if machine.is_empty() || machine.len() > 64 || key.is_empty() || key.len() > 200 {
                return json(400, serde_json::json!({ "error": "Missing or invalid license key or machine id." }));
            }
            match check_key(&ctx.cfg, key) {
                Err((code, msg)) => json(code, serde_json::json!({ "error": msg })),
                Ok(()) => {
                    let token = serde_json::json!({
                        "v": 1,
                        "license": hex::encode(&Sha256::digest(key.as_bytes())[..8]),
                        "machine": machine,
                        "entitlements": ["pro"],
                        "issued_at": now(),
                        "expires_at": now() + ctx.cfg.ttl,
                    })
                    .to_string();
                    let signature = ctx.signer.sign(token.as_bytes());
                    json(200, serde_json::json!({ "token": token, "signature": signature }))
                }
            }
        }
        (Method::Get, "/v1/index") => match std::fs::read(ctx.cfg.packs.join("index.json")) {
            Ok(bytes) => {
                let sig = ctx.signer.sign(&bytes);
                Response::from_data(bytes)
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap())
                    .with_header(Header::from_bytes(&b"X-Signature"[..], sig.as_bytes()).unwrap())
            }
            Err(_) => json(404, serde_json::json!({ "error": "No pro index has been published." })),
        },
        (Method::Get, path) if path.starts_with("/v1/packs/") => {
            let file = &path["/v1/packs/".len()..];
            let safe = !file.is_empty() && file.ends_with(".tar.gz") && !file.contains(['/', '\\']) && !file.contains("..");
            if !ctx.limiter.allow("pack", ip, PACK_LIMIT) {
                too_many()
            } else if !authorized(req, &ctx.signer) {
                json(401, serde_json::json!({ "error": "A valid license is required." }))
            } else if !safe {
                json(400, serde_json::json!({ "error": "Bad pack name." }))
            } else {
                match std::fs::read(ctx.cfg.packs.join(file)) {
                    Ok(bytes) => Response::from_data(bytes),
                    Err(_) => json(404, serde_json::json!({ "error": "No such pack." })),
                }
            }
        }
        _ => json(404, serde_json::json!({ "error": "Not found." })),
    }
}

fn handle(mut req: Request, ctx: &Ctx) {
    let ip = client_ip(&req, ctx.cfg.trust_proxy);
    let resp = route(&mut req, ctx, &ip);
    // Access log: never the body or headers, so license keys and tokens stay out of the logs.
    println!("{ip} {} {} {}", req.method(), req.url().split('?').next().unwrap_or(""), resp.status_code().0);
    let _ = req.respond(resp);
}

// ----------------------------------------------------------------- main ----

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

fn main() {
    let mut cfg = Config {
        bind: "127.0.0.1".into(),
        port: 8787,
        packs: PathBuf::from("dist-pro"),
        keys: PathBuf::from("license-server/dev-keys"),
        ttl: 30 * 24 * 3600,
        dev: false,
        trust_proxy: false,
        gumroad_product: std::env::var("GUMROAD_PRODUCT_ID").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
    };
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let command = if args.first().is_some_and(|a| !a.starts_with("--")) { args.remove(0) } else { "serve".to_string() };
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut next = || it.next().unwrap_or_else(|| fail(&format!("{a} needs a value")));
        match a.as_str() {
            "--bind" => cfg.bind = next(),
            "--port" => cfg.port = next().parse().unwrap_or_else(|_| fail("bad --port")),
            "--packs" => cfg.packs = PathBuf::from(next()),
            "--keys" => cfg.keys = PathBuf::from(next()),
            "--ttl-secs" => cfg.ttl = next().parse().unwrap_or_else(|_| fail("bad --ttl-secs")),
            "--dev" => cfg.dev = true,
            "--trust-proxy" => cfg.trust_proxy = true,
            other => fail(&format!("unknown argument {other}")),
        }
    }

    match command.as_str() {
        "keygen" => {
            Signer::create(&cfg.keys).unwrap_or_else(|e| fail(&e));
            let signer = Signer::load(&cfg.keys, false).unwrap_or_else(|e| fail(&e));
            println!("Created a signing key in {}", cfg.keys.display());
            println!("Back up license.key somewhere safe. If it is lost, every launcher needs a new public key.");
            println!("Launcher public key (PUBKEY in src-tauri/src/license.rs):\n{}", signer.public_key());
        }
        "pubkey" => {
            let signer = Signer::load(&cfg.keys, false).unwrap_or_else(|e| fail(&e));
            println!("{}", signer.public_key());
        }
        "serve" => serve(cfg),
        other => fail(&format!("unknown command {other} (use serve, keygen or pubkey)")),
    }
}

fn serve(cfg: Config) {
    if cfg.gumroad_product.is_none() && !cfg.dev {
        fail("Set GUMROAD_PRODUCT_ID to verify keys with Gumroad, or pass --dev to use the stub keys (never in production).");
    }
    let signer = Signer::load(&cfg.keys, cfg.dev).unwrap_or_else(|e| fail(&e));
    println!("LabForge license server on http://{}:{}", cfg.bind, cfg.port);
    println!("mode: {}   packs: {}   token ttl: {}s", if cfg.gumroad_product.is_some() { "gumroad" } else { "dev (stub keys)" }, cfg.packs.display(), cfg.ttl);
    if cfg.dev {
        println!("LABFORGE_LICENSE_URL=http://127.0.0.1:{}", cfg.port);
        println!("LABFORGE_LICENSE_PUBKEY={}", signer.public_key());
    }
    let server = Server::http((cfg.bind.as_str(), cfg.port)).unwrap_or_else(|e| fail(&format!("couldn't bind: {e}")));
    let ctx = Arc::new(Ctx { cfg, signer, limiter: Limiter::new() });
    let in_flight = Arc::new(AtomicUsize::new(0));
    for req in server.incoming_requests() {
        if in_flight.fetch_add(1, Ordering::SeqCst) >= MAX_IN_FLIGHT {
            in_flight.fetch_sub(1, Ordering::SeqCst);
            let _ = req.respond(json(503, serde_json::json!({ "error": "Busy. Try again shortly." })));
            continue;
        }
        let (ctx, in_flight) = (ctx.clone(), in_flight.clone());
        std::thread::spawn(move || {
            handle(req, &ctx);
            in_flight.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(dev: bool, product: Option<&str>) -> Config {
        Config {
            bind: "127.0.0.1".into(),
            port: 0,
            packs: PathBuf::new(),
            keys: PathBuf::new(),
            ttl: 60,
            dev,
            trust_proxy: false,
            gumroad_product: product.map(String::from),
        }
    }

    #[test]
    fn stub_keys() {
        assert!(check_stub("LF-TEST-1").is_ok());
        assert_eq!(check_stub("LF-REVOKED-1").unwrap_err().0, 403);
        assert_eq!(check_stub("x").unwrap_err().0, 404);
    }

    #[test]
    fn stub_keys_never_work_outside_dev_mode() {
        // production without a product id must not mint licenses from test keys
        assert_eq!(check_key(&cfg(false, None), "LF-TEST-ABCD").unwrap_err().0, 503);
        assert!(check_key(&cfg(true, None), "LF-TEST-ABCD").is_ok());
    }

    #[test]
    fn gumroad_replies_are_interpreted() {
        assert!(interpret_gumroad(&json!({"success": true, "uses": 1, "purchase": {"refunded": false}})).is_ok());
        assert_eq!(interpret_gumroad(&json!({"success": false, "message": "That license does not exist"})).unwrap_err().0, 404);
        assert_eq!(interpret_gumroad(&json!({"success": true, "purchase": {"refunded": true}})).unwrap_err().0, 403);
        assert_eq!(interpret_gumroad(&json!({"success": true, "purchase": {"chargebacked": true}})).unwrap_err().0, 403);
        assert_eq!(interpret_gumroad(&json!({"success": true, "purchase": {"disputed": true, "dispute_won": false}})).unwrap_err().0, 403);
        assert!(interpret_gumroad(&json!({"success": true, "purchase": {"disputed": true, "dispute_won": true}})).is_ok());
        assert_eq!(interpret_gumroad(&json!({"success": true, "purchase": {"subscription_cancelled_at": "2026-01-01T00:00:00Z"}})).unwrap_err().0, 403);
        assert!(interpret_gumroad(&json!({"success": true, "purchase": {"subscription_cancelled_at": null}})).is_ok());
        assert_eq!(interpret_gumroad(&json!({})).unwrap_err().0, 404);
    }

    #[test]
    fn limiter_blocks_after_the_limit_per_client() {
        let l = Limiter::new();
        for _ in 0..3 {
            assert!(l.allow("license", "1.2.3.4", 3));
        }
        assert!(!l.allow("license", "1.2.3.4", 3));
        assert!(l.allow("license", "5.6.7.8", 3), "other clients are unaffected");
        assert!(l.allow("pack", "1.2.3.4", 3), "buckets are separate");
    }

    #[test]
    fn keygen_refuses_to_overwrite_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!("lic-test-{}", now()));
        Signer::create(&dir).unwrap();
        assert!(Signer::create(&dir).is_err());
        assert!(Signer::load(&dir.join("missing"), false).is_err());
        let s = Signer::load(&dir, false).unwrap();
        let sig = s.sign(b"hello");
        assert!(s.verify(b"hello", &sig));
        assert!(!s.verify(b"hullo", &sig));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
