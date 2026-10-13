//! Runs labs with Docker Compose: build, start, wait for health, plant flags.

use crate::catalog::{Difficulty, Lab};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const HEALTH_TIMEOUT: Duration = Duration::from_secs(240);

/// A `docker` command that never flashes a console window on Windows.
pub fn docker(args: &[&str]) -> Command {
    let mut cmd = Command::new("docker");
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub fn project_name(lab: &Lab) -> String {
    format!("labforge-{}", lab.manifest.id)
}

/// Variables every lab compose file can use: subnet, gateway, per-host IPs, difficulty.
pub fn compose_env(lab: &Lab, difficulty: Difficulty) -> Vec<(String, String)> {
    let m = &lab.manifest;
    let mut env = vec![
        ("LAB_ID".into(), m.id.clone()),
        ("LAB_SUBNET".into(), m.subnet()),
        ("LAB_GATEWAY".into(), m.gateway()),
        ("DIFFICULTY".into(), difficulty.as_str().into()),
    ];
    for (i, h) in m.hosts.iter().enumerate() {
        let key = format!("IP_{}", h.name.to_uppercase().replace('-', "_"));
        env.push((key, m.host_ip(i)));
    }
    env
}

fn compose(lab: &Lab, difficulty: Difficulty, args: &[&str]) -> Command {
    let mut full: Vec<String> = vec!["compose".into(), "-p".into(), project_name(lab)];
    full.push("--project-directory".into());
    full.push(lab.dir.to_string_lossy().into());
    for f in lab.compose_files(difficulty) {
        full.push("-f".into());
        full.push(f.to_string_lossy().into());
    }
    full.extend(args.iter().map(|s| s.to_string()));
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    let mut cmd = docker(&refs);
    cmd.envs(compose_env(lab, difficulty));
    cmd
}

fn run(mut cmd: Command, what: &str) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{what}: could not run docker ({e})"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(format!("{what} failed:\n{}", tail(&String::from_utf8_lossy(&out.stderr), 12)))
    }
}

/// Runs a command and reports each line of its stderr (where Compose writes progress).
fn run_streaming(mut cmd: Command, what: &str, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{what}: could not run docker ({e})"))?;
    let mut lines = Vec::new();
    if let Some(stderr) = child.stderr.take() {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let line = line.trim().to_string();
            if !line.is_empty() {
                on_line(&line);
                lines.push(line);
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed:\n{}", tail(&lines.join("\n"), 12)))
    }
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    Running,
    Done,
    Error,
}

/// Progress callback: (step id, state, message).
pub type Emit<'a> = &'a mut dyn FnMut(&str, StepState, &str);

/// Builds images and starts every host, then waits until all are healthy.
pub fn up(lab: &Lab, difficulty: Difficulty, emit: Emit) -> Result<(), String> {
    step(emit, "build", "Building lab images (first start can take a few minutes)", |emit| {
        let mut last = Instant::now() - Duration::from_secs(1);
        run_streaming(compose(lab, difficulty, &["--progress", "plain", "build"]), "Image build", &mut |line| {
            if last.elapsed() > Duration::from_millis(400) {
                last = Instant::now();
                emit("build", StepState::Running, line);
            }
        })
    })?;
    step(emit, "up", "Creating the isolated network and starting hosts", |_| {
        run(compose(lab, difficulty, &["up", "-d", "--force-recreate", "--remove-orphans"]), "Start").map(|_| ())
    })?;
    step(emit, "health", "Waiting for services to come up", |emit| wait_healthy(lab, difficulty, emit))
}

fn step(emit: Emit, id: &str, label: &str, f: impl FnOnce(Emit) -> Result<(), String>) -> Result<(), String> {
    emit(id, StepState::Running, label);
    match f(emit) {
        Ok(()) => {
            emit(id, StepState::Done, label);
            Ok(())
        }
        Err(e) => {
            emit(id, StepState::Error, &e);
            Err(e)
        }
    }
}

fn container_id(lab: &Lab, difficulty: Difficulty, service: &str) -> Option<String> {
    let out = run(compose(lab, difficulty, &["ps", "-a", "-q", service]), "Lookup").ok()?;
    out.lines().next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[derive(Debug, Clone, Serialize)]
pub struct HostState {
    pub name: String,
    pub ip: String,
    /// up | booting | down | unhealthy | absent
    pub state: &'static str,
}

pub fn host_states(lab: &Lab, difficulty: Difficulty) -> Vec<HostState> {
    lab.manifest
        .hosts
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let state = match container_id(lab, difficulty, &h.service) {
                None => "absent",
                Some(id) => {
                    let fmt = "{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{end}}";
                    match run(docker(&["inspect", "-f", fmt, &id]), "Inspect") {
                        Ok(s) => classify(s.trim()),
                        Err(_) => "absent",
                    }
                }
            };
            HostState { name: h.name.clone(), ip: lab.manifest.host_ip(i), state }
        })
        .collect()
}

