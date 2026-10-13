//! Lab packs: a signed index, verified downloads and safe unpacking into the app data directory.
//!
//! The index (`index.json`) lists every installable lab and the SHA-256 of its pack. It is signed
//! with the same minisign key that signs launcher updates (`index.json.sig`), so a pack is trusted
//! only if the signed index vouches for its hash. Packs are `.tar.gz` archives with one top-level
//! folder named after the lab id.

use crate::catalog::{Catalog, LabManifest, Tier};
use base64::Engine;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

pub const INDEX_URL: &str = "https://github.com/mattrfield/labforge/releases/download/labs/index.json";

const MAX_INDEX_BYTES: u64 = 1024 * 1024;
const MAX_PACK_BYTES: u64 = 100 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 300 * 1024 * 1024;
const MAX_FILES: usize = 5000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub min_app_version: Option<String>,
    #[serde(default)]
    pub tier: Tier,
    #[serde(default)]
    pub blurb: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Index {
    pub schema: u32,
    pub labs: Vec<IndexEntry>,
}

/// One lab the learner can install or update.
#[derive(Debug, Clone, Serialize)]
pub struct Offer {
    pub id: String,
    pub name: String,
    pub version: String,
    pub installed_version: Option<String>,
    pub tier: Tier,
    pub size: u64,
    pub blurb: String,
    /// Pro labs can't be installed until licensing exists.
    pub locked: bool,
    /// Set when this launcher is too old for the lab.
    pub blocked: Option<String>,
}

// ------------------------------------------------------------ signing ----

fn b64_to_string(s: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| format!("bad base64: {e}"))?;
    String::from_utf8(bytes).map_err(|e| format!("bad utf-8: {e}"))
}

/// Checks `data` against a Tauri-style signature (base64 of a minisign signature file) using the
/// updater's public key (base64 of a minisign public key file).
pub fn verify_signature(pubkey_b64: &str, data: &[u8], sig_b64: &str) -> Result<(), String> {
    let pk = minisign_verify::PublicKey::decode(&b64_to_string(pubkey_b64)?).map_err(|e| format!("public key: {e}"))?;
    let sig = minisign_verify::Signature::decode(&b64_to_string(sig_b64)?).map_err(|e| format!("signature: {e}"))?;
    pk.verify(data, &sig, true).map_err(|e| format!("signature check failed: {e}"))
}

pub fn parse_index(pubkey_b64: &str, index: &[u8], sig_b64: &str) -> Result<Index, String> {
    verify_signature(pubkey_b64, index, sig_b64).map_err(|e| format!("The lab index isn't signed by LabForge ({e})."))?;
    let parsed: Index = serde_json::from_slice(index).map_err(|e| format!("Couldn't read the lab index: {e}"))?;
    if parsed.schema != 1 {
        return Err("The lab index is from a newer LabForge. Update the launcher to see new labs.".into());
    }
    Ok(parsed)
}

// ----------------------------------------------------------- download ----

fn check_url(url: &str) -> Result<(), String> {
    let u = reqwest::Url::parse(url).map_err(|e| format!("bad url `{url}`: {e}"))?;
    let local = matches!(u.host_str(), Some("localhost" | "127.0.0.1"));
    match u.scheme() {
        "https" => Ok(()),
        "http" if local => Ok(()),
        _ => Err(format!("refusing to download from `{url}`: https is required")),
    }
}

pub async fn fetch_limited(url: &str, max: u64) -> Result<Vec<u8>, String> {
    fetch_with(url, max, &[]).await.map(|(body, _)| body)
}

/// Like `fetch_limited`, with extra request headers; also returns the `X-Signature` response header.
pub async fn fetch_with(url: &str, max: u64, headers: &[(String, String)]) -> Result<(Vec<u8>, Option<String>), String> {
    check_url(url)?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("LabForge/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let mut resp = req.send().await.map_err(|e| format!("couldn't reach {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{url} returned {}", resp.status()));
    }
    if resp.content_length().is_some_and(|n| n > max) {
        return Err(format!("{url} is larger than the {} MB limit", max / 1024 / 1024));
    }
    let signature = resp.headers().get("x-signature").and_then(|v| v.to_str().ok()).map(String::from);
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        out.extend_from_slice(&chunk);
        if out.len() as u64 > max {
            return Err(format!("{url} is larger than the {} MB limit", max / 1024 / 1024));
        }
    }
    Ok((out, signature))
}

