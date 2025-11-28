// source — examples/fizzbuzz.hal in an editor pane + kinetic caption "WRITE WHAT YOU MEAN."
HAL.scene({
  id: 'source',
  draw(g, t, { dur, T, fx, W, H, C }) {
    const { ease, range, hash, text, textWidth, rgba } = HAL;
    fx.bloom = 1.15;

    const out = ease.inCubic(range(t, 1.5, 1.8));      // momentum toward the Interpreter cut
    const cam = 1.0 + 0.05 * range(t, 0, 1.8) + 0.22 * out;
    g.translate(HAL.noise1(T * 1.4, 2) * 6, HAL.noise1(T * 1.2, 7) * 4);
    HAL.zoom(g, cam, 560, H / 2);

    // ── ghosted snippets behind ──
    const ghosts = [
      ['Make something calming to watch in the terminal', 1180, 168],
      ['for about a minute. Surprise me.', 1180, 198],
      ['Show a live map of the last 24h of earthquakes', 1110, 935],
      ['and keep a pet alive across runs.', 1110, 965],
      ['Turn my git history into a story.', 100, 930],
    ];
    ghosts.forEach(([s, x, y], i) => {
      const k = range(t, 0.05 + i * 0.05, 0.4);
      text(g, HAL.typed(s, t - 0.05 - i * 0.05, 120), x - 60 * (1 - ease.outExpo(k)), y, { size: 22, color: C.line2, alpha: 0.7 });
    });
    // faint horizontal rules
    g.fillStyle = rgba(C.line, 0.6);
    for (let y = 108; y < H; y += 108) g.fillRect(0, y, W, 1);

    // ── editor pane ──
    const px = 110, py = 150, pw = 930, ph = 700;
    const pk = ease.outExpo(range(t, 0, 0.22));
    g.save();
    g.globalAlpha = pk;
    g.translate(0, (1 - pk) * 30);
    g.fillStyle = 'rgba(18,22,26,0.88)';
    g.fillRect(px, py, pw, ph);
    HAL.panel(g, px, py, pw, ph, { color: C.line2, radius: 8, title: 'examples/fizzbuzz.hal', titleColor: C.interp, fill: null });
    g.restore();

    const file = [
      '# fizzbuzz.hal — a canonical smoke test.',
      '',
      'Print the numbers 1 through 30, one per line, except:',
      '',
      '- multiples of 3 print `Fizz`',
      '- multiples of 5 print `Buzz`',
      '- multiples of both print `FizzBuzz`',
      '',
      'Exit 0. No arguments, no input, no network.',
    ];
    const sz = 24, lh = 60, tx = px + 100, ty0 = py + 96;
    const cw = sz * 0.6;
    const tok = line => {
      if (line.startsWith('#')) return [...line].map(ch => ({ ch, col: C.muted }));
      const res = [];
      line.split(/(`[^`]+`|\d+)/).forEach(seg => {
        if (!seg) return;
        if (seg[0] === '`') {
          res.push({ ch: '`', col: C.muted });
          for (const ch of seg.slice(1, -1)) res.push({ ch, col: C.cond, w: 700, hot: 1 });
          res.push({ ch: '`', col: C.muted });
        } else if (/^\d+$/.test(seg)) for (const ch of seg) res.push({ ch, col: C.interp, w: 700 });
        else for (const ch of seg) res.push({ ch, col: ch === '-' && res.length === 0 ? C.muted : C.soft });
      });
      return res;
    };

    // scan bar sweeping the lines
    const sk = range(t, 0.28, 0.85);
    if (sk > 0 && sk < 1) {
      const sy = ty0 - lh * 0.65 + (ease.inOutQuad(sk)) * (file.length * lh);
      g.save(); g.globalCompositeOperation = 'lighter';
      const gr = g.createLinearGradient(0, sy - 50, 0, sy + 6);
      gr.addColorStop(0, rgba(C.interp, 0)); gr.addColorStop(1, rgba(C.interp, 0.2));
      g.fillStyle = gr; g.fillRect(px + 2, sy - 50, pw - 4, 56);
      g.fillStyle = rgba('#d8fffb', 0.9); g.fillRect(px + 2, sy + 4, pw - 4, 2);
      g.restore();
    }

    file.forEach((line, i) => {
      const t0 = 0.08 + i * 0.045;
      const p = range(t, t0, t0 + 0.2);
      const y = ty0 + i * lh;
      const na = range(t, t0, t0 + 0.1);
      text(g, String(i + 1).padStart(2, ' '), px + 36, y, { size: 22, color: C.line2, alpha: na, weight: 500 });
      if (!line) return;
      const toks = tok(line), n = toks.length;
      const hot = toks.some(k => k.hot);
      toks.forEach((k, j) => {
        const th = j / n * 0.7;
        const gl = 'abcdef0123456789<>/\\|_-=+*#%';
        if (p >= th + 0.3) {
          text(g, k.ch, tx + j * cw, y, { size: sz, color: k.col, weight: k.w || 400, alpha: i === 0 ? 0.9 : 1 });
        } else if (p > th) {
          text(g, gl[Math.floor(hash(j, i, Math.floor(T * 30)) * gl.length)], tx + j * cw, y, { size: sz, color: C.interp, alpha: 0.8 });
        }
      });
      // amber highlight bars behind the key words, pulsing after reveal
      if (hot && p >= 1) {
        const s = line.indexOf('`') + 1, e = line.lastIndexOf('`');
        const pulse = 0.12 + 0.1 * Math.max(0, 1 - (t - (t0 + 0.2) - 0.1 * i) * 3);
        g.fillStyle = rgba(C.cond, pulse);
        g.fillRect(tx + s * cw - 6, y - sz * 0.85, (e - s) * cw + 12, sz * 1.15);
        const wk = ease.outExpo(range(t, t0 + 0.25, t0 + 0.5));
        g.fillStyle = C.cond; g.fillRect(tx + s * cw - 6, y + 10, ((e - s) * cw + 12) * wk, 2);
      }
    });
    // blinking cursor at end of last line
    HAL.cursor(g, tx + file[8].length * cw + 6, ty0 + 8 * lh, sz, T, C.interp, 3);
    // status bar
    g.fillStyle = rgba(C.line, 0.9); g.fillRect(px + 2, py + ph - 52, pw - 4, 1);
    text(g, 'NORMAL', px + 30, py + ph - 20, { size: 17, color: C.interp, weight: 700, spacing: 4 });
    text(g, 'plain english · 9 lines · no code', px + pw - 30, py + ph - 20, { size: 17, color: C.muted, align: 'right' });

    // ── caption: WRITE / WHAT YOU / MEAN. ──
    const cx = 1110, size = 104;
    const words = [['WRITE', C.text], ['WHAT YOU', C.text], ['MEAN.', C.interp]];
    words.forEach(([w, col], i) => {
      const t0 = 0.16 + i * 0.13;
      const k = ease.outExpo(range(t, t0, t0 + 0.3));
      if (k <= 0) return;
      const y = 380 + i * 142;
      g.save();
      g.beginPath(); g.rect(cx - 10, y - size, 800, size * 1.25); g.clip();
      const slide = (1 - k) * 160;
      const str = k < 1 ? HAL.scramble(w, range(t, t0, t0 + 0.2), i + 5, T) : w;
      text(g, str, cx - slide * 0.2, y + (1 - k) * size, { size, color: col, weight: 800, spacing: 10 });
      g.restore();
      if (i === 2) {
        HAL.glow(g, cx + 160, y - 40, 260, C.interp, 0.1 * k);
      }
    });
    // small tag line under caption
    text(g, 'NO SYNTAX.  NO CONFIG.  JUST INTENT.', cx, 830, { size: 20, color: C.muted, weight: 700, spacing: 6, alpha: range(t, 0.7, 0.95) });
    const bk = ease.outExpo(range(t, 0.6, 1.0));
    g.fillStyle = C.interp; g.fillRect(cx, 775, 140 * bk, 4);

    // ── exit: cyan sweep ──
    if (out > 0) {
      g.save(); g.globalCompositeOperation = 'lighter';
      const sx = -400 + out * (W + 900);
      const gr = g.createLinearGradient(sx - 420, 0, sx, 0);
      gr.addColorStop(0, rgba(C.interp, 0)); gr.addColorStop(0.85, rgba(C.interp, 0.5)); gr.addColorStop(1, 'rgba(235,255,253,0.95)');
      g.fillStyle = gr; g.fillRect(sx - 420, -50, 420, H + 100);
      g.restore();
      fx.aberration = 1 + 2.5 * out;
      fx.glitch = 0.15 * out;
      fx.flash = 0.15 * out * out;
    }
  },
});
