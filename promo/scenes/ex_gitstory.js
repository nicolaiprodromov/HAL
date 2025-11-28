// ex_gitstory — example 04/06: git-story.hal, the repo's life as a growing heat timeline.
HAL.scene({
  id: 'ex_gitstory',
  draw(g, t, { dur, T, fx, C }) {
    const { ease, range, clamp, lerp, hexToRgb } = HAL;
    fx.bloom = 1.25;
    const accent = '#ff8a2a';
    HAL.exampleFrame(g, t, T, { n: 4, file: 'git-story.hal', blurb: 'the life of a git repo, as an animation', accent });

    // ── deterministic repo history ──
    const N = 156, SILENT = [62, 63, 64, 65], BUSY = 118, BIG = 96;
    const v = [];
    for (let w = 0; w < N; w++) {
      let x = 14 + 0.09 * w + 12 * HAL.noise1(w * 0.11, 3) + 7 * HAL.noise1(w * 0.47, 9) + 6 * HAL.hash(w, 5);
      if (w > 108 && w < 128) x += 12;
      v.push(Math.round(clamp(x, 2, 48)));
    }
    v[0] = 3; SILENT.forEach(w => v[w] = 0); v[BUSY] = 58;
    const total = v.reduce((a, b) => a + b, 0);
    const X0 = 110, X1 = 1820, pitch = (X1 - X0) / N, BASE = 830, CELL = 13, SCALE = 0.78;
    const colX = w => X0 + w * pitch;
    const cells = w => Math.round(v[w] * SCALE);
    const topY = w => BASE - cells(w) * CELL;
    const heat = k => {
      const st = [[0, '#2a5bff'], [0.3, '#4f9bff'], [0.58, '#ffb04a'], [1, '#ff6a10']];
      k = clamp(k);
      for (let i = 1; i < st.length; i++) if (k <= st[i][0]) {
        const a = hexToRgb(st[i - 1][1]), b = hexToRgb(st[i][1]), f = (k - st[i - 1][0]) / (st[i][0] - st[i - 1][0]);
        return [lerp(a[0], b[0], f), lerp(a[1], b[1], f), lerp(a[2], b[2], f)];
      }
      return hexToRgb(st[3][1]);
    };

    // playhead position in weeks (already mid-flight at frame 1)
    const pw = lerp(62, N - 1, ease.inOutQuad(range(t, 0, 1.3)) * 0.35 + range(t, 0, 1.3) * 0.65);
    const camZ = 1 + 0.05 * ease.outQuad(t / dur);

    g.save();
    HAL.zoom(g, camZ, 960, 600);

    // faint grid + baseline
    g.fillStyle = 'rgba(255,255,255,0.045)';
    for (let r = 1; r <= 11; r++) g.fillRect(X0, BASE - r * 52, X1 - X0, 1);
    g.fillStyle = C.line2; g.fillRect(X0 - 10, BASE + 8, X1 - X0 + 20, 2);
    const months = ['JAN', 'FEB', 'MAR', 'APR', 'MAY', 'JUN', 'JUL', 'AUG', 'SEP', 'OCT', 'NOV', 'DEC'];
    const monthOf = w => { const m = Math.floor(w * 7 / 30.44) + 2; return [months[m % 12], 2021 + Math.floor(m / 12)]; };
    for (let y = 1; y < 3; y++) {
      const w = (y * 12 - 2) * 30.44 / 7;
      g.fillStyle = C.line2; g.fillRect(Math.round(colX(w)), BASE + 8, 1, 18);
      HAL.text(g, String(2021 + y), colX(w) + 8, BASE + 26, { size: 16, color: C.muted, weight: 500, spacing: 2 });
    }

    // upcoming weeks: dim ticks
    g.fillStyle = 'rgba(255,255,255,0.13)';
    for (let w = Math.floor(pw) + 1; w < N; w++) g.fillRect(Math.round(colX(w)), BASE - 3, Math.max(6, Math.floor(pitch) - 2), 3);

    // columns: stacked cells, quantised like terminal blocks
    const shown = Math.floor(pw);
    const colW = Math.max(6, Math.floor(pitch) - 2);
    for (let w = 0; w <= shown; w++) {
      const n = cells(w); if (!n) continue;
      const k = v[w] / 44;
      const grow = w === shown ? ease.outCubic(pw - shown) : 1;
      const nn = Math.max(1, Math.ceil(n * grow));
      const x = Math.round(colX(w));
      const head = clamp(1 - (pw - w) / 5); // freshly-grown columns burn hotter
      for (let c = 0; c < nn; c++) {
        const kk = clamp(k * (0.75 + 0.45 * c / Math.max(1, n)) + head * 0.12);
        const [r, gg, b] = heat(kk);
        g.fillStyle = `rgb(${r | 0},${gg | 0},${b | 0})`;
        g.fillRect(x, BASE - (c + 1) * CELL + 1, colW, CELL - 2);
      }
      // bright cap
      const [r, gg, b] = heat(clamp(k + 0.15));
      g.fillStyle = `rgb(${Math.min(255, r + 70) | 0},${Math.min(255, gg + 70) | 0},${Math.min(255, b + 70) | 0})`;
      g.fillRect(x, BASE - nn * CELL + 1, colW, 3);
      if (v[w] > 40) HAL.glow(g, x + colW / 2, BASE - nn * CELL, 70, '#ff7a1a', 0.25);
    }

    // playhead
    const phx = colX(pw) + pitch / 2;
    const gr = g.createLinearGradient(0, 260, 0, BASE + 8);
    gr.addColorStop(0, 'rgba(255,170,90,0)'); gr.addColorStop(1, 'rgba(255,190,120,0.95)');
    g.fillStyle = gr; g.fillRect(Math.round(phx) - 1, 260, 3, BASE - 252);
    HAL.glow(g, phx, BASE, 110, '#ff8a2a', 0.5);
    g.fillStyle = '#fff4e0'; g.fillRect(Math.round(phx) - 7, BASE + 2, 14, 14);
    const [mn, yr] = monthOf(pw);
    const lx = Math.min(phx, 1700);
    HAL.runs(g, [[mn + ' ', '#ffb26a', { weight: 800 }], [String(yr), C.text, { weight: 800 }]], lx, BASE + 84, { size: 46, align: 'center', spacing: 2 });
    const upTo = v.slice(0, shown + 1).reduce((a, b) => a + b, 0);

    // pop-up callouts
    const callout = (w, label, col, dx, dy, o = {}) => {
      const age = pw - w; if (age < 0) return;
      const k = ease.outBack(clamp(age / 3));
      if (k <= 0) return;
      const size = 22, wd = HAL.textWidth(label, size, 1) + 36, hh = 40;
      const ax = colX(w) + colW / 2, ay = o.ay !== undefined ? o.ay : topY(w) - 6;
      let bx = ax + dx - (o.right ? wd : o.center ? wd / 2 : 0), by = ay + dy - hh;
      g.save();
      HAL.zoom(g, 0.85 + 0.15 * k, ax + dx, by + hh);
      g.globalAlpha = clamp(k * 1.4);
      g.strokeStyle = col; g.lineWidth = 2;
      g.beginPath(); g.moveTo(ax, ay); g.lineTo(clamp(ax + dx, bx + 10, bx + wd - 10), by + hh); g.stroke();
      g.fillStyle = col; g.fillRect(ax - 4, ay - 4, 8, 8);
      g.fillStyle = 'rgba(11,13,15,0.92)'; g.fillRect(bx, by, wd, hh);
      g.strokeRect(bx + 1, by + 1, wd - 2, hh - 2);
      HAL.text(g, label, bx + 18, by + hh / 2 + 1, { size, color: '#fff', weight: 700, spacing: 1, baseline: 'middle' });
      g.restore();
      HAL.glow(g, bx + wd / 2, by + hh / 2, 90, col, 0.16 * k);
    };
    callout(0, 'first commit', '#6aa8ff', 30, -150);
    // silence bracket
    if (pw > SILENT[3] + 1) {
      const a = colX(SILENT[0]), b = colX(SILENT[3] + 1) - 3, by = BASE - 22;
      const k = ease.outCubic(clamp((pw - SILENT[3]) / 3));
      g.strokeStyle = '#6aa8ff'; g.lineWidth = 2; g.setLineDash([6, 5]);
      g.globalAlpha = k; g.beginPath(); g.moveTo(a, by - 14); g.lineTo(a, by); g.lineTo(b, by); g.lineTo(b, by - 14); g.stroke(); g.setLineDash([]); g.globalAlpha = 1;
    }
    callout(SILENT[3], '31 days of silence', '#8fb4ff', -30, -190, { ay: BASE - 36, center: true });
    callout(BIG, 'biggest commit +4,812', '#ffd27a', -30, -90, { right: true });
    callout(BUSY, 'busiest week · 58', '#ff7a1a', -50, -26, { right: true });

    // HUD: counters
    HAL.runs(g, [['COMMITS ', C.muted, { weight: 700 }], [String(upTo).replace(/\B(?=(\d{3})+(?!\d))/g, ','), '#ffb26a', { weight: 800 }]], X1 - 20, 214, { size: 30, align: 'right', spacing: 3 });
    HAL.text(g, `WEEK ${String(shown + 1).padStart(3, '0')} / ${N}`, X1 - 20, 246, { size: 18, color: C.muted, align: 'right', spacing: 3 });

    // leaderboard: top 5 authors
    const lk = ease.outExpo(range(t, 0.95, 1.2));
    if (lk > 0) {
      const names = ['mara', 'jonas', 'priya', 'dex', 'ines'], shares = [0.34, 0.23, 0.17, 0.11, 0.07];
      g.save(); g.globalAlpha = lk; g.translate((1 - lk) * -40, 0);
      const px = X0, py = 200, pwid = 430;
      g.fillStyle = 'rgba(11,13,15,0.9)'; g.fillRect(px - 16, py - 20, pwid + 32, 300);
      HAL.text(g, 'TOP AUTHORS', px, py + 6, { size: 18, color: '#ffb26a', weight: 700, spacing: 4 });
      g.fillStyle = C.line2; g.fillRect(px, py + 20, pwid, 1);
      names.forEach((nm, i) => {
        const y = py + 62 + i * 44, bk = ease.outCubic(range(t, 1.0 + i * 0.05, 1.3 + i * 0.05));
        const cnt = Math.round(total * shares[i]);
        HAL.text(g, nm, px, y, { size: 22, color: C.text, weight: 500 });
        const bw = 190 * shares[i] / shares[0] * bk;
        const [r, gg, b] = heat(1 - i * 0.2);
        g.fillStyle = `rgb(${r | 0},${gg | 0},${b | 0})`;
        for (let s = 0; s < bw; s += 14) g.fillRect(px + 90 + s, y - 18, Math.min(11, bw - s), 22);
        HAL.text(g, String(cnt).replace(/\B(?=(\d{3})+(?!\d))/g, ','), px + pwid, y, { size: 22, color: C.soft, align: 'right' });
      });
      g.restore();
    }
    // story.html hint
    const hk = ease.outCubic(range(t, 1.2, 1.4));
    if (hk > 0) HAL.runs(g, [['▸ ', accent], ['story.html', C.text, { weight: 700 }], [' written', C.muted]], X1 - 20, 990, { size: 22, align: 'right', alpha: hk });
    g.restore();
  },
});
