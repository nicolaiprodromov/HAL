// tui — the live dashboard during `hal run fizzbuzz.hal`: a film-quality redraw of src/tui/app.rs.
// Phase bar (HAL chip, phase chips with spinner/elapsed), Compiler pane (phase banners with the agent
// logo, thinking/response bubbles, tool cards), Output pane (container stdout), key-hint line.
// Camera: tight on banner -> pull out -> push into Output -> pull out -> slow drift on Done.
HAL.scene({
  id: 'tui',
  draw(g, t, { dur, T, fx, W, H, C }) {
    const { ease, range, lerp, clamp, text, runs, textWidth, rgba, mix } = HAL;

    // ── layout (matches the TUI: 3-row bar, split body, 1-row hint) ───────────
    const BX = 96, BW = 1728;
    const BAR_Y = 92, BAR_H = 62;
    const BODY_Y = 166, BODY_H = 776;
    const LX = BX, LW = 920;
    const RX = BX + LW + 16, RW = BW - LW - 16;
    const HINT_Y = 982;
    const DIM = '#5b626d', DIM2 = '#3f454e';

    // ── phases ────────────────────────────────────────────────────────
    const PH = [
      { key: 'interp', label: 'Interpret', name: 'INTERPRETING', logo: 'interpreter', color: C.interp, t0: 0.0 },
      { key: 'arch', label: 'Build', name: 'BUILDING', logo: 'architect', color: C.arch, t0: 1.1 },
      { key: 'cond', label: 'Run', name: 'RUNNING', logo: 'conductor', color: C.cond, t0: 2.0 },
      { key: 'refl', label: 'Reflect', name: 'REFLECTING', logo: 'reflector', color: C.refl, t0: 2.9 },
    ];
    const T_DONE = 3.62;
    const SPEED = 4.6; // displayed seconds per film second
    let cur = 0;
    PH.forEach((p, i) => { if (t >= p.t0) cur = i; });
    const done = t >= T_DONE;
    const accent = done ? C.ok : PH[cur].color;
    const spin = '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'[Math.floor(t * 16) % 10];

    // ── camera ────────────────────────────────────────────────────────
    const CAM = [
      { t: 0, s: 2.35, x: 556, y: 292 },
      { t: 0.66, s: 1.0, x: 960, y: 540, d: 0.5 },
      { t: 1.92, s: 1.45, x: 1428, y: 505, d: 0.42 },
      { t: 2.86, s: 1.0, x: 960, y: 540, d: 0.5 },
      { t: 3.5, s: 1.045, x: 960, y: 540, d: 0.5, e: 'inOutCubic' },
    ];
    let cam = CAM[0];
    for (let i = 1; i < CAM.length; i++) {
      const a = CAM[i - 1], b = CAM[i];
      const k = range(t, b.t, b.t + b.d);
      if (t < b.t) break;
      const e = HAL.ease[b.e || 'outExpo'](k);
      cam = { s: Math.exp(lerp(Math.log(a.s), Math.log(b.s), e)), x: lerp(a.x, b.x, e), y: lerp(a.y, b.y, e) };
    }
    // slow drift in the holds
    const drift = 1 + 0.018 * t / 4;
    const camS = cam.s * drift;
    const camX = cam.x + HAL.noise1(t * 0.9, 3) * 6, camY = cam.y + HAL.noise1(t * 0.8, 9) * 5;
    // glitch kicks on the hard moves
    let kick = 0;
    for (let i = 1; i < CAM.length - 1; i++) kick = Math.max(kick, Math.max(0, 1 - (t - CAM[i].t) / 0.13) * (t >= CAM[i].t ? 1 : 0));
    PH.forEach((p, i) => { if (i > 0) kick = Math.max(kick, 0.55 * (t >= p.t0 ? Math.max(0, 1 - (t - p.t0) / 0.09) : 0)); });
    kick = Math.max(kick, 0.6 * (t >= T_DONE ? Math.max(0, 1 - (t - T_DONE) / 0.12) : 0));
    fx.glitch = Math.max(fx.glitch, 0.5 * kick);
    fx.aberration = Math.max(fx.aberration, 1 + 3 * kick);
    fx.bloom = 1.15 + (done ? 0.15 * Math.max(0, 1 - (t - T_DONE) / 0.6) : 0);
    if (t >= T_DONE) fx.flash = 0.14 * Math.max(0, 1 - (t - T_DONE) / 0.12);
    if (kick > 0.5) fx.shake = [(HAL.hash(Math.floor(t * 60), 1) - .5) * 10 * kick, (HAL.hash(Math.floor(t * 60), 2) - .5) * 6 * kick];

    g.translate(W / 2, H / 2); g.scale(camS, camS); g.translate(-camX, -camY);

    // ── backdrop: faint ticks + HUD just outside the dashboard ────────
    g.fillStyle = 'rgba(255,255,255,0.025)';
    for (let x = 0; x <= W; x += 48) g.fillRect(x, 0, 1, H);
    for (let y = 0; y <= H; y += 48) g.fillRect(0, y, W, 1);
    text(g, 'HAL  ·  fizzbuzz.hal  ·  build 4f2a91c8', BX, 76, { size: 14, color: DIM2, spacing: 3 });
    text(g, `FRAME ${String(Math.round(T * 60)).padStart(4, '0')}`, BX + BW, 76, { size: 14, color: DIM2, spacing: 3, align: 'right' });

    // ── helpers ───────────────────────────────────────────────────────
    function wrap(str, cols) {
      const out = []; let line = '';
      for (const w of str.split(' ')) {
        if ((line + ' ' + w).trim().length > cols) { out.push(line); line = w; } else line = (line + ' ' + w).trim();
      }
      if (line) out.push(line);
      return out;
    }
    function chip(x, y, str, size, fg, bg, weight = 700) {
      const w = textWidth(str, size);
      g.fillStyle = bg; g.fillRect(Math.round(x), Math.round(y - size * 0.95), Math.round(w), Math.round(size * 1.35));
      text(g, str, x, y, { size, color: fg, weight });
      return w;
    }
    function pixelCheck(x, y, c, color, alpha = 1) {
      const rows = ['.......XX', '......XXX', 'XX...XXX.', 'XXX.XXX..', '.XXXXX...', '..XXX....', '...X.....'];
      g.save(); g.globalAlpha *= alpha; g.fillStyle = color;
      rows.forEach((r, j) => [...r].forEach((ch, i) => { if (ch === 'X') g.fillRect(x + i * c, y + j * c, c, c); }));
      g.fillStyle = 'rgba(0,0,0,0.32)';
      rows.forEach((r, j) => [...r].forEach((ch, i) => { if (ch === 'X') for (let yy = 3; yy < c; yy += 4) g.fillRect(x + i * c, y + j * c + yy, c, 1); }));
      g.restore();
    }

    // ── pane frame (sharp hairline, title in the border) ──────────────
    function pane(x, y, w, h, title, color, tcolor) {
      g.fillStyle = 'rgba(18,22,26,0.55)'; g.fillRect(x, y, w, h);
      HAL.panel(g, x, y, w, h, { color, radius: 0, lw: 1.5, title, titleColor: tcolor, titleSize: 16, titleSpacing: 1.5 });
    }

    // ══ PHASE BAR ═════════════════════════════════════════════════════
    {
      pane(BX, BAR_Y, BW, BAR_H, '', rgba(accent, 0.55));
      const by = BAR_Y + 40, size = 22;
      let x = BX + 22;
      x += chip(x, by, ' HAL ', size, '#0b0d0f', C.ok);
      x += text(g, ' ▸ ', x, by, { size, color: DIM });
      PH.forEach((p, i) => {
        const started = t >= p.t0;
        const finished = i < cur || done;
        const running = i === cur && !done;
        const el = started ? ((finished ? (i < 3 ? PH[i + 1].t0 : T_DONE) : t) - p.t0) * SPEED : 0;
        const icon = finished ? '✓' : running ? spin : '·';
        const s = ` ${icon} ${p.label}${started ? ' ' + el.toFixed(1) + 's' : ''} `;
        if (running) {
          const pop = range(t, p.t0, p.t0 + 0.2);
          HAL.glow(g, x + textWidth(s, size) / 2, by - 6, 90, p.color, 0.22 + 0.25 * (1 - pop));
          x += chip(x, by, s, size, '#0b0d0f', mix('#ffffff', p.color, 0.4 + 0.6 * pop));
        } else if (finished) x += text(g, s, x, by, { size, color: done ? C.ok : p.color, weight: 500 });
        else x += text(g, s, x, by, { size, color: DIM });
        if (i < 3) x += text(g, ' » ', x, by, { size, color: DIM2 });
      });
      x += text(g, '   ', x, by, { size });
      text(g, `${(t * SPEED).toFixed(1)}s elapsed`, x, by, { size, color: DIM });
      text(g, done ? 'exit 0' : 'fizzbuzz.hal', BX + BW - 24, by, { size: 18, color: done ? C.ok : C.muted, align: 'right', weight: done ? 700 : 400 });
    }

    // ══ COMPILER PANE: timeline entries ════════════════════════════════
    const IN_X = LX + 26, IN_W = LW - 52, COLS = Math.floor(IN_W / 12);
    const VIEW_Y = BODY_Y + 22, VIEW_H = BODY_H - 34;
    const GAP = 12;
    const TOOLC = { files_read: C.key, files_write: C.ok, memory_search: C.interp, memory_write: C.arch, run_cmd: C.cond };
    const AC = { interp: C.interp, arch: C.arch, cond: C.cond, refl: C.refl };
    const NM = { interp: 'Interpreter', arch: 'Architect', cond: 'Conductor', refl: 'Reflector' };
    const E = [];
    const banner = (i) => E.push({ k: 'banner', ts: PH[i].t0, ph: i });
    const tool = (ts, ag, name, args, res, ms) => E.push({ k: 'tool', ts, ag, name, args, res, ms });
    const bubble = (ts, ag, str, final) => E.push({ k: 'bubble', ts, ag, final, lines: wrap(str, COLS - (final ? 6 : 6)), str, cps: final ? 150 : 320 });
    banner(0);
    tool(0.26, 'interp', 'files_read', 'fizzbuzz.hal', '9 lines · 214 bytes', 3);
    tool(0.40, 'interp', 'memory_search', '"fizzbuzz"', '1 note · conductor: exit 0', 41);
    bubble(0.54, 'interp', 'plain english in, no code asked for. rules: print 1..30, Fizz on 3, Buzz on 5, FizzBuzz on both. exit 0.', false);
    tool(0.88, 'interp', 'files_write', 'logic.md', '4 rules', 12);
    bubble(0.98, 'interp', 'logic.md written: 4 rules, no input, no network.', true);
    banner(1);
    bubble(1.2, 'arch', 'a shell loop is enough. debian:stable ships sh, so no deps and no network.', false);
    tool(1.42, 'arch', 'files_write', 'artifacts/fizzbuzz.sh', '24 lines', 9);
    tool(1.54, 'arch', 'files_write', 'runtime.yaml', 'debian:stable', 7);
    tool(1.66, 'arch', 'run_cmd', 'sh -n fizzbuzz.sh', 'exit 0 · syntax ok', 118);
    bubble(1.78, 'arch', 'runtime.yaml: debian:stable, entrypoint sh fizzbuzz.sh', true);
    banner(2);
    E.push({ k: 'run', ts: 2.1 });
    E.push({ k: 'log', ts: 2.24, ag: 'cond', msg: 'container 4f2a91 up · /work rw · --init' });
    E.push({ k: 'log', ts: 2.78, ag: 'cond', msg: 'exit 0 · 30 lines · 0.6s' });
    banner(3);
    tool(3.0, 'refl', 'memory_search', 'namespace=conductor', '1 run fact', 38);
    bubble(3.1, 'refl', 'exit 0, 30 lines, Fizz and Buzz on schedule, FizzBuzz on 15 and 30. nothing to fix.', false);
    tool(3.3, 'refl', 'memory_write', '"fizzbuzz: sh loop verified"', 'saved', 6);
    bubble(3.4, 'refl', 'noted: FizzBuzz lands on 15 and 30. next build reuses sh.', true);
    E.push({ k: 'done', ts: T_DONE });

    const typedN = (e, tt) => Math.max(0, Math.floor((tt - e.ts - 0.06) * e.cps));
    function entryH(e, tt) {
      const age = tt - e.ts;
      if (age < 0) return 0;
      switch (e.k) {
        case 'banner': return 236;
        case 'tool': return 30 + (age > 0.1 ? 26 : 0) + GAP;
        case 'bubble': {
          const lh = e.final ? 29 : 25, n = typedN(e, tt);
          let seen = 0, vis = 0;
          for (const l of e.lines) { if (seen < n) vis++; seen += l.length + 1; }
          return 32 + Math.max(1, vis) * lh + GAP + 4;
        }
        case 'run': return 62 + GAP;
        case 'log': return 28 + 4;
        case 'done': return 150;
      }
      return 0;
    }
    const totalH = tt => E.reduce((s, e) => s + entryH(e, tt), 0);
    // smoothed bottom-anchored scroll
    let wsum = 0, hs = 0;
    for (let i = 0; i < 16; i++) { const d = i * 0.02, w = Math.exp(-d / 0.07); hs += w * totalH(Math.max(0, t - d)); wsum += w; }
    hs /= wsum;
    const off = Math.max(0, hs - VIEW_H);

    // title
    const nEv = E.filter(e => t >= e.ts).length;
    const think = E.find(e => e.k === 'bubble' && !e.final && t >= e.ts && typedN(e, t) < e.str.length + 8);
    const compTitle = done ? ` Compiler  ·  ${nEv} events ` : think ? ` Compiler · ${NM[think.ag]} ${spin} — ${nEv} events ` : ` Compiler — ${nEv} events `;
    const lf = 1 - ease.outCubic(range(t, 1.92, 2.12)) * (1 - ease.outCubic(range(t, 2.86, 3.1)));
    g.save(); g.globalAlpha *= lf;
    pane(LX, BODY_Y, LW, BODY_H, compTitle, rgba(accent, 0.5), think ? AC[think.ag] : accent);

    g.save();
    g.beginPath(); g.rect(LX + 3, VIEW_Y, LW - 6, VIEW_H); g.clip();
    let y = VIEW_Y - off;
    E.forEach((e, idx) => {
      const age = t - e.ts;
      const h = entryH(e, t);
      if (age < 0) return;
      const y0 = y; y += h;
      if (y0 + h < VIEW_Y - 10 || y0 > VIEW_Y + VIEW_H + 10) return;
      const fa = clamp(age / 0.08);
      g.save(); g.globalAlpha *= fa;
      if (e.k === 'banner') {
        const P = PH[e.ph], cw = 15, ch = 26;
        const s = HAL.logoSize(P.logo, cw, ch);
        const lx = LX + LW / 2 - s.w / 2, ly = y0 + 16;
        const rv = ease.outExpo(range(age, 0, 0.42));
        if (age < 0.5) HAL.glow(g, LX + LW / 2, ly + s.h / 2, 190, P.color, 0.30 * (1 - age / 0.5));
        HAL.logo(g, P.logo, lx, ly, cw, ch, { color: P.color, reveal: rv, mode: e.ph % 2 ? 'cols' : 'rows', seed: e.ph + 5, T, jitter: age < 0.32 ? 7 * (1 - age / 0.32) : 0 });
        const label = HAL.scramble(`── ${P.name} ──`, range(age, 0.05, 0.4), e.ph + 1, T);
        text(g, label, LX + LW / 2, ly + s.h + 40, { size: 22, color: P.color, weight: 700, spacing: 4, align: 'center' });
        const lw = textWidth(`── ${P.name} ──`, 22, 4) / 2 + 24, rl = ease.outExpo(range(age, 0.1, 0.5)) * 300;
        g.fillStyle = rgba(P.color, 0.5);
        g.fillRect(LX + LW / 2 - lw - rl, ly + s.h + 34, rl, 1); g.fillRect(LX + LW / 2 + lw, ly + s.h + 34, rl, 1);
      } else if (e.k === 'tool') {
        const c = TOOLC[e.name] || C.muted, ly = y0 + 22;
        g.fillStyle = c; g.fillRect(IN_X + 6, ly - 12, 10, 10);
        const ph = AC[e.ag];
        let x = IN_X + 30;
        x += text(g, e.name, x, ly, { size: 20, color: c, weight: 700 });
        x += text(g, '  ', x, ly, { size: 20 });
        text(g, e.args, x, ly, { size: 20, color: C.soft });
        const ok = age > 0.1;
        const st = ok ? `✓ ok · ${e.ms} ms` : `${spin} running`;
        text(g, st, IN_X + IN_W - 4, ly, { size: 18, color: ok ? C.ok : C.warn, weight: 700, align: 'right' });
        if (ok) {
          g.fillStyle = rgba(ph, 0.5); g.fillRect(IN_X + 10, ly + 6, 2, 22);
          text(g, e.res, IN_X + 30, ly + 24, { size: 18, color: C.muted, alpha: clamp((age - 0.1) / 0.1) });
        }
      } else if (e.k === 'bubble') {
        const c = AC[e.ag], hy = y0 + 22;
        const hdr = e.final ? ` ⟵ response · ${NM[e.ag]} ` : (typedN(e, t) >= e.str.length ? ` thoughts · ${NM[e.ag]} ` : ` streaming… · ${NM[e.ag]} `);
        const hbg = e.final ? C.ok : (typedN(e, t) >= e.str.length ? c : C.warn);
        chip(IN_X, hy, hdr, 16, '#0b0d0f', hbg);
        const lh = e.final ? 29 : 25, size = e.final ? 20 : 18;
        const n = typedN(e, t);
        let seen = 0, row = 0;
        const barC = e.final ? C.ok : c;
        for (const l of e.lines) {
          if (seen >= n) break;
          const shown = l.slice(0, n - seen);
          const ty = hy + 32 + row * lh;
          g.fillStyle = rgba(barC, e.final ? 0.9 : 0.55); g.fillRect(IN_X + 6, ty - lh + 8, 3, lh);
          text(g, shown, IN_X + 30, ty, { size, color: e.final ? C.text : DIM, weight: e.final ? 500 : 400 });
          if (n - seen <= l.length && n < e.str.length) HAL.cursor(g, IN_X + 30 + textWidth(shown, size) + 2, ty, size, T, c, 4);
          seen += l.length + 1; row++;
        }
      } else if (e.k === 'run') {
        const ly = y0 + 22;
        g.fillStyle = C.cond; g.fillRect(IN_X + 4, ly - 16, 4, 52);
        runs(g, [['▸ run action', C.cond, { weight: 700 }]], IN_X + 22, ly, { size: 20 });
        runs(g, [['debian:stable', C.text], ['  ·  ', DIM], ['sh fizzbuzz.sh', C.soft]], IN_X + 22, ly + 30, { size: 18 });
      } else if (e.k === 'log') {
        const ly = y0 + 20;
        runs(g, [['·  ', DIM], ['[Conductor] ', AC[e.ag], { weight: 700 }], [e.msg, C.muted]], IN_X + 6, ly, { size: 18 });
      } else if (e.k === 'done') {
        const c = 12;
        pixelCheck(LX + LW / 2 - 4.5 * c, y0 + 4, c, C.ok);
        text(g, '── DONE ──', LX + LW / 2, y0 + 124, { size: 22, color: C.ok, weight: 700, spacing: 4, align: 'center' });
        text(g, '● pipeline done — exit 0', LX + LW / 2, y0 + 148, { size: 18, color: C.ok, weight: 700, align: 'center' });
        HAL.glow(g, LX + LW / 2, y0 + 45, 150, C.ok, 0.25 * Math.max(0, 1 - age / 0.8));
      }
      g.restore();
    });
    // fade at the top of the viewport
    const gr = g.createLinearGradient(0, VIEW_Y, 0, VIEW_Y + 46);
    gr.addColorStop(0, 'rgba(11,13,15,0.96)'); gr.addColorStop(1, 'rgba(11,13,15,0)');
    g.fillStyle = gr; g.fillRect(LX + 3, VIEW_Y, LW - 6, 46);
    g.restore();
    // scroll thumb
    if (hs > VIEW_H) {
      const th = Math.max(30, VIEW_H * VIEW_H / hs), ty = VIEW_Y + (VIEW_H - th) * (off / (hs - VIEW_H));
      g.fillStyle = rgba(accent, 0.6); g.fillRect(LX + LW - 9, ty, 3, th);
    }

    g.restore();
    // ══ OUTPUT PANE ═══════════════════════════════════════════════════
    {
      const OUT0 = 2.0, RATE = 0.0215;
      const lines = [['[hal] debian:stable · /work rw', 'meta'], ['+ sh fizzbuzz.sh', 'meta']];
      for (let i = 1; i <= 30; i++) lines.push([i % 15 === 0 ? 'FizzBuzz' : i % 3 === 0 ? 'Fizz' : i % 5 === 0 ? 'Buzz' : String(i), 'out']);
      const n = clamp(Math.floor((t - OUT0) / RATE) + 1, 0, lines.length);
      const title = n === 0 ? ` Output — (waiting for container) ${spin} ` : ` Output — ${n} lines `;
      const ocol = n > 0 ? rgba(C.cond, 0.6) : C.line2;
      pane(RX, BODY_Y, RW, BODY_H, title, ocol, n > 0 ? C.cond : DIM);
      // amber Running logo + HUD on the right side of the pane (balance for the stdout column)
      const LC = 22, LH = 36, gs = HAL.logoSize('conductor', LC, LH);
      const gx = RX + RW - gs.w - 60, gy = BODY_Y + 120;
      const run = n > 0 ? ease.outExpo(range(t, OUT0 - 0.1, OUT0 + 0.35)) : 0;
      HAL.logo(g, 'conductor', gx, gy, LC, LH, { color: C.cond, alpha: n > 0 ? 0.75 * run : 0, reveal: n > 0 ? run : 1, mode: 'rows', seed: 11, T });
      if (n > 0) HAL.glow(g, gx + gs.w / 2, gy + gs.h / 2, 260, C.cond, 0.16);
      const hx = gx + gs.w / 2;
      text(g, '── RUNNING ──', hx, gy + gs.h + 44, { size: 22, color: C.cond, weight: 700, spacing: 4, align: 'center', alpha: run });
      text(g, `${Math.max(0, n - 2)} / 30`, hx, gy + gs.h + 130, { size: 56, color: C.cond, weight: 800, align: 'center', alpha: 0.9 * run });
      text(g, 'lines', hx, gy + gs.h + 166, { size: 18, color: C.muted, spacing: 4, align: 'center', alpha: run });
      g.fillStyle = rgba(C.cond, 0.2 * run); g.fillRect(hx - 170, gy + gs.h + 196, 340, 6);
      g.fillStyle = C.cond; g.fillRect(hx - 170, gy + gs.h + 196, 340 * Math.max(0, n - 2) / 30 * run, 6);
      const fin = range(t, OUT0 + lines.length * RATE, OUT0 + lines.length * RATE + 0.12);
      text(g, 'exit 0 · 30 lines · 0.6s', hx, gy + gs.h + 250, { size: 20, color: C.ok, weight: 700, spacing: 1, align: 'center', alpha: fin });
      const lh = 30, vh = Math.floor((BODY_H - 64) / lh);
      g.save(); g.beginPath(); g.rect(RX + 3, BODY_Y + 16, RW - 6, BODY_H - 30); g.clip();
      const first = Math.max(0, n - vh);
      for (let i = first; i < n; i++) {
        const ly = BODY_Y + 40 + (i - first) * lh + 14;
        const [s, kind] = lines[i];
        const fresh = i === n - 1 ? 1 : 0;
        if (kind === 'meta') { text(g, s, RX + 28, ly, { size: 20, color: DIM }); continue; }
        const word = /[A-Z]/.test(s);
        if (s === 'FizzBuzz') { g.fillStyle = rgba(C.cond, 0.16); g.fillRect(RX + 18, ly - 21, 330, 28); }
        text(g, String(i - 1).padStart(2, ' '), RX + 28, ly, { size: 14, color: DIM2 });
        text(g, s, RX + 74, ly, { size: 22, color: word ? C.cond : C.soft, weight: s === 'FizzBuzz' ? 800 : word ? 700 : 400 });
        if (fresh) { g.fillStyle = rgba('#ffffff', 0.5); g.fillRect(RX + 14, ly - 20, 3, 26); }
      }
      if (n > 0 && n < lines.length) HAL.cursor(g, RX + 74, BODY_Y + 40 + Math.min(n, vh) * lh + 14, 22, T, C.cond, 4);
      if (n >= lines.length) text(g, '[exit 0]', RX + 28, BODY_Y + 40 + Math.min(n, vh) * lh + 14, { size: 18, color: C.ok, weight: 700, alpha: range(t, OUT0 + lines.length * RATE, OUT0 + lines.length * RATE + 0.1) });
      g.restore();
      if (n > vh) { const th = (BODY_H - 40) * vh / n, ty = BODY_Y + 20 + (BODY_H - 40 - th) * (first / (n - vh)); g.fillStyle = rgba(C.cond, 0.6); g.fillRect(RX + RW - 9, ty, 3, th); }
    }

    // ══ HINT / STATUS LINE ════════════════════════════════════════════
    {
      const pairs = [['q', 'quit'], ['Space', 'pause'], ['Ctrl+O', 'artifacts'], ['m', 'mouse'], ['↑↓', 'scroll'], ['[ ]', 'collapse/expand']];
      const parts = [];
      pairs.forEach(([k, v], i) => { if (i) parts.push(['  ·  ', DIM2]); parts.push([k, C.key, { weight: 700 }]); parts.push([' ' + v, C.muted]); });
      runs(g, parts, BX + BW / 2, HINT_Y, { size: 18, align: 'center' });
      runs(g, [[done ? '●' : spin, accent, { weight: 700 }], [' ' + (done ? 'DONE' : PH[cur].name), accent, { weight: 700 }]], BX + 4, HINT_Y, { size: 18, spacing: 1 });
      text(g, 'grok-4.20 · debian:stable', BX + BW - 4, HINT_Y, { size: 18, color: DIM, align: 'right' });
    }

    // phase-switch sweep: an accent scan line racing down the dashboard
    PH.forEach((p, i) => {
      if (i === 0) return;
      const k = range(t, p.t0, p.t0 + 0.22);
      if (k > 0 && k < 1) {
        const sy = BAR_Y + (BODY_Y + BODY_H - BAR_Y) * ease.outQuart(k);
        g.fillStyle = rgba(p.color, 0.8 * (1 - k)); g.fillRect(BX, sy, BW, 2);
        HAL.glow(g, BX + BW * ease.outQuart(k), sy, 120, p.color, 0.25 * (1 - k));
      }
    });
  },
});
