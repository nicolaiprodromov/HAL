# HAL promo: 30-second motion graphic

A 30 s, 1080p60 motion graphic about **HAL**, a runtime that turns a plain-English `.hal` file into a
running program in a Docker container. Four agents do the work: **Interpreter → Architect → Conductor →
Reflector**. The Reflector writes what it learned into shared memory for the next build. Read
`../README.md`, `../HAL/flow.md` and `../examples/*.hal` for the real content. Use real file names, real
commands, and real agent jobs so the film is accurate.

## The vibe

- **An old CRT, but very sharp and high quality.** The look is crisp vector type and pixel-block logos
  on a near-black screen. The CRT pass (`engine/post.js`) adds bloom, scanlines, a slight curve, grain,
  and a glitch on cuts automatically. Draw clean and sharp, and let the post pass make it glow.
- **Bloomy, beautiful, professional, not corporate.** No stock-corporate gradients, no rounded
  "SaaS cards", no emoji, no drop shadows. It should feel like a demoscene/terminal piece made with
  taste: confident type, generous negative space, strong single-accent colour per moment.
- **Fast cuts.** Every scene is short (1.5–4 s), so things must read at a glance. Use one clear idea
  per shot, motion from the first frame (never open on a static or empty frame), and keep moving until
  the last frame. Hard cuts between scenes; the engine adds ~6 frames of tearing on each cut.
- **Keep the existing design** (see `../docs/header.png`): background `#0b0d0f`, JetBrains Mono for
  everything, spaced bold caps for labels, the four striped block-face logos from `../HAL/*.logo`, and
  dashed `- - ->` connectors. Agent colours: interpreter cyan `C.interp`, architect magenta
  `C.arch`, conductor amber `C.cond`, reflector mint `C.refl`. Text: `C.text` (primary), `C.soft`,
  `C.muted` (secondary), `C.line`/`C.line2` (hairlines).

## How it works

- `index.html` loads `engine/*.js`, `timeline.js`, then `scenes/<id>.js` for every scene in the timeline.
- A scene file registers itself:

```js
HAL.scene({
  id: 'interpreter',
  draw(g, t, ctx) {   // g = CanvasRenderingContext2D (1920x1080), t = seconds since this scene started
    // ctx = { dur, p (t/dur), T (global seconds), frame, fx, W, H, C, id }
  },
});
```

- **Deterministic.** A frame is a pure function of `t`. Never use `Math.random`, `Date`,
  `performance.now`, `requestAnimationFrame`, or state carried between frames. Use `HAL.hash(a, b, ...)`
  (a float in 0..1) or `HAL.rng(seed)` created inside `draw`, and recompute everything from `t`. Frames
  are rendered out of order across parallel workers.
- The background is already cleared to `C.bg`. The context is saved and restored around `draw`.
- Set post effects through `ctx.fx` (all optional):
  `fx.glitch` (0..1 tearing), `fx.flash` (0..1 white flash), `fx.bloom` (default 1), `fx.aberration`,
  `fx.curvature`, `fx.noise`, `fx.scan`, `fx.vignette`, `fx.exposure`, `fx.shake = [dx, dy]` (px), and
  `fx.crt = [scaleX, scaleY, white]` for the CRT power on/off squash (`[1,1,0]` is normal; a power-off
  is roughly `[1, 0.004, 1]`, then `[0.0, 0.004, 1]`).
  Note: the engine already sets `fx.glitch`/`fx.aberration` for the first 0.1 s of every scene except
  the first. Overwrite it only if you have a reason.

## Engine API (`engine/core.js`, `engine/kit.js`)

Read those two files. They are short. The main calls:

- `HAL.text(g, str, x, y, {size, weight, color, alpha, align, baseline, spacing})` returns the width.
  Weights: 300/400/500/700/800. Monospace advance = `size * 0.6`. `HAL.textWidth(str, size, spacing)`.
- `HAL.runs(g, [[str, color, {weight}], ...], x, y, opts)`: multi-colour line.
- `HAL.typed(str, t, cps)`, `HAL.scramble(str, p, seed, T)` (decode effect), `HAL.cursor(...)`,
  `HAL.prompt(g, cmd, x, y, t, {size, cps, start, T})`.
- `HAL.logo(g, name, x, y, cw, ch, {color, reveal, mode:'rows'|'random'|'cols', blink, jitter, T, seed, alpha})`:
  draws `interpreter|architect|conductor|reflector|all` logos in the header style. `cw` × `ch` is the
  cell size. The header uses about 24×40, so keep roughly a 0.6 ratio. `HAL.logoSize(name, cw, ch)`.
  `HAL.agentCard(...)` draws a logo plus a label.
- `HAL.panel(g, x, y, w, h, {color, fill, radius, title, titleColor})`: TUI-style hairline box with a title in the border.
- `HAL.dashed(g, pts, {color, lw, dash, offset, progress, arrow})`, `HAL.along(pts, f)`, `HAL.glow(g, x, y, r, color, a)`.
- `HAL.grid(g, x, y, {size, lh})`: terminal character grid with `.put(col,row,str,color,{bg,alpha,weight})`, `.fill(...)`.
- `HAL.exampleFrame(g, t, T, {n, file, args, blurb, accent})`: the shared header/footer for the six
  example cuts. It returns the content area.
- Easing and maths: `HAL.ease.*` (outExpo, outBack, inOutCubic, ...), `HAL.range(t, a, b)` (clamped
  0..1 progress), `HAL.lerp`, `HAL.clamp`, `HAL.mix(hex1, hex2, t)`, `HAL.rgba(hex, a)`,
  `HAL.noise1(x, seed)`, `HAL.zoom(g, s, cx, cy)`.
- `scenes/loop.js` is a finished reference scene. Read it to see the conventions in practice.

Do **not** edit `engine/`, `index.html`, `timeline.js`, `lib.mjs`, `render.mjs`, `preview.mjs`, or
anyone else's scene. Put any helper you need inside your own scene file (as a local function). If
you think the engine needs a change, say so in your final report.

## Checking your work (do this a lot)

```sh
cd /workspaces/mapertui/projects/HAL/promo
node preview.mjs --scene <id> --sheet 12          # contact sheet of 12 frames -> out/preview/<id>_sheet.png
node preview.mjs --scene <id> --times 0.1,0.75     # full-res stills at local times
node preview.mjs --scene <id> --times 0.5 --nopost # raw canvas without the CRT pass
```

Open the PNGs with the Read tool and look at them critically. Check that text is legible at full
resolution, that nothing is cut off at the curved edges (keep important content at least 70 px from
the frame edge), that the first and last frames are interesting, and that the motion reads in the
sheet. Errors are printed after the run and also drawn in red on the frame. Iterate until it looks
like a frame from a high-end title sequence. A still takes ~4 s.

## Quality bar

- Pixel-crisp type. Use integer-ish sizes. Align things to a grid. Keep the layout consistent within a scene.
- Use a hierarchy of 2–3 type sizes per shot, with one hero element.
- Colour discipline: mostly neutral greys plus the scene's accent. Use a full rainbow only where the
  content calls for it (the aquarium, the synth instruments, the quake magnitudes).
- Add depth and richness with layering: faint background grids or giant dim glyphs, small HUD details
  (timestamps, byte counts, hashes), and particles. Keep it subtle. The hero must stay obvious.
- Motion should feel snappy: `outExpo`/`outQuart` entrances, small overshoots, staggered reveals.
  Ease the camera with small zoom and drift. Avoid linear, floaty, slow fades.
