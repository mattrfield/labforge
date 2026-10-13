// Draws the lab map (MapView from the backend) as an SVG string.
// Level 1: hosts and subnet. Level 2: + open ports. Level 3: + relationships.

import { esc } from "./util.js";

const W = 720;
const HOST_X = 430;
const HOST_W = 250;

export function renderMap(map) {
  const portRows = (h) => (map.level >= 2 ? Math.max(1, h.ports.length) : 0);
  const heights = map.hosts.map((h) => 64 + portRows(h) * 22 + (portRows(h) ? 10 : 0));
  const gap = 24;
  const total = heights.reduce((a, b) => a + b, 0) + gap * (map.hosts.length - 1);
  const H = Math.max(260, total + 80);
  let y = (H - total) / 2;
  const hosts = map.hosts.map((h, i) => {
    const box = { ...h, x: HOST_X, y, w: HOST_W, h: heights[i] };
    y += heights[i] + gap;
    return box;
  });
  const midY = H / 2;
  const sw = { x: 250, y: midY };

  const links = hosts
    .map((h) => {
      const hy = h.y + 32;
      return `<path d="M${sw.x + 34} ${sw.y} C ${sw.x + 110} ${sw.y}, ${HOST_X - 80} ${hy}, ${HOST_X} ${hy}" fill="none" stroke="var(--line)" stroke-width="2"/>`;
    })
    .join("");

  const edges = (map.edges || [])
    .map((e, i) => {
      const a = hosts.find((h) => h.name === e.from);
      const b = hosts.find((h) => h.name === e.to);
      if (!a || !b) return "";
      const x = HOST_X + HOST_W;
      const ay = a.y + 32;
      const by = b.y + 32;
      const bend = x + 40 + i * 14;
      return `<path d="M${x} ${ay} C ${bend} ${ay}, ${bend} ${by}, ${x} ${by}" fill="none" stroke="var(--track-ad)" stroke-width="2" stroke-dasharray="6 5" marker-end="url(#map-arrow)"/>
        <text x="${bend - 4}" y="${(ay + by) / 2 + 4}" font-size="12" fill="var(--track-ad)">${esc(e.label)}</text>`;
    })
    .join("");

  const hostBoxes = hosts
    .map((h) => {
      const ports =
        map.level >= 2
          ? (h.ports.length ? h.ports : [{ port: "—", proto: "", service: "no open ports listed" }])
              .map(
                (p, i) =>
                  `<text x="${h.x + 18}" y="${h.y + 76 + i * 22}" font-size="13" fill="var(--muted)" font-family="ui-monospace, monospace">${esc(String(p.port))}${p.proto ? "/" + esc(p.proto) : ""}</text>
                   <text x="${h.x + 92}" y="${h.y + 76 + i * 22}" font-size="13" fill="var(--text)">${esc(p.service)}</text>`,
              )
              .join("")
          : "";
      const divider = map.level >= 2 ? `<line x1="${h.x + 14}" x2="${h.x + h.w - 14}" y1="${h.y + 56}" y2="${h.y + 56}" stroke="var(--line)"/>` : "";
      return `<g>
        <rect x="${h.x}" y="${h.y}" width="${h.w}" height="${h.h}" rx="12" fill="var(--panel)" stroke="var(--line)" stroke-width="1.5"/>
        <rect x="${h.x}" y="${h.y}" width="5" height="${h.h}" rx="2" fill="var(--accent-2)"/>
        <text x="${h.x + 18}" y="${h.y + 26}" font-size="15" font-weight="650" fill="var(--text)">${esc(h.name)}</text>
        <text x="${h.x + h.w - 14}" y="${h.y + 26}" font-size="13" text-anchor="end" fill="var(--accent-2)" font-family="ui-monospace, monospace">${esc(h.ip)}</text>
        <text x="${h.x + 18}" y="${h.y + 45}" font-size="12.5" fill="var(--faint)">${esc(h.os)}</text>
        ${divider}${ports}
      </g>`;
    })
    .join("");

  const width = edges ? W + 60 : HOST_X + HOST_W + 20;
  return `<svg viewBox="0 0 ${width} ${H}" role="img" aria-label="Lab network map">
    <defs><marker id="map-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="var(--track-ad)"/></marker></defs>
    <path d="M150 ${midY} H ${sw.x - 34}" stroke="var(--accent)" stroke-width="2" stroke-dasharray="5 5" fill="none"/>
    <text x="${(150 + sw.x - 34) / 2}" y="${midY - 10}" font-size="12" text-anchor="middle" fill="var(--faint)">route</text>
    ${links}
    <g>
      <rect x="20" y="${midY - 36}" width="130" height="72" rx="14" fill="rgba(124,92,255,0.14)" stroke="var(--accent)" stroke-width="1.5"/>
      <text x="85" y="${midY - 6}" font-size="15" font-weight="650" text-anchor="middle" fill="var(--text)">You</text>
      <text x="85" y="${midY + 16}" font-size="12" text-anchor="middle" fill="var(--muted)" font-family="ui-monospace, monospace">${esc(map.gateway)}</text>
    </g>
    <g>
      <circle cx="${sw.x}" cy="${sw.y}" r="34" fill="var(--panel-2)" stroke="var(--line)" stroke-width="1.5"/>
      <path d="M${sw.x - 14} ${sw.y - 6}h28M${sw.x - 14} ${sw.y + 6}h28M${sw.x + 8} ${sw.y - 12}l6 6-6 6M${sw.x - 8} ${sw.y}l-6 6 6 6" stroke="var(--muted)" stroke-width="2" fill="none" stroke-linecap="round"/>
      <text x="${sw.x}" y="${sw.y + 56}" font-size="12.5" text-anchor="middle" fill="var(--muted)" font-family="ui-monospace, monospace">${esc(map.subnet)}</text>
    </g>
    ${hostBoxes}
    ${edges}
  </svg>`;
}
