# HAL promo film

A 30 s, 1080p60 motion graphic of HAL, rendered from code: canvas scenes, then a WebGL CRT pass, then ffmpeg.

```sh
npm install                                       # playwright-core (uses the cached ms-playwright chromium)
node render.mjs                                   # -> out/hal-promo.mp4 (master) + out/hal-promo-web.mp4
node preview.mjs --scene tui --sheet 12           # contact sheet of one scene
node preview.mjs --at 3.5,10.2                    # stills at global times
```

- `timeline.js`: scene order and durations
- `scenes/<id>.js`: one file per shot
- `engine/`: drawing helpers (`core.js`, `kit.js`), the CRT post pass (`post.js`), and the logos generated from `../HAL/*.logo`
- `BRIEF.md`: the style guide and API notes used to write the scenes

Rendering is deterministic, so every frame is a pure function of time. Workers render frames in parallel, out of order.
