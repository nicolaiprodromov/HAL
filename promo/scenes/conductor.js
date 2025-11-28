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
  id: 'conductor',
  draw(g, t, ctx) {
    const { C } = ctx;
    const { range, ease } = HAL;
    const col = C.cond;
    const P = agentFrame(g, t, ctx, {
      key: 'conductor', flip: false, title: 'container · stdout', mode: 'random', sub: 'deterministic · no LLM', blinkAt: 1.2,
      hudBytes: t => `${Math.floor(Math.max(0, t - 0.5) * 160)} B stdout`, hudR: 'bollard · --init · /work',
    });
    if (P.pk < 0.5) return;
    const x = P.x + 48, y = P.y + 70, M = C.muted, S = C.soft, X = C.text;
    // command
    const cmd = [['$ ', M], ['docker run ', col, { weight: 700 }], ['--init -v artifacts:/work debian:stable', X]];
    const cn = Math.min(runLen(cmd), Math.floor((t - 0.2) * 150));
    if (cn > 0) typedRuns(g, cmd, cn, x, y, { size: 22, weight: 400 });
    // container status row
    const sk = range(t, 0.5, 0.65);
    const done = t > 1.45;
    const cx = x + 36, cy = y + 90;
    if (sk > 0) {
      // spinning nested squares = container box spinning up
      g.save(); g.translate(cx, cy);
      g.strokeStyle = col; g.lineWidth = 2;
      for (let i = 0; i < 3; i++) {
        g.save(); g.rotate((t * (done ? 0 : 5) + i * 0.5) * (i % 2 ? -1 : 1) + (done ? i * 0.0 : 0));
        g.globalAlpha = (1 - i * 0.25) * sk;
        const s = 52 - i * 15; g.strokeRect(-s / 2, -s / 2, s, s); g.restore();
      }
      g.fillStyle = done ? '#fff7d6' : col; g.fillRect(-5, -5, 10, 10);
      g.restore();
      const st = t < 0.7 ? 'created' : (done ? 'exited (0)' : 'running');
      HAL.runs(g, [['c0ffee7a91b2', X, { weight: 700 }], ['   ', M], [st, done ? C.ok : col, { weight: 700 }], ['   debian:stable', M]], x + 90, cy + 8, { size: 22, alpha: sk });
    }
    // stdout window
    const seq = [];
    for (let n = 1; n <= 30; n++) seq.push(n % 15 === 0 ? 'FizzBuzz' : n % 3 === 0 ? 'Fizz' : n % 5 === 0 ? 'Buzz' : String(n));
    const rows = 8, lh = 40, wy = y + 215;
    g.fillStyle = C.line; g.fillRect(x, wy - 38, P.w - 96, 1);
    HAL.text(g, 'stdout', x, wy - 48, { size: 15, color: C.muted, spacing: 3 });
    const prog = Math.max(0, (t - 0.55) / 0.85);
    const n = Math.min(30, Math.floor(Math.min(1, prog) * 30.99));
    const first = Math.max(0, n - rows);
    for (let i = first; i < n; i++) {
      const r = i - first, age = n - i;
      const word = seq[i], hot = /[A-Z]/.test(word);
      const a = i === n - 1 ? 1 : 0.55 + 0.45 * (1 - (n - 1 - i) / rows);
      HAL.text(g, String(i + 1).padStart(2, '0'), x, wy + r * lh + 30, { size: 17, color: C.line2, alpha: a });
      HAL.text(g, word, x + 52, wy + r * lh + 30, { size: 28, color: hot ? col : (age < 3 ? X : M), weight: hot ? 700 : 400, alpha: a });
    }
    if (n < 30 && n > 0) HAL.cursor(g, x + 52 + HAL.textWidth(seq[n - 1], 28) + 10, wy + (n - first - 1) * lh + 30, 28, 0, col, 1);
    // exit line
    const ek = range(t, 1.45, 1.55);
    if (ek > 0) {
      g.fillStyle = 'rgba(14,17,20,0.96)'; g.fillRect(P.x + 24, P.y + P.h - 100, P.w - 48, 64);
      HAL.runs(g, [['exit 0', C.ok, { weight: 800 }], ['  ·  ', M], ['0.41s', col, { weight: 700 }], ['  ·  ', M], ['30 lines', S], ['  ·  ', M], ['--rm', M]], P.x + P.w / 2, P.y + P.h - 56, { size: 26, align: 'center', alpha: ek });
    }
    // side column: scrolling hex to the right of the list
    for (let i = 0; i < 9; i++) {
      HAL.text(g, Math.floor(HAL.hash(i, Math.floor(t * 20)) * 4294967295).toString(16).padStart(8, '0'), P.x + P.w - 56, wy + i * lh + 30, { size: 16, color: C.line2, align: 'right', alpha: 0.7 });
    }
  },
});
})();
