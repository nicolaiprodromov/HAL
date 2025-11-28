// ex_synth — example 05/06: synth.hal, a scrolling piano roll of the song being rendered.
HAL.scene({
  id: 'ex_synth',
  draw(g, t, { dur, T, fx, C }) {
    const { ease, range, clamp, lerp } = HAL;
    fx.bloom = 1.3;
    const accent = '#c58bff';
    HAL.exampleFrame(g, t, T, { n: 5, file: 'synth.hal', args: 'song', blurb: 'a 16-bar song, synthesized from scratch', accent });

    const INST = {
      pad: '#a77bff', bass: '#ffb23e', lead: '#58f0d2', kick: '#ff4f93', snare: '#ff9ac4', hat: '#ffe3f0',
    };
    // ── the score ──
    const rows = ['E5', 'D5', 'C5', 'B4', 'A4', 'G4', 'F4', 'E4', 'D4', 'C4', 'B3', 'A3', 'G3', 'C3', 'A2', 'G2', 'F2', 'hat', 'snr', 'kck'];
    const ri = n => rows.indexOf(n);
    const chordOf = b => b < 12 ? ['Am', 'F', 'C', 'G'][b % 4] : ['Am', 'F', 'G', 'Am'][b - 12];
    const PAD = { Am: ['A3', 'C4', 'E4'], F: ['A3', 'C4', 'F4'], C: ['G3', 'C4', 'E4'], G: ['G3', 'B3', 'D4'] };
    const ROOT = { Am: 'A2', F: 'F2', C: 'C3', G: 'G2' };
    const LEAD = {
      Am: [['E5', 0, 1], ['C5', 1, .5], ['D5', 1.5, .5], ['E5', 2, 1], ['A4', 3, 1]],
      F: [['C5', 0, 1], ['A4', 1, .5], ['C5', 1.5, .5], ['D5', 2, 1], ['C5', 3, 1]],
      C: [['E5', 0, 1.5], ['D5', 1.5, .5], ['C5', 2, 1], ['G4', 3, 1]],
      G: [['D5', 0, 1], ['B4', 1, 1], ['G4', 2, 1], ['B4', 3, .5], ['D5', 3.5, .5]],
    };
    const notes = []; // [startBar(float), lenBars, row, inst]
    for (let b = 0; b < 16; b++) {
      const ch = chordOf(b), next = chordOf(Math.min(15, b + 1));
      PAD[ch].forEach(n => notes.push([b + 0.02, 0.96, ri(n), 'pad']));
      notes.push([b, 0.42, ri(ROOT[ch]), 'bass'], [b + 0.5, 0.42, ri(ROOT[ch]), 'bass']);
      if (b < 15) notes.push([b + 0.9, 0.1, ri(ROOT[next]), 'bass']);
      if (b >= 4) {
        (b === 15 ? [['A4', 0, 4]] : LEAD[ch]).forEach(([n, s, l]) => notes.push([b + s / 4, l / 4 * 0.92, ri(n), 'lead']));
      }
      if (b >= 4 && b < 15) for (let q = 0; q < 4; q++) {
        notes.push([b + q / 4, 0.06, ri(q % 2 ? 'snr' : 'kck'), q % 2 ? 'snare' : 'kick']);
        notes.push([b + q / 4, 0.05, ri('hat'), 'hat'], [b + q / 4 + 0.125 + 0.02, 0.05, ri('hat'), 'hat']);
      }
    }

    // ── layout ──
    const LX = 190, RW = 1640, VIS = 8, BARW = RW / VIS;
    const TOP = 296, RH = 28, ROLL_B = TOP + rows.length * RH;
    const pb = lerp(4.5, 16, ease.outQuad(range(t, 0, 1.4)) * 0.4 + range(t, 0, 1.4) * 0.6);
    const L = pb - 6.7;
    const bx = b => LX + (b - L) * BARW;
    const zk = 1 + 0.05 * ease.outQuad(t / dur);

    g.save();
    HAL.zoom(g, zk, 960, 600);

    // chord labels per bar
    g.save(); g.beginPath(); g.rect(LX - 4, 200, RW + 8, 90); g.clip();
    for (let b = Math.floor(L) - 1; b < L + VIS + 2; b++) {
      if (b < 0 || b > 15) continue;
      const x = bx(b), cur = pb >= b && pb < b + 1, ch = chordOf(b);
      const col = cur ? '#ffffff' : C.muted;
      HAL.text(g, ch, x + 18, 254, { size: cur ? 54 : 44, weight: 800, color: col, spacing: 1 });
      HAL.text(g, 'BAR ' + (b + 1), x + 18, 278, { size: 15, weight: 700, color: cur ? accent : C.line2, spacing: 3 });
      if (b >= 12) HAL.text(g, 'ENDING', x + BARW - 18, 278, { size: 15, weight: 700, color: '#ff4f93', spacing: 3, align: 'right' });
    }
    g.restore();

    // pitch rows
    g.save(); g.beginPath(); g.rect(LX, TOP - 2, RW, rows.length * RH + 4); g.clip();
    rows.forEach((n, i) => {
      const y = TOP + i * RH, black = /#/.test(n);
      if (i % 2 === 0) { g.fillStyle = 'rgba(255,255,255,0.025)'; g.fillRect(LX, y, RW, RH); }
      if (n[0] === 'A' && n.length === 2) { g.fillStyle = 'rgba(197,139,255,0.09)'; g.fillRect(LX, y, RW, RH); }
    });
    // bar + beat lines
    for (let b = Math.floor(L); b < L + VIS + 2; b++) {
      for (let q = 0; q < 4; q++) {
        g.fillStyle = q ? 'rgba(255,255,255,0.05)' : 'rgba(255,255,255,0.22)';
        g.fillRect(Math.round(bx(b + q / 4)), TOP, 1, rows.length * RH);
      }
    }
    // notes (drawn only up to the playhead, like the real program)
    for (const [s, l, r, inst] of notes) {
      const x0 = bx(s), x1 = bx(s + l);
      if (x1 < LX - 5 || x0 > LX + RW || s > pb) continue;
      const xe = Math.min(x1, bx(pb));
      const y = TOP + r * RH + 3, hh = RH - 6, col = INST[inst];
      const age = clamp((pb - s) * 6);
      const hot = clamp(1 - (pb - s - l) * 5) * (s <= pb ? 1 : 0); // brightness while sounding / just played
      g.save();
      g.globalAlpha = (inst === 'pad' ? 0.62 : 0.95) * (0.55 + 0.45 * age);
      g.fillStyle = col;
      if (inst === 'pad') {
        const gr = g.createLinearGradient(x0, 0, xe, 0);
        gr.addColorStop(0, HAL.rgba(col, 0.2)); gr.addColorStop(0.18, HAL.rgba(col, 1)); gr.addColorStop(0.85, HAL.rgba(col, 1)); gr.addColorStop(1, HAL.rgba(col, 0.25));
        g.fillStyle = gr;
      }
      g.fillRect(Math.round(x0), y, Math.max(3, Math.round(xe - x0) - 2), hh);
      if (inst !== 'pad' && inst !== 'hat') { g.fillStyle = '#fff'; g.globalAlpha = 0.45 * hot; g.fillRect(Math.round(x0), y, Math.max(3, Math.round(xe - x0) - 2), hh); }
      g.restore();
    }
    g.restore();

    // row labels
    rows.forEach((n, i) => {
      const y = TOP + i * RH + RH / 2 + 1;
      const drum = n === 'hat' || n === 'snr' || n === 'kck';
      const col = drum ? INST[n === 'hat' ? 'hat' : n === 'snr' ? 'snare' : 'kick'] : (n[0] === 'A' ? accent : C.muted);
      HAL.text(g, n === 'hat' ? 'HAT' : n === 'snr' ? 'SNR' : n === 'kck' ? 'KCK' : n, LX - 18, y, { size: 17, weight: n[0] === 'A' ? 700 : 500, color: col, align: 'right', baseline: 'middle', spacing: drum ? 1 : 0 });
    });
    g.fillStyle = C.line2; g.fillRect(LX, ROLL_B, RW, 1);
    g.fillStyle = 'rgba(255,255,255,0.12)'; g.fillRect(LX, TOP + 17 * RH - 3, RW, 1);

    // playhead
    const px = Math.round(bx(pb));
    const gr = g.createLinearGradient(0, TOP - 30, 0, ROLL_B);
    gr.addColorStop(0, 'rgba(255,255,255,0)'); gr.addColorStop(0.15, 'rgba(255,255,255,0.95)'); gr.addColorStop(1, 'rgba(197,139,255,0.8)');
    g.fillStyle = gr; g.fillRect(px - 1, TOP - 30, 3, ROLL_B - TOP + 30);
    HAL.glow(g, px, TOP + 200, 220, accent, 0.28);

    // legend
    const leg = [['PAD', 'pad'], ['BASS', 'bass'], ['LEAD', 'lead'], ['DRUMS', 'kick']];
    let lx = LX + RW - 40;
    for (let i = leg.length - 1; i >= 0; i--) {
      const [nm, k] = leg[i], w = HAL.textWidth(nm, 17, 3);
      lx -= w; HAL.text(g, nm, lx, 192, { size: 17, weight: 700, color: C.soft, spacing: 3 });
      lx -= 24; g.fillStyle = INST[k]; g.fillRect(lx, 178, 16, 16); lx -= 30;
    }
    HAL.text(g, 'A MINOR · 92 BPM', LX, 192, { size: 17, weight: 700, color: C.muted, spacing: 3 });

    // stereo waveform strip → final print
    const WY = ROLL_B + 52, WH = 56;
    const fin = ease.outExpo(range(t, 1.0, 1.25));
    const amp = x => {
      const b = L + (x - LX) / BARW;
      const beat = (b * 4) % 1, env = 0.35 + 0.65 * Math.pow(1 - beat, 2.2);
      const base = b < 4 ? 0.28 : 0.7;
      return clamp((base * env + 0.15) * (0.7 + 0.3 * HAL.noise1(b * 16, 4)) * (b > 15 ? Math.max(0, 1 - (b - 15) * 1.2) : 1) * (b < 0 ? 0 : 1));
    };
    g.save(); g.globalAlpha = 1 - fin * 0.85;
    for (let x = LX; x < LX + RW; x += 4) {
      if (x > px) break;
      const aL = amp(x) * WH / 2, aR = amp(x + 1.7) * WH / 2 * 0.95;
      const near = clamp(1 - (px - x) / 700);
      g.fillStyle = HAL.mix('#7a56d8', '#ff7ac8', near);
      g.fillRect(x, WY - aL, 3, aL); g.fillRect(x, WY + 4, 3, aR);
    }
    g.fillStyle = C.line2; g.fillRect(LX, WY + 1, RW, 1);
    HAL.text(g, 'L', LX - 18, WY - 6, { size: 15, color: C.muted, align: 'right' });
    HAL.text(g, 'R', LX - 18, WY + 22, { size: 15, color: C.muted, align: 'right' });
    g.restore();
    if (fin > 0) {
      const s = '/work/song.wav · 41.7 s · 7.4 MB';
      const str = HAL.typed(s, t - 1.0, 150);
      HAL.glow(g, 960, WY + 2, 420, accent, 0.18 * fin);
      HAL.runs(g, [['▸ ', accent, { weight: 700 }], [str, '#ffffff', { weight: 700 }]], 960 + 20, WY + 14, { size: 38, align: 'center', alpha: clamp(fin * 2) });
    }
    g.restore();
  },
});
