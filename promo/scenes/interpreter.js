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
  id: 'interpreter',
  draw(g, t, ctx) {
    const { C } = ctx;
    const col = C.interp;
    const P = agentFrame(g, t, ctx, {
      key: 'interpreter', flip: false, title: 'logic.md', mode: 'rows', sub: 'fizzbuzz.hal → logic.md',
      hudBytes: t => `${Math.floor(t * 1900)} B read`, hudR: 'memory · 2 notes recalled',
    });
    if (P.pk < 0.5) return;
    const x = P.x + 48, y = P.y + 78, lh = 50, size = 30;
    const K = C.key, M = C.muted, S = C.soft, X = C.text;
    const lines = [
      [['◆ ', col], ['memory.search ', M], ['"fizz buzz"', S], ['  → 2 notes', col]],
      null,
      [['for ', col, { weight: 700 }], ['n ', X], ['in ', col, { weight: 700 }], ['1..30', X]],
      [['  n % 15 == 0 ', X], ['→ ', M], ['FizzBuzz', col, { weight: 700 }]],
      [['  n % 3  == 0 ', X], ['→ ', M], ['Fizz', col, { weight: 700 }]],
      [['  n % 5  == 0 ', X], ['→ ', M], ['Buzz', col, { weight: 700 }]],
      [['  else        ', X], ['→ ', M], ['n', X]],
      [['exit ', col, { weight: 700 }], ['0', X]],
      null,
      [['✓ ', col, { weight: 700 }], ['write_file logic.md', S], ['  214 B', M]],
    ];
    stream(g, lines, x, y, lh, size, t, 0.2, 0.085, 150);
    // line numbers gutter
    for (let i = 0; i < 10; i++) if (t > 0.2 + i * 0.085) HAL.text(g, String(i + 1).padStart(2, '0'), P.x + P.w - 48, y + i * lh, { size: 16, color: C.line2, align: 'right' });
    // checksum bar at pane bottom
    const bk = HAL.range(t, 1.2, 1.5);
    g.fillStyle = HAL.rgba(col, 0.5); g.fillRect(P.x + 48, P.y + P.h - 56, Math.round((P.w - 96) * HAL.ease.outExpo(bk)), 3);
    HAL.text(g, 'logic.md', P.x + 48, P.y + P.h - 70, { size: 16, color: C.muted, spacing: 2, alpha: bk });
  },
});
})();
