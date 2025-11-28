// HAL promo — shared building blocks used by several scenes.
(() => {
  const { C, text, textWidth, prompt, range, ease, rgba } = HAL;

  // Character grid, like a terminal. cols x rows of monospace cells at (x, y).
  //   const G = HAL.grid(g, x, y, { size: 20, lh: 1.3 })
  //   G.put(col, row, 'text', color, { alpha, weight, bg })   // bg = cell background colour
  //   G.fill(col, row, w, h, color, alpha)                   // solid cell rectangle
  //   G.cw, G.ch                                              // cell size in px
  function grid(g, x, y, o = {}) {
    const size = o.size || 20, lh = o.lh || 1.3;
    const cw = size * HAL.ADV, ch = Math.round(size * lh);
    return {
      cw, ch, size,
      x: col => x + col * cw,
      y: row => y + row * ch,
      put(col, row, str, color = C.text, po = {}) {
        if (po.bg) { g.fillStyle = po.bg; g.fillRect(Math.round(x + col * cw), Math.round(y + row * ch), Math.ceil([...str].length * cw), ch); }
        text(g, str, x + col * cw, y + row * ch + ch * 0.5, { size, color, weight: po.weight || 400, alpha: po.alpha, baseline: 'middle' });
      },
      fill(col, row, w, h, color, alpha = 1) {
        g.save(); g.globalAlpha *= alpha; g.fillStyle = color;
        g.fillRect(Math.round(x + col * cw), Math.round(y + row * ch), Math.ceil(w * cw), Math.ceil(h * ch));
        g.restore();
      },
    };
  }

  // Shared frame for the examples montage, so every example cut rhymes.
  //   const area = HAL.exampleFrame(g, t, T, { n: 1, file: 'aquarium.hal', args: '20', flags: '-f', blurb: 'a terminal aquarium', accent: C.interp })
  // Draws: typed command top-left, "EXAMPLE 0n / 06" + blurb top-right, progress ticks bottom-left.
  // Returns the free content area { x, y, w, h } (below the header, above the footer).
  function exampleFrame(g, t, T, o) {
    const accent = o.accent || C.interp;
    const cmd = `hal -d ${o.flags ? o.flags + ' ' : ''}examples/${o.file}${o.args ? ' ' + o.args : ''}`;
    // command is mostly typed when the cut lands, finishes fast
    const pre = Math.floor([...cmd].length * 0.55);
    const shown = [...cmd].slice(0, pre + Math.floor(t * 90)).join('');
    const x = 64, y = 86;
    const w1 = text(g, '$ ', x, y, { size: 28, color: C.muted, weight: 500 });
    const w2 = text(g, shown, x + w1, y, { size: 28, color: C.text, weight: 500 });
    HAL.cursor(g, x + w1 + w2 + 4, y, 28, T, accent, 3);
    // right: index + blurb
    text(g, `EXAMPLE ${String(o.n).padStart(2, '0')} / 06`, 1920 - 64, y - 4, { size: 18, color: accent, weight: 700, spacing: 4, align: 'right' });
    text(g, o.blurb || '', 1920 - 64, y + 30, { size: 20, color: C.muted, align: 'right', alpha: range(t, 0.05, 0.25) });
    // hairline under the header
    g.fillStyle = C.line; g.fillRect(64, 132, 1792, 1);
    g.fillStyle = accent; g.fillRect(64, 132, 1792 * ease.outCubic(range(t, 0, 0.5)), 1);
    // footer progress ticks
    for (let i = 0; i < 6; i++) {
      g.fillStyle = i + 1 === o.n ? accent : (i + 1 < o.n ? C.line2 : C.line);
      g.fillRect(64 + i * 34, 1020, 26, 4);
    }
    text(g, o.file, 64 + 6 * 34 + 14, 1027, { size: 18, color: C.muted });
    return { x: 64, y: 156, w: 1792, h: 840 };
  }

  Object.assign(HAL, { grid, exampleFrame });
})();
