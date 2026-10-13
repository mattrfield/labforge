//! Licenses for pro labs.
//!
//! The learner enters a license key. The launcher sends it to the license server, which answers
//! with a short-lived signed token (see [`Token`]). Everything after that is checked offline: the
//! token's signature against the license public key compiled into the launcher, its expiry, and the
//! machine it was issued to. A token only gates *installing* pro labs and their updates; labs that
//! are already installed keep working, so an expired token or a dead server never locks anyone out.
//!
//! Until a real license server exists, `SERVER` and `PUBKEY` are empty and the whole feature stays
//! hidden. Debug builds can point at a dev server with `LABFORGE_LICENSE_URL` and
//! `LABFORGE_LICENSE_PUBKEY`.

use crate::packs;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Base URL of the production license server, e.g. `https://license.example.com`.
const SERVER: &str = "";
/// Base64 minisign public key (same format as the updater key) that signs license tokens and the pro index.
const PUBKEY: &str = "";

/// Refresh a token this long before it expires.
const REFRESH_WINDOW_SECS: u64 = 7 * 24 * 3600;

pub fn server_url() -> Option<String> {
    let url = if cfg!(debug_assertions) { std::env::var("LABFORGE_LICENSE_URL").ok() } else { None };
    let url = url.unwrap_or_else(|| SERVER.to_string());
    let url = url.trim_end_matches('/').to_string();
    (!url.is_empty()).then_some(url)
}

pub fn pubkey() -> Option<String> {
    let key = if cfg!(debug_assertions) { std::env::var("LABFORGE_LICENSE_PUBKEY").ok() } else { None };
    let key = key.unwrap_or_else(|| PUBKEY.to_string());
    let key = key.trim().to_string();
    (!key.is_empty()).then_some(key)
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What the license server vouches for. Serialized as JSON and signed exactly as sent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Token {
    pub v: u32,
    /// Opaque id derived from the key; never the key itself.
    pub license: String,
    pub machine: String,
    pub entitlements: Vec<String>,
    pub issued_at: u64,
    pub expires_at: u64,
}

/// What is kept in `license.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stored {
    pub key: String,
    /// The token JSON, byte for byte as the server signed it.
    pub token: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    None,
    Active,
    Expired,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// False when no license server is configured; the UI hides licensing entirely.
    pub available: bool,
    pub state: State,
    pub expires_at: Option<u64>,
    /// Last four characters of the key, for display.
    pub key_hint: Option<String>,
}

/// Checks a token's signature, machine and `pro` entitlement. Expiry is checked separately so an
/// expired token can still be recognised and refreshed.
pub fn verify_token(pubkey_b64: &str, token_json: &str, sig_b64: &str, machine: &str) -> Result<Token, String> {
    packs::verify_signature(pubkey_b64, token_json.as_bytes(), sig_b64)
        .map_err(|_| "The license wasn't signed by the license server.".to_string())?;
    let token: Token = serde_json::from_str(token_json).map_err(|e| format!("bad license token: {e}"))?;
    if token.v != 1 {
        return Err("This license is from a newer LabForge. Update the launcher.".into());
    }
    if token.machine != machine {
        return Err("This license was issued to a different computer.".into());
    }
    if !token.entitlements.iter().any(|e| e == "pro") {
        return Err("This license doesn't include pro labs.".into());
    }
    Ok(token)
}

pub fn status(stored: Option<&Stored>, machine: &str, now: u64) -> Status {
    let available = server_url().is_some() && pubkey().is_some();
    let none = Status { available, state: State::None, expires_at: None, key_hint: None };
    let (Some(s), Some(pk)) = (stored, pubkey()) else { return none };
    match verify_token(&pk, &s.token, &s.signature, machine) {
        Ok(t) => Status {
            available,
            state: if now < t.expires_at { State::Active } else { State::Expired },
            expires_at: Some(t.expires_at),
            key_hint: Some(key_hint(&s.key)),
        },
        Err(_) => none,
    }
}

pub fn is_active(stored: Option<&Stored>, machine: &str, now: u64) -> bool {
    status(stored, machine, now).state == State::Active
}

/// True when the stored token should be renewed (expiring soon or already expired).
pub fn needs_refresh(stored: Option<&Stored>, machine: &str, now: u64) -> bool {
    let (Some(s), Some(pk)) = (stored, pubkey()) else { return false };
    match verify_token(&pk, &s.token, &s.signature, machine) {
        Ok(t) => now + REFRESH_WINDOW_SECS >= t.expires_at,
        Err(_) => true,
    }
}

fn key_hint(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("…{tail}")
}

/// Request headers that prove the license to the license server.
pub fn auth_headers(stored: &Stored) -> Vec<(String, String)> {
    let token = base64::engine::general_purpose::STANDARD.encode(stored.token.as_bytes());
    vec![("x-license-token".into(), token), ("x-license-signature".into(), stored.signature.clone())]
}