pub async fn fetch_index(index_url: &str, pubkey_b64: &str) -> Result<Index, String> {
    let body = fetch_limited(index_url, MAX_INDEX_BYTES).await?;
    let sig = fetch_limited(&format!("{index_url}.sig"), MAX_INDEX_BYTES).await?;
    let sig = String::from_utf8(sig).map_err(|e| e.to_string())?;
    parse_index(pubkey_b64, &body, &sig)
}

pub async fn fetch_pack(entry: &IndexEntry, headers: &[(String, String)]) -> Result<Vec<u8>, String> {
    fetch_with(&entry.url, MAX_PACK_BYTES.min(entry.size.max(1)), headers).await.map(|(body, _)| body)
}

/// Fetches a signed index whose signature comes back in the `X-Signature` header (the pro index).
pub async fn fetch_signed_index(url: &str, pubkey_b64: &str, headers: &[(String, String)]) -> Result<Index, String> {
    let (body, sig) = fetch_with(url, MAX_INDEX_BYTES, headers).await?;
    let sig = sig.ok_or("The license server didn't sign the pro index.")?;
    parse_index(pubkey_b64, &body, &sig)
}

// ------------------------------------------------------------- offers ----

/// What the index offers that the learner doesn't already have at that version or newer.
pub fn offers(index: &Index, catalog: &Catalog, app_version: &str, licensed: bool) -> Vec<Offer> {
    let app = semver::Version::parse(app_version).ok();
    let mut out = Vec::new();
    for e in &index.labs {
        let Ok(version) = semver::Version::parse(&e.version) else { continue };
        let have = catalog.get(&e.id);
        if have.is_some_and(|l| l.version() >= version) {
            continue;
        }
        let blocked = match (&app, e.min_app_version.as_deref().map(semver::Version::parse)) {
            (Some(app), Some(Ok(min))) if *app < min => Some(format!("Needs LabForge {min} or newer")),
            _ => None,
        };
        out.push(Offer {
            id: e.id.clone(),
            name: e.name.clone(),
            version: e.version.clone(),
            installed_version: have.map(|l| l.manifest.version.clone()),
            tier: e.tier,
            size: e.size,
            blurb: e.blurb.clone(),
            locked: e.tier == Tier::Pro && !licensed,
            blocked,
        });
    }
    out
}

