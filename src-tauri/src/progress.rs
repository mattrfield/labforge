//! Learner profile, flag generation, XP and levels.

use crate::catalog::{Difficulty, LabManifest, Track};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

/// Share of a lab's XP lost for opening map level 2 / level 3.
pub const MAP_LEVEL_COST: [f64; 4] = [0.0, 0.0, 0.10, 0.15];
/// Hints and map reveals never cost more than this share of a lab's XP.
pub const MAX_PENALTY: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Skill {
    Beginner,
    Intermediate,
    Advanced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capture {
    pub lab_id: String,
    pub difficulty: Difficulty,
    pub flag_id: String,
    pub xp: u32,
    pub at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub username: String,
    pub skill: Skill,
    pub xp: u32,
    #[serde(default)]
    pub captures: Vec<Capture>,
    /// Labs fully pwned, as "<lab_id>:<difficulty>".
    #[serde(default)]
    pub pwned: Vec<String>,
    #[serde(default)]
    pub badges: Vec<String>,
    pub created_at: u64,
}

impl Profile {
    pub fn new(username: String, skill: Skill, now: u64) -> Self {
        Profile { username, skill, xp: 0, captures: vec![], pwned: vec![], badges: vec![], created_at: now }
    }

    fn already_captured(&self, lab: &str, d: Difficulty, flag: &str) -> bool {
        self.captures.iter().any(|c| c.lab_id == lab && c.difficulty == d && c.flag_id == flag)
    }
}

/// XP needed to reach `level` (level 1 = 0 XP): 0, 100, 300, 600, 1000, ...
pub fn xp_for_level(level: u32) -> u32 {
    50 * level * level.saturating_sub(1)
}

pub fn level_for_xp(xp: u32) -> u32 {
    let mut level = 1;
    while xp >= xp_for_level(level + 1) {
        level += 1;
    }
    level
}

pub fn title_for_level(level: u32) -> &'static str {
    match level {
        0..=2 => "Script Kiddie",
        3..=4 => "Recon Rookie",
        5..=7 => "Operator",
        8..=10 => "Pentester",
        11..=14 => "Red Teamer",
        _ => "Legend",
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LevelInfo {
    pub level: u32,
    pub title: &'static str,
    pub xp: u32,
    pub level_floor: u32,
    pub next_level_xp: u32,
}

pub fn level_info(xp: u32) -> LevelInfo {
    let level = level_for_xp(xp);
    LevelInfo {
        level,
        title: title_for_level(level),
        xp,
        level_floor: xp_for_level(level),
        next_level_xp: xp_for_level(level + 1),
    }
}

pub fn new_flag() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("LF{{{}}}", hex::encode(bytes))
}

pub fn hash_flag(flag: &str) -> String {
    hex::encode(Sha256::digest(flag.trim().as_bytes()))
}

/// The running lab's progress: flags (hashed), captures and assists used.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub lab_id: String,
    pub difficulty: Difficulty,
    pub flag_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub captured: Vec<String>,
    #[serde(default)]
    pub hints_revealed: usize,
    #[serde(default = "one")]
    pub map_level: u8,
}

fn one() -> u8 {
    1
}

impl Attempt {
    pub fn penalty(&self, lab: &LabManifest) -> f64 {
        let hints: f64 = lab.hints.iter().take(self.hints_revealed).map(|h| h.cost).sum();
        let map: f64 = MAP_LEVEL_COST.iter().take(self.map_level as usize + 1).sum();
        (hints + map).min(MAX_PENALTY)
    }

