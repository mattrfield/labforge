//! End-to-end checks against a real Docker engine: build + up + health, flag planting
//! (content, owner, mode, privilege boundary), an optional reference exploit run from an
//! attacker container on the lab network, then flag submission and XP.
//! Run with: `cargo test --lib e2e -- --ignored --nocapture --test-threads=1`
//! (tests share compose project names per lab, so they must not run in parallel).

use crate::catalog::{Catalog, Difficulty, Lab};
use crate::progress::{self, Attempt, Profile, Skill};
use crate::runtime::{self, StepState};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

const ATTACKER_IMAGE: &str = "labforge-e2e-attacker";

fn root_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn exec(container: &str, user: &str, cmd: &str) -> (bool, String) {
    let out = runtime::docker(&["exec", "-u", user, container, "sh", "-c", cmd]).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text.trim().to_string())
}

fn container(lab: &Lab, service: &str) -> String {
    let out = runtime::docker(&[
        "ps",
        "-q",
        "--filter",
        &format!("label=com.docker.compose.project={}", runtime::project_name(lab)),
        "--filter",
        &format!("label=com.docker.compose.service={service}"),
    ])
    .output()
    .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().next().expect("container running").trim().to_string()
}

/// Runs a Python solution script on an attacker container joined to the lab network.
/// The script prints one JSON line: {"user": "...", "root": "..."}.
fn run_exploit(lab: &Lab, script: &str, d: Difficulty) -> BTreeMap<String, String> {
    let dir = root_dir().join("e2e/attacker");
    let build = runtime::docker(&["build", "-q", "-t", ATTACKER_IMAGE, dir.to_str().unwrap()]).output().unwrap();
    assert!(build.status.success(), "attacker image: {}", String::from_utf8_lossy(&build.stderr));

    let network = format!("{}_lab", runtime::project_name(lab));
    let mut child = runtime::docker(&["run", "--rm", "-i", "--network", &network, ATTACKER_IMAGE, "python3", "-", d.as_str()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "exploit failed:\n{stderr}\n{}", String::from_utf8_lossy(&out.stdout));
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(stdout.lines().last().expect("exploit output")).expect("exploit JSON")
}

fn run_lab(lab_id: &str, d: Difficulty, expect_xp: (u32, u32), exploit: Option<&str>) {
    let catalog = Catalog::load(&root_dir().join("../labs"));
    assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
    let lab = catalog.get(lab_id).unwrap_or_else(|| panic!("{lab_id} in catalog")).clone();
    let m = &lab.manifest;

    // Same sequence as `start_lab` in lib.rs.
    let flags: BTreeMap<String, String> = m.flags.iter().map(|f| (f.id.clone(), progress::new_flag())).collect();
    let mut attempt = Attempt {
        lab_id: m.id.clone(),
        difficulty: d,
        flag_hashes: flags.iter().map(|(k, v)| (k.clone(), progress::hash_flag(v))).collect(),
        captured: vec![],
        hints_revealed: 0,
        map_level: 1,
    };
    let mut steps = vec![];
    let mut emit = |id: &str, st: StepState, msg: &str| steps.push(format!("{id}:{st:?}:{msg}"));
    let res = runtime::up(&lab, d, &mut emit).and_then(|_| runtime::plant_flags(&lab, d, &flags));
    let states = runtime::host_states(&lab, d);

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
        res?;
        assert!(states.iter().all(|h| h.state == "up"), "hosts: {states:?}");
        for (i, h) in m.hosts.iter().enumerate() {
            assert_eq!(states[i].ip, m.host_ip(i), "{}", h.name);
        }

        // Planting: content, owner, mode, and who can read what.
        for spec in &m.flags {
            let host = m.hosts.iter().find(|h| h.name == spec.host).unwrap();
            let c = container(&lab, &host.service);
            let (ok, content) = exec(&c, "root", &format!("cat {}", spec.path));
            assert!(ok && content == flags[&spec.id], "{} content on {}: {content}", spec.id, host.name);
            let (_, stat) = exec(&c, "root", &format!("stat -c '%U:%G %a' {}", spec.path));
            println!("{} on {}: {} (want {} {})", spec.id, host.name, stat, spec.owner, spec.mode);
            assert_eq!(stat, format!("{} {}", spec.owner, spec.mode.trim_start_matches('0')));
            let owner = spec.owner.split(':').next().unwrap();
            if owner == "root" {
                assert!(!exec(&c, "nobody", &format!("cat {}", spec.path)).0, "{} readable by nobody", spec.id);
            } else {
                assert!(exec(&c, owner, &format!("cat {}", spec.path)).0, "{} unreadable by {owner}", spec.id);
            }
            let inspect = runtime::docker(&["inspect", &c]).output().unwrap();
            assert!(!String::from_utf8_lossy(&inspect.stdout).contains(&flags[&spec.id]), "flag leaked into metadata");
        }

        // Reference exploit chain from the attacker box: it must recover exactly the planted flags.
        let (user_flag, root_flag) = match exploit {
            Some(script) => {
                let got = run_exploit(&lab, script, d);
                assert_eq!(got["user"], flags["user"], "exploit user flag");
                assert_eq!(got["root"], flags["root"], "exploit root flag");
                println!("{lab_id} {}: exploit chain recovered both flags", d.as_str());
                (got["user"].clone(), got["root"].clone())
            }
            None => (flags["user"].clone(), flags["root"].clone()),
        };

        // XP flow.
        let mut profile = Profile::new("tester".into(), Skill::Beginner, 0);
        let all = [m];
        let wrong = progress::submit_flag(&mut profile, &mut attempt, m, &all, "LF{00000000000000000000000000000000}", 1);
        assert!(!wrong.correct && profile.xp == 0);
        let u = progress::submit_flag(&mut profile, &mut attempt, m, &all, &user_flag, 2);
        assert!(u.correct && u.xp_awarded == expect_xp.0 && !u.lab_complete, "{u:?}");
        let r = progress::submit_flag(&mut profile, &mut attempt, m, &all, &format!("  {root_flag}\n"), 3);
        assert!(r.correct && r.xp_awarded == expect_xp.1 && r.lab_complete, "{r:?}");
        assert_eq!(profile.xp, expect_xp.0 + expect_xp.1);
        assert!(profile.pwned.contains(&format!("{lab_id}:{}", d.as_str())));
        println!(
            "{lab_id} {}: user +{} root +{} total {} level {} badges {:?} level_up {:?}",
            d.as_str(), u.xp_awarded, r.xp_awarded, profile.xp, r.level.level, r.new_badges, r.level_up.map(|l| l.level)
        );
        let again = progress::submit_flag(&mut profile, &mut attempt, m, &all, &root_flag, 4);
        assert!(again.already_captured && again.xp_awarded == 0);

        // Pause / resume keeps planted flags.
        runtime::stop(&lab, d)?;
        runtime::resume(&lab, d, &mut |_, _, _| {})?;
        let spec = m.flags.iter().find(|f| f.id == "root").unwrap();
        let host = m.hosts.iter().find(|h| h.name == spec.host).unwrap();
        assert_eq!(exec(&container(&lab, &host.service), "root", &format!("cat {}", spec.path)).1, flags["root"]);
        Ok(())
    }));

    runtime::down(&lab, d).unwrap();
    println!("steps: {steps:?}");
    match outcome {
        Ok(r) => r.unwrap(),
        Err(p) => std::panic::resume_unwind(p),
    }
}