fn classify(inspect: &str) -> &'static str {
    let (status, health) = inspect.split_once('|').unwrap_or((inspect, ""));
    match (status, health) {
        ("running", "" | "healthy") => "up",
        ("running", "unhealthy") => "unhealthy",
        ("running", _) | ("created" | "restarting", _) => "booting",
        _ => "down",
    }
}

fn wait_healthy(lab: &Lab, difficulty: Difficulty, emit: Emit) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let states = host_states(lab, difficulty);
        let pending: Vec<&HostState> = states.iter().filter(|h| h.state != "up").collect();
        if pending.is_empty() {
            return Ok(());
        }
        if let Some(bad) = states.iter().find(|h| h.state == "down" || h.state == "unhealthy") {
            return Err(format!("{} stopped or failed its health check. Check its logs and try Reset.", bad.name));
        }
        if started.elapsed() > HEALTH_TIMEOUT {
            let names: Vec<&str> = pending.iter().map(|h| h.name.as_str()).collect();
            return Err(format!("Timed out waiting for {}.", names.join(", ")));
        }
        let up = states.len() - pending.len();
        emit("health", StepState::Running, &format!("{up} of {} hosts ready", states.len()));
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Writes each flag into its host, passing the value on stdin so it never
/// appears in process arguments or container metadata.
pub fn plant_flags(lab: &Lab, difficulty: Difficulty, flags: &BTreeMap<String, String>) -> Result<(), String> {
    const SCRIPT: &str = r#"umask 077; mkdir -p "$(dirname "$1")" && cat > "$1" && chown "$2" "$1" && chmod "$3" "$1""#;
    for spec in &lab.manifest.flags {
        let value = flags.get(&spec.id).ok_or_else(|| format!("no value for flag {}", spec.id))?;
        let host = lab.manifest.hosts.iter().find(|h| h.name == spec.host).ok_or("unknown host")?;
        let id = container_id(lab, difficulty, &host.service).ok_or_else(|| format!("{} is not running", host.name))?;
        let mut child = docker(&["exec", "-i", "-u", "root", &id, "sh", "-c", SCRIPT, "sh", &spec.path, &spec.owner, &spec.mode])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        child
            .stdin
            .take()
            .ok_or("no stdin")?
            .write_all(format!("{value}\n").as_bytes())
            .map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "Could not plant {} on {}: {}",
                spec.label,
                host.name,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(())
}

pub fn stop(lab: &Lab, difficulty: Difficulty) -> Result<(), String> {
    run(compose(lab, difficulty, &["stop"]), "Stop").map(|_| ())
}

pub fn resume(lab: &Lab, difficulty: Difficulty, emit: Emit) -> Result<(), String> {
    step(emit, "up", "Starting hosts", |_| run(compose(lab, difficulty, &["start"]), "Resume").map(|_| ()))?;
    step(emit, "health", "Waiting for services to come up", |emit| wait_healthy(lab, difficulty, emit))
}

/// Removes containers, networks and volumes. Images stay cached for fast restarts.
pub fn down(lab: &Lab, difficulty: Difficulty) -> Result<(), String> {
    run(compose(lab, difficulty, &["down", "-v", "--remove-orphans"]), "Remove").map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_container_states() {
        assert_eq!(classify("running|"), "up");
        assert_eq!(classify("running|healthy"), "up");
        assert_eq!(classify("running|starting"), "booting");
        assert_eq!(classify("running|unhealthy"), "unhealthy");
        assert_eq!(classify("exited|"), "down");
        assert_eq!(classify("created|"), "booting");
    }
}
