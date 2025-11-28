// boot — CRT power-on, diagnostics flicker, then the hero prompt `hal run fizzbuzz.hal`.
HAL.scene({
  id: 'boot',
  draw(g, t, { dur, T, fx, W, H, C }) {
    const { ease, range, clamp, hash, text, runs, textWidth, AGENTS, AGENT_ORDER } = HAL;
    fx.glitch = 0;
    fx.aberration = 1;

    // ── CRT power-on ──
    const a = ease.outExpo(range(t, 0.0, 0.14));
    const b = ease.outCubic(range(t, 0.14, 0.36));
    fx.crt = [Math.max(0.0005, a), HAL.lerp(0.004, 1, b), 1 - ease.outQuad(range(t, 0.16, 0.36))];
    const spike = Math.max(0, 1 - Math.abs(t - 0.2) / 0.07);
    fx.flash = 0.35 * spike;
    fx.bloom = 1 + 1.2 * spike + 0.25 * (1 - range(t, 0.36, 0.8));
    if (t < 0.12) return; // only the line is visible

    // ── camera: drift + slow push, enter punch at the end ──
    const enter = ease.outExpo(range(t, 2.05, 2.2));
    const cam = 1.0 + 0.035 * range(t, 0.2, 2.05) + 0.16 * enter;
    const dx = HAL.noise1(T * 1.3, 4) * 7, dy = HAL.noise1(T * 1.1, 9) * 5;
    g.translate(dx, dy);
    HAL.zoom(g, cam, W / 2, H / 2 - 20);

    // ── background: faint grid + dim giant glyphs ──
    const gk = ease.outCubic(range(t, 0.15, 0.7));
    g.fillStyle = HAL.rgba(C.line, 0.55 * gk);
    const off = (T * 18) % 96;
    for (let x = -96; x < W + 96; x += 96) g.fillRect(Math.round(x - off), 0, 1, H);
    for (let y = 0; y < H + 96; y += 96) g.fillRect(0, Math.round(y), W, 1);
    // vignette the grid toward the centre so the hero stays clean
    const rg = g.createRadialGradient(W / 2, H / 2 - 20, 80, W / 2, H / 2 - 20, 760);
    rg.addColorStop(0, 'rgba(11,13,15,0.92)'); rg.addColorStop(1, 'rgba(11,13,15,0)');
    g.fillStyle = rg; g.fillRect(-100, -100, W + 200, H + 200);
    // big dim hex columns on the right edge
    for (let c = 0; c < 3; c++) for (let r = 0; r < 14; r++) {
      const hv = Math.floor(hash(r, c, Math.floor(T * 6 + r * 0.3)) * 65535).toString(16).padStart(4, '0');
      text(g, hv, 1560 + c * 100, 120 + r * 62, { size: 24, color: C.line2, alpha: 0.45 * gk * (0.5 + 0.5 * hash(r, c, 3)) });
    }

    // ── diagnostics, top-left ──
    const x0 = 96, y0 = 120, lh = 34, sz = 20;
    const lines = [
      [['HAL 0.1.0', C.text, { weight: 700 }], [' :: heuristic agentic loop', C.soft]],
      [['model ', C.soft], ['grok-4.20-0309-reasoning', C.text], [' · embeddings ', C.soft], ['embedding-beta-3-large', C.text]],
      [['image ', C.soft], ['debian:stable', C.text], [' · memory ', C.soft], ['hal.sqlite', C.text]],
    ];
    const subs = { interpreter: 'what?   plain English → logic.md', architect: 'how?    logic.md → runtime.yaml', conductor: 'docker run → stdout', reflector: 'observe → memory' };
    const dimA = 1 - 0.3 * range(t, 0.95, 1.2);
    let li = 0;
    const lineT = i => 0.36 + i * 0.065 + hash(i, 5) * 0.03;
    const pop = i => { const k = t - lineT(i); return k < 0 ? 0 : (k < 0.05 ? (Math.floor(k * 60) % 2 ? 0.4 : 1) : 1); };
    lines.forEach((parts, i) => {
      const al = pop(i) * dimA; if (!al) return;
      runs(g, [['> ', C.line2], ...parts], x0, y0 + i * lh, { size: sz, alpha: al });
    });
    AGENT_ORDER.forEach((k, j) => {
      const i = 3 + j, A = AGENTS[k];
      const al = pop(i) * dimA; if (!al) return;
      const y = y0 + i * lh;
      const ok = t > lineT(i) + 0.05;
      g.fillStyle = A.color; g.globalAlpha = al; g.fillRect(x0 + 2, y - 13, 12, 12); g.globalAlpha = 1;
      HAL.glow(g, x0 + 8, y - 7, 22, A.color, 0.25 * al);
      text(g, A.name.toLowerCase().padEnd(12), x0 + 30, y, { size: sz, color: A.color, weight: 700, alpha: al });
      text(g, subs[k], x0 + 30 + 13 * sz * 0.6, y, { size: sz, color: C.soft, alpha: al });
      text(g, ok ? '[ok]' : '[..]', x0 + 30 + 52 * sz * 0.6, y, { size: sz, color: ok ? C.ok : C.soft, weight: 700, alpha: al });
    });
    // tiny HUD, top-right-ish and bottom-left
    text(g, 'CRT-01 · 60 Hz · 1920×1080', 96, 1000, { size: 16, color: C.line2, spacing: 2, alpha: gk });
    text(g, `T+${(T).toFixed(3).padStart(6, '0')}`, 1824, 1000, { size: 16, color: C.line2, spacing: 2, align: 'right', alpha: gk });

    // ── hero prompt ──
    const hs = 72, cmd = 'hal run fizzbuzz.hal';
    const t1 = 0.9;
    const hw = textWidth('$ ' + cmd, hs);
    const hx = Math.round(W / 2 - hw / 2 - 20), hy = 560;
    const hk = ease.outExpo(range(t, 0.8, 1.0));
    if (hk > 0) {
      // uneven typing rhythm
      let n = 0;
      for (let i = 0; i < cmd.length; i++) { const ti = t1 + i * 0.047 + hash(i, 2) * 0.02; if (t >= ti) n = i + 1; }
      const shown = cmd.slice(0, n);
      g.save();
      g.globalAlpha = hk;
      const w1 = text(g, '$ ', hx, hy, { size: hs, color: C.interp, weight: 700 });
      let cx = hx + w1;
      // colour the words: command, subcommand, file
      const toks = [['hal', C.text], [' run', C.soft], [' fizzbuzz', C.text], ['.hal', C.cond]];
      let used = 0;
      for (const [s, col] of toks) {
        const part = shown.slice(used, used + s.length);
        if (part) text(g, part, cx, hy, { size: hs, color: col, weight: 700 });
        cx += textWidth(part, hs); used += s.length;
      }
      const done = n >= cmd.length;
      const lastT = t1 + (cmd.length - 1) * 0.047 + 0.03;
      const blinkOn = !done || t < lastT + 0.05 || Math.floor((t - lastT) * 8) % 2 === 0 || t > 2.0;
      if (blinkOn) {
        g.fillStyle = C.text;
        g.fillRect(Math.round(cx + 6), hy - Math.round(hs * 0.74), Math.round(hs * 0.55), Math.round(hs * 0.95));
      }
      g.restore();
      HAL.glow(g, W / 2, hy - 20, 520, C.interp, 0.06 * hk);
      // underline rule + label
      const rk = ease.outExpo(range(t, 0.95, 1.4));
      g.fillStyle = C.line2; g.fillRect(Math.round(W / 2 - hw / 2), hy + 40, Math.round(hw * rk), 1);
      text(g, 'PLAIN ENGLISH  →  RUNNING PROGRAM', W / 2, hy + 84, { size: 20, color: C.muted, weight: 700, spacing: 8, align: 'center', alpha: range(t, 1.3, 1.6) * (1 - enter) });
    }

    // ── enter punch ──
    if (t > 2.05) {
      const k = range(t, 2.05, 2.2);
      fx.flash = Math.max(fx.flash, 0.45 * Math.pow(1 - k, 2));
      fx.aberration = 1 + 3 * (1 - k);
      fx.glitch = 0.25 * (1 - k);
      fx.bloom = 1.5;
      // horizontal streak through the prompt line
      g.save();
      g.globalCompositeOperation = 'lighter';
      const sw = 2200 * ease.outExpo(k);
      const gr = g.createLinearGradient(W / 2 - sw / 2, 0, W / 2 + sw / 2, 0);
      gr.addColorStop(0, 'rgba(95,208,200,0)'); gr.addColorStop(0.5, 'rgba(255,255,255,0.95)'); gr.addColorStop(1, 'rgba(95,208,200,0)');
      g.fillStyle = gr;
      g.fillRect(W / 2 - sw / 2, hy - 24 - 3 * (1 - k), sw, 6 * (1 - k * 0.6));
      g.restore();
    }
  },
});