#[test]
#[ignore = "needs Docker"]
fn e2e_bean_there_easy() {
    run_lab("bean-there", Difficulty::Easy, (40, 60), None);
}

#[test]
#[ignore = "needs Docker"]
fn e2e_bean_there_medium() {
    run_lab("bean-there", Difficulty::Medium, (60, 90), None);
}

#[test]
#[ignore = "needs Docker"]
fn e2e_postmarked_easy() {
    // 120 XP: user 0.4 -> 48, root 0.6 -> 72.
    run_lab("postmarked", Difficulty::Easy, (48, 72), Some(include_str!("../e2e/postmarked.py")));
}

#[test]
#[ignore = "needs Docker"]
fn e2e_postmarked_medium() {
    run_lab("postmarked", Difficulty::Medium, (72, 108), Some(include_str!("../e2e/postmarked.py")));
}

#[test]
#[ignore = "needs Docker"]
fn e2e_room_service_easy() {
    // 150 XP: user 60, root 90.
    run_lab("room-service", Difficulty::Easy, (60, 90), Some(include_str!("../e2e/room-service.py")));
}

#[test]
#[ignore = "needs Docker"]
fn e2e_room_service_medium() {
    run_lab("room-service", Difficulty::Medium, (90, 135), Some(include_str!("../e2e/room-service.py")));
}
