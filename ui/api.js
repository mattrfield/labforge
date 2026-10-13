// Talks to the Rust backend through Tauri. Opened in a plain browser (no Tauri),
// it falls back to an in-memory mock so the UI can be previewed and styled.

const T = window.__TAURI__;
export const isTauri = Boolean(T);

export function call(cmd, args = {}) {
  return isTauri ? T.core.invoke(cmd, args) : mock.call(cmd, args);
}

export function on(event, cb) {
  return isTauri ? T.event.listen(event, (e) => cb(e.payload)) : mock.on(event, cb);
}

// ---------------------------------------------------------------- mock ----

const mock = (() => {
  const listeners = {};
  const emit = (ev, payload) => (listeners[ev] || []).forEach((cb) => cb(payload));
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const levelInfo = (xp) => {
    const need = (l) => 50 * l * (l - 1);
    let level = 1;
    while (xp >= need(level + 1)) level++;
    const titles = [[2, "Script Kiddie"], [4, "Recon Rookie"], [7, "Operator"], [10, "Pentester"], [14, "Red Teamer"], [Infinity, "Legend"]];
    return { level, title: titles.find(([max]) => level <= max)[1], xp, level_floor: need(level), next_level_xp: need(level + 1) };
  };

  const labs = [
    {
      id: "bean-there", name: "Bean There", track: "web", track_name: "Web",
      blurb: "A neighbourhood coffee shop just launched online ordering. Their staff portal looks... hand-rolled.",
      story: "Bean There's owner built the shop's website over a weekend. Get into the staff portal, land a shell on the web server, and take root.",
      skills: ["sql-injection", "command-injection", "suid-privesc", "enumeration"], xp: 100, est_minutes: 45,
      difficulties: ["easy", "medium"],
      difficulty_notes: { easy: "Verbose errors and no input filtering. A classic first box.", medium: "A homemade filter guards the inputs, errors are quiet, and the obvious privesc is gone." },
      resources: { ram_mb: 256, cpus: 0.5, disk_mb: 250 }, subnet: "10.66.11.0/24",
      hosts: [{ name: "web01", os: "Debian Linux", ip: "10.66.11.10" }],
      flags: [{ id: "user", label: "User flag", host: "web01" }, { id: "root", label: "Root flag", host: "web01" }],
      hint_count: 3, pwned: [],
    },
  ];
  const hints = [["Preview hint one.", 0.1], ["Preview hint two.", 0.1], ["Preview hint three.", 0.15]];
  const mapCost = [0, 0, 0.1, 0.15];
  let profile = null;
  let active = null;

  const activeView = () => active && {
    ...active,
    hints: hints.slice(0, active.hints_revealed).map((h) => h[0]),
    next_hint_cost: hints[active.hints_revealed]?.[1] ?? null,
    max_map_level: 2,
    penalty: Math.min(0.5, hints.slice(0, active.hints_revealed).reduce((s, h) => s + h[1], 0) + mapCost.slice(0, active.map_level + 1).reduce((a, b) => a + b, 0)),
  };
  const profileView = () => profile && { ...profile, level: levelInfo(profile.xp) };

  async function runStart(steps) {
    for (const [step, label, ms] of steps) {
      emit("lab-progress", { lab_id: active.lab_id, step, state: "running", message: label });
      await wait(ms);
      emit("lab-progress", { lab_id: active.lab_id, step, state: "done", message: label });
    }
    active.phase = "ready";
    emit("lab-ready", active.lab_id);
  }

  const handlers = {
    check_lab_updates: () => (location.search.includes("labs") ? [{ id: "tidewater", name: "Tidewater", version: "1.0.0", installed_version: null, tier: "free", size: 48200, blurb: "A harbour-master portal with a leaky API.", locked: false, blocked: null }, { id: "bean-there", name: "Bean There", version: "1.1.0", installed_version: "1.0.0", tier: "free", size: 12100, blurb: "", locked: false, blocked: null }, { id: "deep-vault", name: "Deep Vault", version: "1.0.0", installed_version: null, tier: "pro", size: 90000, blurb: "", locked: true, blocked: null }] : []),
    install_labs: ({ ids }) => ({ installed: ids, failed: [] }),
    app_info: () => ({ version: "0.1.0" }),
    check_update: () => (location.search.includes("update") ? { version: "0.2.0", notes: null } : null),
    install_update: () => { throw new Error("Updates only work in the desktop app."); },
    get_profile: () => profileView(),
    create_profile: ({ username, skill }) => {
      profile = { username, skill, xp: 0, badges: [], pwned: [] };
      return profileView();
    },
    run_preflight: async () => {
      await wait(400);
      return {
        ready: true,
        host: { os: "linux", ram_total_mb: 16384, ram_available_mb: 9800, cpus: 8, disk_free_mb: 120000 },
        checks: [
          { id: "docker-cli", label: "Docker installed", status: "ok", detail: "Docker 28.1.1 (preview data)", fix: null },
          { id: "docker-daemon", label: "Docker engine running", status: "ok", detail: "The engine is reachable.", fix: null },
          { id: "docker-compose", label: "Docker Compose v2", status: "ok", detail: "Compose 2.35.1", fix: null },
          { id: "ram", label: "Memory", status: "ok", detail: "16.0 GB total, 9.6 GB free", fix: null },
          { id: "cpu", label: "CPU cores", status: "ok", detail: "8 cores", fix: null },
          { id: "disk", label: "Free disk space", status: "ok", detail: "117.2 GB free", fix: null },
        ],
      };
    },
    list_labs: () => ({
      labs: labs.map((l) => ({ ...l, pwned: l.difficulties.filter((d) => profile?.pwned.includes(`${l.id}:${d}`)) })),
      errors: [],
      labs_dir: "(browser preview)",
    }),
    get_active: () => activeView(),
    start_lab: ({ labId, difficulty }) => {
      if (active) throw new Error("Another lab is still active.");
      active = { lab_id: labId, difficulty, phase: "starting", error: null, started_at: Math.floor(Date.now() / 1000), captured: [], hints_revealed: 0, map_level: 1 };
      runStart([["build", "Building lab images", 1600], ["up", "Creating the isolated network and starting hosts", 900], ["health", "Waiting for services to come up", 1200], ["flags", "Planting flags", 500]]);
      return activeView();
    },
    stop_lab: () => { active.phase = "stopped"; return activeView(); },
    resume_lab: () => {
      active.phase = "starting";
      runStart([["up", "Starting hosts", 700], ["health", "Waiting for services to come up", 900]]);
      return activeView();
    },
    reset_lab: () => {
      Object.assign(active, { phase: "starting", captured: [], hints_revealed: 0, map_level: 1 });
      runStart([["build", "Building lab images", 600], ["up", "Starting hosts", 600], ["health", "Health checks", 600], ["flags", "Planting flags", 300]]);
      return activeView();
    },
    destroy_lab: () => { active = null; },
    host_status: () => labs.find((l) => l.id === active.lab_id).hosts.map((h) => ({ name: h.name, ip: h.ip, state: active.phase === "ready" ? "up" : "down" })),
    submit_flag: ({ value }) => {
      // Preview flags: LF{user} and LF{root}.
      const id = { "LF{user}": "user", "LF{root}": "root" }[value.trim()];
      const before = levelInfo(profile.xp).level;
      const res = { correct: Boolean(id), flag_id: id ?? null, flag_label: id ? `${id[0].toUpperCase()}${id.slice(1)} flag` : null, already_captured: false, xp_awarded: 0, lab_complete: false, level_up: null, new_badges: [] };
      if (id && active.captured.includes(id)) res.already_captured = true;
      else if (id) {
        active.captured.push(id);
        res.xp_awarded = Math.round(100 * (id === "user" ? 0.4 : 0.6) * (active.difficulty === "medium" ? 1.5 : 1) * (1 - activeView().penalty));
        profile.xp += res.xp_awarded;
        if (active.captured.length === 2) {
          res.lab_complete = true;
          profile.pwned.push(`${active.lab_id}:${active.difficulty}`);
          res.new_badges = ["First Blood", "Web Track Complete"].filter((b) => !profile.badges.includes(b));
          profile.badges.push(...res.new_badges);
        }
      }
      const lvl = levelInfo(profile.xp);
      if (lvl.level > before) res.level_up = lvl;
      res.level = lvl;
      return res;
    },
    reveal_hint: () => { active.hints_revealed++; return activeView(); },
    get_map: ({ reveal }) => {
      if (reveal) active.map_level = Math.max(active.map_level, Math.min(2, reveal));
      const l = labs.find((x) => x.id === active.lab_id);
      return {
        level: active.map_level, max_level: 2, next_level_cost: active.map_level < 2 ? mapCost[active.map_level + 1] : null,
        subnet: l.subnet, gateway: "10.66.11.1",
        hosts: l.hosts.map((h) => ({ ...h, ports: active.map_level >= 2 ? [{ port: 80, proto: "tcp", service: "http (Flask)" }] : [] })),
        edges: [],
      };
    },
  };

  return {
    async call(cmd, args) {
      await wait(60);
      if (!handlers[cmd]) throw `mock: unknown command ${cmd}`;
      try {
        return await handlers[cmd](args);
      } catch (e) {
        throw e.message || String(e);
      }
    },
    on(ev, cb) {
      (listeners[ev] ||= []).push(cb);
      return Promise.resolve(() => {});
    },
  };
})();