// ------------------------------------------------------------ install ----

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Unpacks a pack into `dest`, stripping the top-level `<id>/` folder. Only plain files and
/// directories are accepted, every path must stay inside `dest`, and size and count are capped.
pub fn unpack(data: &[u8], id: &str, dest: &Path) -> Result<(), String> {
    let limited = GzDecoder::new(data).take(MAX_UNPACKED_BYTES + 1);
    let mut archive = tar::Archive::new(limited);
    let (mut total, mut files) = (0u64, 0usize);
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in archive.entries().map_err(|e| format!("not a valid lab pack: {e}"))? {
        let mut entry = entry.map_err(|e| format!("not a valid lab pack: {e}"))?;
        let kind = entry.header().entry_type();
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let mut parts = path.components();
        match parts.next() {
            Some(Component::Normal(top)) if top == id => {}
            _ => return Err(format!("pack entry `{}` is outside the `{id}/` folder", path.display())),
        }
        let mut rel = PathBuf::new();
        for c in parts {
            match c {
                Component::Normal(p) => rel.push(p),
                _ => return Err(format!("pack entry `{}` has an unsafe path", path.display())),
            }
        }
        if rel.as_os_str().is_empty() {
            continue; // the top-level folder itself
        }
        let target = dest.join(&rel);
        if kind.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else if kind.is_file() {
            files += 1;
            if files > MAX_FILES {
                return Err(format!("pack has more than {MAX_FILES} files"));
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = fs::File::create(&target).map_err(|e| e.to_string())?;
            total += io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
            if total > MAX_UNPACKED_BYTES {
                return Err(format!("pack unpacks to more than {} MB", MAX_UNPACKED_BYTES / 1024 / 1024));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = entry.header().mode().unwrap_or(0o644) & 0o755;
                fs::set_permissions(&target, fs::Permissions::from_mode(mode | 0o600)).map_err(|e| e.to_string())?;
            }
        } else {
            return Err(format!("pack entry `{}` is a link or special file, which isn't allowed", path.display()));
        }
    }
    Ok(())
}

fn check_staged(dir: &Path, entry: &IndexEntry, app_version: &str) -> Result<(), String> {
    let yaml = fs::read_to_string(dir.join("lab.yaml")).map_err(|_| "the pack has no lab.yaml".to_string())?;
    let manifest: LabManifest = serde_yaml::from_str(&yaml).map_err(|e| format!("lab.yaml: {e}"))?;
    let problems = manifest.validate();
    if !problems.is_empty() {
        return Err(format!("lab.yaml: {}", problems.join("; ")));
    }
    if manifest.tier != entry.tier {
        return Err(format!("the pack's tier ({:?}) doesn't match the index ({:?})", manifest.tier, entry.tier));
    }
    if manifest.id != entry.id || manifest.version != entry.version {
        return Err(format!(
            "the pack contains {} {} but the index promised {} {}",
            manifest.id, manifest.version, entry.id, entry.version
        ));
    }
    if !dir.join("compose.yaml").is_file() {
        return Err("the pack has no compose.yaml".into());
    }
    if let (Ok(app), Some(Ok(min))) = (semver::Version::parse(app_version), manifest.min_app_version.as_deref().map(semver::Version::parse)) {
        if app < min {
            return Err(format!("needs LabForge {min} or newer"));
        }
    }
    Ok(())
}

/// Verifies, unpacks and installs one pack into `<installed_root>/<id>`, replacing any older copy.
pub fn install(entry: &IndexEntry, data: &[u8], installed_root: &Path, app_version: &str, allow_pro: bool) -> Result<(), String> {
    if entry.tier == Tier::Pro && !allow_pro {
        return Err("Pro labs need an active license.".into());
    }
    let valid_id = !entry.id.is_empty() && entry.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid_id {
        return Err(format!("invalid lab id `{}`", entry.id));
    }
    if sha256_hex(data) != entry.sha256.to_ascii_lowercase() {
        return Err("the download doesn't match the index checksum, so it was discarded".into());
    }
    fs::create_dir_all(installed_root).map_err(|e| e.to_string())?;
    let nonce: u32 = rand::random();
    let staging = installed_root.join(format!(".staging-{}-{nonce:08x}", entry.id));
    let result = unpack(data, &entry.id, &staging).and_then(|_| check_staged(&staging, entry, app_version));
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    let target = installed_root.join(&entry.id);
    let old = installed_root.join(format!(".old-{}-{nonce:08x}", entry.id));
    let had_old = target.exists();
    if had_old {
        fs::rename(&target, &old).map_err(|e| format!("couldn't replace the installed lab: {e}"))?;
    }
    if let Err(e) = fs::rename(&staging, &target) {
        if had_old {
            let _ = fs::rename(&old, &target);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("couldn't install the lab: {e}"));
    }
    if had_old {
        let _ = fs::remove_dir_all(&old);
    }
    Ok(())
}

/// Removes staging and backup folders left behind by an interrupted install.
pub fn clean_leftovers(installed_root: &Path) {
    if let Ok(entries) = fs::read_dir(installed_root) {
        for e in entries.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(".staging-") || name.starts_with(".old-") {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};

    const MANIFEST: &str = "id: demo\nname: Demo\nversion: 1.2.0\ntrack: web\nblurb: b\nxp: 100\nest_minutes: 10\ndifficulties: [easy]\nresources: { ram_mb: 64, cpus: 0.5, disk_mb: 10 }\nsubnet_octet: 99\nhosts:\n  - { name: web01, service: web, os: Linux }\nflags:\n  - { id: user, label: User, host: web01, path: /u.txt, xp_share: 1.0 }\n";

    fn tmp(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("labforge-test-{label}-{:08x}", rand::random::<u32>()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    /// Builds a pack from (path, contents) pairs. `raw` entries bypass the tar crate's path checks.
    fn pack(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            for (path, body) in files {
                let mut h = tar::Header::new_gnu();
                h.set_size(body.len() as u64);
                h.set_mode(0o644);
                h.set_entry_type(tar::EntryType::Regular);
                let name = &mut h.as_old_mut().name;
                name.fill(0);
                name[..path.len()].copy_from_slice(path.as_bytes());
                h.set_cksum();
                b.append(&h, *body).unwrap();
            }
            b.finish().unwrap();
        }
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    fn entry_for(data: &[u8]) -> IndexEntry {
        IndexEntry {
            id: "demo".into(),
            name: "Demo".into(),
            version: "1.2.0".into(),
            url: "https://example.com/demo.tar.gz".into(),
            sha256: sha256_hex(data),
            size: data.len() as u64,
            min_app_version: None,
            tier: Tier::Free,
            blurb: String::new(),
        }
    }

    fn good_pack() -> Vec<u8> {
        pack(&[("demo/lab.yaml", MANIFEST.as_bytes()), ("demo/compose.yaml", b"services: {}\n"), ("demo/app/run.sh", b"#!/bin/sh\n")])
    }

    #[test]
    fn installs_a_good_pack_and_replaces_older_copies() {
        let root = tmp("install");
        let data = good_pack();
        install(&entry_for(&data), &data, &root, "0.1.1", false).unwrap();
        assert!(root.join("demo/lab.yaml").is_file());
        assert!(root.join("demo/app/run.sh").is_file());
        fs::write(root.join("demo/stale.txt"), "old").unwrap();
        install(&entry_for(&data), &data, &root, "0.1.1", false).unwrap();
        assert!(!root.join("demo/stale.txt").exists(), "reinstall must replace the folder");
        let leftovers: Vec<_> = fs::read_dir(&root).unwrap().filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(leftovers, vec!["demo"], "no staging or backup folders left");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_bad_checksums() {
        let root = tmp("sha");
        let data = good_pack();
        let mut e = entry_for(&data);
        e.sha256 = "00".repeat(32);
        let err = install(&e, &data, &root, "0.1.1", false).unwrap_err();
        assert!(err.contains("checksum"), "{err}");
        assert!(!root.join("demo").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_path_traversal_and_foreign_folders() {
        for bad in ["demo/../../evil.txt", "/etc/passwd", "other/lab.yaml"] {
            let root = tmp("trav");
            let data = pack(&[(bad, b"x"), ("demo/lab.yaml", MANIFEST.as_bytes())]);
            let err = install(&entry_for(&data), &data, &root, "0.1.1", false).unwrap_err();
            assert!(err.contains("outside") || err.contains("unsafe"), "{bad}: {err}");
            assert!(!root.parent().unwrap().join("evil.txt").exists());
            let _ = fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn rejects_symlinks() {
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_link_name("/etc").unwrap();
            h.set_path("demo/link").unwrap();
            h.set_cksum();
            b.append(&h, io::empty()).unwrap();
            b.finish().unwrap();
        }
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar_bytes).unwrap();
        let data = gz.finish().unwrap();
        let root = tmp("link");
        let err = install(&entry_for(&data), &data, &root, "0.1.1", false).unwrap_err();
        assert!(err.contains("link"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_manifest_mismatch_and_old_launchers() {
        let root = tmp("mismatch");
        let data = good_pack();
        let mut e = entry_for(&data);
        e.version = "9.9.9".into();
        assert!(install(&e, &data, &root, "0.1.1", false).unwrap_err().contains("promised"));
        let with_min = MANIFEST.replace("track: web", "track: web\nmin_app_version: 2.0.0");
        let data = pack(&[("demo/lab.yaml", with_min.as_bytes()), ("demo/compose.yaml", b"x")]);
        assert!(install(&entry_for(&data), &data, &root, "0.1.1", false).unwrap_err().contains("2.0.0"));
        assert!(!root.join("demo").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn refuses_pro_labs() {
        let root = tmp("pro");
        let data = good_pack();
        let mut e = entry_for(&data);
        e.tier = Tier::Pro;
        assert!(install(&e, &data, &root, "0.1.1", false).unwrap_err().contains("license"));
        // with a license the pack must really be a pro pack
        assert!(install(&e, &data, &root, "0.1.1", true).unwrap_err().contains("tier"));
        let pro = MANIFEST.replace("track: web", "track: web
tier: pro");
        let data = pack(&[("demo/lab.yaml", pro.as_bytes()), ("demo/compose.yaml", b"services: {}
")]);
        let mut e = entry_for(&data);
        e.tier = Tier::Pro;
        install(&e, &data, &root, "0.1.1", true).unwrap();
        assert!(root.join("demo/lab.yaml").is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn only_https_or_local_urls() {
        assert!(check_url("https://github.com/x").is_ok());
        assert!(check_url("http://127.0.0.1:8099/x").is_ok());
        assert!(check_url("http://example.com/x").is_err());
        assert!(check_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn offers_new_and_newer_labs_only() {
        let root = tmp("offers");
        let data = good_pack();
        install(&entry_for(&data), &data, &root, "0.1.1", false).unwrap();
        let catalog = crate::catalog::Catalog::load_roots(&tmp("none"), &root, "0.1.1");
        assert_eq!(catalog.labs.len(), 1, "{:?}", catalog.errors);
        let mk = |id: &str, v: &str, min: Option<&str>| IndexEntry {
            id: id.into(),
            version: v.into(),
            min_app_version: min.map(String::from),
            ..entry_for(&data)
        };
        let index = Index { schema: 1, labs: vec![mk("demo", "1.2.0", None), mk("demo", "1.3.0", None), mk("fresh", "1.0.0", Some("5.0.0"))] };
        let o = offers(&index, &catalog, "0.1.1", false);
        assert_eq!(o.len(), 2);
        assert_eq!((o[0].id.as_str(), o[0].installed_version.as_deref()), ("demo", Some("1.2.0")));
        assert!(o[1].blocked.as_deref().unwrap().contains("5.0.0"));
        let _ = fs::remove_dir_all(&root);
    }

    // A throwaway key pair made with `cargo tauri signer generate`; signs `{"schema":1,"labs":[]}`.
    const TEST_PUB: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDI2QzdGOUY0MDg3QTQyRTQKUldUa1Fub0k5UG5ISnB5c2xVMFZ4dXE1RnNUQVVzdTBEazhHYzBncWtmSENJYzY2K1RPZzgrSkMK";
    const TEST_SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUa1Fub0k5UG5ISnBUQnVZVm9CSHZ1TjNzNTlHTkdPNEl0WGxjQVAwV25mcnhjWnJ2V1c3bGZsSGFmRlpZSmxiSmljYkg1OXdSTzNDWVdnemtrUGNEQlpFWDIxcGNIc2dnPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkxOTExMTY0CWZpbGU6aW5kZXguanNvbgp1TFBZYmVzZStXdjBDVnpzcG1UTitaRkxVT3ZYb2hKY001dzIxcFBqUFBLT2JrbUZteDNhVENhTzVsQzAwSXByU2NqY1h4b01CTWdDQzJjbUJQNGFBUT09Cg==";
    const TEST_INDEX: &[u8] = br#"{"schema":1,"labs":[]}"#;

    #[test]
    fn index_signature_is_enforced() {
        assert!(parse_index(TEST_PUB, TEST_INDEX, TEST_SIG).unwrap().labs.is_empty());
        let tampered = br#"{"schema":1,"labs":[{"id":"x"}]}"#;
        assert!(parse_index(TEST_PUB, tampered, TEST_SIG).unwrap_err().contains("isn't signed"));
        // the real launcher key must not accept the test signature
        let real = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDdGMUFEMTk1RjU5MDZENEMKUldSTWJaRDFsZEVhZjNXZlh3ZjdzM0pOQUNVRlc4bS84aEdxaXJwQXdZZDM2V2tQeEt2Q3o0TCsK";
        assert!(parse_index(real, TEST_INDEX, TEST_SIG).is_err());
        assert!(parse_index(TEST_PUB, TEST_INDEX, "not base64!").is_err());
    }

    #[test]
    fn newer_installed_lab_beats_bundled_and_ties_keep_bundled() {
        let bundled = tmp("bundled");
        let installed = tmp("installed");
        let write = |root: &Path, version: &str| {
            let d = root.join("demo");
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("lab.yaml"), MANIFEST.replace("version: 1.2.0", &format!("version: {version}"))).unwrap();
            fs::write(d.join("compose.yaml"), "services: {}\n").unwrap();
        };
        write(&bundled, "1.0.0");
        write(&installed, "1.1.0");
        let c = Catalog::load_roots(&bundled, &installed, "0.1.1");
        assert_eq!(c.labs.len(), 1);
        assert_eq!(c.get("demo").unwrap().manifest.version, "1.1.0");
        assert_eq!(c.get("demo").unwrap().source, crate::catalog::Source::Installed);
        write(&installed, "1.0.0");
        let c = Catalog::load_roots(&bundled, &installed, "0.1.1");
        assert_eq!(c.get("demo").unwrap().source, crate::catalog::Source::Bundled);
        let _ = fs::remove_dir_all(&bundled);
        let _ = fs::remove_dir_all(&installed);
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Run by hand against a served index: LABFORGE_TEST_INDEX_URL + LABFORGE_TEST_PUBKEY.
    #[tokio::test]
    #[ignore = "needs a local lab index server"]
    async fn fetch_verify_and_install_from_a_served_index() {
        let url = std::env::var("LABFORGE_TEST_INDEX_URL").expect("LABFORGE_TEST_INDEX_URL");
        let pubkey = std::env::var("LABFORGE_TEST_PUBKEY").expect("LABFORGE_TEST_PUBKEY");
        let index = fetch_index(&url, &pubkey).await.unwrap();
        assert!(!index.labs.is_empty());
        let root = std::env::temp_dir().join(format!("labforge-live-{:08x}", rand::random::<u32>()));
        for e in &index.labs {
            let data = fetch_pack(e, &[]).await.unwrap();
            install(e, &data, &root, "0.1.1", false).unwrap();
        }
        let empty = root.join("empty-bundled");
        fs::create_dir_all(&empty).unwrap();
        let catalog = Catalog::load_roots(&empty, &root, "0.1.1");
        assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
        assert_eq!(catalog.labs.len(), index.labs.len());
        let _ = fs::remove_dir_all(&root);
    }
}
