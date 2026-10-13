import { call, on, isTauri } from "./api.js";
import { renderMap } from "./map.js";
import { $, $$, esc, cap, pct, duration, toast } from "./util.js";

const TRACKS = [
  { id: "web", name: "Web", blurb: "Injection, broken auth and logic flaws in real-looking web apps.", icon: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>' },
  { id: "api", name: "API", blurb: "REST and GraphQL targets: object-level auth, tokens, mass assignment.", icon: '<path d="M8 4C6 4 5 5 5 7v2c0 1-1 2-2 3 1 1 2 2 2 3v2c0 2 1 3 3 3M16 4c2 0 3 1 3 3v2c0 1 1 2 2 3-1 1-2 2-2 3v2c0 2-1 3-3 3"/>' },
  { id: "network", name: "Network", blurb: "Enumerate services, chain footholds and pivot between subnets.", icon: '<circle cx="12" cy="5" r="2.5"/><circle cx="5" cy="19" r="2.5"/><circle cx="19" cy="19" r="2.5"/><path d="M12 7.5v4M12 11.5 6.5 17M12 11.5l5.5 5.5"/>' },
  { id: "linux", name: "Linux PrivEsc", blurb: "Start as a low-privilege user and work your way to root.", icon: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/>' },
  { id: "windows", name: "Windows", blurb: "Standalone Windows hosts: services, tokens and misconfigurations.", icon: '<rect x="3" y="3" width="8" height="8" rx="1"/><rect x="13" y="3" width="8" height="8" rx="1"/><rect x="3" y="13" width="8" height="8" rx="1"/><rect x="13" y="13" width="8" height="8" rx="1"/>' },
  { id: "ad", name: "Active Directory", blurb: "Domain controllers, Kerberos and forest trusts on your machine.", icon: '<rect x="9" y="3" width="6" height="5" rx="1"/><rect x="3" y="16" width="6" height="5" rx="1"/><rect x="15" y="16" width="6" height="5" rx="1"/><path d="M12 8v4M6 16v-4h12v4"/>' },
];
const TRACK = Object.fromEntries(TRACKS.map((t) => [t.id, t]));
const DIFFS = [
  { id: "easy", mult: "1×" },
  { id: "medium", mult: "1.5×" },
  { id: "hard", mult: "2×" },
  { id: "insane", mult: "3×" },
];
const START_STEPS = [
  { id: "build", label: "Build lab images" },
  { id: "up", label: "Start hosts on an isolated network" },
  { id: "health", label: "Wait for services" },
  { id: "flags", label: "Plant fresh flags" },
];
const ALL_BADGES = [
  ["First Blood", "Pwn your first lab", "🩸"],
  ["No Assist", "Pwn a lab without hints or map reveals", "🎯"],
  ["Medium Rare", "Pwn a lab on Medium or harder", "🔥"],
  ["Hardened", "Pwn a lab on Hard or Insane", "🛡️"],
  ["Web Track Complete", "Pwn every Web lab", "🌐"],
  ["API Track Complete", "Pwn every API lab", "🧩"],
];

const S = {
  profile: null,
  labs: [],
  errors: [],
  labsDir: "",
  active: null,
  preflight: null,
  view: "home",
  filter: { track: "all", status: "all", q: "" },
  steps: {},
  hostStates: [],
  modal: null,
  pickedDiff: {},
  onboardSkill: "beginner",
  version: "",
  update: null,
  updateState: "idle",
  updateMsg: "",
  labOffers: null,
  labState: "idle",
  labMsg: "",
};

const lab = (id) => S.labs.find((l) => l.id === id);
const trackColor = (t) => `var(--track-${t})`;

// ---------------------------------------------------------------- boot ----

async function boot() {
  const [profile, catalogue, active] = await Promise.all([call("get_profile"), call("list_labs"), call("get_active")]);
  S.profile = profile;
  setCatalogue(catalogue);
  S.active = active;
  if (active?.phase === "ready") START_STEPS.forEach((s) => (S.steps[s.id] = { state: "done", message: "" }));

  on("lab-progress", (p) => {
    S.steps[p.step] = { state: p.state, message: p.message };
    if (S.view === "mission") renderSteps();
  });
  on("lab-ready", async () => {
    S.active = await call("get_active");
    toast(`${lab(S.active.lab_id)?.name} is ready. Go get it!`, "ok");
    refreshHosts();
    render();
  });
  on("lab-error", async (msg) => {
    S.active = await call("get_active");
    toast("The lab couldn't start. See the details in Active lab.", "bad");
    render();
  });

  document.addEventListener("click", onClick);
  document.addEventListener("submit", onSubmit);
  document.addEventListener("keydown", (e) => e.key === "Escape" && S.modal && closeModal());
  document.addEventListener("input", (e) => {
    if (e.target.id === "lab-search") {
      S.filter.q = e.target.value;
      renderLabGrid();
    }
  });
  setInterval(tick, 1000);
  setInterval(refreshHosts, 5000);

  if (!S.profile) renderOnboarding();
  else render();
  runPreflight();
  call("app_info").then((i) => { S.version = i.version; if (S.view === "system") render(); });
  checkUpdate();
  checkLabUpdates();
}

function setCatalogue(c) {
  S.labs = c.labs;
  S.errors = c.errors;
  S.labsDir = c.labs_dir;
}

async function runPreflight() {
  S.preflight = null;
  if (S.view === "system") render();
  S.preflight = await call("run_preflight");
  $("#nav-warn").hidden = S.preflight.ready;
  if (["system", "labs", "home"].includes(S.view) && S.profile) render();
}

async function refreshHosts() {
  if (!S.active || S.active.phase !== "ready") return;
  try {
    S.hostStates = await call("host_status");
    if (S.view === "mission") renderTargets();
  } catch {
    /* host status is best-effort */
  }
}

function tick() {
  const el = $("#elapsed");
  if (el && S.active) el.textContent = duration(Math.max(0, Math.floor(Date.now() / 1000) - S.active.started_at));
}


// ------------------------------------------------------------ updates ----

async function checkUpdate(manual = false) {
  if (S.updateState === "checking" || S.updateState === "installing") return;
  S.updateState = "checking";
  S.updateMsg = "";
  if (S.view === "system") render();
  try {
    S.update = await call("check_update");
    S.updateMsg = S.update ? "" : "You're on the latest version.";
    if (manual && !S.update) toast(S.updateMsg, "ok");
  } catch (e) {
    // A failed background check (offline, no release yet) shouldn't nag.
    S.updateMsg = manual ? `Couldn't check for updates: ${e}` : "";
    if (manual) toast(S.updateMsg, "bad");
  }
  S.updateState = "idle";
  renderSidebarXP();
  if (S.view === "system") render();
}

async function installUpdate() {
  if (S.updateState === "installing") return;
  S.updateState = "installing";
  S.updateMsg = "Downloading…";
  renderSidebarXP();
  if (S.view === "system") render();
  try {
    await call("install_update"); // restarts the app on success
  } catch (e) {
    S.updateState = "idle";
    S.updateMsg = `Update failed: ${e}`;
    toast(S.updateMsg, "bad");
    renderSidebarXP();
    if (S.view === "system") render();
  }
}

function updateBanner() {
  if (!S.update) return "";
  const busy = S.updateState === "installing";
  return `<div class="update-card"><strong>Update available</strong><span class="muted">LabForge ${esc(S.update.version)}</span>
    <button class="btn small primary" data-action="install-update" ${busy ? "disabled" : ""}>${busy ? esc(S.updateMsg) : "Install & restart"}</button></div>`;
}

// ---------------------------------------------------------- lab packs ----

const fmtSize = (b) => (b >= 1048576 ? `${(b / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1024))} KB`);

async function checkLabUpdates(manual = false) {
  if (S.labState !== "idle") return;
  S.labState = "checking";
  S.labMsg = "";
  if (S.view === "labs") render();
  try {
    S.labOffers = await call("check_lab_updates");
    S.labMsg = S.labOffers.length ? "" : "All labs are up to date.";
    if (manual && !S.labOffers.length) toast(S.labMsg, "ok");
  } catch (e) {
    // Offline or no index published yet: only complain when the learner asked.
    S.labOffers = [];
    S.labMsg = manual ? `Couldn't check for new labs: ${e}` : "";
    if (manual) toast(S.labMsg, "bad");
  }
  S.labState = "idle";
  if (S.view === "labs") render();
}

async function installLabs(ids) {
  if (S.labState !== "idle" || !ids.length) return;
  S.labState = "installing";
  S.labMsg = "";
  if (S.view === "labs") render();
  try {
    const r = await call("install_labs", { ids });
    setCatalogue(await call("list_labs"));
    S.labOffers = S.labOffers.filter((o) => !r.installed.includes(o.id));
    if (r.installed.length) toast(`Installed ${r.installed.length} lab${r.installed.length === 1 ? "" : "s"}.`, "ok");
    r.failed.forEach((f) => toast(`${f.id}: ${f.error}`, "bad"));
  } catch (e) {
    toast(`Couldn't install: ${e}`, "bad");
  }
  S.labState = "idle";
  render();
}

function labOffersPanel() {
  const busy = S.labState !== "idle";
  const offers = S.labOffers ?? [];
  if (!offers.length) return S.labMsg ? `<p class="faint" style="margin-bottom:16px">${esc(S.labMsg)}</p>` : "";
  const installable = offers.filter((o) => !o.locked && !o.blocked);
  const rows = offers
    .map((o) => {
      const what = o.installed_version ? `Update ${esc(o.installed_version)} → ${esc(o.version)}` : `New · v${esc(o.version)}`;
      const action = o.locked
        ? `<span class="faint">Pro</span>`
        : o.blocked
          ? `<span class="faint">${esc(o.blocked)}</span>`
          : `<button class="btn small" data-action="install-lab" data-id="${esc(o.id)}" ${busy ? "disabled" : ""}>Install</button>`;
      return `<div class="offer"><div class="grow"><strong>${esc(o.name)}</strong> <span class="faint">${what} · ${fmtSize(o.size)}</span>${o.blurb ? `<p class="faint" style="font-size:13px">${esc(o.blurb)}</p>` : ""}</div>${action}</div>`;
    })
    .join("");
  return `<div class="panel" style="margin-bottom:20px"><div class="section-title"><h2>${offers.length} lab${offers.length === 1 ? "" : "s"} available</h2>
      ${installable.length > 1 ? `<button class="btn small primary" data-action="install-all-labs" ${busy ? "disabled" : ""}>${S.labState === "installing" ? "Installing…" : "Install all"}</button>` : ""}</div>${rows}</div>`;
}
// ------------------------------------------------------------- render ----

function render() {
  $$("#nav button").forEach((b) => b.classList.toggle("active", b.dataset.view === S.view));
  $("#nav-live").hidden = !S.active;
  renderSidebarXP();
  const main = $("#main");
  main.innerHTML = { home: viewHome, labs: viewLabs, mission: viewMission, progress: viewProgress, system: viewSystem }[S.view]();
  if (S.view === "labs") renderLabGrid();
  if (S.view === "mission") {
    renderSteps();
    renderTargets();
    tick();
  }
  renderModal();
}

function go(view) {
  S.view = view;
  render();
  $("#main").focus({ preventScroll: true });
  window.scrollTo(0, 0);
}

function xpBar(level, extraClass = "") {
  const span = level.next_level_xp - level.level_floor;
  const into = level.xp - level.level_floor;
  return `<div class="xp ${extraClass}">
    <div class="xp-top"><span class="xp-level">Lv ${level.level} · ${esc(level.title)}</span><span class="muted">${level.xp} XP</span></div>
    <div class="bar"><span style="width:${Math.min(100, (into / span) * 100)}%"></span></div>
    <div class="xp-top faint"><span>${span - into} XP to level ${level.level + 1}</span></div>
  </div>`;
}

function renderSidebarXP() {
  $("#sidebar-foot").innerHTML = updateBanner() + (S.profile
    ? `<div class="faint" style="font-size:12px;margin:0 4px 8px">Signed in as <strong class="muted">${esc(S.profile.username)}</strong></div>${xpBar(S.profile.level)}`
    : "");
}

function icon(track) {
  return `<svg viewBox="0 0 24 24">${TRACK[track]?.icon ?? ""}</svg>`;
}

// home
function viewHome() {
  const p = S.profile;
  const next = recommendedLab();
  const activeLab = S.active && lab(S.active.lab_id);
  return `
    <section class="hero">
      <div>
        <p class="muted">${p.level.xp || p.pwned.length ? "Welcome back," : "Welcome,"}</p>
        <h1>${esc(p.username)}</h1>
        <p class="muted" style="margin-top:10px;max-width:520px">Pick a track, choose how hard you want it, and hit start. Everything runs on your machine, offline, on an isolated network.</p>
        ${next ? `<div class="row" style="margin-top:18px"><button class="btn primary" data-action="open-lab" data-id="${next.id}">Recommended: ${esc(next.name)}</button><button class="btn ghost" data-view="labs">Browse all labs</button></div>` : ""}
      </div>
      ${xpBar(p.level)}
    </section>
    ${activeLab ? `<section><div class="banner"><span class="live-dot" style="margin:0"></span><div class="grow"><strong>${esc(activeLab.name)}</strong> <span class="muted">· ${cap(S.active.difficulty)} · ${phaseLabel(S.active.phase)}</span></div><button class="btn go" data-view="mission">Open lab</button></div></section>` : ""}
    ${S.preflight && !S.preflight.ready ? `<section><div class="banner" style="border-color:rgba(255,181,71,.45);background:rgba(255,181,71,.07)"><div class="grow"><strong>Almost there.</strong> <span class="muted">Your machine needs a quick setup before labs can start.</span></div><button class="btn" data-view="system">Fix it</button></div></section>` : ""}
    <section>
      <div class="section-title"><h2>Choose a track</h2><span class="faint">${S.labs.length} lab${S.labs.length === 1 ? "" : "s"} installed</span></div>
      <div class="grid tracks">
        ${TRACKS.map((t) => {
          const n = S.labs.filter((l) => l.track === t.id).length;
          return `<button class="track" style="--c:${trackColor(t.id)}" data-action="track" data-track="${t.id}" ${n ? "" : "disabled"}>
            <span class="icon">${icon(t.id)}</span>
            <h3>${esc(t.name)}</h3>
            <p class="muted" style="font-size:14px">${esc(t.blurb)}</p>
            <span class="count">${n ? `${n} lab${n === 1 ? "" : "s"}` : "Coming soon"}</span>
          </button>`;
        }).join("")}
      </div>
    </section>`;
}

function recommendedLab() {
  const done = new Set((S.profile?.pwned ?? []).map((k) => k.split(":")[0]));
  return S.labs.find((l) => !done.has(l.id)) ?? S.labs[0];
}

function phaseLabel(phase) {
  return { starting: "Starting…", ready: "Ready", stopped: "Paused", failed: "Failed to start" }[phase] ?? phase;
}

// labs
function viewLabs() {
  const f = S.filter;
  const trackChips = [{ id: "all", name: "All" }, ...TRACKS.filter((t) => S.labs.some((l) => l.track === t.id))]
    .map((t) => `<button class="chip ${f.track === t.id ? "on" : ""}" data-action="filter-track" data-track="${t.id}">${esc(t.name)}</button>`)
    .join("");
  const statusChips = [["all", "Any status"], ["new", "Not pwned"], ["pwned", "Pwned"]]
    .map(([id, label]) => `<button class="chip ${f.status === id ? "on" : ""}" data-action="filter-status" data-status="${id}">${label}</button>`)
    .join("");
  return `
    <div class="page-head"><div><h1>Labs</h1><p class="muted">Every lab runs locally in containers on its own isolated network.</p></div>
      <div class="row-gap"><button class="btn" data-action="check-labs" ${S.labState !== "idle" ? "disabled" : ""}>${S.labState === "checking" ? "Checking…" : "Check for new labs"}</button>
      <input id="lab-search" class="search" type="search" placeholder="Search labs or skills" value="${esc(f.q)}" aria-label="Search labs"></div></div>
    ${labOffersPanel()}
    <div class="filters"><div class="chips">${trackChips}</div><div class="chips">${statusChips}</div></div>
    <div class="grid labs" id="lab-grid"></div>
    ${S.errors.length ? `<p class="faint" style="margin-top:20px">${S.errors.length} lab package${S.errors.length === 1 ? "" : "s"} couldn't be loaded. See System.</p>` : ""}`;
}

function fit(l) {
  const avail = S.preflight?.host.ram_available_mb;
  if (!avail) return null;
  if (avail >= l.resources.ram_mb * 2) return ["ok", "Runs great here"];
  if (avail >= l.resources.ram_mb) return ["warn", "Tight on memory"];
  return ["bad", "Not enough memory"];
}

function renderLabGrid() {
  const grid = $("#lab-grid");
  if (!grid) return;
  const f = S.filter;
  const q = f.q.trim().toLowerCase();
  const labs = S.labs.filter(
    (l) =>
      (f.track === "all" || l.track === f.track) &&
      (f.status === "all" || (f.status === "pwned") === l.pwned.length > 0) &&
      (!q || [l.name, l.blurb, ...l.skills].some((s) => s.toLowerCase().includes(q))),
  );
  grid.innerHTML = labs.length
    ? labs.map(labCard).join("")
    : `<div class="empty" style="grid-column:1/-1">No labs match. ${S.labs.length ? "Try clearing the filters." : `No lab packages found in <code>${esc(S.labsDir)}</code>.`}</div>`;
}

function labCard(l) {
  const ft = fit(l);
  return `<button class="lab-card" style="--c:${trackColor(l.track)}" data-action="open-lab" data-id="${l.id}">
    <div class="lab-art">
      <span class="pill" style="color:${trackColor(l.track)}">${icon(l.track).replace("<svg", '<svg width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"')}${esc(l.track_name)}</span>
      ${l.pwned.length ? `<span class="pill ok pwned">Pwned · ${l.pwned.map(cap).join(", ")}</span>` : ""}
    </div>
    <div class="lab-body">
      <h3>${esc(l.name)}</h3>
      <p>${esc(l.blurb)}</p>
      <div class="lab-meta">
        <span class="pill accent">${l.xp} XP</span>
        <span class="pill">~${l.est_minutes} min</span>
        <span class="pill">${l.hosts.length} host${l.hosts.length === 1 ? "" : "s"}</span>
        ${ft ? `<span class="pill ${ft[0]}">${ft[1]}</span>` : ""}
      </div>
    </div>
  </button>`;
}

// mission
function viewMission() {
  const a = S.active;
  if (!a) {
    return `<div class="page-head"><div><h1>Active lab</h1></div></div>
      <div class="empty">No lab is running. <button class="btn primary" data-view="labs" style="margin-left:12px">Pick a lab</button></div>`;
  }
  const l = lab(a.lab_id);
  const phasePill = { starting: "warn", ready: "ok", stopped: "", failed: "bad" }[a.phase];
  return `
    <div class="mission-head">
      <div>
        <div class="row"><span class="pill" style="color:${trackColor(l.track)}">${esc(l.track_name)}</span><span class="pill accent">${cap(a.difficulty)}</span><span class="pill ${phasePill}">${phaseLabel(a.phase)}</span></div>
        <h1 style="margin-top:10px">${esc(l.name)}</h1>
      </div>
      <div class="row">
        <span class="muted">Time <strong id="elapsed" class="mono"></strong></span>
        ${a.phase === "ready" ? `<button class="btn" data-action="stop">Pause</button>` : ""}
        ${a.phase === "stopped" ? `<button class="btn go" data-action="resume">Resume</button>` : ""}
        ${a.phase !== "starting" ? `<button class="btn" data-action="reset">Reset</button><button class="btn danger" data-action="destroy">Destroy</button>` : ""}
      </div>
    </div>
    ${a.phase === "failed" ? `<div class="error-box" style="margin-bottom:16px"><strong>The lab couldn't start.</strong>\n${esc(a.error)}</div>` : ""}
    <div class="mission-grid">
      <div class="stack">
        ${a.phase !== "ready" ? `<div class="panel"><h3 style="margin-bottom:8px">Launch progress</h3><div class="steps" id="steps"></div></div>` : ""}
        ${a.phase === "ready" ? flagsPanel(l, a) : ""}
        <div class="panel" id="targets"></div>
      </div>
      <div class="stack">
        <div class="panel">
          <div class="section-title" style="margin-bottom:10px"><h3>Stuck?</h3><span class="faint">${a.penalty ? `−${pct(a.penalty)} XP so far` : "No XP spent yet"}</span></div>
          <p class="muted" style="font-size:14px">The map shows how the lab fits together. Deeper levels and hints cost part of this lab's XP.</p>
          <div class="row" style="margin-top:14px">
            <button class="btn" data-action="map">Show lab map</button>
            ${a.next_hint_cost != null ? `<button class="btn ghost" data-action="hint">Hint (−${pct(a.next_hint_cost)} XP)</button>` : ""}
          </div>
          ${a.hints.length ? `<div class="stack" style="margin-top:16px;gap:10px">${a.hints.map((h, i) => `<div class="hint"><strong>Hint ${i + 1}.</strong> ${esc(h)}</div>`).join("")}</div>` : ""}
        </div>
        <div class="panel">
          <h3 style="margin-bottom:10px">Briefing</h3>
          <p class="muted" style="font-size:14px">${esc(l.story || l.blurb)}</p>
        </div>
      </div>
    </div>`;
}

function flagsPanel(l, a) {
  return `<div class="panel">
    <div class="section-title" style="margin-bottom:12px"><h3>Submit a flag</h3><span class="faint">${a.captured.length} / ${l.flags.length} captured</span></div>
    <form class="flag-form" id="flag-form" autocomplete="off">
      <input name="flag" placeholder="LF{...}" aria-label="Flag" spellcheck="false" required>
      <button class="btn primary">Submit</button>
    </form>
    <div class="flag-list">
      ${l.flags.map((f) => {
        const got = a.captured.includes(f.id);
        return `<div class="flag-item ${got ? "got" : ""}"><span>${got ? "✅" : "🚩"} ${esc(f.label)} <span class="faint">on ${esc(f.host)}</span></span><span class="pill ${got ? "ok" : ""}">${got ? "Captured" : "Not yet"}</span></div>`;
      }).join("")}
    </div>
  </div>`;
}

function renderSteps() {
  const el = $("#steps");
  if (!el) return;
  const ids = S.active?.phase === "starting" && !S.steps.build && S.steps.up ? START_STEPS.filter((s) => s.id === "up" || s.id === "health") : START_STEPS;
  el.innerHTML = ids
    .map((s) => {
      const st = S.steps[s.id];
      const cls = st?.state ?? "";
      const mark = cls === "done" ? "✓" : cls === "error" ? "!" : "";
      const msg = st && st.message && st.message !== s.label && cls !== "done" ? `<div class="msg">${esc(st.message)}</div>` : "";
      return `<div class="step ${cls}"><span class="dot">${mark}</span><div><div>${s.label}</div>${msg}</div></div>`;
    })
    .join("");
}

function renderTargets() {
  const el = $("#targets");
  if (!el || !S.active) return;
  const l = lab(S.active.lab_id);
  const states = Object.fromEntries(S.hostStates.map((h) => [h.name, h.state]));
  const ready = S.active.phase === "ready";
  el.innerHTML = `
    <div class="section-title" style="margin-bottom:6px"><h3>Targets</h3><span class="faint mono">${esc(l.subnet)}</span></div>
    <table>
      <thead><tr><th>Host</th><th>IP</th><th>OS</th><th>Status</th></tr></thead>
      <tbody>${l.hosts.map((h) => {
        const st = ready ? states[h.name] ?? "up" : S.active.phase === "starting" ? "booting" : "down";
        const label = { up: "Up", booting: "Starting", down: "Down", unhealthy: "Unhealthy", absent: "Missing" }[st];
        return `<tr><td><strong>${esc(h.name)}</strong></td><td><button class="btn small ghost mono" data-action="copy" data-copy="${esc(h.ip)}" title="Copy IP">${esc(h.ip)}</button></td><td class="muted">${esc(h.os)}</td><td><span class="status ${st}">${label}</span></td></tr>`;
      }).join("")}</tbody>
    </table>
    <p class="faint" style="font-size:13px;margin-top:12px">Attack from this machine: the lab subnet is routed straight to your host, so point your own tools at these IPs. Connecting a separate attack VM (WireGuard) is coming next.</p>`;
}

// progress
function viewProgress() {
  const p = S.profile;
  const pwned = p.pwned.map((k) => {
    const [id, d] = k.split(":");
    return { lab: lab(id), d };
  }).filter((x) => x.lab);
  return `
    <div class="page-head"><div><h1>Progress</h1><p class="muted">Every flag you capture earns XP. Harder difficulty pays more; hints cost a little.</p></div></div>
    <section class="grid" style="grid-template-columns:1fr 1fr">
      ${xpBar(p.level)}
      <div class="xp"><div class="xp-top"><span class="xp-level">Labs pwned</span><span class="muted">${pwned.length}</span></div>
      <div class="xp-top faint"><span>Badges earned</span><span>${p.badges.length}</span></div></div>
    </section>
    <section>
      <div class="section-title"><h2>Badges</h2></div>
      <div class="grid badges">
        ${ALL_BADGES.map(([name, desc, emoji]) => {
          const got = p.badges.includes(name);
          return `<div class="badge ${got ? "" : "locked"}"><div class="medal">${emoji}</div><strong>${esc(name)}</strong><span class="faint" style="font-size:13px">${esc(desc)}</span></div>`;
        }).join("")}
      </div>
    </section>
    <section>
      <div class="section-title"><h2>Pwned labs</h2></div>
      ${pwned.length
        ? `<div class="panel"><table><thead><tr><th>Lab</th><th>Track</th><th>Difficulty</th></tr></thead><tbody>${pwned.map((x) => `<tr><td><strong>${esc(x.lab.name)}</strong></td><td class="muted">${esc(x.lab.track_name)}</td><td><span class="pill accent">${cap(x.d)}</span></td></tr>`).join("")}</tbody></table></div>`
        : `<div class="empty">Nothing pwned yet. Your first lab is waiting.</div>`}
    </section>`;
}

// system
function viewSystem() {
  const pf = S.preflight;
  const checks = pf
    ? pf.checks.map((c) => {
        const mark = { ok: "✓", warn: "!", fail: "✕" }[c.status];
        const fix = c.fix ? `<div class="fix">${c.fix.split("\n").map((line, i) => (i === 0 ? `<p class="muted" style="font-size:14px">${esc(line)}</p>` : `<div class="copy" style="margin-top:6px"><code>${esc(line)}</code><button class="btn small" data-action="copy" data-copy="${esc(line)}">Copy</button></div>`)).join("")}</div>` : "";
        return `<div class="check"><span class="icon ${c.status}">${mark}</span><div><strong>${esc(c.label)}</strong><p class="faint" style="font-size:14px">${esc(c.detail)}</p>${fix}</div></div>`;
      }).join("")
    : `<div class="empty">Checking your machine…</div>`;
  return `
    <div class="page-head"><div><h1>System</h1><p class="muted">What your machine has, and anything it needs before labs can run.</p></div>
      <button class="btn" data-action="preflight">Run checks again</button></div>
    ${pf ? `<div class="banner" style="margin-bottom:20px;${pf.ready ? "" : "border-color:rgba(255,92,122,.45);background:rgba(255,92,122,.07)"}"><div class="grow"><strong>${pf.ready ? "Ready to hack." : "Labs can't start yet."}</strong> <span class="muted">${pf.ready ? "Docker and Compose are good to go." : "Fix the items marked ✕ below."}</span></div></div>` : ""}
    <div class="checks">${checks}</div>
    <section>
      <div class="section-title"><h2>About this install</h2></div>
      <div class="panel"><dl class="kv">
        <dt>Version</dt><dd>${esc(S.version || "unknown")} <button class="btn small" data-action="check-update" ${S.updateState === "checking" || S.updateState === "installing" ? "disabled" : ""}>${S.updateState === "checking" ? "Checking…" : "Check for updates"}</button>${S.update ? ` <span class="ok-text">${esc(S.update.version)} available</span> <button class="btn small primary" data-action="install-update">Install & restart</button>` : S.updateMsg ? ` <span class="faint">${esc(S.updateMsg)}</span>` : ""}</dd>
        <dt>Mode</dt><dd>${isTauri ? "Desktop app" : "Browser preview (mock data)"}</dd>
        ${pf ? `<dt>Host OS</dt><dd>${esc(pf.host.os)}</dd>` : ""}
        <dt>Lab packages</dt><dd class="mono">${esc(S.labsDir)}</dd>
        <dt>Labs loaded</dt><dd>${S.labs.length}</dd>
      </dl>
      ${S.errors.length ? `<div class="error-box" style="margin-top:16px"><strong>Packages that failed to load</strong>\n${S.errors.map(esc).join("\n")}</div>` : ""}
      </div>
    </section>`;
}

// onboarding
function renderOnboarding() {
  $(".shell").hidden = true;
  let el = $(".onboard");
  if (!el) {
    el = document.createElement("div");
    el.className = "onboard";
    document.body.prepend(el);
  }
  const skills = [
    ["beginner", "Beginner", "New to hacking. Show me hints and start easy."],
    ["intermediate", "Intermediate", "I've rooted a few boxes. Medium by default."],
    ["advanced", "Advanced", "No hand-holding. Give me the hard stuff."],
  ];
  el.innerHTML = `<form class="panel card" id="onboard-form" autocomplete="off">
    <div class="brand" style="padding:0"><img src="logo.svg" alt="" width="40" height="40"><span style="font-size:24px">LabForge</span></div>
    <div><h1>Hack real machines. On your own machine.</h1><p class="muted" style="margin-top:8px">Pick a lab, set the difficulty, hit start. LabForge builds an isolated network of vulnerable hosts for you to break into, then levels you up as you capture flags.</p></div>
    <label class="field"><strong>Choose a handle</strong><input name="username" maxlength="24" placeholder="e.g. zero_cool" required></label>
    <div class="field"><strong>Where are you starting from?</strong>
      <div class="grid skills">${skills.map(([id, name, d]) => `<button type="button" class="skill ${S.onboardSkill === id ? "on" : ""}" data-action="skill" data-skill="${id}"><strong>${name}</strong><span class="faint" style="font-size:13px">${d}</span></button>`).join("")}</div>
    </div>
    <p class="faint" style="font-size:13px">Labs are for practice on your own machine. Only use these techniques on systems you're authorized to test.</p>
    <div class="row" style="justify-content:flex-end"><button class="btn primary big">Let's go</button></div>
  </form>`;
  $("input[name=username]", el).focus();
}

// ------------------------------------------------------------- modals ----

function renderModal() {
  const root = $("#modal-root");
  const m = S.modal;
  if (!m) {
    root.innerHTML = "";
    return;
  }
  const body = { lab: modalLab, map: modalMap, confirm: modalConfirm, celebrate: modalCelebrate }[m.type](m);
  root.innerHTML = `<div class="backdrop" data-action="backdrop"><div class="modal" role="dialog" aria-modal="true">${body}</div></div>`;
  if (m.type === "celebrate") confetti($(".celebrate"));
}

function closeModal() {
  S.modal = null;
  renderModal();
}

function defaultDiff(l) {
  const pref = { beginner: "easy", intermediate: "medium", advanced: "hard" }[S.profile?.skill] ?? "easy";
  const order = DIFFS.map((d) => d.id);
  const supported = l.difficulties;
  return supported.includes(pref) ? pref : supported.filter((d) => order.indexOf(d) <= order.indexOf(pref)).pop() ?? supported[0];
}

function modalLab({ id }) {
  const l = lab(id);
  const d = S.pickedDiff[id] ?? defaultDiff(l);
  S.pickedDiff[id] = d;
  const mult = { easy: 1, medium: 1.5, hard: 2, insane: 3 }[d];
  const ft = fit(l);
  const blocked = S.active && S.active.lab_id !== id;
  const running = S.active && S.active.lab_id === id;
  const notReady = S.preflight && !S.preflight.ready;
  return `
    <div class="lab-art" style="--c:${trackColor(l.track)};height:120px;border-radius:20px 20px 0 0"><span class="pill" style="color:${trackColor(l.track)}">${esc(l.track_name)}</span></div>
    <div class="modal-head"><div><h2 style="font-size:26px">${esc(l.name)}</h2><p class="muted" style="margin-top:6px">${esc(l.blurb)}</p></div><button class="close" data-action="close" aria-label="Close">✕</button></div>
    <div class="modal-body">
      ${l.story ? `<p>${esc(l.story)}</p>` : ""}
      <div class="chips">${l.skills.map((s) => `<span class="pill">${esc(s.replaceAll("-", " "))}</span>`).join("")}</div>
      <div class="field"><strong>Difficulty</strong>
        <div class="difficulty">${DIFFS.map((x) => `<button data-action="pick-diff" data-id="${id}" data-d="${x.id}" class="${d === x.id ? "on" : ""}" ${l.difficulties.includes(x.id) ? "" : "disabled title='Not available for this lab yet'"}><strong>${cap(x.id)}</strong><small>${l.difficulties.includes(x.id) ? `${x.mult} XP` : "Soon"}</small>${l.pwned.includes(x.id) ? "<small style='color:var(--ok)'>Pwned</small>" : ""}</button>`).join("")}</div>
        ${l.difficulty_notes[d] ? `<div class="note">${esc(l.difficulty_notes[d])}</div>` : ""}
      </div>
      <dl class="kv">
        <dt>Reward</dt><dd><strong>${Math.round(l.xp * mult)} XP</strong> <span class="faint">for all ${l.flags.length} flags</span></dd>
        <dt>Time</dt><dd>~${l.est_minutes} minutes</dd>
        <dt>Needs</dt><dd>${l.resources.ram_mb} MB RAM · ${l.resources.cpus} CPU · ${l.resources.disk_mb} MB disk ${ft ? `<span class="pill ${ft[0]}" style="margin-left:6px">${ft[1]}</span>` : ""}</dd>
        <dt>Network</dt><dd class="mono">${esc(l.subnet)} · ${l.hosts.length} host${l.hosts.length === 1 ? "" : "s"}</dd>
        <dt>Version</dt><dd>v${esc(l.version ?? "1.0.0")} <span class="faint">· ${l.source === "installed" ? "downloaded update" : "bundled with LabForge"}${l.tier === "pro" ? " · Pro" : ""}</span></dd>
      </dl>
      <div class="modal-foot">
        <span class="faint" style="font-size:13px">${blocked ? `${esc(lab(S.active.lab_id)?.name)} is still active. Destroy it first.` : notReady ? "Your machine needs setup first. See System." : running ? "This lab is already running." : "You'll get a notification when it's ready."}</span>
        ${running ? `<button class="btn go big" data-action="goto-mission">Open lab</button>` : `<button class="btn go big" data-action="start" data-id="${id}" ${blocked || notReady ? "disabled" : ""}>▶ Start lab</button>`}
      </div>
    </div>`;
}

function modalMap({ map }) {
  const next = map.level < map.max_level;
  const names = ["", "Hosts", "Services", "Relationships"];
  return `
    <div class="modal-head"><div><h2>Lab map</h2><p class="muted" style="margin-top:6px">Level ${map.level} of ${map.max_level}: ${names.slice(1, map.level + 1).join(" + ")}</p></div><button class="close" data-action="close" aria-label="Close">✕</button></div>
    <div class="modal-body">
      <div class="map-wrap">${renderMap(map)}</div>
      <div class="modal-foot">
        <span class="faint" style="font-size:13px">${next ? `Next level reveals ${names[map.level + 1].toLowerCase()}.` : "Everything is revealed."}</span>
        ${next ? `<button class="btn" data-action="map-reveal" data-level="${map.level + 1}">Reveal ${names[map.level + 1].toLowerCase()} (−${pct(map.next_level_cost)} XP)</button>` : ""}
      </div>
    </div>`;
}

function modalConfirm(m) {
  return `<div class="modal-head"><h2>${esc(m.title)}</h2><button class="close" data-action="close" aria-label="Close">✕</button></div>
    <div class="modal-body"><p class="muted">${esc(m.text)}</p>
    <div class="row" style="justify-content:flex-end"><button class="btn ghost" data-action="close">Cancel</button><button class="btn ${m.danger ? "danger" : "primary"}" data-action="confirm-yes">${esc(m.yes)}</button></div></div>`;
}

function modalCelebrate(m) {
  return `<div class="celebrate">
    <div class="big">${esc(m.headline)}</div>
    <h2>${esc(m.title)}</h2>
    <p class="muted">${esc(m.text)}</p>
    ${m.badges?.length ? `<div class="chips" style="justify-content:center">${m.badges.map((b) => `<span class="pill accent">🏅 ${esc(b)}</span>`).join("")}</div>` : ""}
    <div style="width:min(360px,100%)">${xpBar(S.profile.level)}</div>
    <div class="row">${m.lab ? `<button class="btn" data-action="destroy-after">Shut lab down</button>` : ""}<button class="btn primary" data-action="close">${m.lab ? "Keep exploring" : "Nice!"}</button></div>
  </div>`;
}

function confetti(el) {
  if (!el || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  const colors = ["#7c5cff", "#18d6a3", "#ffb547", "#ff5c7a", "#4f8cff"];
  for (let i = 0; i < 60; i++) {
    const c = document.createElement("span");
    c.className = "confetti";
    c.style.left = `${Math.random() * 100}%`;
    c.style.background = colors[i % colors.length];
    c.style.animationDelay = `${Math.random() * 0.6}s`;
    el.append(c);
  }
}

// ------------------------------------------------------------- actions ----

async function act(fn) {
  try {
    return await fn();
  } catch (e) {
    toast(String(e), "bad");
  }
}

async function refreshAll() {
  const [profile, catalogue, active] = await Promise.all([call("get_profile"), call("list_labs"), call("get_active")]);
  S.profile = profile;
  setCatalogue(catalogue);
  S.active = active;
}

function confirmThen(title, text, yes, danger, fn) {
  S.modal = { type: "confirm", title, text, yes, danger, fn };
  renderModal();
}

async function onClick(e) {
  const nav = e.target.closest("[data-view]");
  if (nav) {
    S.modal = null;
    return go(nav.dataset.view);
  }
  const el = e.target.closest("[data-action]");
  if (!el) return;
  const { action, id } = el.dataset;
  if (action === "backdrop" && e.target !== el) return;

  switch (action) {
    case "backdrop":
    case "close":
      return closeModal();
    case "skill":
      S.onboardSkill = el.dataset.skill;
      $$(".skill").forEach((b) => b.classList.toggle("on", b === el));
      return;
    case "track":
      S.filter.track = el.dataset.track;
      return go("labs");
    case "filter-track":
      S.filter.track = el.dataset.track;
      return render();
    case "filter-status":
      S.filter.status = el.dataset.status;
      return render();
    case "open-lab":
      S.modal = { type: "lab", id };
      return renderModal();
    case "pick-diff":
      S.pickedDiff[id] = el.dataset.d;
      return renderModal();
    case "goto-mission":
      S.modal = null;
      return go("mission");
    case "start":
      return act(async () => {
        S.steps = {};
        S.hostStates = [];
        S.active = await call("start_lab", { labId: id, difficulty: S.pickedDiff[id] });
        S.modal = null;
        go("mission");
      });
    case "stop":
      return act(async () => {
        el.disabled = true;
        S.active = await call("stop_lab");
        toast("Lab paused. Your progress is kept.");
        render();
      });
    case "resume":
      return act(async () => {
        S.steps = {};
        S.active = await call("resume_lab");
        render();
      });
    case "reset":
      return confirmThen("Reset this lab?", "Hosts are rebuilt from scratch with new flags. XP you already earned is kept.", "Reset lab", false, async () => {
        S.steps = {};
        S.active = await call("reset_lab");
        render();
      });
    case "destroy":
    case "destroy-after":
      return confirmThen("Destroy this lab?", "Containers and networks are removed to free resources. XP and badges are kept.", "Destroy", true, async () => {
        await call("destroy_lab");
        S.active = null;
        S.steps = {};
        S.hostStates = [];
        toast("Lab destroyed. Resources freed.");
        go("home");
      });
    case "confirm-yes": {
      const fn = S.modal.fn;
      S.modal = null;
      renderModal();
      return act(fn);
    }
    case "hint":
      return act(async () => {
        S.active = await call("reveal_hint");
        render();
      });
    case "map":
      return act(async () => {
        S.modal = { type: "map", map: await call("get_map", { reveal: null }) };
        renderModal();
      });
    case "map-reveal":
      return act(async () => {
        S.modal = { type: "map", map: await call("get_map", { reveal: Number(el.dataset.level) }) };
        S.active = await call("get_active");
        render();
      });
    case "copy":
      await navigator.clipboard?.writeText(el.dataset.copy);
      return toast("Copied to clipboard");
    case "check-labs":
      return checkLabUpdates(true);
    case "install-lab":
      return installLabs([id]);
    case "install-all-labs":
      return installLabs((S.labOffers ?? []).filter((o) => !o.locked && !o.blocked).map((o) => o.id));
    case "check-update":
      return checkUpdate(true);
    case "install-update":
      return installUpdate();
    case "preflight":
      return runPreflight();
  }
}

async function onSubmit(e) {
  e.preventDefault();
  const form = e.target;
  if (form.id === "onboard-form") {
    return act(async () => {
      S.profile = await call("create_profile", { username: form.username.value, skill: S.onboardSkill });
      $(".onboard").remove();
      $(".shell").hidden = false;
      go("home");
      toast(`Welcome, ${S.profile.username}!`, "ok");
    });
  }
  if (form.id === "flag-form") {
    const value = form.flag.value.trim();
    if (!value) return;
    return act(async () => {
      const r = await call("submit_flag", { value });
      if (!r.correct) {
        form.flag.select();
        return toast("That's not a flag for this lab. Keep digging.", "bad");
      }
      await refreshAll();
      if (r.already_captured) return toast(`${r.flag_label} was already captured.`);
      if (r.lab_complete) {
        S.modal = {
          type: "celebrate",
          lab: true,
          headline: "PWNED",
          title: `${lab(S.active.lab_id).name} · ${cap(S.active.difficulty)}`,
          text: r.xp_awarded ? `+${r.xp_awarded} XP${r.level_up ? ` · Level up! You're now level ${r.level_up.level}, ${r.level_up.title}.` : ""}` : "Already pwned at this difficulty, so no new XP this time.",
          badges: r.new_badges,
        };
      } else if (r.level_up) {
        S.modal = { type: "celebrate", headline: `LV ${r.level_up.level}`, title: r.level_up.title, text: `${r.flag_label} captured for +${r.xp_awarded} XP.` };
      } else {
        toast(`🚩 ${r.flag_label} captured! ${r.xp_awarded ? `+${r.xp_awarded} XP` : "(no new XP on replays)"}`, "ok");
      }
      render();
    });
  }
}

boot().catch((e) => {
  document.body.innerHTML = `<div class="onboard"><div class="error-box">LabForge failed to start: ${esc(e)}</div></div>`;
});
