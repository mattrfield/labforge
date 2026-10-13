//! Host checks run before any lab starts: Docker, Compose and resources.

use crate::runtime::docker;
use serde::Serialize;
use std::path::Path;
use sysinfo::{Disks, System};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub label: &'static str,
    pub status: Status,
    pub detail: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostInfo {
    pub os: &'static str,
    pub ram_total_mb: u64,
    pub ram_available_mb: u64,
    pub cpus: usize,
    pub disk_free_mb: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub host: HostInfo,
    pub checks: Vec<Check>,
    /// True when labs can be started at all.
    pub ready: bool,
}

pub fn run(data_dir: &Path) -> Report {
    let host = host_info(data_dir);
    let mut checks = Vec::new();

    let cli = docker(&["version", "--format", "{{.Client.Version}}"]).output();
    let cli_ok = matches!(&cli, Ok(o) if !o.stdout.is_empty());
    checks.push(Check {
        id: "docker-cli",
        label: "Docker installed",
        status: if cli_ok { Status::Ok } else { Status::Fail },
        detail: match &cli {
            Ok(o) if !o.stdout.is_empty() => format!("Docker {}", String::from_utf8_lossy(&o.stdout).trim()),
            _ => "The docker command was not found.".into(),
        },
        fix: (!cli_ok).then(install_hint),
    });

    let daemon = docker(&["info", "--format", "{{.ServerVersion}}"]).output();
    let daemon_ok = matches!(&daemon, Ok(o) if o.status.success());
    let daemon_err = match &daemon {
        Ok(o) => String::from_utf8_lossy(&o.stderr).to_string(),
        Err(e) => e.to_string(),
    };
    checks.push(Check {
        id: "docker-daemon",
        label: "Docker engine running",
        status: if daemon_ok { Status::Ok } else { Status::Fail },
        detail: if daemon_ok {
            "The engine is reachable.".into()
        } else if daemon_err.contains("permission denied") {
            "Docker is running, but your user can't talk to it.".into()
        } else {
            "The Docker engine isn't running or can't be reached.".into()
        },
        fix: (!daemon_ok).then(|| {
            if daemon_err.contains("permission denied") {
                "Add yourself to the docker group, then log out and back in:\nsudo usermod -aG docker $USER".into()
            } else if cfg!(target_os = "linux") {
                "Start the engine:\nsudo systemctl enable --now docker".into()
            } else {
                "Open Docker Desktop and wait until it says the engine is running.".into()
            }
        }),
    });

    let compose = docker(&["compose", "version", "--short"]).output();
    let compose_ok = matches!(&compose, Ok(o) if o.status.success());
    checks.push(Check {
        id: "docker-compose",
        label: "Docker Compose v2",
        status: if compose_ok { Status::Ok } else { Status::Fail },
        detail: match &compose {
            Ok(o) if o.status.success() => format!("Compose {}", String::from_utf8_lossy(&o.stdout).trim()),
            _ => "The `docker compose` plugin was not found.".into(),
        },
        fix: (!compose_ok).then(|| {
            if cfg!(target_os = "linux") {
                "Install the Compose v2 plugin for your user (works on any distro):\nmkdir -p ~/.docker/cli-plugins && curl -SL https://github.com/docker/compose/releases/latest/download/docker-compose-linux-x86_64 -o ~/.docker/cli-plugins/docker-compose && chmod +x ~/.docker/cli-plugins/docker-compose".into()
            } else {
                "Update Docker Desktop; Compose v2 ships with it.".into()
            }
        }),
    });

    checks.push(threshold(
        "ram",
        "Memory",
        host.ram_total_mb,
        8 * 1024,
        format!("{:.1} GB total, {:.1} GB free", gb(host.ram_total_mb), gb(host.ram_available_mb)),
        "Container labs need about 8 GB of RAM; close other apps if starts fail.",
    ));
    checks.push(threshold(
        "cpu",
        "CPU cores",
        host.cpus as u64,
        4,
        format!("{} cores", host.cpus),
        "Labs will run, but builds and starts will be slower with fewer than 4 cores.",
    ));
    checks.push(threshold(
        "disk",
        "Free disk space",
        host.disk_free_mb,
        10 * 1024,
        format!("{:.1} GB free", gb(host.disk_free_mb)),
        "Free up at least 10 GB for lab images.",
    ));

    if !cfg!(target_os = "linux") {
        checks.push(Check {
            id: "platform",
            label: "Host platform",
            status: Status::Warn,
            detail: "LabForge targets Linux hosts first.".into(),
            fix: Some(
                "On Docker Desktop, lab IPs aren't reachable from your machine yet. Use a Linux host for the full experience."
                    .into(),
            ),
        });
    }

    let ready = cli_ok && daemon_ok && compose_ok;
    Report { host, checks, ready }
}

fn threshold(id: &'static str, label: &'static str, have: u64, want: u64, detail: String, fix: &str) -> Check {
    let ok = have >= want;
    Check { id, label, status: if ok { Status::Ok } else { Status::Warn }, detail, fix: (!ok).then(|| fix.to_string()) }
}

fn gb(mb: u64) -> f64 {
    mb as f64 / 1024.0
}

fn install_hint() -> String {
    if cfg!(target_os = "linux") {
        // get.docker.com doesn't support rolling distros like Kali; use their own packages there.
        let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let id = os_release
            .lines()
            .find_map(|l| l.strip_prefix("ID="))
            .map(|v| v.trim_matches('"').to_string())
            .unwrap_or_default();
        match id.as_str() {
            "kali" | "parrot" => "Install Docker from your distro's packages, then log out and back in:\nsudo apt install -y docker.io docker-compose && sudo systemctl enable --now docker && sudo usermod -aG docker $USER".into(),
            "arch" | "manjaro" | "endeavouros" => "Install Docker from your distro's packages, then log out and back in:\nsudo pacman -S --needed docker docker-compose && sudo systemctl enable --now docker && sudo usermod -aG docker $USER".into(),
            _ => "Install Docker Engine (https://docs.docker.com/engine/install/), e.g.:\ncurl -fsSL https://get.docker.com | sh".into(),
        }
    } else {
        "Install Docker Desktop from https://www.docker.com/products/docker-desktop/".into()
    }
}

pub fn host_info(data_dir: &Path) -> HostInfo {
    let mut sys = System::new();
    sys.refresh_memory();
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    // Free space on the disk that holds the app data (longest matching mount point).
    let disks = Disks::new_with_refreshed_list();
    let disk_free = disks
        .list()
        .iter()
        .filter(|d| data_dir.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
        .unwrap_or(0);
    HostInfo {
        os: std::env::consts::OS,
        ram_total_mb: sys.total_memory() / 1024 / 1024,
        ram_available_mb: sys.available_memory() / 1024 / 1024,
        cpus,
        disk_free_mb: disk_free / 1024 / 1024,
    }
}
