// outro — HAL wordmark built from striped block cells, tagline, install chip, CRT power-off.
HAL.scene({
  id: 'outro',
  draw(g, t, { dur, T, fx, W, H, C }) {
    const { ease, range, hash, text, textWidth, rgba, mix, lerp, AGENTS, AGENT_ORDER } = HAL;

    // ── CRT power-off, from 3.05 s ──
    const p1 = ease.inCubic(range(t, 3.05, 3.3));    // collapse vertically
    const p2 = ease.outExpo(range(t, 3.3, 3.5));      // then horizontally to a dot
    const fade = range(t, 3.5, 3.58);
    fx.crt = [Math.max(0.0005, 1 - p2), lerp(1, 0.004, p1), range(t, 3.05, 3.3) * (1 - fade)];
    if (t >= 3.5) { fx.crt = [0.0005, 0.004, 0]; }
    fx.bloom = 1.2 + 0.8 * range(t, 3.05, 3.3);
    if (t > 3.52) return;

    // ── camera ──
    g.translate(HAL.noise1(T * 1.2, 21) * 5, HAL.noise1(T * 1.1, 33) * 4);
    HAL.zoom(g, 1.0 + 0.045 * range(t, 0, 3.1), W / 2, H / 2 - 40);

    // ── background: drifting grid + dim giant logos ──
    const bk = ease.outCubic(range(t, 0, 0.6));
    g.fillStyle = rgba(C.line, 0.45 * bk);
    const off = (T * 14) % 96;
    for (let x = -96; x < W + 96; x += 96) g.fillRect(Math.round(x - off), 0, 1, H);
    for (let y = 0; y < H + 96; y += 96) g.fillRect(0, Math.round(y + off * 0.0), W, 1);
    const rg = g.createRadialGradient(W / 2, 420, 100, W / 2, 420, 900);
    rg.addColorStop(0, 'rgba(11,13,15,0.9)'); rg.addColorStop(1, 'rgba(11,13,15,0)');
    g.fillStyle = rg; g.fillRect(-100, -100, W + 200, H + 200);

    // ── HAL wordmark from block cells ──
    const GL = {
      H: ['██   ██', '██   ██', '██   ██', '██   ██', '███████', '███████', '██   ██', '██   ██', '██   ██'],
      A: [' █████ ', '███████', '██   ██', '██   ██', '███████', '███████', '██   ██', '██   ██', '██   ██'],
      L: ['██     ', '██     ', '██     ', '██     ', '██     ', '██     ', '██     ', '███████', '███████'],
    };
    const word = [['H', C.interp], ['A', C.arch], ['L', C.cond]];
    const cw = 36, ch = 40, gap = 1;
    const totalCols = 7 * 3 + gap * 2;
    const wx = Math.round(W / 2 - totalCols * cw / 2), wy = 120;
    const drawn = [];
    // shared brightness flash as each letter lands
    word.forEach(([L, col], li) => {
      const t0 = -0.06 + li * 0.11; // already assembling on the cut
      const rv = ease.outCubic(range(t, t0, t0 + 0.42));
      if (rv <= 0) return;
      const ox = wx + li * (7 + gap) * cw;
      const rows = GL[L];
      const lcol = col;
      for (let r = 0; r < 9; r++) for (let c = 0; c < 7; c++) {
        if (rows[r][c] !== '█') continue;
        // letters build from random cells; fresh cells flash white
        const th = hash(r, c, li * 5 + 1) * 0.55 + (r / 9) * 0.45;
        if (th > rv) continue;
        const fresh = th > rv - 0.1 && rv < 1;
        const px = ox + c * cw, py = wy + r * ch;
        g.fillStyle = fresh ? '#ffffff' : lcol;
        g.fillRect(px, py, cw, ch);
        drawn.push([px, py, cw, ch]);
      }
      // landing pulse
      const land = Math.max(0, 1 - (t - t0 - 0.4) / 0.25);
      if (land > 0 && t > t0 + 0.4) HAL.glow(g, ox + 3.5 * cw, wy + 4.5 * ch, 420, col, 0.22 * land);
    });
    g.fillStyle = 'rgba(0,0,0,0.34)';
    for (const [px, py, w, h] of drawn) for (let yy = 3; yy < h; yy += 4) g.fillRect(px, py + yy, w, 1);
    // subtle hot sheen sweeping across once settled
    const sh = range(t, 0.75, 1.25);
    if (sh > 0 && sh < 1) {
      g.save(); g.globalCompositeOperation = 'lighter';
      const sx = wx - 100 + sh * (totalCols * cw + 200);
      const gr = g.createLinearGradient(sx - 90, 0, sx + 90, 0);
      gr.addColorStop(0, 'rgba(255,255,255,0)'); gr.addColorStop(0.5, 'rgba(255,255,255,0.35)'); gr.addColorStop(1, 'rgba(255,255,255,0)');
      g.beginPath(); for (const [px, py, w, h] of drawn) g.rect(px, py, w, h);
      g.clip(); g.fillStyle = gr; g.fillRect(sx - 90, wy, 180, 9 * ch);
      g.restore();
    }

    // ── four-colour bar under the wordmark ──
    const barY = wy + 9 * ch + 36, barW = totalCols * cw;
    AGENT_ORDER.forEach((k, i) => {
      const bk2 = ease.outExpo(range(t, 0.55 + i * 0.05, 0.9 + i * 0.05));
      const sw = barW / 4;
      g.fillStyle = AGENTS[k].color;
      g.fillRect(wx + i * sw, barY, Math.round((sw - 8) * bk2), 6);
    });

    // ── subtitle, tagline, chip ──
    const sk = range(t, 0.7, 1.05);
    text(g, HAL.scramble('HEURISTIC AGENTIC LOOP', sk, 3, T), W / 2, barY + 80, { size: 30, color: C.soft, weight: 700, spacing: 14, align: 'center', alpha: sk > 0 ? 1 : 0 });

    const tk = range(t, 1.1, 1.5);
    if (tk > 0) {
      const tagline = 'write what you mean. hal runs it.';
      const shown = HAL.typed(tagline, t - 1.1, 60);
      const w = textWidth(tagline, 34);
      let cx = W / 2 - w / 2;
      const i1 = tagline.indexOf('hal runs');
      text(g, shown.slice(0, i1), cx, barY + 150, { size: 34, color: C.soft });
      if (shown.length > i1) text(g, shown.slice(i1), cx + textWidth(tagline.slice(0, i1), 34), barY + 150, { size: 34, color: C.text, weight: 700 });
    }

    const ck = ease.outBack(range(t, 1.5, 1.8));
    if (ck > 0) {
      const cmd = 'just install', csz = 32;
      const cwid = textWidth('$ ' + cmd, csz) + 72, chh = 72;
      const cx = W / 2 - cwid / 2, cy = barY + 200;
      g.save();
      HAL.zoom(g, 0.9 + 0.1 * ck, W / 2, cy + chh / 2);
      g.globalAlpha = Math.min(1, ck);
      g.fillStyle = 'rgba(18,22,26,0.9)'; g.fillRect(cx, cy, cwid, chh);
      HAL.panel(g, cx, cy, cwid, chh, { color: C.interp, radius: 4, lw: 2 });
      const w1 = text(g, '$ ', cx + 36, cy + 47, { size: csz, color: C.interp, weight: 700 });
      const n = HAL.typed(cmd, t - 1.65, 40);
      const w2 = text(g, n, cx + 36 + w1, cy + 47, { size: csz, color: C.text, weight: 700 });
      HAL.cursor(g, cx + 36 + w1 + w2 + 4, cy + 47, csz, T, C.interp, 2.5);
      g.restore();
      HAL.glow(g, W / 2, cy + chh / 2, 420, C.interp, 0.07 * ck);
    }

    // ── signature: four small agent logos ──
    const lw = 10, lh2 = 16, lsz = HAL.logoSize('interpreter', lw, lh2);
    const lgap = 60, totalW = 4 * lsz.w + 3 * lgap;
    AGENT_ORDER.forEach((k, i) => {
      const lk = range(t, 1.8 + i * 0.06, 2.1 + i * 0.06);
      if (lk <= 0) return;
      const x = W / 2 - totalW / 2 + i * (lsz.w + lgap);
      HAL.logo(g, AGENTS[k].logo, x, barY + 310, lw, lh2, { color: AGENTS[k].color, reveal: lk, mode: 'rows', seed: i + 2, T, blink: (t > 2.5 && t < 2.58) ? 1 : 0 });
      if (i < 3) text(g, '>', x + lsz.w + lgap / 2, barY + 310 + lsz.h / 2 + 7, { size: 18, color: C.line2, align: 'center', alpha: lk });
    });

    // landing glitch when the wordmark first arrives
    fx.glitch = Math.max(fx.glitch, 0.12 * Math.max(0, 1 - Math.abs(t - 0.36) / 0.08));
  },
});
