(() => {
// Agent title card — shared template (duplicated in the four agent scenes so they rhyme).
// Hero logo slams in left/right, spaced-caps name + role tag + step ticks, and an output pane
// where the agent's real output streams in. Everything is a pure function of t.
const KEYS = ['interpreter', 'architect', 'conductor', 'reflector'];
const LOGO_W = 612, LOGO_H = 360, PANE_W = 940, PANE_Y = 170, PANE_H = 700;

// type a line made of runs [[str, color, opts], ...]: reveals `n` characters
function typedRuns(g, parts, n, x, y, o) {
  let left = n, cx = x;
  for (const p of parts) {
    if (left <= 0) break;
    const s = [...p[0]].slice(0, left).join('');
    left -= [...p[0]].length;
    HAL.text(g, s, cx, y, Object.assign({}, o, { color: p[1] }, p[2] || {}));
    cx += HAL.textWidth(p[0], o.size, o.spacing || 0);
  }
  return cx;
}
const runLen = parts => parts.reduce((a, p) => a + [...p[0]].length, 0);

// stream lines; returns {cx, cy, n} of the last (cursor) position. lines[i] = runs or null (blank)
function stream(g, lines, x, y, lh, size, t, t0, dt, cps) {
  let cur = null;
  lines.forEach((parts, i) => {
    const ts = t - (t0 + i * dt);
    if (ts < 0 || !parts) return;
    const len = runLen(parts), n = Math.min(len, Math.floor(ts * cps) + 1);
    const cx = typedRuns(g, parts, n, x, y + i * lh, { size, weight: 400 });
    cur = { x: cx, y: y + i * lh, done: n >= len };
  });
  if (cur) HAL.cursor(g, cur.x + 3, cur.y, size, 0, 'rgba(238,240,243,0.9)', 1);
  return cur;
}

function agentFrame(g, t, ctx, cfg) {
  const { C, W, H, fx, T } = ctx;
  const { ease, range, hash, rgba } = HAL;
  const A = HAL.AGENTS[cfg.key], col = A.color, idx = KEYS.indexOf(cfg.key);
  fx.bloom = cfg.bloom || 0.5; fx.exposure = 0.92;

  // kick on frame 1
  fx.flash = 0.22 * Math.max(0, 1 - t / 0.07);
  if (t < 0.12) { fx.glitch = 0.5 * (1 - t / 0.12); fx.aberration = 1 + 3 * (1 - t / 0.12); }
  fx.shake = [Math.sin(t * 90) * 6 * Math.max(0, 1 - t / 0.14), 0];

  // camera: slow push + drift
  const p = t / ctx.dur;
  HAL.zoom(g, 1.0 + 0.032 * ease.inOutQuad(p), W / 2, H / 2);
  g.translate((cfg.flip ? 1 : -1) * 14 * p, -8 * p);

  // ---- background depth ----
  // faint grid, scrolling
  g.fillStyle = 'rgba(190,200,215,0.035)';
  const gs = 60, go = (t * 24) % gs;
  for (let x = -go; x < W + gs; x += gs) g.fillRect(Math.round(x), 0, 1, H);
  for (let y = -go; y < H + gs; y += gs) g.fillRect(0, Math.round(y), W, 1);
  // giant outline step number
  g.save();
  g.font = `800 940px ${HAL.FONT}`;
  g.textBaseline = 'alphabetic';
  g.lineWidth = 2;
  g.strokeStyle = rgba(col, 0.12 + 0.03 * Math.sin(t * 5));
  const num = '0' + (idx + 1);
  const gx = (cfg.flip ? 1000 : 880) - 20 * p * (cfg.flip ? -1 : 1);
  g.strokeText(num, gx - 8 - ctx.W * 0.0 + (cfg.flip ? 60 : -60) * 0, 900 - 30 * p);
  g.restore();
  // drifting particles
  for (let i = 0; i < 34; i++) {
    const sp = 20 + hash(i, 1) * 60, x = hash(i, 2) * W;
    const y = H + 20 - ((hash(i, 3) * H + t * sp * (1 + hash(i, 4))) % (H + 40));
    const a = 0.12 + 0.3 * hash(i, 5), s = hash(i, 6) < 0.3 ? 4 : 2;
    g.fillStyle = rgba(col, a * (0.6 + 0.4 * Math.sin(t * 6 + i)));
    g.fillRect(Math.round(x + Math.sin(t * 2 + i) * 10), Math.round(y), s, s);
  }

  // ---- layout ----
  const logoX = cfg.flip ? 1800 - LOGO_W : 120;
  const paneX = cfg.flip ? 120 : 1800 - PANE_W;
  const logoY = 200;
  const cxL = logoX + LOGO_W / 2;

  // ---- hero logo ----
  const k = range(t, 0, 0.3);
  const pop = 1 + 0.12 * (1 - ease.outBack(range(t, 0, 0.34)));
  const blinkT = cfg.blinkAt === undefined ? 0.95 : cfg.blinkAt;
  const blink = Math.max(0, 1 - Math.abs(t - blinkT - 0.05) / 0.05);
  HAL.glow(g, cxL, logoY + LOGO_H / 2, 520, col, 0.10 * ease.outCubic(range(t, 0, 0.3)));
  g.save();
  HAL.zoom(g, pop, cxL, logoY + LOGO_H / 2);
  HAL.logo(g, A.logo, logoX, logoY, 36, 60, {
    color: col, reveal: ease.outExpo(k), mode: cfg.mode || 'rows', seed: 11 + idx, T,
    jitter: t < 0.25 ? 14 * (1 - t / 0.25) : 0, blink,
  });
  // scan bar sweeping the logo
  const sw = ((t * 1.1 + 0.15) % 1);
  g.globalCompositeOperation = 'lighter';
  g.fillStyle = 'rgba(255,255,255,0.10)';
  g.fillRect(logoX - 10, logoY + sw * LOGO_H, LOGO_W + 20, 5);
  g.restore();

  // ---- name, tag, step ----
  const nameA = range(t, 0.12, 0.3);
  const nameX = cxL;
  HAL.text(g, HAL.scramble(A.name, range(t, 0.08, 0.36), 5 + idx, T), nameX, 680, { size: 46, color: col, weight: 800, spacing: 12, align: 'center', alpha: Math.min(1, nameA * 3) });
  const tagK = range(t, 0.22, 0.42);
  HAL.text(g, cfg.tag === undefined ? A.tag : cfg.tag, nameX, 738, { size: 30, color: C.text, weight: 500, spacing: 3, align: 'center', alpha: tagK });
  if (cfg.sub) HAL.text(g, cfg.sub, nameX, 782, { size: 20, color: C.muted, spacing: 2, align: 'center', alpha: tagK });
  // step index + ticks
  const sy = cfg.sub ? 850 : 810;
  HAL.text(g, `0${idx + 1} / 04`, nameX, sy, { size: 24, color: col, weight: 700, spacing: 6, align: 'center', alpha: range(t, 0.2, 0.4) });
  for (let i = 0; i < 4; i++) {
    const tx = nameX - 4 * 56 / 2 + i * 56 + 4, cur = i === idx;
    const w = Math.round(48 * ease.outCubic(range(t, 0.25 + i * 0.04, 0.45 + i * 0.04)));
    g.fillStyle = cur ? HAL.AGENTS[KEYS[i]].color : (i < idx ? HAL.rgba(HAL.AGENTS[KEYS[i]].color, 0.45) : C.line2);
    g.fillRect(tx, sy + 22, w, cur ? 6 : 4);
  }

  // ---- output pane ----
  const pk = ease.outExpo(range(t, 0.05, 0.35));
  const ph = Math.round(PANE_H * pk);
  g.save();
  const py = PANE_Y + (PANE_H - ph) / 2;
  g.fillStyle = 'rgba(14,17,20,0.82)';
  g.fillRect(paneX, py, PANE_W, ph);
  HAL.panel(g, paneX, py, PANE_W, ph, { color: rgba(col, 0.55), title: cfg.title, titleColor: col, radius: 4, lw: 2, alpha: Math.min(1, pk * 2) });
  g.restore();
  // corner ticks
  g.fillStyle = col;
  [[paneX - 6, PANE_Y - 6], [paneX + PANE_W - 6, PANE_Y - 6], [paneX - 6, PANE_Y + PANE_H - 6], [paneX + PANE_W - 6, PANE_Y + PANE_H - 6]].forEach(([x, y]) => g.fillRect(x, y, 12, 12));

  // ---- HUD ----
  const hy = 112;
  const hudL = cfg.flip ? 1800 : 120, hudA = cfg.flip ? 'right' : 'left';
  HAL.text(g, '$ hal run fizzbuzz.hal', 120, hy, { size: 20, color: C.muted, weight: 500 });
  const ms = Math.floor((ctx.T - ctx.T + t) * 1000);
  HAL.text(g, `T+${(4.0 + idx * 1.6 + t).toFixed(3)}s`, 1800, hy, { size: 20, color: C.muted, align: 'right', weight: 500 });
  g.fillStyle = C.line; g.fillRect(120, hy + 18, 1680, 1);
  g.fillStyle = col; g.fillRect(120, hy + 18, Math.round(1680 * ((idx + p) / 4)), 2);
  const hex = i => Math.floor(hash(i, Math.floor(t * 30), idx) * 65536).toString(16).padStart(4, '0');
  HAL.text(g, `0x${hex(1)}${hex(2)}  ${cfg.hudBytes ? cfg.hudBytes(t) : ''}`, 120, 1000, { size: 18, color: C.line2, weight: 500 });
  HAL.text(g, cfg.hudR || '', 1800, 1000, { size: 18, color: C.line2, align: 'right', weight: 500 });

  return { x: paneX, y: PANE_Y, w: PANE_W, h: PANE_H, pk, col, cxL };
}

HAL.scene({
  id: 'reflector',
  draw(g, t, ctx) {
    const { C } = ctx;
    const { range, ease, hash } = HAL;
    const col = C.refl;
    const P = agentFrame(g, t, ctx, {
      key: 'reflector', flip: true, title: 'reflector · memory', mode: 'random', sub: 'read-only · run_cmd probes', blinkAt: 1.15,
      hudBytes: t => `${Math.floor(t * 410)} B observed`, hudR: 'memory/hal.sqlite',
    });
    if (P.pk < 0.5) return;
    const x = P.x + 48, y = P.y + 74, lh = 44, size = 26;
    const M = C.muted, S = C.soft, X = C.text;
    const lines = [
      [['observe ', col, { weight: 700 }], ['runs/20260318T101407Z/stdout.log', X]],
      [['  30 lines · exit 0 · stderr empty', M]],
      [['remember ', col, { weight: 700 }], ['reflector', S]],
      [['  "fizzbuzz.sh prints 30 lines, exit 0"', X]],
      [['fix: ', col, { weight: 700 }], ['none needed', S]],
    ];
    stream(g, lines, x, y, lh, size, t, 0.2, 0.17, 150);
    // memory field
    const fx0 = P.x + 48, fy0 = P.y + 340, fw = P.w - 96, fh = 240;
    const fk = range(t, 0.2, 0.5);
    g.save(); g.globalAlpha = fk;
    g.strokeStyle = C.line2; g.lineWidth = 1; g.setLineDash([6, 6]);
    g.strokeRect(fx0 + .5, fy0 + .5, fw, fh);
    g.setLineDash([]);
    HAL.text(g, 'memory/hal.sqlite', fx0 + 14, fy0 - 14, { size: 17, color: M, spacing: 2 });
    HAL.text(g, 'embedding-beta-3-large · 4096d', fx0 + fw, fy0 - 14, { size: 15, color: C.line2, align: 'right', spacing: 1 });
    g.restore();
    // existing memories drift slowly
    const pos = (i, t0 = 0) => [fx0 + 22 + hash(i, 1) * (fw - 44) + Math.sin(t * 1.3 + i * 2) * 6, fy0 + 22 + hash(i, 2) * (fh - 44) + Math.cos(t * 1.1 + i * 3) * 5];
    const N = 70;
    for (let i = 0; i < N; i++) {
      const [px, py] = pos(i);
      const ap = range(t, 0.25 + hash(i, 3) * 0.3, 0.45 + hash(i, 3) * 0.3);
      g.fillStyle = HAL.rgba(col, 0.5 * ap + (hash(i, 9) < 0.2 ? 0.35 * ap : 0));
      const s = hash(i, 4) < 0.25 ? 6 : 4;
      g.fillRect(Math.round(px), Math.round(py), s, s);
    }
    // three new memories fly in, link to nearest neighbours
    const newDots = [
      { at: 0.6, from: [P.x + 300, y + 3 * lh], to: [fx0 + fw * 0.30, fy0 + fh * 0.42] },
      { at: 0.85, from: [P.x + 300, y + 3 * lh], to: [fx0 + fw * 0.58, fy0 + fh * 0.64] },
      { at: 1.1, from: [P.x + 300, y + 4 * lh], to: [fx0 + fw * 0.76, fy0 + fh * 0.33] },
    ];
    newDots.forEach((d, j) => {
      const k = range(t, d.at, d.at + 0.22);
      if (k <= 0) return;
      const e = ease.outCubic(k);
      const px = HAL.lerp(d.from[0], d.to[0], e), py = HAL.lerp(d.from[1], d.to[1], e) - Math.sin(e * Math.PI) * 60;
      if (k < 1) { HAL.glow(g, px, py, 40, col, 0.5); }
      else {
        // links to 3 nearest
        const near = [];
        for (let i = 0; i < N; i++) { const [qx, qy] = pos(i); near.push([Math.hypot(qx - d.to[0], qy - d.to[1]), qx, qy]); }
        near.sort((a, b) => a[0] - b[0]);
        const lk = ease.outExpo(range(t, d.at + 0.22, d.at + 0.4));
        g.strokeStyle = HAL.rgba(col, 0.5); g.lineWidth = 1.5;
        for (let q = 0; q < 3; q++) { g.beginPath(); g.moveTo(d.to[0], d.to[1]); g.lineTo(HAL.lerp(d.to[0], near[q][1], lk), HAL.lerp(d.to[1], near[q][2], lk)); g.stroke(); }
        const rr = 10 + 60 * range(t, d.at + 0.22, d.at + 0.6);
        g.strokeStyle = HAL.rgba(col, 0.6 * (1 - range(t, d.at + 0.22, d.at + 0.6))); g.lineWidth = 2;
        g.beginPath(); g.arc(d.to[0], d.to[1], rr, 0, 7); g.stroke();
      }
      g.fillStyle = '#e9fff3'; g.fillRect(Math.round(px) - 6, Math.round(py) - 6, 12, 12);
      if (k >= 1) { g.fillStyle = col; g.fillRect(Math.round(px) - 4, Math.round(py) - 4, 8, 8); }
    });
    // query sweep across the field
    const sx = fx0 + ((t * 0.9) % 1) * fw;
    const sg = g.createLinearGradient(sx - 60, 0, sx, 0);
    sg.addColorStop(0, HAL.rgba(col, 0)); sg.addColorStop(1, HAL.rgba(col, 0.18 * fk));
    g.fillStyle = sg; g.fillRect(sx - 60, fy0 + 1, 60, fh - 1);
    g.fillStyle = HAL.rgba(col, 0.5 * fk); g.fillRect(Math.round(sx), fy0 + 1, 2, fh - 1);
    // result line
    const rk = range(t, 1.25, 1.4);
    if (rk > 0) {
      const cnt = Math.min(3, 1 + Math.floor((t - 1.25) * 12));
      HAL.runs(g, [['+3 memories', col, { weight: 800 }], ['  →  ', M], ['next build', X, { weight: 700 }]], P.x + P.w / 2, P.y + P.h - 42, { size: 28, align: 'center', alpha: rk });
    }
  },
});
})();
