//! Reference license server for LabForge pro labs.
//!
//! This is the contract the launcher talks to, with a **stub** key verifier so it can be run and
//! tested without a Gumroad account. Set GUMROAD_PRODUCT_ID to verify keys against Gumroad's
//! `POST /v2/licenses/verify` instead. Put the same three endpoints behind HTTPS in production
//! (a Cloudflare Worker or any small host works; nothing here is stateful).
//!
//!   POST /v1/license   {"key","machine"} -> {"token","signature"}   signed, expiring token
//!   GET  /v1/index                       -> pro index.json + X-Signature header (public metadata)
//!   GET  /v1/packs/<file>  X-License-Token / X-License-Signature -> pack bytes (needs a valid token)
//!
//! Stub keys: `LF-TEST-*` are valid, `LF-REVOKED-*` are refused as refunded, anything else is unknown.

use base64::Engine;
use minisign::{KeyPair, SecretKey};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Request, Response, Server};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

struct Config {
    port: u16,
    packs: PathBuf,
    keys: PathBuf,
    ttl: u64,
}

struct Signer {
    sk: SecretKey,
    pk: minisign::PublicKey,
}

impl Signer {
    /// Loads the dev key pair, creating one on first run. Raw key bytes are stored as hex: minisign's
    /// own key-file format wants a terminal password prompt, which a server can't answer.
    fn load(dir: &Path) -> Signer {
        std::fs::create_dir_all(dir).expect("create key dir");
        let (sk_path, pk_path) = (dir.join("license.key"), dir.join("license.pub"));
        if !sk_path.exists() {
            let KeyPair { pk, sk } = KeyPair::generate_unencrypted_keypair().expect("keygen");
            std::fs::write(&sk_path, hex::encode(sk.to_bytes())).unwrap();
            std::fs::write(&pk_path, hex::encode(pk.to_bytes())).unwrap();
        }
        let read = |p: &Path| hex::decode(std::fs::read_to_string(p).unwrap().trim()).expect("hex key");
        let sk = SecretKey::from_bytes(&read(&sk_path)).expect("secret key");
        let pk = minisign::PublicKey::from_bytes(&read(&pk_path)).expect("public key");
        Signer { sk, pk }
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

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

type KeyError = (u16, String);

/// Verifies a license key. With `GUMROAD_PRODUCT_ID` set this asks Gumroad; otherwise it uses the
/// offline stub (`LF-TEST-*` valid, `LF-REVOKED-*` refunded) so the server runs without an account.
fn check_key(key: &str) -> Result<(), KeyError> {
    match std::env::var("GUMROAD_PRODUCT_ID") {
        Ok(product) if !product.trim().is_empty() => check_gumroad(&product, key),
        _ => check_stub(key),
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
        .timeout(std::time::Duration::from_secs(10))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stub_keys() {
        assert!(check_stub("LF-TEST-1").is_ok());
        assert_eq!(check_stub("LF-REVOKED-1").unwrap_err().0, 403);
        assert_eq!(check_stub("x").unwrap_err().0, 404);
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
}

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

fn handle(mut req: Request, cfg: &Config, signer: &Signer) {
    let url = req.url().split('?').next().unwrap_or("").to_string();
    let method = req.method().clone();
    let resp: Response<Cursor<Vec<u8>>> = match (&method, url.as_str()) {
        (Method::Post, "/v1/license") => {
            let mut body = String::new();
            let _ = req.as_reader().take(4096).read_to_string(&mut body);
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let (key, machine) = (v["key"].as_str().unwrap_or(""), v["machine"].as_str().unwrap_or(""));
            if machine.is_empty() {
                json(400, serde_json::json!({ "error": "Missing machine id." }))
            } else {
                match check_key(key) {
                    Err((code, msg)) => json(code, serde_json::json!({ "error": msg })),
                    Ok(()) => {
                        let token = serde_json::json!({
                            "v": 1,
                            "license": hex::encode(&Sha256::digest(key.as_bytes())[..8]),
                            "machine": machine,
                            "entitlements": ["pro"],
                            "issued_at": now(),
                            "expires_at": now() + cfg.ttl,
                        })
                        .to_string();
                        let signature = signer.sign(token.as_bytes());
                        json(200, serde_json::json!({ "token": token, "signature": signature }))
                    }
                }
            }
        }
        (Method::Get, "/v1/index") => match std::fs::read(cfg.packs.join("index.json")) {
            Ok(bytes) => {
                let sig = signer.sign(&bytes);
                Response::from_data(bytes)
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap())
                    .with_header(Header::from_bytes(&b"X-Signature"[..], sig.as_bytes()).unwrap())
            }
            Err(_) => json(404, serde_json::json!({ "error": "No pro index has been built." })),
        },
        (Method::Get, path) if path.starts_with("/v1/packs/") => {
            let file = &path["/v1/packs/".len()..];
            let safe = !file.is_empty() && file.ends_with(".tar.gz") && !file.contains(['/', '\\']) && !file.contains("..");
            if !authorized(&req, signer) {
                json(401, serde_json::json!({ "error": "A valid license is required." }))
            } else if !safe {
                json(400, serde_json::json!({ "error": "Bad pack name." }))
            } else {
                match std::fs::read(cfg.packs.join(file)) {
                    Ok(bytes) => Response::from_data(bytes),
                    Err(_) => json(404, serde_json::json!({ "error": "No such pack." })),
                }
            }
        }
        _ => json(404, serde_json::json!({ "error": "Not found." })),
    };
    let _ = req.respond(resp);
}

fn main() {
    let mut cfg = Config { port: 8787, packs: PathBuf::from("dist-pro"), keys: PathBuf::from("license-server/dev-keys"), ttl: 30 * 24 * 3600 };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut next = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--port" => cfg.port = next().parse().expect("port"),
            "--packs" => cfg.packs = PathBuf::from(next()),
            "--keys" => cfg.keys = PathBuf::from(next()),
            "--ttl-secs" => cfg.ttl = next().parse().expect("ttl"),
            other => panic!("unknown argument {other}"),
        }
    }
    let signer = Signer::load(&cfg.keys);
    let pubkey = B64.encode(signer.pk.to_box().unwrap().to_string());
    println!("LabForge dev license server on http://127.0.0.1:{}", cfg.port);
    println!("packs: {}   token ttl: {}s", cfg.packs.display(), cfg.ttl);
    println!("LABFORGE_LICENSE_URL=http://127.0.0.1:{}", cfg.port);
    println!("LABFORGE_LICENSE_PUBKEY={pubkey}");
    let server = Server::http(("127.0.0.1", cfg.port)).expect("bind");
    for req in server.incoming_requests() {
        handle(req, &cfg, &signer);
    }
}
