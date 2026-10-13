//! Lab manifests (`labs/<id>/lab.yaml`) and the catalog built from them.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Track {
    Web,
    Api,
    Network,
    Linux,
    Windows,
    Ad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    #[default]
    Free,
    Pro,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
    Insane,
}

impl Difficulty {
    pub fn multiplier(self) -> f64 {
        match self {
            Difficulty::Easy => 1.0,
            Difficulty::Medium => 1.5,
            Difficulty::Hard => 2.0,
            Difficulty::Insane => 3.0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Difficulty::Easy => "easy",
            Difficulty::Medium => "medium",
            Difficulty::Hard => "hard",
            Difficulty::Insane => "insane",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resources {
    pub ram_mb: u32,
    pub cpus: f32,
    pub disk_mb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Port {
    pub port: u16,
    #[serde(default = "default_proto")]
    pub proto: String,
    pub service: String,
}

fn default_proto() -> String {
    "tcp".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    /// Hostname shown to the learner, e.g. `web01`.
    pub name: String,
    /// Service name in the lab's compose file.
    pub service: String,
    pub os: String,
    #[serde(default)]
    pub ports: Vec<Port>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagSpec {
    pub id: String,
    pub label: String,
    pub host: String,
    /// Absolute path inside the host's container where the flag is planted.
    pub path: String,
    #[serde(default = "default_owner")]
    pub owner: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Share of the lab's XP this flag is worth (all shares sum to 1.0).
    pub xp_share: f64,
}

fn default_version() -> String {
    "1.0.0".into()
}

fn default_owner() -> String {
    "root:root".into()
}

fn default_mode() -> String {
    "0400".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hint {
    pub text: String,
    /// Share of the lab's XP deducted when revealed.
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapEdge {
    pub from: String,
    pub to: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MapSpec {
    /// Level 3 relationships (pivots, shared creds, trusts).
    #[serde(default)]
    pub edges: Vec<MapEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabManifest {
    pub id: String,
    pub name: String,
    /// Semantic version of the lab package. Higher wins when a lab is both bundled and installed.
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub tier: Tier,
    /// Oldest launcher version that can run this lab.
    #[serde(default)]
    pub min_app_version: Option<String>,
    pub track: Track,
    pub blurb: String,
    #[serde(default)]
    pub story: String,
    #[serde(default)]
    pub skills: Vec<String>,
    pub xp: u32,
    pub est_minutes: u32,
    pub difficulties: Vec<Difficulty>,
    /// What each difficulty changes, without spoiling the path.
    #[serde(default)]
    pub difficulty_notes: BTreeMap<Difficulty, String>,
    pub resources: Resources,
    /// Third octet of the lab subnet: 10.66.<octet>.0/24.
    pub subnet_octet: u8,
    pub hosts: Vec<Host>,
    pub flags: Vec<FlagSpec>,
    #[serde(default)]
    pub hints: Vec<Hint>,
    #[serde(default)]
    pub map: MapSpec,
}

impl LabManifest {
    pub fn subnet(&self) -> String {
        format!("10.66.{}.0/24", self.subnet_octet)
    }

    pub fn gateway(&self) -> String {
        format!("10.66.{}.1", self.subnet_octet)
    }

    /// Hosts get .10, .11, ... in manifest order.
    pub fn host_ip(&self, index: usize) -> String {
        format!("10.66.{}.{}", self.subnet_octet, 10 + index)
    }

    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let valid_id = !self.id.is_empty()
            && self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid_id {
            problems.push(format!("id `{}` must be lowercase letters, digits and dashes", self.id));
        }
        if semver::Version::parse(&self.version).is_err() {
            problems.push(format!("version `{}` must be a semantic version like 1.2.0", self.version));
        }
        if let Some(min) = &self.min_app_version {
            if semver::Version::parse(min).is_err() {
                problems.push(format!("min_app_version `{min}` must be a semantic version"));
            }
        }
        if self.difficulties.is_empty() {
            problems.push("at least one difficulty is required".into());
        }
        if self.hosts.is_empty() {
            problems.push("at least one host is required".into());
        }
        if !(2..=250).contains(&self.subnet_octet) {
            problems.push("subnet_octet must be between 2 and 250".into());
        }
        let host_names: HashSet<&str> = self.hosts.iter().map(|h| h.name.as_str()).collect();
        if host_names.len() != self.hosts.len() {
            problems.push("host names must be unique".into());
        }
        let mut flag_ids = HashSet::new();
        for f in &self.flags {
            if !host_names.contains(f.host.as_str()) {
                problems.push(format!("flag `{}` references unknown host `{}`", f.id, f.host));
            }
            if !flag_ids.insert(f.id.as_str()) {
                problems.push(format!("duplicate flag id `{}`", f.id));
            }
            if !f.path.starts_with('/') {
                problems.push(format!("flag `{}` path must be absolute", f.id));
            }
        }
        let share: f64 = self.flags.iter().map(|f| f.xp_share).sum();
        if self.flags.is_empty() || (share - 1.0).abs() > 0.001 {
            problems.push(format!("flag xp_share values must sum to 1.0 (got {share:.2})"));
        }
        for e in &self.map.edges {
            if !host_names.contains(e.from.as_str()) || !host_names.contains(e.to.as_str()) {
                problems.push(format!("map edge {} -> {} references an unknown host", e.from, e.to));
            }
        }
        problems
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Shipped inside the launcher.
    Bundled,
    /// Downloaded later into the app data directory.
    Installed,
}

#[derive(Debug, Clone)]
pub struct Lab {
    pub manifest: LabManifest,
    pub dir: PathBuf,
    pub source: Source,
}

impl Lab {
    pub fn version(&self) -> semver::Version {
        semver::Version::parse(&self.manifest.version).unwrap_or_else(|_| semver::Version::new(0, 0, 0))
    }

    pub fn compose_files(&self, difficulty: Difficulty) -> Vec<PathBuf> {
        let mut files = vec![self.dir.join("compose.yaml")];
        let overlay = self.dir.join(format!("compose.{}.yaml", difficulty.as_str()));
        if overlay.exists() {
            files.push(overlay);
        }
        files
    }
}

#[derive(Debug, Default)]
pub struct Catalog {
    pub labs: Vec<Lab>,
    /// Labs that failed to load, as "<dir>: <problem>".
    pub errors: Vec<String>,
}

impl Catalog {
    /// Loads the bundled labs, then the installed ones. A lab present in both keeps the higher
    /// version (bundled wins a tie), and labs that need a newer launcher are skipped.
    pub fn load_roots(bundled: &Path, installed: &Path, app_version: &str) -> Catalog {
        let mut out = Catalog::load_dir(bundled, Source::Bundled);
        let extra = if installed.is_dir() { Catalog::load_dir(installed, Source::Installed) } else { Catalog::default() };
        out.errors.extend(extra.errors);
        let app = semver::Version::parse(app_version).ok();
        for lab in extra.labs {
            if let (Some(app), Some(min)) = (&app, lab.manifest.min_app_version.as_deref().and_then(|v| semver::Version::parse(v).ok())) {
                if *app < min {
                    out.errors.push(format!("{}: needs LabForge {min} or newer (this is {app})", lab.manifest.id));
                    continue;
                }
            }
            match out.labs.iter().position(|l| l.manifest.id == lab.manifest.id) {
                Some(i) if lab.version() > out.labs[i].version() => out.labs[i] = lab,
                Some(_) => {}
                None => out.labs.push(lab),
            }
        }
        out.labs.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        let mut octets: HashSet<u8> = HashSet::new();
        let mut kept = Vec::new();
        for lab in std::mem::take(&mut out.labs) {
            if octets.insert(lab.manifest.subnet_octet) {
                kept.push(lab);
            } else {
                out.errors.push(format!("{}: subnet_octet {} is already used by another lab", lab.manifest.id, lab.manifest.subnet_octet));
            }
        }
        out.labs = kept;
        out
    }

    pub fn load_dir(root: &Path, source: Source) -> Catalog {
        let mut catalog = Catalog::default();
        let entries = match fs::read_dir(root) {
            Ok(e) => e,
            Err(e) => {
                catalog.errors.push(format!("{}: {e}", root.display()));
                return catalog;
            }
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join("lab.yaml").is_file())
            .collect();
        dirs.sort();
        for dir in dirs {
            let name = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
            let manifest = fs::read_to_string(dir.join("lab.yaml"))
                .map_err(|e| e.to_string())
                .and_then(|s| serde_yaml::from_str::<LabManifest>(&s).map_err(|e| e.to_string()));
            match manifest {
                Ok(m) => {
                    let problems = m.validate();
                    if !dir.join("compose.yaml").is_file() {
                        catalog.errors.push(format!("{name}: missing compose.yaml"));
                    } else if problems.is_empty() {
                        catalog.labs.push(Lab { manifest: m, dir, source });
                    } else {
                        catalog.errors.push(format!("{name}: {}", problems.join("; ")));
                    }
                }
                Err(e) => catalog.errors.push(format!("{name}: {e}")),
            }
        }
        catalog
    }

    pub fn get(&self, id: &str) -> Option<&Lab> {
        self.labs.iter().find(|l| l.manifest.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> LabManifest {
        serde_yaml::from_str(
            r#"
id: test-lab
name: Test
track: web
blurb: b
xp: 100
est_minutes: 30
difficulties: [easy, medium]
difficulty_notes: { easy: "soft", medium: "harder" }
resources: { ram_mb: 512, cpus: 1, disk_mb: 300 }
subnet_octet: 11
hosts:
  - { name: web01, service: web, os: Linux, ports: [{ port: 80, service: http }] }
flags:
  - { id: user, label: User, host: web01, path: /home/u/user.txt, xp_share: 0.4 }
  - { id: root, label: Root, host: web01, path: /root/root.txt, xp_share: 0.6 }
"#,
        )
        .unwrap()
    }

    #[test]
    fn parses_and_validates() {
        let m = manifest();
        assert!(m.validate().is_empty(), "{:?}", m.validate());
        assert_eq!(m.host_ip(0), "10.66.11.10");
        assert_eq!(m.subnet(), "10.66.11.0/24");
        assert_eq!(m.flags[0].mode, "0400");
        assert_eq!(m.difficulty_notes[&Difficulty::Medium], "harder");
    }

    #[test]
    fn rejects_bad_shares_and_hosts() {
        let mut m = manifest();
        m.flags[0].xp_share = 0.9;
        m.flags[1].host = "nope".into();
        let p = m.validate();
        assert!(p.iter().any(|s| s.contains("sum to 1.0")));
        assert!(p.iter().any(|s| s.contains("unknown host")));
    }

    #[test]
    fn bundled_labs_are_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../labs");
        let catalog = Catalog::load_dir(&root, Source::Bundled);
        assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
        assert!(!catalog.labs.is_empty());
        let octets: HashSet<u8> = catalog.labs.iter().map(|l| l.manifest.subnet_octet).collect();
        assert_eq!(octets.len(), catalog.labs.len(), "labs must use distinct subnets");
    }
}
