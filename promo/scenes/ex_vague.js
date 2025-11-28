// ex_vague — example 06/06: vague.hal. Two sentences become a calm generative terminal piece.
HAL.scene({
  id: 'ex_vague',
  draw(g, t, { dur, T, fx, C }) {
    const { ease, range, clamp, lerp, hash } = HAL;
    fx.bloom = 1.35;
    const accent = C.refl;
    HAL.exampleFrame(g, t, T, { n: 6, file: 'vague.hal', args: '', flags: '-f', blurb: 'two sentences. HAL does the rest.', accent });

    // ── character field geometry ──
    const FS = 20, cw = FS * 0.6, chh = 26;
    const AX = 90, AY = 160, COLS = 144, ROWS = 31;
    const HOR = 21; // horizon row

    // ── the spec, big ──
    const lines = ['Make something calming', 'to watch in the terminal', 'for about a minute.', 'Surprise me.'];
    const SZ = 88, adv = SZ * 0.6, LH = 118, TY0 = 292;
    const chars = [];
    let idx = 0;
    lines.forEach((ln, li) => {
      const w = [...ln].length * adv, x0 = 960 - w / 2;
      [...ln].forEach((ch, ci) => chars.push({ ch, x: x0 + ci * adv, y: TY0 + li * LH, idx: idx++, li }));
    });
    const nCh = chars.length;
    const typedN = Math.floor(nCh * 0.4 + t * 260);
    const zk = 1 + 0.05 * ease.outQuad(t / dur);

    // per-cell reveal time: spreads out from the letters
    const boxes = chars.filter(c => c.ch !== ' ');
    const nearest = (px, py) => {
      let d = 1e9;
      for (const c of boxes) { const dx = Math.max(0, Math.abs(px - (c.x + adv / 2)) - adv / 2), dy = Math.max(0, Math.abs(py - (c.y - SZ * 0.3)) - SZ * 0.5); d = Math.min(d, dx * dx + dy * dy); }
      return Math.sqrt(d);
    };

    g.save();
    HAL.zoom(g, zk, 960, 580);

    // ── generative field ──
    const field = (col, row, tt) => {
      const x = col / COLS;
      let best = 0, hue = 0;
      for (let b = 0; b < 2; b++) {
        const ph = b * 4.7;
        // hem: slow folding line near the horizon
        const hem = 18.6 + b * 1.2 + 2.0 * Math.sin(x * 7 + tt * 0.55 + ph) + 1.4 * Math.sin(x * 17 - tt * 0.8 + ph * 2) + 0.8 * HAL.noise1(col * 0.07 + tt * 0.35, 20 + b);
        // envelope along the sky: gaps and a few hot ribbons
        const env = Math.pow(clamp(0.95 + 0.7 * HAL.noise1(x * 3.2 + tt * 0.22 + ph, 30 + b)), 1.1);
        const rays = Math.pow(clamp(0.5 + 0.55 * HAL.noise1(col * 0.21 + tt * 0.3, 40 + b) + 0.4 * HAL.noise1(col * 0.8 - tt * 0.5, 44 + b)), 2.0);
        const ht = 11 + 12 * clamp(0.4 + 0.6 * HAL.noise1(col * 0.05 + tt * 0.3, 50 + b)) + 5 * env;
        const d = hem - row;
        let v;
        if (d >= 0) v = Math.exp(-d / (ht * 0.62)) * (0.06 + 0.94 * rays);
        else v = Math.exp(-d * d * 1.4) * 0.9;
        v *= env * (b ? 0.75 : 1);
        if (d >= 0 && d < 2.2) v *= 1.2 - 0.1 * d; // bright lower hem
        if (v > best) {
          best = v;
          hue = clamp(0.0 + 2.3 * clamp(d / ht) + 0.35 * x + 0.12 * b + 0.08 * Math.sin(tt * 0.4 + x * 3));
        }
      }
      return [Math.min(0.92, best * 1.2), hue];
    };
    const RAMP = ' .:-=+*#%@';
    const MINT = [132, 226, 176], TEAL = [90, 214, 224], VIO = [166, 140, 255];
    const col3 = (h, k) => {
      h = clamp(h);
      const a = h < 0.5 ? MINT : TEAL, b = h < 0.5 ? TEAL : VIO, f = h < 0.5 ? h * 2 : (h - 0.5) * 2;
      return [lerp(a[0], b[0], f) * k + 255 * (1 - k) * 0, lerp(a[1], b[1], f) * k, lerp(a[2], b[2], f) * k];
    };

    g.save();
    g.font = `500 ${FS}px "JBM", monospace`; g.textBaseline = 'middle'; g.textAlign = 'left'; g.letterSpacing = '0px';
    const tt = t + 3.2; // field has been "running" already
    for (let row = 0; row < ROWS; row++) {
      for (let col = 0; col < COLS; col++) {
        const px = AX + col * cw, py = AY + row * chh + chh / 2;
        // reveal wave from the letters
        const rv = 0.42 + 0.55 * clamp(nearest(px, py) / 520) + hash(col, row, 3) * 0.14;
        const k = ease.outCubic(range(t, rv, rv + 0.28));
        if (k <= 0) continue;
        let ch, color, a;
        if (row < HOR) {
          const [v, hu] = field(col, row, tt);
          // stars
          const sh = hash(col, row, 11);
          let val = v;
          if (sh > 0.982 && v < 0.12) {
            const tw = 0.5 + 0.5 * Math.sin(tt * (1.5 + sh * 3) + sh * 50);
            ch = tw > 0.8 ? '*' : tw > 0.35 ? '+' : '·'; a = (0.25 + 0.6 * tw) * k; color = [215, 235, 255];
          } else {
            if (val < 0.09) continue;
            ch = RAMP[Math.min(RAMP.length - 1, Math.floor(val * (RAMP.length - 0.01)))];
            if (ch === ' ') continue;
            color = col3(hu, 1); a = clamp(0.18 + val * 0.95) * k;
            if (val > 0.7) color = [lerp(color[0], 255, 0.3), lerp(color[1], 255, 0.3), lerp(color[2], 255, 0.3)];
          }
        } else if (row === HOR) {
          ch = '─'; color = [90, 120, 130]; a = 0.55 * k;
        } else {
          // reflection with ripples
          const mr = 2 * HOR - row + 1.2 * Math.sin(col * 0.35 + tt * 1.6 + row * 0.9);
          const [v, hu] = field(col, Math.round(mr), tt);
          const fade = 1 - (row - HOR) / (ROWS - HOR + 2);
          const val = v * fade * 0.55;
          if (val < 0.16) {
            if (hash(col, row, 5) > 0.975) { ch = '~'; color = [70, 100, 120]; a = 0.35 * k; } else continue;
          } else {
            ch = RAMP[Math.min(RAMP.length - 1, Math.floor(val * (RAMP.length - 0.01)))];
            if (ch === ' ') continue;
            color = col3(hu, 1); a = clamp(0.15 + val * 0.7) * 0.6 * k;
          }
        }
        // arrival flash: freshly revealed cells burn white-mint
        const fl = 1 - clamp((t - rv) / 0.22);
        if (fl > 0) { color = [lerp(color[0], 235, fl), lerp(color[1], 255, fl), lerp(color[2], 245, fl)]; a = Math.min(1, a + fl * 0.5); }
        g.fillStyle = `rgba(${color[0] | 0},${color[1] | 0},${color[2] | 0},${a.toFixed(3)})`;
        g.fillText(ch, Math.round(px), Math.round(py));
      }
    }
    g.restore();

    // soft aurora haze behind the field (bloom fodder)
    const hz = ease.outCubic(range(t, 0.55, 1.1));
    if (hz > 0) {
      HAL.glow(g, 700 + 160 * Math.sin(tt * 0.5), 440, 520, '#59e0b5', 0.17 * hz);
      HAL.glow(g, 1250 + 120 * Math.sin(tt * 0.4 + 2), 400, 460, '#9a7bff', 0.15 * hz);
    }

    // ── the spec dissolving ──
    chars.forEach(c => {
      if (c.ch === ' ' || c.idx >= typedN) return;
      const td = 0.38 + hash(c.idx, 2) * 0.22 + c.idx / nCh * 0.12;
      const k = range(t, td, td + 0.3);
      if (k >= 1) return;
      const e = ease.inCubic(k);
      const jitter = k > 0 ? (hash(c.idx, Math.floor(T * 30)) - 0.5) * 10 * k : 0;
      const alpha = 1 - e;
      const scr = k > 0.15 && hash(c.idx, Math.floor(T * 24), 9) < k ? ',.:;*+~·'[Math.floor(hash(c.idx, Math.floor(T * 20)) * 8)] : c.ch;
      const col = k > 0 ? HAL.mix('#eef0f3', '#84e2b0', clamp(k * 1.6)) : '#eef0f3';
      HAL.text(g, scr, c.x + jitter, c.y + k * 24 * hash(c.idx, 4) - k * 14, { size: SZ * (1 - 0.35 * e), weight: 700, color: col, alpha });
      if (k < 0.01) HAL.glow(g, c.x + adv / 2, c.y - SZ * 0.3, 70, '#ffffff', 0.05);
    });
    // typing caret
    if (t < 0.5 && typedN < nCh) {
      const c = chars[Math.min(nCh - 1, typedN)];
      g.fillStyle = accent; g.fillRect(Math.round(c.x), c.y - SZ * 0.78, 6, SZ * 0.95);
    }

    // caption once the piece is alive
    const ck = ease.outCubic(range(t, 1.0, 1.25));
    if (ck > 0) {
      g.fillStyle = 'rgba(11,13,15,0.85)'; g.fillRect(AX, 150 + 31 * chh + 6 - 2, 0, 0);
      HAL.runs(g, [['◆ ', accent], ['interpreted as ', C.muted], ['“aurora over still water”', C.text, { weight: 700 }], ['  ·  ~60 s', C.muted]], 960, 986, { size: 22, align: 'center', alpha: ck });
    }
    g.restore();
  },
});
