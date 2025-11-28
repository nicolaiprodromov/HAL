// HAL promo — engine core.
// Deterministic: every frame is a pure function of global time T.
// Never use Math.random / Date / performance.now in scenes — use HAL.rng / HAL.hash.
(() => {
  const W = 1920, H = 1080, FPS = 60;

  // ── palette (from docs/header.png + README mermaid) ────────────────
  const C = {
    bg: '#0b0d0f',      // screen background
    bg2: '#12161a',     // raised panel
    bg3: '#181c21',
    line: '#2a2e35',    // hairline borders
    line2: '#3a3f47',
    muted: '#7b828f',   // secondary text
    soft: '#bcc1cc',    // body text
    text: '#eef0f3',    // primary text
    interp: '#5fd0c8',  // INTERPRETER cyan
    arch: '#f287e4',    // ARCHITECT magenta
    cond: '#ecd27f',    // CONDUCTOR amber
    refl: '#84e2b0',    // REFLECTOR mint
    ok: '#84e2b0', warn: '#ecd27f', err: '#ff6b6b', key: '#7cb7ff',
  };

  const AGENTS = {
    interpreter: { name: 'INTERPRETER', color: C.interp, tag: 'what?',      logo: 'interpreter' },
    architect:   { name: 'ARCHITECT',   color: C.arch,   tag: 'how?',       logo: 'architect' },
    conductor:   { name: 'CONDUCTOR',   color: C.cond,   tag: 'docker run', logo: 'conductor' },
    reflector:   { name: 'REFLECTOR',   color: C.refl,   tag: 'observe',    logo: 'reflector' },
  };
  const AGENT_ORDER = ['interpreter', 'architect', 'conductor', 'reflector'];

  // ── math / easing ──────────────────────────────────────────────────
  const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
  const lerp = (a, b, t) => a + (b - a) * t;
  // normalized progress of t through [a, b], clamped to 0..1
  const range = (t, a, b) => clamp((t - a) / (b - a));
  const ease = {
    linear: x => x,
    inQuad: x => x * x,
    outQuad: x => 1 - (1 - x) * (1 - x),
    inOutQuad: x => x < .5 ? 2 * x * x : 1 - Math.pow(-2 * x + 2, 2) / 2,
    inCubic: x => x * x * x,
    outCubic: x => 1 - Math.pow(1 - x, 3),
    inOutCubic: x => x < .5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2,
    outQuart: x => 1 - Math.pow(1 - x, 4),
    outQuint: x => 1 - Math.pow(1 - x, 5),
    inExpo: x => x === 0 ? 0 : Math.pow(2, 10 * x - 10),
    outExpo: x => x === 1 ? 1 : 1 - Math.pow(2, -10 * x),
    inOutExpo: x => x === 0 ? 0 : x === 1 ? 1 : x < .5 ? Math.pow(2, 20 * x - 10) / 2 : (2 - Math.pow(2, -20 * x + 10)) / 2,
    outBack: x => { const c1 = 1.70158, c3 = c1 + 1; return 1 + c3 * Math.pow(x - 1, 3) + c1 * Math.pow(x - 1, 2); },
    outElastic: x => x === 0 ? 0 : x === 1 ? 1 : Math.pow(2, -10 * x) * Math.sin((x * 10 - .75) * (2 * Math.PI) / 3) + 1,
  };

  // ── deterministic randomness ───────────────────────────────────────
  // hash(...ints) -> float in [0,1)
  function hash(...xs) {
    let h = 2166136261 >>> 0;
    for (const x of xs) {
      h ^= Math.floor(x * 1000003) >>> 0;
      h = Math.imul(h, 16777619) >>> 0;
      h ^= h >>> 13; h = Math.imul(h, 0x5bd1e995) >>> 0; h ^= h >>> 15;
    }
    return (h >>> 0) / 4294967296;
  }
  // rng(seed) -> () => float in [0,1)   (mulberry32)
  function rng(seed) {
    let a = (seed * 2654435761) >>> 0;
    return () => {
      a = (a + 0x6D2B79F5) >>> 0;
      let t = a;
      t = Math.imul(t ^ t >>> 15, t | 1);
      t ^= t + Math.imul(t ^ t >>> 7, t | 61);
      return ((t ^ t >>> 14) >>> 0) / 4294967296;
    };
  }
  // smooth 1D value noise in [-1, 1]
  function noise1(x, seed = 0) {
    const i = Math.floor(x), f = x - i, u = f * f * (3 - 2 * f);
    return lerp(hash(i, seed) * 2 - 1, hash(i + 1, seed) * 2 - 1, u);
  }

  // ── colour ─────────────────────────────────────────────────────────
  function hexToRgb(hex) {
    const n = parseInt(hex.slice(1), 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  // rgba(colorHex, alpha)
  function rgba(hex, a = 1) { const [r, g, b] = hexToRgb(hex); return `rgba(${r},${g},${b},${a})`; }
  // mix two hex colours, returns css rgb()
  function mix(h1, h2, t) {
    const a = hexToRgb(h1), b = hexToRgb(h2);
    return `rgb(${Math.round(lerp(a[0], b[0], t))},${Math.round(lerp(a[1], b[1], t))},${Math.round(lerp(a[2], b[2], t))})`;
  }

  // ── text ───────────────────────────────────────────────────────────
  const FONT = '"JBM", monospace';
  const ADV = 0.6; // JetBrains Mono advance width = 0.6em
  // draw text; returns drawn width in px.
  // opts: size(=24) weight(=400) color(=C.text) alpha(=1) align('left'|'center'|'right')
  //       baseline('alphabetic'|'middle'|'top') spacing(px letter spacing)
  function text(g, str, x, y, o = {}) {
    const size = o.size || 24, weight = o.weight || 400, spacing = o.spacing || 0;
    g.save();
    g.font = `${weight} ${size}px ${FONT}`;
    g.letterSpacing = spacing + 'px';
    g.textAlign = 'left';
    g.textBaseline = o.baseline || 'alphabetic';
    g.globalAlpha *= (o.alpha === undefined ? 1 : o.alpha);
    g.fillStyle = o.color || C.text;
    const w = textWidth(str, size, spacing);
    let dx = 0;
    if (o.align === 'center') dx = -w / 2; else if (o.align === 'right') dx = -w;
    g.fillText(str, Math.round(x + dx), Math.round(y));
    g.restore();
    return w;
  }
  function textWidth(str, size = 24, spacing = 0) {
    const n = [...str].length;
    return n * size * ADV + Math.max(0, n) * spacing;
  }
  // draw coloured runs: [[str, color], [str, color, {weight, alpha}], ...]; returns width
  function runs(g, parts, x, y, o = {}) {
    const size = o.size || 24, spacing = o.spacing || 0;
    let total = 0;
    for (const p of parts) total += textWidth(p[0], size, spacing);
    let cx = x;
    if (o.align === 'center') cx -= total / 2; else if (o.align === 'right') cx -= total;
    for (const [s, color, extra] of parts) {
      text(g, s, cx, y, Object.assign({}, o, { color, align: 'left' }, extra || {}));
      cx += textWidth(s, size, spacing);
    }
    return total;
  }
  // typewriter: first n chars of str after `t` seconds at `cps` chars/sec
  function typed(str, t, cps = 30) {
    const n = Math.max(0, Math.floor(t * cps));
    return [...str].slice(0, n).join('');
  }
  // scramble/decode effect: chars resolve left→right over p (0..1), unresolved chars are random glyphs
  function scramble(str, p, seed = 1, T = 0, glyphs = '█▓▒░<>/\\|_-=+*#%@$&01') {
    const chars = [...str];
    const n = chars.length;
    let out = '';
    for (let i = 0; i < n; i++) {
      const th = i / Math.max(1, n) * 0.7;
      if (chars[i] === ' ' || p >= th + 0.3) out += chars[i];
      else if (p > th) out += glyphs[Math.floor(hash(i, seed, Math.floor(T * 30)) * glyphs.length)];
      else out += ' ';
    }
    return out;
  }
  // blinking block cursor at (x, baselineY) for font size
  function cursor(g, x, y, size, T, color = C.text, rate = 2) {
    if (Math.floor(T * rate * 2) % 2 === 1) return;
    g.save();
    g.fillStyle = color;
    g.fillRect(Math.round(x), Math.round(y - size * 0.78), Math.round(size * 0.55), Math.round(size * 0.98));
    g.restore();
  }
  // shell prompt "$ cmd█" typed over time. opts: size, cps, start(delay s), prompt('$ '), color
  // returns {w, done}
  function prompt(g, cmd, x, y, t, o = {}) {
    const size = o.size || 32, cps = o.cps || 28, start = o.start || 0;
    const pr = o.prompt === undefined ? '$ ' : o.prompt;
    const shown = typed(cmd, t - start, cps);
    const w1 = text(g, pr, x, y, { size, color: o.promptColor || C.muted, weight: o.weight || 500 });
    const w2 = text(g, shown, x + w1, y, { size, color: o.color || C.text, weight: o.weight || 500 });
    const done = shown.length >= [...cmd].length;
    if (o.cursor !== false) cursor(g, x + w1 + w2 + size * 0.1, y, size, done ? (o.T || t) : 0, o.cursorColor || C.text);
    return { w: w1 + w2, done };
  }

  // ── shapes ─────────────────────────────────────────────────────────
  function rrect(g, x, y, w, h, r) {
    g.beginPath();
    g.roundRect(x, y, w, h, r);
  }
  // TUI-like panel: rounded hairline box with optional title in the top border
  // opts: color(border), fill, radius, title, titleColor, lw, alpha
  function panel(g, x, y, w, h, o = {}) {
    g.save();
    g.globalAlpha *= (o.alpha === undefined ? 1 : o.alpha);
    const r = o.radius === undefined ? 10 : o.radius;
    rrect(g, x + .5, y + .5, w - 1, h - 1, r);
    if (o.fill) { g.fillStyle = o.fill; g.fill(); }
    g.lineWidth = o.lw || 1.5;
    g.strokeStyle = o.color || C.line2;
    g.stroke();
    if (o.title) {
      const size = o.titleSize || 18;
      const tw = textWidth(o.title, size, o.titleSpacing || 2) + 20;
      g.fillStyle = o.titleBg || C.bg;
      g.fillRect(x + 22, y - 2, tw, 5);
      text(g, o.title, x + 32, y + size * 0.35, { size, color: o.titleColor || o.color || C.muted, weight: 700, spacing: o.titleSpacing || 2 });
    }
    g.restore();
  }
  // dashed polyline. pts=[[x,y],...]; opts: color, lw, dash([8,8]), offset (animate marching), progress (0..1 drawn length), arrow(bool)
  function dashed(g, pts, o = {}) {
    let len = 0; const segs = [];
    for (let i = 1; i < pts.length; i++) {
      const l = Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]);
      segs.push(l); len += l;
    }
    const prog = o.progress === undefined ? 1 : clamp(o.progress);
    let remain = len * prog;
    g.save();
    g.strokeStyle = o.color || C.muted;
    g.lineWidth = o.lw || 2;
    g.setLineDash(o.dash || [8, 8]);
    g.lineDashOffset = -(o.offset || 0);
    g.lineCap = 'butt';
    g.beginPath();
    g.moveTo(pts[0][0], pts[0][1]);
    let end = pts[0], dir = [1, 0];
    for (let i = 1; i < pts.length && remain > 0; i++) {
      const l = segs[i - 1], k = Math.min(1, remain / l);
      const p = [lerp(pts[i - 1][0], pts[i][0], k), lerp(pts[i - 1][1], pts[i][1], k)];
      g.lineTo(p[0], p[1]);
      dir = [(pts[i][0] - pts[i - 1][0]) / l, (pts[i][1] - pts[i - 1][1]) / l];
      end = p; remain -= l;
    }
    g.stroke();
    if (o.arrow && prog > 0) {
      g.setLineDash([]);
      const s = o.arrowSize || 10;
      const nx = -dir[1], ny = dir[0];
      g.beginPath();
      g.moveTo(end[0] - dir[0] * s + nx * s * .6, end[1] - dir[1] * s + ny * s * .6);
      g.lineTo(end[0], end[1]);
      g.lineTo(end[0] - dir[0] * s - nx * s * .6, end[1] - dir[1] * s - ny * s * .6);
      g.stroke();
    }
    g.restore();
    return len;
  }
  // point at fraction f along polyline
  function along(pts, f) {
    let len = 0; const segs = [];
    for (let i = 1; i < pts.length; i++) { const l = Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]); segs.push(l); len += l; }
    let d = clamp(f) * len;
    for (let i = 1; i < pts.length; i++) {
      if (d <= segs[i - 1]) { const k = d / segs[i - 1]; return [lerp(pts[i - 1][0], pts[i][0], k), lerp(pts[i - 1][1], pts[i][1], k)]; }
      d -= segs[i - 1];
    }
    return pts[pts.length - 1].slice();
  }
  // soft radial glow blob (additive)
  function glow(g, x, y, r, color, a = 0.5) {
    g.save();
    g.globalCompositeOperation = 'lighter';
    const gr = g.createRadialGradient(x, y, 0, x, y, r);
    gr.addColorStop(0, rgba(color, a));
    gr.addColorStop(0.4, rgba(color, a * 0.35));
    gr.addColorStop(1, rgba(color, 0));
    g.fillStyle = gr;
    g.fillRect(x - r, y - r, r * 2, r * 2);
    g.restore();
  }

  // ── logos (HAL/*.logo) ─────────────────────────────────────────────
  // Each logo cell: 1 = block, 2 = enclosed hole (eye / mouth), 0 = outside.
  const LOGO = {};
  function parseLogo(lines) {
    while (lines.length && !lines[lines.length - 1].trim()) lines = lines.slice(0, -1);
    const rows = lines.length, cols = Math.max(...lines.map(l => [...l].length));
    const grid = lines.map(l => { const a = [...l]; return Array.from({ length: cols }, (_, c) => a[c] === '█' ? 1 : 0); });
    // flood-fill outside from a padded border; remaining empty cells are holes
    const R = rows + 2, Cc = cols + 2;
    const out = Array.from({ length: R }, () => new Array(Cc).fill(false));
    const at = (r, c) => (r < 1 || c < 1 || r > rows || c > cols) ? 0 : grid[r - 1][c - 1];
    const st = [[0, 0]]; out[0][0] = true;
    while (st.length) {
      const [r, c] = st.pop();
      for (const [dr, dc] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
        const nr = r + dr, nc = c + dc;
        if (nr < 0 || nc < 0 || nr >= R || nc >= Cc || out[nr][nc] || at(nr, nc) === 1) continue;
        out[nr][nc] = true; st.push([nr, nc]);
      }
    }
    for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++)
      if (grid[r][c] === 0 && !out[r + 1][c + 1]) grid[r][c] = 2;
    return { rows, cols, grid };
  }
  function getLogo(name) {
    if (!LOGO[name]) LOGO[name] = parseLogo(window.HAL_LOGOS[name].slice());
    return LOGO[name];
  }
  function logoSize(name, cw, ch) { const L = getLogo(name); return { w: L.cols * cw, h: L.rows * ch, cols: L.cols, rows: L.rows }; }
  // Draw an agent logo in the header.png style: striped glowing blocks with dim tinted holes.
  // (x, y) = top-left. cw, ch = cell size in px (header uses ~24 x 40).
  // opts: color, alpha, reveal(0..1), mode('rows'|'random'|'cols'), stripe(true), holeAlpha(0.16),
  //       blink(0..1 closes the eyes), seed, T (for reveal flicker), jitter(px glitch per row)
  function logo(g, name, x, y, cw, ch, o = {}) {
    const L = getLogo(name);
    const color = o.color || C.text, reveal = o.reveal === undefined ? 1 : o.reveal;
    const mode = o.mode || 'rows', seed = o.seed || 7, T = o.T || 0;
    const holeA = o.holeAlpha === undefined ? 0.16 : o.holeAlpha;
    const blink = o.blink || 0;
    g.save();
    g.globalAlpha *= (o.alpha === undefined ? 1 : o.alpha);
    const drawn = [];
    for (let r = 0; r < L.rows; r++) {
      const jx = o.jitter ? Math.round((hash(r, seed, Math.floor(T * 24)) - .5) * 2 * o.jitter) : 0;
      for (let c = 0; c < L.cols; c++) {
        const v = L.grid[r][c];
        if (!v) continue;
        let th;
        if (mode === 'random') th = hash(r, c, seed);
        else if (mode === 'cols') th = (c + hash(r, c, seed) * 2) / (L.cols + 2);
        else th = (r + hash(r, c, seed) * 1.5) / (L.rows + 1.5);
        if (reveal < 1 && th > reveal) continue;
        const edge = reveal < 1 && th > reveal - 0.08; // freshly revealed cells flash white
        const px = Math.round(x + c * cw + jx), py = Math.round(y + r * ch);
        const w = Math.round(x + (c + 1) * cw + jx) - px, h = Math.round(y + (r + 1) * ch) - py;
        if (v === 2) {
          g.fillStyle = rgba(color, holeA);
          g.fillRect(px, py, w, h);
          if (blink > 0) { g.fillStyle = color; g.fillRect(px, py, w, Math.round(h * blink)); drawn.push([px, py, w, Math.round(h * blink)]); }
          continue;
        }
        g.fillStyle = edge ? '#ffffff' : color;
        g.fillRect(px, py, w, h);
        drawn.push([px, py, w, h]);
      }
    }
    if (o.stripe !== false) {
      // thin dark scan stripes like the header art
      g.fillStyle = 'rgba(0,0,0,0.32)';
      const step = o.stripeStep || Math.max(3, Math.round(ch / 12));
      for (const [px, py, w, h] of drawn)
        for (let yy = step - 1; yy < h; yy += step) g.fillRect(px, py + yy, w, 1);
    }
    g.restore();
    return { w: L.cols * cw, h: L.rows * ch };
  }
  // logo + spaced caps label + subtitle, centered on cx. Returns bottom y.
  function agentCard(g, key, cx, y, cw, ch, o = {}) {
    const A = AGENTS[key];
    const s = logoSize(A.logo, cw, ch);
    logo(g, A.logo, cx - s.w / 2, y, cw, ch, Object.assign({ color: A.color }, o));
    const ls = o.labelSize || Math.round(cw * 0.95);
    let by = y + s.h + ls * 2.2;
    if (o.label !== false) text(g, A.name, cx, by, { size: ls, color: A.color, weight: 700, spacing: ls * 0.18, align: 'center', alpha: o.labelAlpha });
    if (o.sub) { by += ls * 1.8; text(g, o.sub, cx, by, { size: ls * 0.95, color: C.text, align: 'center', alpha: o.labelAlpha }); }
    return by;
  }

  // ── camera helpers ─────────────────────────────────────────────────
  // scale around (cx, cy)
  function zoom(g, s, cx = W / 2, cy = H / 2) { g.translate(cx, cy); g.scale(s, s); g.translate(-cx, -cy); }

  // ── scenes & timeline ──────────────────────────────────────────────
  const SCENES = {};
  // HAL.scene({ id, draw(g, t, ctx) })   ctx = { dur, p, T, frame, fx, W, H, C }
  function scene(def) { SCENES[def.id] = def; }

  function defaultsFx() {
    return {
      glitch: 0,       // 0..1 horizontal tearing + rgb split
      flash: 0,        // 0..1 white flash
      bloom: 1,        // bloom strength multiplier
      aberration: 1,   // chromatic aberration multiplier
      curvature: 1,    // screen curvature multiplier
      noise: 1,        // grain multiplier
      scan: 1,         // scanline multiplier
      vignette: 1,
      crt: [1, 1, 0],  // [scaleX, scaleY, white] — power on/off. [1,1,0] = normal
      shake: [0, 0],   // px
      exposure: 1,
    };
  }

  const state = { canvas: null, g: null, post: null, timeline: [] };

  function timelineInfo() {
    let t = 0; const out = [];
    for (const [id, dur] of state.timeline) { out.push({ id, start: t, dur }); t += dur; }
    return { scenes: out, total: t };
  }

  function renderFrame(T) {
    const { g } = state;
    const info = timelineInfo();
    let s = info.scenes[info.scenes.length - 1], idx = info.scenes.length - 1;
    for (let i = 0; i < info.scenes.length; i++) {
      if (T < info.scenes[i].start + info.scenes[i].dur) { s = info.scenes[i]; idx = i; break; }
    }
    const t = T - s.start;
    const fx = defaultsFx();
    // automatic cut accent: a few frames of tearing right after every cut
    if (idx > 0 && t < 0.1) { const k = 1 - t / 0.1; fx.glitch = 0.55 * k; fx.aberration = 1 + 3 * k; }
    g.setTransform(1, 0, 0, 1, 0, 0);
    g.globalAlpha = 1; g.globalCompositeOperation = 'source-over';
    g.fillStyle = C.bg; g.fillRect(0, 0, W, H);
    const def = SCENES[s.id];
    const frame = Math.round(T * FPS);
    g.save();
    try {
      if (!def) throw new Error(`scene "${s.id}" not loaded`);
      def.draw(g, t, { dur: s.dur, p: t / s.dur, T, frame, fx, W, H, C, id: s.id });
    } catch (e) {
      g.restore(); g.save();
      g.setTransform(1, 0, 0, 1, 0, 0);
      g.fillStyle = '#300'; g.fillRect(0, 0, W, 120);
      text(g, `[${s.id}] ${e.message}`.slice(0, 110), 30, 70, { size: 26, color: '#ff8080' });
      window.HAL_ERRORS = (window.HAL_ERRORS || []); window.HAL_ERRORS.push(`${s.id}@${T.toFixed(3)}: ${e.stack}`);
    }
    g.restore();
    if (state.post) state.post.render(state.canvas, fx, T, frame);
    return { id: s.id, t };
  }

  window.HAL = {
    W, H, FPS, C, AGENTS, AGENT_ORDER,
    clamp, lerp, range, ease, hash, rng, noise1, rgba, mix, hexToRgb,
    FONT, ADV, text, textWidth, runs, typed, scramble, cursor, prompt,
    rrect, panel, dashed, along, glow,
    getLogo, logoSize, logo, agentCard, zoom,
    scene, SCENES, state, timelineInfo, renderFrame,
  };
})();
