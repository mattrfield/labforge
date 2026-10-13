# LabForge

A desktop launcher for hands-on cybersecurity labs that run locally. Pick a track, set a difficulty, hit Start: LabForge builds an isolated network of target hosts with Docker, tells you when it's ready, and awards XP for every flag you capture.

## Run it

Requirements: Rust (stable), the Tauri 2 CLI (`cargo install tauri-cli --version "^2"`), and Docker Engine with the Compose v2 plugin to actually run labs. Linux is the primary target; Windows/macOS work for UI development.

```bash
cd src-tauri
cargo tauri dev        # or: cargo run
cargo test             # catalog, XP/levels and runtime unit tests
cargo tauri build      # installers (.deb, .AppImage, .rpm, NSIS)
```

The UI has no build step. To work on it in a plain browser, serve `ui/` with any static server and open it. Without Tauri, `ui/api.js` switches to an in-memory mock backend; in the mock, flags `LF{user}` and `LF{root}` are accepted.

## Layout

| Path | What it is |
| --- | --- |
| `src-tauri/src/catalog.rs` | Lab manifest (`lab.yaml`) schema, validation, catalog loading |
| `src-tauri/src/runtime.rs` | Docker Compose orchestration: build, up, health wait, flag planting, stop/resume/down |
| `src-tauri/src/progress.rs` | Profile, flag generation and hashing, XP, levels, badges |
| `src-tauri/src/preflight.rs` | Host checks: Docker, Compose, RAM, CPU, disk |
| `src-tauri/src/lib.rs` | Tauri commands, app state, persistence (`profile.json`, `active.json` in the app data dir) |
| `ui/` | Launcher UI (vanilla HTML/CSS/JS ES modules) |
| `labs/<id>/` | One lab package per folder |

## Lab packages

Each lab lives in `labs/<id>/` with:

- `lab.yaml`: metadata, difficulties, resources, hosts, flags, hints. See `labs/bean-there/lab.yaml`.
- `compose.yaml`: the hosts. The launcher supplies `LAB_SUBNET`, `LAB_GATEWAY`, `DIFFICULTY` and `IP_<HOST>` variables. Each lab gets `10.66.<subnet_octet>.0/24`, and hosts get `.10`, `.11`, … in manifest order. Use an `internal: true` network so the targets can't reach the internet or your LAN.
- `compose.<difficulty>.yaml` (optional): an overlay merged in for that difficulty.
- A healthcheck on every service. "Ready" means every container is running and healthy.

Flags are generated fresh on every start as `LF{<32 hex>}`. They're written into the containers over `docker exec` stdin, at the manifest's path, owner and mode. Only SHA-256 hashes are stored locally.

## MVP status

Done:
- Launcher shell, onboarding, home with track tiles, lab catalog with filters and a resource-fit badge
- Difficulty picker with per-lab notes and XP multipliers
- One-click start with live progress steps and a desktop notification when ready; pause, resume, reset, destroy
- Flag submission, XP with hint/map penalties, levels, badges, PWNED and level-up screens
- Lab map, levels 1–3 (hosts → services → relationships), with XP cost
- Host preflight with copy-paste fixes

Not done yet:
- **Lab target content.** Three labs are complete and tested against Docker: `bean-there` (web), `postmarked` (API) and `room-service` (network), each with easy and medium. The remaining MVP labs haven't been started. The launcher UI's in-memory mock (`ui/api.js`) still only lists Bean There.

Docker end-to-end tests (`src-tauri/src/e2e.rs`) build each lab, plant flags, check owner/mode/privilege boundaries, run a reference exploit chain from an attacker container on the lab network (`src-tauri/e2e/*.py`), then verify flag submission and XP. They are `#[ignore]`d because they need Docker:

```bash
cd src-tauri && cargo test --lib e2e -- --ignored --nocapture --test-threads=1
```
- WireGuard gateway, so a separate attack VM can join the lab network (today: attack from the host, which routes to the lab subnet directly on Linux)
- Prebuilt, signed lab images pulled from a registry instead of local builds
- Splitting the orchestrator into a separate daemon with a privileged helper (it currently runs inside the app process)
