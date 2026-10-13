mod catalog;
#[cfg(test)]
mod e2e;
mod preflight;
mod progress;
mod runtime;

use catalog::{Catalog, Difficulty, Lab};
use progress::{Attempt, LevelInfo, Profile, Skill, SubmitResult};
use runtime::StepState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

type CmdResult<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Starting,
    Ready,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveLab {
    attempt: Attempt,
    phase: Phase,
    started_at: u64,
    error: Option<String>,
}

#[derive(Default)]
struct Store {
    profile: Option<Profile>,
    active: Option<ActiveLab>,
}

struct AppState {
    catalog: Catalog,
    labs_dir: PathBuf,
    data_dir: PathBuf,
    store: Mutex<Store>,
    /// Set while a start/reset/resume pipeline is running.
    busy: AtomicBool,
}

impl AppState {
    fn lab(&self, id: &str) -> CmdResult<&Lab> {
        self.catalog.get(id).ok_or_else(|| format!("Unknown lab `{id}`"))
    }

    fn save(&self, store: &Store) {
        write_json(&self.data_dir.join("profile.json"), &store.profile);
        write_json(&self.data_dir.join("active.json"), &store.active);
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn write_json<T: Serialize>(path: &Path, value: &T) {
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_vec_pretty(value) {
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, path);
        }
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

// ---------- views sent to the UI ----------

#[derive(Serialize)]
struct ProfileView {
    username: String,
    skill: Skill,
    badges: Vec<String>,
    pwned: Vec<String>,
    level: LevelInfo,
}

fn profile_view(p: &Profile) -> ProfileView {
    ProfileView {
        username: p.username.clone(),
        skill: p.skill,
        badges: p.badges.clone(),
        pwned: p.pwned.clone(),
        level: progress::level_info(p.xp),
    }
}

#[derive(Serialize)]
struct HostView {
    name: String,
    os: String,
    ip: String,
}

#[derive(Serialize)]
struct FlagView {
    id: String,
    label: String,
    host: String,
}

/// A lab as the catalog shows it: no flag paths, hint texts or ports (those are spoilers).
#[derive(Serialize)]
struct LabView {
    id: String,
    name: String,
    track: catalog::Track,
    track_name: &'static str,
    blurb: String,
    story: String,
    skills: Vec<String>,
    xp: u32,
    est_minutes: u32,
    difficulties: Vec<Difficulty>,
    difficulty_notes: BTreeMap<Difficulty, String>,
    resources: catalog::Resources,
    subnet: String,
    hosts: Vec<HostView>,
    flags: Vec<FlagView>,
    hint_count: usize,
    pwned: Vec<Difficulty>,
}

#[derive(Serialize)]
struct Catalogue {
    labs: Vec<LabView>,
    errors: Vec<String>,
    labs_dir: String,
}

#[derive(Serialize)]
struct ActiveView {
    lab_id: String,
    difficulty: Difficulty,
    phase: Phase,
    error: Option<String>,
    started_at: u64,
    captured: Vec<String>,
    hints: Vec<String>,
    next_hint_cost: Option<f64>,
    map_level: u8,
    max_map_level: u8,
    penalty: f64,
}

fn max_map_level(lab: &Lab) -> u8 {
    if lab.manifest.map.edges.is_empty() {
        2
    } else {
        3
    }
}

fn active_view(state: &AppState, a: &ActiveLab) -> Option<ActiveView> {
    let lab = state.catalog.get(&a.attempt.lab_id)?;
    let m = &lab.manifest;
    Some(ActiveView {
        lab_id: m.id.clone(),
        difficulty: a.attempt.difficulty,
        phase: a.phase,
        error: a.error.clone(),
        started_at: a.started_at,
        captured: a.attempt.captured.clone(),
        hints: m.hints.iter().take(a.attempt.hints_revealed).map(|h| h.text.clone()).collect(),
        next_hint_cost: m.hints.get(a.attempt.hints_revealed).map(|h| h.cost),
        map_level: a.attempt.map_level,
        max_map_level: max_map_level(lab),
        penalty: a.attempt.penalty(m),
    })
}

#[derive(Serialize)]
struct MapHost {
    name: String,
    os: String,
    ip: String,
    ports: Vec<catalog::Port>,
}

#[derive(Serialize)]
struct MapView {
    level: u8,
    max_level: u8,
    next_level_cost: Option<f64>,
    subnet: String,
    gateway: String,
    hosts: Vec<MapHost>,
    edges: Vec<catalog::MapEdge>,
}

// ---------- commands ----------

#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

#[derive(Serialize)]
struct AppInfo {
    version: String,
}

#[tauri::command]
fn app_info(app: AppHandle) -> AppInfo {
    AppInfo { version: app.package_info().version.to_string() }
}

#[tauri::command]
async fn check_update(app: AppHandle) -> CmdResult<Option<UpdateInfo>> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    Ok(update.map(|u| UpdateInfo { version: u.version, notes: u.body }))
}

#[tauri::command]
async fn install_update(app: AppHandle) -> CmdResult<()> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("LabForge is already up to date.")?;
    let mut got = 0u64;
    let progress = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                got += chunk as u64;
                let _ = progress.emit("update-progress", total.map(|t| got as f64 / t as f64));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

#[tauri::command]
fn get_profile(state: State<AppState>) -> Option<ProfileView> {
    state.store.lock().unwrap().profile.as_ref().map(profile_view)
}

#[tauri::command]
fn create_profile(state: State<AppState>, username: String, skill: Skill) -> CmdResult<ProfileView> {
    let username = username.trim().to_string();
    if username.is_empty() || username.chars().count() > 24 {
        return Err("Pick a handle between 1 and 24 characters.".into());
    }
    let mut store = state.store.lock().unwrap();
    let profile = Profile::new(username, skill, now());
    let view = profile_view(&profile);
    store.profile = Some(profile);
    state.save(&store);
    Ok(view)
}

#[tauri::command]
async fn run_preflight(state: State<'_, AppState>) -> CmdResult<preflight::Report> {
    let dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || preflight::run(&dir)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn list_labs(state: State<AppState>) -> Catalogue {
    let store = state.store.lock().unwrap();
    let pwned = store.profile.as_ref().map(|p| p.pwned.clone()).unwrap_or_default();
    let labs = state
        .catalog
        .labs
        .iter()
        .map(|lab| {
            let m = &lab.manifest;
            LabView {
                id: m.id.clone(),
                name: m.name.clone(),
                track: m.track,
                track_name: progress::track_name(m.track),
                blurb: m.blurb.clone(),
                story: m.story.clone(),
                skills: m.skills.clone(),
                xp: m.xp,
                est_minutes: m.est_minutes,
                difficulties: m.difficulties.clone(),
                difficulty_notes: m.difficulty_notes.clone(),
                resources: m.resources.clone(),
                subnet: m.subnet(),
                hosts: m
                    .hosts
                    .iter()
                    .enumerate()
                    .map(|(i, h)| HostView { name: h.name.clone(), os: h.os.clone(), ip: m.host_ip(i) })
                    .collect(),
                flags: m
                    .flags
                    .iter()
                    .map(|f| FlagView { id: f.id.clone(), label: f.label.clone(), host: f.host.clone() })
                    .collect(),
                hint_count: m.hints.len(),
                pwned: m
                    .difficulties
                    .iter()
                    .copied()
                    .filter(|d| pwned.contains(&format!("{}:{}", m.id, d.as_str())))
                    .collect(),
            }
        })
        .collect();
    Catalogue { labs, errors: state.catalog.errors.clone(), labs_dir: state.labs_dir.display().to_string() }
}

#[tauri::command]
fn get_active(state: State<AppState>) -> Option<ActiveView> {
    let store = state.store.lock().unwrap();
    store.active.as_ref().and_then(|a| active_view(&state, a))
}

#[derive(Clone, Serialize)]
struct ProgressEvent {
    lab_id: String,
    step: String,
    state: StepState,
    message: String,
}

/// Generates fresh flags, then builds, starts and seeds the lab on a background thread.
fn launch(app: &AppHandle, state: &AppState, lab_id: &str, difficulty: Difficulty) -> CmdResult<ActiveView> {
    let lab = state.lab(lab_id)?.clone();
    if !lab.manifest.difficulties.contains(&difficulty) {
        return Err(format!("{} doesn't support {} difficulty yet.", lab.manifest.name, difficulty.as_str()));
    }
    let flags: BTreeMap<String, String> = lab.manifest.flags.iter().map(|f| (f.id.clone(), progress::new_flag())).collect();
    let active = ActiveLab {
        attempt: Attempt {
            lab_id: lab.manifest.id.clone(),
            difficulty,
            flag_hashes: flags.iter().map(|(k, v)| (k.clone(), progress::hash_flag(v))).collect(),
            captured: vec![],
            hints_revealed: 0,
            map_level: 1,
        },
        phase: Phase::Starting,
        started_at: now(),
        error: None,
    };
    let view = active_view(state, &active).ok_or("lab vanished")?;
    {
        let mut store = state.store.lock().unwrap();
        store.active = Some(active);
        state.save(&store);
    }

    let app = app.clone();
    std::thread::spawn(move || {
        let id = lab.manifest.id.clone();
        let mut emit = |step: &str, s: StepState, message: &str| {
            let _ = app.emit(
                "lab-progress",
                ProgressEvent { lab_id: id.clone(), step: step.into(), state: s, message: message.into() },
            );
        };
        let result = runtime::up(&lab, difficulty, &mut emit).and_then(|_| {
            emit("flags", StepState::Running, "Planting flags");
            let r = runtime::plant_flags(&lab, difficulty, &flags);
            emit("flags", if r.is_ok() { StepState::Done } else { StepState::Error }, "Planting flags");
            r
        });
        finish_start(&app, &lab, result);
    });
    Ok(view)
}

fn finish_start(app: &AppHandle, lab: &Lab, result: Result<(), String>) {
    let state = app.state::<AppState>();
    {
        let mut store = state.store.lock().unwrap();
        if let Some(a) = store.active.as_mut().filter(|a| a.attempt.lab_id == lab.manifest.id) {
            match &result {
                Ok(()) => {
                    a.phase = Phase::Ready;
                    a.error = None;
                }
                Err(e) => {
                    a.phase = Phase::Failed;
                    a.error = Some(e.clone());
                }
            }
        }
        state.save(&store);
    }
    state.busy.store(false, Ordering::SeqCst);
    match result {
        Ok(()) => {
            let _ = app.emit("lab-ready", &lab.manifest.id);
            let _ = app
                .notification()
                .builder()
                .title(format!("{} is ready", lab.manifest.name))
                .body("Your targets are up. Happy hacking!")
                .show();
        }
        Err(e) => {
            let _ = app.emit("lab-error", e);
        }
    }
}

fn claim_busy(state: &AppState) -> CmdResult<()> {
    if state.busy.swap(true, Ordering::SeqCst) {
        Err("Hold on, a lab is still starting.".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
async fn start_lab(app: AppHandle, state: State<'_, AppState>, lab_id: String, difficulty: Difficulty) -> CmdResult<ActiveView> {
    {
        let store = state.store.lock().unwrap();
        if store.profile.is_none() {
            return Err("Create your profile first.".into());
        }
        if let Some(a) = &store.active {
            let name = state.catalog.get(&a.attempt.lab_id).map(|l| l.manifest.name.clone()).unwrap_or_default();
            return Err(format!("{name} is still active. Destroy it before starting another lab."));
        }
    }
    claim_busy(&state)?;
    launch(&app, &state, &lab_id, difficulty).inspect_err(|_| state.busy.store(false, Ordering::SeqCst))
}

fn current(state: &AppState) -> CmdResult<(Lab, Difficulty)> {
    let store = state.store.lock().unwrap();
    let a = store.active.as_ref().ok_or("No lab is active.")?;
    Ok((state.lab(&a.attempt.lab_id)?.clone(), a.attempt.difficulty))
}

fn set_phase(state: &AppState, phase: Phase) -> Option<ActiveView> {
    let mut store = state.store.lock().unwrap();
    let a = store.active.as_mut()?;
    a.phase = phase;
    a.error = None;
    let view = active_view(state, store.active.as_ref()?);
    state.save(&store);
    view
}

#[tauri::command]
async fn stop_lab(state: State<'_, AppState>) -> CmdResult<Option<ActiveView>> {
    let (lab, d) = current(&state)?;
    tauri::async_runtime::spawn_blocking(move || runtime::stop(&lab, d)).await.map_err(|e| e.to_string())??;
    Ok(set_phase(&state, Phase::Stopped))
}

#[tauri::command]
async fn resume_lab(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<ActiveView>> {
    let (lab, d) = current(&state)?;
    claim_busy(&state)?;
    let view = set_phase(&state, Phase::Starting);
    std::thread::spawn(move || {
        let id = lab.manifest.id.clone();
        let mut emit = |step: &str, s: StepState, message: &str| {
            let _ = app.emit(
                "lab-progress",
                ProgressEvent { lab_id: id.clone(), step: step.into(), state: s, message: message.into() },
            );
        };
        let result = runtime::resume(&lab, d, &mut emit);
        finish_start(&app, &lab, result);
    });
    Ok(view)
}

#[tauri::command]
async fn destroy_lab(state: State<'_, AppState>) -> CmdResult<()> {
    if state.busy.load(Ordering::SeqCst) {
        return Err("Wait for the lab to finish starting first.".into());
    }
    let (lab, d) = current(&state)?;
    tauri::async_runtime::spawn_blocking(move || runtime::down(&lab, d)).await.map_err(|e| e.to_string())??;
    let mut store = state.store.lock().unwrap();
    store.active = None;
    state.save(&store);
    Ok(())
}

/// Tears the lab down and starts it again from clean images with new flags.
#[tauri::command]
async fn reset_lab(app: AppHandle, state: State<'_, AppState>) -> CmdResult<ActiveView> {
    let (lab, d) = current(&state)?;
    claim_busy(&state)?;
    let down_lab = lab.clone();
    let down = tauri::async_runtime::spawn_blocking(move || runtime::down(&down_lab, d)).await;
    if let Err(e) = down.map_err(|e| e.to_string()).and_then(|r| r) {
        state.busy.store(false, Ordering::SeqCst);
        return Err(e);
    }
    launch(&app, &state, &lab.manifest.id, d).inspect_err(|_| state.busy.store(false, Ordering::SeqCst))
}

#[tauri::command]
async fn host_status(state: State<'_, AppState>) -> CmdResult<Vec<runtime::HostState>> {
    let (lab, d) = current(&state)?;
    tauri::async_runtime::spawn_blocking(move || runtime::host_states(&lab, d)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn submit_flag(state: State<AppState>, value: String) -> CmdResult<SubmitResult> {
    let mut store = state.store.lock().unwrap();
    let Store { profile, active } = &mut *store;
    let profile = profile.as_mut().ok_or("Create your profile first.")?;
    let active = active.as_mut().ok_or("No lab is active.")?;
    let lab = &state.lab(&active.attempt.lab_id)?.manifest;
    let all: Vec<&catalog::LabManifest> = state.catalog.labs.iter().map(|l| &l.manifest).collect();
    let result = progress::submit_flag(profile, &mut active.attempt, lab, &all, &value, now());
    state.save(&store);
    Ok(result)
}

#[tauri::command]
fn reveal_hint(state: State<AppState>) -> CmdResult<ActiveView> {
    let mut store = state.store.lock().unwrap();
    let active = store.active.as_mut().ok_or("No lab is active.")?;
    let lab = state.lab(&active.attempt.lab_id)?;
    if active.attempt.hints_revealed >= lab.manifest.hints.len() {
        return Err("No more hints for this lab.".into());
    }
    active.attempt.hints_revealed += 1;
    let view = active_view(&state, store.active.as_ref().unwrap()).ok_or("lab vanished")?;
    state.save(&store);
    Ok(view)
}

#[tauri::command]
fn get_map(state: State<AppState>, reveal: Option<u8>) -> CmdResult<MapView> {
    let mut store = state.store.lock().unwrap();
    let active = store.active.as_mut().ok_or("No lab is active.")?;
    let lab = state.lab(&active.attempt.lab_id)?;
    let max = max_map_level(lab);
    if let Some(level) = reveal {
        active.attempt.map_level = active.attempt.map_level.max(level.min(max));
    }
    let level = active.attempt.map_level;
    let m = &lab.manifest;
    let view = MapView {
        level,
        max_level: max,
        next_level_cost: (level < max).then(|| progress::MAP_LEVEL_COST[level as usize + 1]),
        subnet: m.subnet(),
        gateway: m.gateway(),
        hosts: m
            .hosts
            .iter()
            .enumerate()
            .map(|(i, h)| MapHost {
                name: h.name.clone(),
                os: h.os.clone(),
                ip: m.host_ip(i),
                ports: if level >= 2 { h.ports.clone() } else { vec![] },
            })
            .collect(),
        edges: if level >= 3 { m.map.edges.clone() } else { vec![] },
    };
    state.save(&store);
    Ok(view)
}

fn labs_dir(app: &AppHandle) -> PathBuf {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../labs");
    if let Some(dir) = std::env::var_os("LABFORGE_LABS_DIR") {
        return PathBuf::from(dir);
    }
    if cfg!(debug_assertions) && dev.is_dir() {
        return dev;
    }
    app.path()
        .resource_dir()
        .map(|d| d.join("labs"))
        .ok()
        .filter(|d| d.is_dir())
        .unwrap_or(dev)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's DMA-BUF renderer draws a blank window in many VMs (VMware, VirtualBox).
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let labs_dir = labs_dir(app.handle());
            let catalog = Catalog::load(&labs_dir);
            let mut active: Option<ActiveLab> = read_json::<Option<ActiveLab>>(&data_dir.join("active.json")).flatten();
            // A start that was interrupted by closing the app can't be resumed mid-way.
            if let Some(a) = active.as_mut().filter(|a| a.phase == Phase::Starting) {
                a.phase = Phase::Failed;
                a.error = Some("The launcher closed while this lab was starting. Reset it to try again.".into());
            }
            let store = Store {
                profile: read_json::<Option<Profile>>(&data_dir.join("profile.json")).flatten(),
                active: active.filter(|a| catalog.get(&a.attempt.lab_id).is_some()),
            };
            app.manage(AppState { catalog, labs_dir, data_dir, store: Mutex::new(store), busy: AtomicBool::new(false) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            check_update,
            install_update,
            get_profile,
            create_profile,
            run_preflight,
            list_labs,
            get_active,
            start_lab,
            stop_lab,
            resume_lab,
            reset_lab,
            destroy_lab,
            host_status,
            submit_flag,
            reveal_hint,
            get_map,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LabForge");
}
