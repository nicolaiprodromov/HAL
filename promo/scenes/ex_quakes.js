// ex_quakes — cut 2/6. Spinning dotted globe, pulsing quakes, live terminal log.
HAL.scene({
  id: 'ex_quakes',
  draw(g, t, { dur, T, fx, W, H, C }) {
    const { hash, mix, clamp, range, ease, rgba } = HAL;
    fx.bloom = 1.35;
    const accent = '#2fe0c8';
    HAL.exampleFrame(g, t, T, { n: 2, file: 'quakes.hal', blurb: 'a live 3D globe of the world’s earthquakes', accent });

    const D = Math.PI / 180;
    const hex = s => '#' + s.match(/\d+/g).map(n => (+n).toString(16).padStart(2, '0')).join('');
    const magCol = m => {
      const f = clamp((m - 2) / 4.2);
      return hex(f < 0.5 ? mix('#27e6c9', '#ffd04a', f * 2) : mix('#ffd04a', '#ff3b30', (f - 0.5) * 2));
    };

    // ── stars (behind, no zoom) ──
    for (let i = 0; i < 140; i++) {
      const x = 64 + hash(i, 1) * 1792, y = 156 + hash(i, 2) * 840;
      const tw = 0.4 + 0.6 * Math.abs(Math.sin(t * (1 + hash(i, 3) * 3) + i));
      g.fillStyle = rgba('#cfe8ff', 0.18 + 0.5 * tw * hash(i, 4));
      const s = hash(i, 5) > 0.9 ? 3 : 2;
      g.fillRect(Math.round(x), Math.round(y), s, s);
    }

    // ── globe geometry ──
    const R = 350, GX = 1240, GY = 585;
    g.save();
    HAL.zoom(g, 1 + 0.06 * ease.outQuad(t / dur), GX, GY);
    const lonc = 55 + t * 52, tilt = 22 * D;
    const proj = (lat, lon, rad = 1) => {
      const ph = lat * D, la = (lon - lonc) * D;
      const x = Math.cos(ph) * Math.sin(la), y0 = Math.sin(ph), z0 = Math.cos(ph) * Math.cos(la);
      const y = y0 * Math.cos(tilt) - z0 * Math.sin(tilt);
      const z = y0 * Math.sin(tilt) + z0 * Math.cos(tilt);
      return [GX + R * rad * x, GY - R * rad * y, z];
    };

    // atmosphere + disc
    g.save(); g.globalCompositeOperation = 'lighter';
    HAL.glow(g, GX, GY, R * 1.45, '#139e9a', 0.22);
    g.restore();
    const dg = g.createRadialGradient(GX - 90, GY - 110, 20, GX, GY, R);
    dg.addColorStop(0, '#0c2a33'); dg.addColorStop(1, '#04090d');
    g.fillStyle = dg; g.beginPath(); g.arc(GX, GY, R, 0, 7); g.fill();
    g.strokeStyle = rgba('#47f0da', 0.55); g.lineWidth = 2; g.beginPath(); g.arc(GX, GY, R + 1, 0, 7); g.stroke();

    // rough continents: [lat, lon, rLat, rLon]
    const LAND = [
      [50, -105, 22, 38], [35, -100, 12, 24], [18, -98, 9, 10], [-12, -60, 20, 15], [-35, -65, 16, 8], [-3, -65, 12, 14],
      [72, -40, 10, 22], [52, 10, 10, 22], [60, 20, 10, 14], [20, 15, 18, 28], [0, 22, 14, 14], [-20, 25, 14, 14],
      [55, 70, 16, 45], [58, 110, 12, 40], [35, 95, 15, 35], [22, 78, 12, 12], [30, 135, 8, 6], [2, 105, 8, 12],
      [-25, 134, 12, 18], [-80, 0, 10, 180], [65, -150, 8, 14], [-6, 118, 4, 14],
    ];
    const isLand = (lat, lon) => {
      for (const [a, o, ra, ro] of LAND) {
        let dl = Math.abs(((lon - o + 540) % 360) - 180);
        const q = ((lat - a) / ra) ** 2 + (dl / ro) ** 2;
        if (q < 1) return hash(Math.round(lat * 2), Math.round(lon * 2), 3) > q * 0.3;
      }
      return false;
    };

    // dot sphere, batched by two styles
    const STEP = 3.4;
    const rows = [];
    for (let lat = -84; lat <= 84; lat += STEP) {
      const cs = Math.cos(lat * D);
      const n = Math.max(1, Math.round(360 / STEP * cs));
      for (let k = 0; k < n; k++) {
        const lon = -180 + (k + (Math.round(lat / STEP) % 2) * 0.5) * 360 / n;
        const [x, y, z] = proj(lat, lon);
        if (z <= 0.02) continue;
        const land = isLand(lat, lon);
        const a = Math.pow(z, 0.7);
        rows.push([x, y, z, land, a]);
      }
    }
    for (const [x, y, z, land, a] of rows) {
      if (land) {
        g.fillStyle = rgba('#b6fff0', 0.45 + 0.55 * a);
        const s = 2.6 + z * 2.6; g.fillRect(Math.round(x - s / 2), Math.round(y - s / 2), Math.round(s), Math.round(s));
      } else {
        g.fillStyle = rgba('#1f6f78', 0.10 + 0.4 * a);
        const s = 1.4 + z * 1.0; g.fillRect(Math.round(x - s / 2), Math.round(y - s / 2), Math.round(s), Math.round(s));
      }
    }
    // faint graticule on the limb
    g.strokeStyle = rgba('#47f0da', 0.0); g.lineWidth = 1;
    for (let lat = -60; lat <= 60; lat += 30) {
      g.beginPath(); let pen = false;
      for (let lon = -180; lon <= 180; lon += 4) { const [x, y, z] = proj(lat, lon); if (z > 0) { pen ? g.lineTo(x, y) : g.moveTo(x, y); pen = true; } else pen = false; }
      g.stroke();
    }

    // ── quakes ──
    const HOT = [[36, 140], [-5, 125], [14, 122], [-20, -70], [38, -122], [61, -150], [-33, -72], [28, 85], [37, 38], [-10, 160], [18, -100], [10, -85], [39, 20], [-2, 100], [51, 178]];
    const quakes = [];
    for (let i = 0; i < 80; i++) {
      const h = HOT[Math.floor(hash(i, 31) * HOT.length)];
      const lat = h[0] + (hash(i, 32) - 0.5) * 16, lon = h[1] + (hash(i, 33) - 0.5) * 22;
      const m = 1.8 + Math.pow(hash(i, 34), 2.6) * 3.9;
      quakes.push({ lat, lon, m, ph: hash(i, 35) * 6.28 });
    }
    const BIG = { lat: 3, lon: 98, m: 6.1, ph: 0 };
    quakes.push(BIG);
    const qa = range(t, 0, 0.12);
    for (const q of quakes) {
      const [x, y, z] = proj(q.lat, q.lon);
      if (z <= 0.04) continue;
      const col = magCol(q.m);
      const isBig = q === BIG;
      const appear = isBig ? ease.outBack(range(t, 0.25, 0.5)) : 1;
      const pulse = 0.5 + 0.5 * Math.sin(t * (4 + q.m) + q.ph);
      const r = (3 + (q.m - 1.8) * 2.6) * (0.8 + 0.35 * pulse) * appear * (0.5 + 0.5 * z);
      g.save(); g.globalCompositeOperation = 'lighter';
      HAL.glow(g, x, y, r * 5, col, 0.6 * z * appear);
      g.restore();
      g.fillStyle = mix(col, '#ffffff', 0.2); g.beginPath(); g.arc(x, y, Math.max(2, r * 0.6), 0, 7); g.fill();
      g.strokeStyle = rgba(col, 0.8 * z); g.lineWidth = 1.5; g.beginPath(); g.arc(x, y, r, 0, 7); g.stroke();
    }
    // ripple rings across the surface for the new big quake
    for (let k = 0; k < 4; k++) {
      const rt = (t - 0.28 - k * 0.38);
      if (rt < 0) continue;
      const f = rt / 1.1; if (f > 1) continue;
      const ang = (3 + f * 30) * D;
      g.strokeStyle = rgba('#ff4a3d', 0.9 * (1 - f)); g.lineWidth = 3 * (1 - f) + 1;
      g.beginPath(); let pen = false;
      for (let b = 0; b <= 72; b++) {
        const br = b / 72 * 6.283;
        const lat = Math.asin(Math.sin(BIG.lat * D) * Math.cos(ang) + Math.cos(BIG.lat * D) * Math.sin(ang) * Math.cos(br));
        const lon = BIG.lon * D + Math.atan2(Math.sin(br) * Math.sin(ang) * Math.cos(BIG.lat * D), Math.cos(ang) - Math.sin(BIG.lat * D) * Math.sin(lat));
        const [x, y, z] = proj(lat / D, lon / D);
        if (z > 0) { pen ? g.lineTo(x, y) : g.moveTo(x, y); pen = true; } else pen = false;
      }
      g.stroke();
    }
    // callout for the big one
    {
      const [bx, by, bz] = proj(BIG.lat, BIG.lon);
      const k = ease.outExpo(range(t, 0.4, 0.7));
      if (bz > 0.1 && k > 0) {
        g.strokeStyle = rgba('#ff6a5e', 0.9 * k); g.lineWidth = 1.5;
        g.beginPath(); g.moveTo(bx + 14, by - 14); g.lineTo(bx + 70, by - 70); g.lineTo(bx + 70 + 150 * k, by - 70); g.stroke();
        HAL.text(g, 'M6.1', bx + 78, by - 82, { size: 30, weight: 800, color: '#ff6a5e', alpha: k });
        HAL.text(g, 'NEW', bx + 160, by - 82, { size: 18, weight: 700, color: C.text, spacing: 4, alpha: k });
      }
    }
    g.restore();

    // ── terminal log (left) ──
    const LX = 100, LY = 215;
    const places = ['Hualien, Taiwan', 'Ridgecrest, CA', 'Tokyo, Japan', 'Kermadec Islands', 'Reykjanes, Iceland', 'Atacama, Chile', 'Mindanao, Philippines', 'Hawaii, HI', 'Anchorage, AK', 'Banda Sea', 'Sumatra, Indonesia', 'Oaxaca, Mexico', 'Izmir, Turkey', 'Fiji region', 'The Geysers, CA', 'Peru-Bolivia border', 'Kuril Islands', 'Nepal'];
    const dirs = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW'];
    const LOG = 20;
    const nLines = 22 + Math.floor(t * 14);   // already mid-stream when the cut lands
    const G = HAL.grid(g, LX, LY, { size: 20, lh: 1.55 });
    HAL.text(g, 'QUAKE FEED', LX, LY - 30, { size: 16, weight: 700, color: accent, spacing: 4 });
    HAL.text(g, 'stdout', LX + 190, LY - 30, { size: 16, color: C.muted });
    for (let j = 0; j < LOG; j++) {
      const idx = nLines - LOG + j;
      if (idx < 0) continue;
      const m = idx === nLines - 3 && t > 0.3 ? 6.1 : 1.6 + Math.pow(hash(idx, 41), 2.2) * 4.2;
      const mm = Math.min(m, 6.1);
      const col = magCol(mm);
      const clk = 480 + idx * 11 + Math.floor(hash(idx, 42) * 8);
      const hh = String(Math.floor(clk / 60) % 24).padStart(2, '0');
      const mi = String(clk % 60).padStart(2, '0');
      const ss = String(Math.floor(hash(idx, 43) * 60)).padStart(2, '0');
      const km = 2 + Math.floor(hash(idx, 44) * 60);
      const isB = m === 6.1 && idx === nLines - 3;
      const place = isB ? '34km SW of Sumatra, Indonesia' : `${km}km ${dirs[Math.floor(hash(idx, 45) * 8)]} of ${places[Math.floor(hash(idx, 46) * places.length)]}`;
      const age = j / LOG;
      const a = (0.5 + 0.5 * age) * (j === LOG - 1 ? ease.outCubic(range((t * 14) % 1, 0, 0.5) + (t < 0.05 ? 1 : 0)) : 1);
      const big = mm >= 6;
      if (big) G.fill(-0.5, j - 0.04, 40, 1.0, '#ff3b30', 0.18);
      G.put(0, j, `${hh}:${mi}:${ss}`, C.muted, { alpha: a });
      G.put(9.8, j, 'M' + mm.toFixed(1), col, { weight: 800, alpha: a });
      G.put(15, j, place.slice(0, 30), mix(C.soft, col, 0.35), { alpha: a, weight: big ? 700 : 400 });
    }
    HAL.text(g, '▌', LX, LY + LOG * 31 + 2, { size: 19, color: accent, alpha: 0.5 + 0.5 * Math.sin(T * 12) });
    HAL.text(g, '[hal] serving http://localhost:8080', LX, 960, { size: 20, color: accent, weight: 600 });

    // ── stats panel (top-right) ──
    const px = 1530, py = 190, pw = 300, ph2 = 148;
    HAL.panel(g, px, py, pw, ph2, { color: accent, fill: 'rgba(8,16,20,0.82)', title: 'QUAKES · 24H', titleColor: accent });
    HAL.runs(g, [['quakes today ', C.muted], ['241', C.text, { weight: 800 }]], px + 18, py + 42, { size: 18 });
    HAL.runs(g, [['strongest ', C.muted], ['M6.1', '#ff5a4d', { weight: 800 }]], px + 18, py + 76, { size: 18 });
    HAL.runs(g, [['updated ', C.muted], ['12s ago', C.soft]], px + 18, py + 110, { size: 18 });

    // url pill, bottom of globe
    HAL.text(g, 'http://localhost:8080', GX, 975, { size: 24, weight: 700, color: C.text, align: 'center', alpha: 0.95 });
    // legend
    for (let i = 0; i < 24; i++) { g.fillStyle = magCol(2 + i / 23 * 4.2); g.fillRect(1530 + i * 12.5, 345, 12, 6); }
    HAL.text(g, 'M2', 1530, 372, { size: 15, color: C.muted });
    HAL.text(g, 'M6+', 1830, 372, { size: 15, color: C.muted, align: 'right' });
  },
});
