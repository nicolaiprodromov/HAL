// ex_pet — cut 3/6. Mochi plays: bouncing ASCII creature, hearts, mood bars.
HAL.scene({
  id: 'ex_pet',
  draw(g, t, { dur, T, fx, W, H, C }) {
    const { hash, mix, clamp, range, ease, rgba } = HAL;
    fx.bloom = 1.3;
    const BODY = '#ffa8cf', accent = '#ff7eb6', DEEP = '#ff4f93';
    HAL.exampleFrame(g, t, T, { n: 3, file: 'pet.hal', args: 'play', blurb: 'a tiny creature that lives between runs', accent });

    // soft stage light
    g.save(); g.globalCompositeOperation = 'lighter';
    HAL.glow(g, 600, 600, 420, accent, 0.10);
    g.restore();
    // faint big dim name
    HAL.text(g, 'MOCHI', 600, 330, { size: 150, weight: 800, color: accent, alpha: 0.05, align: 'center' });

    g.save();
    HAL.zoom(g, 1 + 0.045 * ease.outQuad(t / dur), 960, 580);

    // ── creature ──
    const ART = [
      ' /\\___/\\ ',
      '( ^   ^ )',
      '( ~ w ~ )',
      ' (     ) ',
      '  U---U  ',
    ];
    const ART_AIR = [
      ' /\\___/\\ ',
      '( >   < )',
      '( ~ o ~ )',
      ' (     ) ',
      '  )   (  ',
    ];
    const SZ = 84, cw = SZ * HAL.ADV, ch = Math.round(SZ * 0.98);
    const CX = 600, GROUND = 810;
    const ph = (t * 1.9 + 0.2) % 1;                       // bounce phase
    const hop = Math.sin(ph * Math.PI);                    // 0 on ground, 1 at apex
    const air = hop > 0.12;
    const lift = Math.pow(hop, 0.85) * 120;
    const squash = ph < 0.1 || ph > 0.9 ? 1 - 0.1 * (1 - Math.abs(ph < 0.5 ? ph : 1 - ph) / 0.1) : 1 + 0.05 * hop;
    const sx = 2 - squash > 1.15 ? 1.15 : 2 - squash;
    // shadow
    const shw = 220 * (1 - 0.35 * hop);
    g.save(); g.globalAlpha = 0.5 - 0.2 * hop;
    g.fillStyle = rgba(accent, 0.35); g.beginPath(); g.ellipse(CX, GROUND + 6, shw / 2, 12 * (1 - 0.3 * hop), 0, 0, 7); g.fill();
    g.restore();
    g.save();
    g.translate(CX, GROUND - lift); g.scale(sx, squash); g.translate(-CX, -GROUND);
    const art = air ? ART_AIR : ART;
    const x0 = CX - 9 * cw / 2;
    g.save(); g.globalCompositeOperation = 'lighter'; HAL.glow(g, CX, GROUND - 2.5 * ch, 300, BODY, 0.2); g.restore();
    art.forEach((ln, j) => {
      for (let i = 0; i < ln.length; i++) {
        const c = ln[i]; if (c === ' ') continue;
        const cheek = c === '~';
        const eye = c === '^' || c === '>' || c === '<' || c === 'w' || c === 'o';
        const col = cheek ? '#ff5a9a' : eye ? '#fff4f9' : BODY;
        HAL.text(g, c, x0 + i * cw, GROUND - (5 - j) * ch + ch * 0.5, { size: SZ, weight: 800, color: col, baseline: 'middle' });
      }
    });
    g.restore();

    // hearts floating up
    for (let k = 0; k < 9; k++) {
      const per = 1.1;
      const tt = ((t + hash(k, 1) * per) % per) / per;
      const x = CX + (k % 2 ? 1 : -1) * (215 + hash(k, 2) * 130) + Math.sin(tt * 7 + k) * 22;
      const y = GROUND - 120 - tt * 380;
      const a = Math.sin(tt * Math.PI) * 0.95;
      const sz = 26 + hash(k, 3) * 24;
      const col = k % 3 === 0 ? '#ff4f6d' : k % 3 === 1 ? '#ff7eb6' : '#ffc2dc';
      g.save(); g.globalCompositeOperation = 'lighter'; HAL.glow(g, x, y, sz * 1.6, col, 0.25 * a); g.restore();
      HAL.text(g, k % 4 === 3 ? '<3' : '♥', x, y, { size: sz, weight: 800, color: col, alpha: a, align: 'center', baseline: 'middle' });
    }
    // sparkle stars
    for (let k = 0; k < 6; k++) {
      const tt = ((t * 1.6 + hash(k, 8)) % 1);
      const x = CX + (k % 2 ? 1 : -1) * (210 + hash(k, 9) * 120), y = GROUND - 120 - hash(k, 10) * 260;
      HAL.text(g, tt < 0.5 ? '+' : '*', x, y, { size: 24, color: '#fff2a8', alpha: Math.sin(tt * Math.PI), align: 'center', baseline: 'middle' });
    }
    // thought line
    HAL.text(g, '“zoomies!!”', CX, 900, { size: 36, weight: 700, color: '#fff4f9', align: 'center', alpha: 0.95 });
    g.restore();

    // ── status panel ──
    const PX = 1130, PY = 330;
    HAL.text(g, 'MOCHI', PX, PY - 62, { size: 40, weight: 800, color: BODY, spacing: 8 });
    HAL.text(g, 'play', PX + 270, PY - 54, { size: 22, color: accent, weight: 700, spacing: 3 });
    const bars = [
      ['hunger', 42, 42, '#ffb347'],
      ['energy', 71, 56, '#5fd0ff'],
      ['happiness', 55, 80, '#ff5fa2'],
    ];
    const BW = 24;
    const G = HAL.grid(g, PX, PY, { size: 30, lh: 1.0 });
    bars.forEach(([name, a, b, col], i) => {
      const row = i * 2.6;
      const k = ease.outCubic(range(t, 0.15 + i * 0.05, 1.2));
      const v = HAL.lerp(a, b, k);
      const filled = Math.round(v / 100 * BW);
      HAL.text(g, name.toUpperCase(), PX, PY + row * 36 - 8, { size: 16, weight: 700, color: C.muted, spacing: 3 });
      const by = row * 36 + 18;
      for (let c = 0; c < BW; c++) {
        const on = c < filled;
        HAL.text(g, on ? '█' : '░', PX + c * G.cw, PY + by + 15, { size: 30, color: on ? col : '#3a3f47', weight: 700, baseline: 'middle' });
      }
      HAL.text(g, String(Math.round(v)).padStart(3, ' '), PX + BW * G.cw + 14, PY + by + 15, { size: 30, weight: 800, color: C.text, baseline: 'middle' });
      if (b !== a) {
        const d = b - a, pop = ease.outBack(range(t, 0.35 + i * 0.08, 0.7));
        g.save(); g.translate(PX + BW * G.cw + 120, PY + by + 15); g.scale(pop, pop);
        HAL.text(g, (d > 0 ? '+' : '−') + Math.abs(d), 0, 0, { size: 30, weight: 800, color: d > 0 ? '#84e2b0' : '#ff7a7a', baseline: 'middle' });
        g.restore();
      }
    });
    // glow on happiness bar head
    const hv = HAL.lerp(55, 80, ease.outCubic(range(t, 0.25, 1.2)));
    g.save(); g.globalCompositeOperation = 'lighter';
    HAL.glow(g, PX + hv / 100 * BW * G.cw, PY + 2 * 2.6 * 36 + 33, 60, '#ff5fa2', 0.4 + 0.2 * Math.sin(t * 14));
    g.restore();

    // closing line
    const cl = 'Mochi is 3 days old, visit #12.';
    const shown = HAL.typed(cl, t - 0.35, 80);
    HAL.text(g, '> ', PX, 740, { size: 30, color: C.muted, weight: 500 });
    const w = HAL.text(g, shown, PX + 34, 740, { size: 30, color: C.text, weight: 600 });
    HAL.cursor(g, PX + 34 + w + 4, 740, 30, T, accent, 3);
  },
});
