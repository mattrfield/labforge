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

## Releases and lab updates

**Launcher.** Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds the installers and publishes a GitHub Release with a signed `latest.json`. Installed launchers check it on start and offer **Install & restart**.

**Labs.** Labs update separately from the launcher, so a new lab doesn't need a new app release:

- Each `lab.yaml` has a `version` (semver). **Bump it whenever a lab's files change**, or installed copies won't update. Optional fields: `tier` (`free` is the default; anything else is never published to the public index) and `min_app_version`.
- Pushing changes under `labs/` to `main` runs `.github/workflows/labs.yml`. It runs `scripts/build_lab_packs.py` to build one `<id>-<version>.tar.gz` per free lab plus `index.json` (URLs and SHA-256 hashes), signs the index with the same key as launcher updates, and uploads everything to the rolling `labs` prerelease.
- In the app, **Labs → Check for new labs** (also run at start) fetches the index, verifies its signature, and lists new and updated labs. Packs are checked against the signed hash, unpacked only if every path stays inside the lab's folder (no links or special files, size and file-count capped), validated, then swapped into `<app data>/labs/`. Newer installed labs override bundled ones; the learner's progress is never touched. A lab that is currently active can't be updated until it's destroyed.

Set `LABFORGE_INDEX_URL` to point the launcher at a different index (it must still be signed by the launcher's key). To try the pipeline locally, build the packs with `--base-url http://127.0.0.1:8098`, sign `index.json` with `cargo tauri signer sign`, serve the folder, and run `cargo test --lib live -- --ignored` with `LABFORGE_TEST_INDEX_URL` and `LABFORGE_TEST_PUBKEY` set.

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
- License-gated pro labs (the index already has a `tier` field; pro entries are shown locked and can't be installed yet)
- Splitting the orchestrator into a separate daemon with a privileged helper (it currently runs inside the app process)
