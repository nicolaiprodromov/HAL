// ex_aquarium — cut 1/6. Terminal aquarium as a true character grid.
HAL.scene({
  id: 'ex_aquarium',
  draw(g, t, { dur, T, fx, W, H, C }) {
    const { hash, mix, clamp, range, ease } = HAL;
    fx.bloom = 1.25;
    const accent = '#4ab4ff';
    HAL.exampleFrame(g, t, T, { n: 1, file: 'aquarium.hal', args: '20', blurb: 'a terminal aquarium', accent });

    const COLS = 110, ROWS = 24, SIZE = 26;
    const cwp = SIZE * HAL.ADV, chp = Math.round(SIZE * 1.3);
    const ox = Math.round(W / 2 - COLS * cwp / 2), oy = 172;
    const cx = ox + COLS * cwp / 2, cy = oy + ROWS * chp / 2;

    g.save();
    HAL.zoom(g, 1 + 0.05 * ease.outQuad(t / dur), cx, cy);
    const G = HAL.grid(g, ox, oy, { size: SIZE, lh: 1.3 });

    // water: deep blue, darker towards the bottom
    for (let r = 0; r < ROWS; r++) {
      const f = r / (ROWS - 1);
      G.fill(0, r, COLS, 1, mix('#0a4c94', '#020918', Math.pow(f, 0.8)), 1);
    }
    // caustic light shafts
    g.save(); g.globalCompositeOperation = 'lighter';
    for (let i = 0; i < 6; i++) {
      const x = ox + ((i * 0.19 + 0.07 + Math.sin(t * 0.9 + i) * 0.015) % 1) * COLS * cwp;
      const gr = g.createLinearGradient(0, oy, 0, oy + ROWS * chp * 0.8);
      gr.addColorStop(0, 'rgba(120,200,255,0.06)'); gr.addColorStop(1, 'rgba(120,200,255,0)');
      g.fillStyle = gr;
      g.beginPath(); g.moveTo(x - 20, oy); g.lineTo(x + 30, oy); g.lineTo(x + 110, oy + ROWS * chp * 0.8); g.lineTo(x - 40, oy + ROWS * chp * 0.8); g.fill();
    }
    g.restore();
    // surface shimmer
    for (let c = 0; c < COLS; c++) {
      const w = Math.sin(c * 0.55 + t * 9) * 0.5 + 0.5;
      G.put(c, 0, w > 0.5 ? '~' : '-', '#9fd8ff', { alpha: 0.35 + 0.35 * w });
    }

    // sand: last 2 rows
    for (let r = ROWS - 2; r < ROWS; r++) {
      G.fill(0, r, COLS, 1, r === ROWS - 2 ? '#2a1d0c' : '#1d1408', 1);
      for (let c = 0; c < COLS; c++) {
        const h = hash(c, r, 7);
        const ch = h < 0.4 ? '.' : h < 0.7 ? ':' : h < 0.85 ? '·' : '°';
        G.put(c, r, ch, h > 0.5 ? '#e8c35a' : '#9a6b32', { alpha: 0.75 + 0.25 * hash(c, r, 9) });
      }
    }

    // sprites
    const SP = {
      small: { R: ['><>'], L: ['<><'] },
      medium: { R: ['><(((°>'], L: ['<°)))><'] },
      big: {
        R: ['   ___  ', '\\/  o \\ ', '/\\____/ '],
        L: ['  ___   ', ' / o  \\/', ' \\____/\\'],
      },
      shark: {
        R: ['         /|       ', '\\_______/ |_____  ', ' >           o  \\ ', '/________vvvvv__/ '],
        L: ['       |\\         ', '  _____| \\_______/', ' /  o           < ', ' \\__vvvvv________\\'],
      },
    };
    const drawSprite = (kind, dir, col, row, color, o = {}) => {
      const lines = SP[kind][dir > 0 ? 'R' : 'L'];
      lines.forEach((ln, j) => {
        for (let i = 0; i < ln.length; i++) {
          const chh = ln[i]; if (chh === ' ') continue;
          const c = col + i, r = row + j;
          if (c < 0 || c >= COLS || r < 0 || r >= ROWS - 2) continue;
          const cc = o.pick ? o.pick(chh, i, j) : color;
          G.put(c, r, chh, cc, { weight: o.weight || 800 });
        }
      });
    };

    const sharkX = -18 + t * 88;
    const palette = ['#ff5fa2', '#ffd23f', '#3dffb5', '#ff8a3d', '#b78cff', '#5ff0ff', '#ff4d4d', '#9dff4a'];

    // seaweed (behind fish)
    for (let s = 0; s < 9; s++) {
      const bc = 5 + Math.floor(hash(s, 1) * 10) + s * 11;
      const hgt = 5 + Math.floor(hash(s, 2) * 5);
      const ph = hash(s, 3) * 6.28;
      for (let k = 0; k < hgt; k++) {
        const f = k / hgt;
        const sw = Math.round(Math.sin(t * 2.6 + ph + k * 0.55) * (0.6 + f * 2.2));
        const row = ROWS - 3 - k;
        const col = bc + sw;
        const gcol = mix('#0f8a3c', '#6dff7a', f * 0.9 + 0.1);
        G.put(col, row, (k + s) % 2 ? ')' : '(', gcol, { weight: 700 });
      }
    }

    // fish
    const kinds = ['small', 'medium', 'small', 'big', 'medium', 'small', 'medium', 'small', 'big', 'small', 'medium', 'small', 'small', 'medium', 'small', 'medium'];
    kinds.forEach((kind, i) => {
      const dir = hash(i, 11) < 0.5 ? 1 : -1;
      const sp = kind === 'small' ? 40 + hash(i, 12) * 26 : kind === 'medium' ? 20 + hash(i, 12) * 12 : 8 + hash(i, 12) * 5;
      const w = SP[kind].R[0].length;
      const span = COLS + 2 * w + 8;
      const x0 = hash(i, 13) * span;
      const row0 = 2 + Math.floor(hash(i, 14) * (ROWS - 8));
      const fx0 = (((x0 + dir * sp * t) % span) + span) % span - w - 4;
      // scatter from the shark
      const dxs = sharkX + 9 - fx0;
      const near = clamp((fx0 + 46 - sharkX) / 30) * (1 - clamp((sharkX - fx0 - 20) / 50));
      const s = clamp(near) * ease.outQuad(clamp(1));
      const flee = (hash(i, 15) < 0.5 ? -1 : 1) * s * (5 + hash(i, 16) * 4);
      const row = clamp(Math.round(row0 + flee + Math.sin(t * 5 + i) * 0.4), 1, ROWS - 5);
      const col = Math.round(fx0 + s * 8);
      const color = palette[Math.floor(hash(i, 17) * palette.length)];
      const px = ox + (col + w / 2) * cwp, py = oy + (row + 0.5) * chp;
      g.save(); g.globalCompositeOperation = 'lighter';
      HAL.glow(g, px, py, 46 + (kind === 'big' ? 34 : 0), color, 0.34);
      g.restore();
      if (kind === 'big') {
        drawSprite(kind, dir, col, row, color, { pick: (ch, i2, j) => (ch === '_' || ch === '/' || ch === '\\') ? ((i2 + j) % 2 ? color : mix(color, '#ffffff', 0.55)) : '#ffffff' });
      } else drawSprite(kind, dir, col, row, color);
    });

    // bubbles: o -> O -> ° while rising, pop at the surface
    for (let b = 0; b < 22; b++) {
      const speed = 5 + hash(b, 21) * 5;
      const life = (ROWS - 3) / speed;
      const tt = ((t + hash(b, 22) * life) % life);
      const f = tt / life;
      const row = Math.round(ROWS - 3 - tt * speed);
      if (row < 1) continue;
      const col = Math.round(4 + hash(b, 23) * (COLS - 8) + Math.sin(tt * 6 + b) * 0.9);
      const chh = f < 0.35 ? 'o' : f < 0.75 ? 'O' : '°';
      G.put(col, row, chh, '#bfe9ff', { weight: 700, alpha: 0.9 });
    }

    // shark on top
    const sRow = 9 + Math.round(Math.sin(t * 2.2) * 0.6);
    const sc = Math.round(sharkX);
    g.save(); g.globalCompositeOperation = 'lighter';
    HAL.glow(g, ox + (sc + 9) * cwp, oy + (sRow + 2) * chp, 150, '#8aa8c8', 0.12);
    g.restore();
    drawSprite('shark', 1, sc, sRow, '#c6d0dc', {
      pick: (ch, i, j) => ch === 'v' ? '#ffffff' : ch === 'o' ? '#ff4040' : '#b4c0cf',
    });

    // vignette within the tank
    const vg = g.createRadialGradient(cx, cy, 300, cx, cy, 950);
    vg.addColorStop(0, 'rgba(0,0,0,0)'); vg.addColorStop(1, 'rgba(0,0,0,0.45)');
    g.fillStyle = vg; g.fillRect(ox, oy, COLS * cwp, ROWS * chp);
    g.restore();

    // tank frame hairline
    g.strokeStyle = HAL.rgba(accent, 0.35); g.lineWidth = 1;
    g.strokeRect(ox - 8.5, oy - 8.5, COLS * cwp + 17, ROWS * chp + 17);
    HAL.text(g, 'fps 20 · 110×24', ox + COLS * cwp, oy + ROWS * chp + 26, { size: 16, color: C.muted, align: 'right' });
  },
});
