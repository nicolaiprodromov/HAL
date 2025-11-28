// loop — the header.png composition, alive: four agents in a row, arrows, and the
// shared-memory loop feeding the next build.
HAL.scene({
  id: 'loop',
  draw(g, t, { dur, T, fx, W, H, C }) {
    const { ease, range, logoSize, AGENTS, AGENT_ORDER } = HAL;
    fx.bloom = 1.1;

    // slow push-in for the whole shot
    HAL.zoom(g, 1 + 0.045 * ease.inOutQuad(t / dur), W / 2, H / 2 - 20);

    const cw = 19, ch = 32;
    const xs = [0.145, 0.382, 0.618, 0.855].map(f => f * W);
    const logoY = 300;
    const subs = [
      'print 1..30, Fizz on 3, Buzz on 5',
      'logic.md → runtime.yaml',
      null, // conductor gets live output
      'noted the Fizz/Buzz rules',
    ];

    // command line, top-left
    HAL.prompt(g, 'hal run fizzbuzz.hal', 64, 92, 10, { size: 30, T, cps: 99 });

    AGENT_ORDER.forEach((key, i) => {
      const A = AGENTS[key];
      const t0 = -0.22 + i * 0.09; // already assembling when the cut lands
      const k = range(t, t0, t0 + 0.35);
      if (k <= 0) return;
      const s = logoSize(A.logo, cw, ch);
      // pulse when the memory packet lands back on the interpreter
      const pulse = i === 0 ? Math.max(0, 1 - Math.abs(t - 2.35) / 0.18) : 0;
      const pop = 1 + 0.06 * (1 - ease.outBack(k)) + 0.05 * pulse;
      g.save();
      HAL.zoom(g, pop, xs[i], logoY + s.h / 2);
      HAL.logo(g, A.logo, xs[i] - s.w / 2, logoY, cw, ch, { color: pulse > 0.3 ? HAL.mix(A.color, '#ffffff', pulse * 0.6) : A.color, reveal: k, mode: 'rows', seed: i + 3, T, blink: (t > 1.6 + i * 0.1 && t < 1.68 + i * 0.1) ? 1 : 0 });
      g.restore();
      const la = range(t, t0 + 0.2, t0 + 0.4);
      HAL.text(g, A.name, xs[i], logoY + s.h + 64, { size: 24, color: A.color, weight: 700, spacing: 4, align: 'center', alpha: la });
      if (subs[i]) {
        const str = HAL.typed(subs[i], t - t0 - 0.3, 70);
        HAL.text(g, str, xs[i], logoY + s.h + 112, { size: 22, color: C.text, align: 'center', alpha: la });
      } else {
        // conductor: stdout ticking
        const seq = Array.from({ length: 30 }, (_, j) => (j + 1) % 15 === 0 ? 'FizzBuzz' : (j + 1) % 3 === 0 ? 'Fizz' : (j + 1) % 5 === 0 ? 'Buzz' : String(j + 1));
        const n = Math.min(30, Math.max(0, Math.floor((t - t0 - 0.3) * 9)));
        const vis = seq.slice(Math.max(0, n - 5), n);
        const parts = [];
        vis.forEach((v, j) => parts.push([v + '  ', /[A-Z]/.test(v) ? C.cond : C.muted]));
        if (parts.length) {
          const w = HAL.runs(g, parts, xs[i], logoY + s.h + 112, { size: 22, align: 'center', alpha: la });
          HAL.cursor(g, xs[i] + w / 2 - 6, logoY + s.h + 112, 24, T, C.cond, 3);
        }
      }
      // arrow to the next agent
      if (i < 3) {
        const ax0 = xs[i] + s.w / 2 + 40, ax1 = xs[i + 1] - s.w / 2 - 40, ay = logoY + s.h / 2;
        const ak = ease.outCubic(range(t, t0 + 0.25, t0 + 0.55));
        HAL.dashed(g, [[ax0, ay], [ax1, ay]], { color: C.muted, lw: 2, dash: [10, 9], offset: t * 40, progress: ak, arrow: true, arrowSize: 9 });
        // data packet running along the arrow
        const pk = (t * 1.4 + i * 0.33) % 1;
        if (ak >= 1) {
          const [px, py] = HAL.along([[ax0, ay], [ax1, ay]], pk);
          g.fillStyle = AGENTS[AGENT_ORDER[i + 1]].color;
          g.fillRect(px - 5, py - 5, 10, 10);
        }
      }
    });

    // shared-memory loop: reflector -> down -> left -> up into interpreter
    const ly = 820;
    const s0 = logoSize('reflector', cw, ch);
    const loopPts = [[xs[3], logoY + s0.h + 140], [xs[3], ly - 16], [xs[3] - 16, ly], [xs[0] + 16, ly], [xs[0], ly - 16], [xs[0], logoY + s0.h + 140]];
    const lk = ease.inOutCubic(range(t, 0.85, 2.2));
    HAL.dashed(g, loopPts, { color: HAL.rgba(C.refl, 0.75), lw: 2, dash: [10, 9], offset: t * 50, progress: lk, arrow: true, arrowSize: 11 });
    if (lk > 0 && lk < 1) {
      const [px, py] = HAL.along(loopPts, lk);
      HAL.glow(g, px, py, 60, C.refl, 0.5);
      g.fillStyle = '#e9fff3'; g.fillRect(px - 6, py - 6, 12, 12);
    }
    // label in the middle of the loop line
    const mk = range(t, 1.2, 1.5);
    if (mk > 0) {
      const parts = [['◆ ', C.refl], ['shared memory', C.text, { weight: 700 }], ['   run facts · observations · fixes → next build', C.muted]];
      const w = HAL.textWidth('◆ shared memory   run facts · observations · fixes → next build', 24);
      g.fillStyle = C.bg; g.fillRect(W / 2 - w / 2 - 24, ly - 20, w + 48, 40);
      HAL.runs(g, parts, W / 2, ly + 8, { size: 24, align: 'center', alpha: mk });
    }

    // closing caption
    const ck = range(t, 2.3, 2.6);
    if (ck > 0) {
      const str = HAL.scramble('EVERY RUN MAKES THE NEXT BUILD SMARTER', ck, 9, T);
      HAL.text(g, str, W / 2, 965, { size: 30, color: C.soft, weight: 700, spacing: 8, align: 'center' });
    }
  },
});