    pub fn complete(&self) -> bool {
        self.captured.len() == self.flag_hashes.len()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SubmitResult {
    pub correct: bool,
    pub flag_id: Option<String>,
    pub flag_label: Option<String>,
    pub already_captured: bool,
    pub xp_awarded: u32,
    pub lab_complete: bool,
    pub level_up: Option<LevelInfo>,
    pub new_badges: Vec<String>,
    pub level: LevelInfo,
}

pub fn submit_flag(
    profile: &mut Profile,
    attempt: &mut Attempt,
    lab: &LabManifest,
    all_labs: &[&LabManifest],
    value: &str,
    now: u64,
) -> SubmitResult {
    let hash = hash_flag(value);
    let matched = attempt.flag_hashes.iter().find(|(_, h)| **h == hash).map(|(id, _)| id.clone());
    let level_before = level_for_xp(profile.xp);
    let mut result = SubmitResult {
        correct: matched.is_some(),
        flag_id: matched.clone(),
        flag_label: None,
        already_captured: false,
        xp_awarded: 0,
        lab_complete: false,
        level_up: None,
        new_badges: vec![],
        level: level_info(profile.xp),
    };
    let Some(flag_id) = matched else { return result };
    let spec = lab.flags.iter().find(|f| f.id == flag_id);
    result.flag_label = spec.map(|f| f.label.clone());
    if attempt.captured.contains(&flag_id) {
        result.already_captured = true;
        result.lab_complete = attempt.complete();
        return result;
    }
    attempt.captured.push(flag_id.clone());

    // XP only for the first capture of this flag at this difficulty, ever.
    if !profile.already_captured(&lab.id, attempt.difficulty, &flag_id) {
        let share = spec.map(|f| f.xp_share).unwrap_or(0.0);
        let xp = (lab.xp as f64 * share * attempt.difficulty.multiplier() * (1.0 - attempt.penalty(lab))).round() as u32;
        profile.xp += xp;
        profile.captures.push(Capture {
            lab_id: lab.id.clone(),
            difficulty: attempt.difficulty,
            flag_id: flag_id.clone(),
            xp,
            at: now,
        });
        result.xp_awarded = xp;
    }

    if attempt.complete() {
        result.lab_complete = true;
        let key = format!("{}:{}", lab.id, attempt.difficulty.as_str());
        if !profile.pwned.contains(&key) {
            profile.pwned.push(key);
        }
        result.new_badges = award_badges(profile, attempt, lab, all_labs);
    }

    let level_after = level_for_xp(profile.xp);
    if level_after > level_before {
        result.level_up = Some(level_info(profile.xp));
    }
    result.level = level_info(profile.xp);
    result
}

fn award_badges(profile: &mut Profile, attempt: &Attempt, lab: &LabManifest, all_labs: &[&LabManifest]) -> Vec<String> {
    let mut earned = vec![];
    let mut give = |profile: &mut Profile, badge: String| {
        if !profile.badges.contains(&badge) {
            profile.badges.push(badge.clone());
            earned.push(badge);
        }
    };
    give(profile, "First Blood".into());
    if attempt.hints_revealed == 0 && attempt.map_level <= 1 {
        give(profile, "No Assist".into());
    }
    if attempt.difficulty >= Difficulty::Medium {
        give(profile, "Medium Rare".into());
    }
    if attempt.difficulty >= Difficulty::Hard {
        give(profile, "Hardened".into());
    }
    let pwned_ids: HashSet<&str> = profile.pwned.iter().filter_map(|k| k.split(':').next()).collect();
    let track_done = all_labs
        .iter()
        .filter(|l| l.track == lab.track)
        .all(|l| pwned_ids.contains(l.id.as_str()));
    if track_done {
        give(profile, format!("{} Track Complete", track_name(lab.track)));
    }
    earned
}

pub fn track_name(t: Track) -> &'static str {
    match t {
        Track::Web => "Web",
        Track::Api => "API",
        Track::Network => "Network",
        Track::Linux => "Linux",
        Track::Windows => "Windows",
        Track::Ad => "Active Directory",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Hint, LabManifest};

    fn lab() -> LabManifest {
        serde_yaml::from_str(
            r#"
id: t
name: T
track: web
blurb: b
xp: 100
est_minutes: 30
difficulties: [easy, medium]
resources: { ram_mb: 512, cpus: 1, disk_mb: 300 }
subnet_octet: 11
hosts: [{ name: web01, service: web, os: Linux }]
flags:
  - { id: user, label: User, host: web01, path: /u, xp_share: 0.4 }
  - { id: root, label: Root, host: web01, path: /r, xp_share: 0.6 }
"#,
        )
        .unwrap()
    }

    fn attempt(d: Difficulty) -> (Attempt, String, String) {
        let (u, r) = (new_flag(), new_flag());
        let mut hashes = BTreeMap::new();
        hashes.insert("user".to_string(), hash_flag(&u));
        hashes.insert("root".to_string(), hash_flag(&r));
        let a = Attempt { lab_id: "t".into(), difficulty: d, flag_hashes: hashes, captured: vec![], hints_revealed: 0, map_level: 1 };
        (a, u, r)
    }

    #[test]
    fn levels_curve() {
        assert_eq!(level_for_xp(0), 1);
        assert_eq!(level_for_xp(99), 1);
        assert_eq!(level_for_xp(100), 2);
        assert_eq!(level_for_xp(300), 3);
        assert_eq!(level_for_xp(999), 4);
        assert_eq!(title_for_level(6), "Operator");
    }

    #[test]
    fn flags_are_unique_and_hash_trims() {
        let f = new_flag();
        assert!(f.starts_with("LF{") && f.len() == 36);
        assert_ne!(f, new_flag());
        assert_eq!(hash_flag(&format!("  {f}\n")), hash_flag(&f));
    }

    #[test]
    fn full_pwn_awards_xp_badges_and_level() {
        let lab = lab();
        let mut p = Profile::new("sam".into(), Skill::Beginner, 0);
        let (mut a, u, r) = attempt(Difficulty::Medium);
        let wrong = submit_flag(&mut p, &mut a, &lab, &[&lab], "LF{nope}", 1);
        assert!(!wrong.correct);
        let first = submit_flag(&mut p, &mut a, &lab, &[&lab], &u, 1);
        assert_eq!(first.xp_awarded, 60); // 100 * 0.4 * 1.5
        assert!(!first.lab_complete);
        let second = submit_flag(&mut p, &mut a, &lab, &[&lab], &r, 2);
        assert_eq!(second.xp_awarded, 90);
        assert!(second.lab_complete);
        assert_eq!(second.level_up.as_ref().map(|l| l.level), Some(2));
        for b in ["First Blood", "No Assist", "Medium Rare", "Web Track Complete"] {
            assert!(second.new_badges.contains(&b.to_string()), "missing {b}");
        }
        let again = submit_flag(&mut p, &mut a, &lab, &[&lab], &r, 3);
        assert!(again.already_captured && again.xp_awarded == 0);
    }

    #[test]
    fn replay_gives_no_xp_and_hints_cost_xp() {
        let mut lab = lab();
        lab.hints = vec![Hint { text: "h".into(), cost: 0.2 }];
        let mut p = Profile::new("sam".into(), Skill::Beginner, 0);
        let (mut a, u, _) = attempt(Difficulty::Easy);
        a.hints_revealed = 1;
        a.map_level = 2;
        assert!((a.penalty(&lab) - 0.3).abs() < 1e-9);
        assert_eq!(submit_flag(&mut p, &mut a, &lab, &[&lab], &u, 1).xp_awarded, 28); // 40 * 0.7
        let (mut replay, u2, _) = attempt(Difficulty::Easy);
        let res = submit_flag(&mut p, &mut replay, &lab, &[&lab], &u2, 2);
        assert!(res.correct && res.xp_awarded == 0);
    }
}