/// Asks the server to issue a token for `key`. The result is verified before it is returned.
pub async fn request(key: &str, machine: &str) -> Result<Stored, String> {
    let server = server_url().ok_or("Licensing isn't available in this build.")?;
    let pubkey = pubkey().ok_or("Licensing isn't available in this build.")?;
    let key = key.trim();
    if key.is_empty() || key.len() > 200 {
        return Err("Enter your license key.".into());
    }
    let url = format!("{server}/v1/license");
    let client = reqwest::Client::builder()
        .user_agent(concat!("LabForge/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let body = serde_json::json!({ "key": key, "machine": machine }).to_string();
    let resp = client
        .post(&url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach the license server: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["error"].as_str().map(String::from))
            .unwrap_or_else(|| format!("the license server returned {status}"));
        return Err(msg);
    }
    #[derive(Deserialize)]
    struct Reply {
        token: String,
        signature: String,
    }
    let reply: Reply = serde_json::from_str(&text).map_err(|e| format!("bad reply from the license server: {e}"))?;
    verify_token(&pubkey, &reply.token, &reply.signature, machine)?;
    Ok(Stored { key: key.to_string(), token: reply.token, signature: reply.signature })
}

pub fn load(path: &Path) -> Option<Stored> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn save(path: &Path, stored: &Stored) -> Result<(), String> {
    let json = serde_json::to_string_pretty(stored).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A random id for this install, created on first use. Tokens are bound to it.
pub fn machine_id(data_dir: &Path) -> String {
    let path = data_dir.join("machine.id");
    if let Ok(id) = fs::read_to_string(&path) {
        let id = id.trim().to_string();
        if id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()) {
            return id;
        }
    }
    let id = hex::encode(rand::random::<[u8; 16]>());
    let _ = fs::write(&path, &id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key pair; the signature covers TOKEN exactly.
    const PUB: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDI2QzdGOUY0MDg3QTQyRTQKUldUa1Fub0k5UG5ISnB5c2xVMFZ4dXE1RnNUQVVzdTBEazhHYzBncWtmSENJYzY2K1RPZzgrSkMK";

    #[test]
    fn rejects_forged_and_foreign_tokens() {
        let token = r#"{"v":1,"license":"abc","machine":"m1","entitlements":["pro"],"issued_at":1,"expires_at":9999999999}"#;
        // no valid signature exists for this body under PUB
        assert!(verify_token(PUB, token, "AAAA", "m1").is_err());
        assert!(verify_token(PUB, token, "", "m1").is_err());
    }

    #[test]
    fn key_hint_shows_only_the_tail() {
        assert_eq!(key_hint("LF-TEST-ABCD-1234"), "…1234");
        assert_eq!(key_hint("ab"), "…ab");
    }

    #[test]
    fn machine_id_is_stable_and_valid() {
        let dir = std::env::temp_dir().join(format!("labforge-mid-{:08x}", rand::random::<u32>()));
        fs::create_dir_all(&dir).unwrap();
        let a = machine_id(&dir);
        assert_eq!(a.len(), 32);
        assert_eq!(a, machine_id(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_license_means_none_and_never_refreshes() {
        let s = status(None, "m1", 1);
        assert_eq!(s.state, State::None);
        assert!(!needs_refresh(None, "m1", 1));
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use crate::{catalog::Tier, packs};

    /// Needs the dev server: `labforge-license-server` plus LABFORGE_LICENSE_URL / LABFORGE_LICENSE_PUBKEY.
    #[tokio::test]
    #[ignore = "needs the dev license server"]
    async fn activate_then_install_a_pro_lab() {
        let server = server_url().expect("LABFORGE_LICENSE_URL");
        let pubkey = pubkey().expect("LABFORGE_LICENSE_PUBKEY");
        let machine = "0123456789abcdef0123456789abcdef";

        // keys the server rejects
        assert!(request("nope", machine).await.unwrap_err().contains("wasn't found"));
        assert!(request("LF-REVOKED-1", machine).await.unwrap_err().contains("refunded"));

        // a valid key yields a token that verifies, only for this machine
        let stored = request("LF-TEST-ABCD-1234", machine).await.unwrap();
        let t = verify_token(&pubkey, &stored.token, &stored.signature, machine).unwrap();
        assert!(t.expires_at > now_secs());
        assert!(verify_token(&pubkey, &stored.token, &stored.signature, "ffffffffffffffffffffffffffffffff").is_err());
        assert!(is_active(Some(&stored), machine, now_secs()));
        assert!(!is_active(Some(&stored), machine, t.expires_at + 1), "expired tokens aren't active");
        assert_eq!(status(Some(&stored), machine, t.expires_at + 1).state, State::Expired);

        // the pro index is public and signed
        let index = packs::fetch_signed_index(&format!("{server}/v1/index"), &pubkey, &[]).await.unwrap();
        let entry = index.labs.iter().find(|e| e.tier == Tier::Pro).expect("a pro lab").clone();
        assert!(entry.url.starts_with(&server));
        // a tampered signature is refused
        assert!(packs::fetch_signed_index(&format!("{server}/v1/index"), &stored.signature, &[]).await.is_err());

        // downloads need a license
        assert!(packs::fetch_pack(&entry, &[]).await.unwrap_err().contains("401"));
        // a token edited after signing (here: expiry pushed far into the future) must be refused
        let forged = Stored { token: stored.token.replace(&t.expires_at.to_string(), "9999999999"), ..stored.clone() };
        assert_ne!(forged.token, stored.token);
        assert!(packs::fetch_pack(&entry, &auth_headers(&forged)).await.unwrap_err().contains("401"));
        let data = packs::fetch_pack(&entry, &auth_headers(&stored)).await.unwrap();

        // installing needs allow_pro
        let root = std::env::temp_dir().join(format!("labforge-pro-{:08x}", rand::random::<u32>()));
        assert!(packs::install(&entry, &data, &root, "0.2.1", false).unwrap_err().contains("license"));
        packs::install(&entry, &data, &root, "0.2.1", true).unwrap();
        assert!(root.join("demo-pro/lab.yaml").is_file());
        let _ = fs::remove_dir_all(&root);
    }
}
