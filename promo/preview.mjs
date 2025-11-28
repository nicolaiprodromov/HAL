// Render stills / contact sheets for checking scenes.
//
//   node preview.mjs --scene interpreter --sheet 12          # 12 evenly spaced frames -> out/preview/interpreter_sheet.png
//   node preview.mjs --scene interpreter --times 0.1,0.8      # local-time stills -> out/preview/interpreter_0.100.png ...
//   node preview.mjs --at 3.5,10.2                            # global-time stills (all scenes loaded)
//   add --nopost to see the raw canvas without the CRT pass, --cols 4 to change the sheet grid
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { ROOT, serve, launch, openPage, frame, errors } from './lib.mjs';

const args = process.argv.slice(2);
const opt = k => { const i = args.indexOf('--' + k); return i < 0 ? undefined : (args[i + 1] && !args[i + 1].startsWith('--') ? args[i + 1] : true); };
const sceneId = opt('scene');
const outDir = path.join(ROOT, 'out/preview');
fs.mkdirSync(outDir, { recursive: true });

const srv = await serve();
const browser = await launch();
const q = [];
if (sceneId) q.push('scenes=' + sceneId);
if (opt('nopost')) q.push('nopost=1');
const page = await openPage(browser, srv.address().port, q.length ? '?' + q.join('&') : '');
const tl = await page.evaluate(() => HAL.timelineInfo());
let start = 0, dur = tl.total;
if (sceneId) {
  const s = tl.scenes.find(s => s.id === sceneId);
  if (!s) { console.error('unknown scene', sceneId); process.exit(1); }
  start = s.start; dur = s.dur;
}

let times = [];
if (opt('sheet')) {
  const n = parseInt(opt('sheet')) || 12;
  for (let i = 0; i < n; i++) times.push(Math.min(dur - 1 / 60, (i + 0.5) * dur / n));
} else if (opt('times')) times = String(opt('times')).split(',').map(Number);
else if (opt('at')) { times = String(opt('at')).split(',').map(Number); start = 0; }
else times = [dur / 2];

const files = [];
for (const lt of times) {
  const T = Math.round((start + lt) * 60) / 60 + 1e-6;
  const { info, png } = await frame(page, T);
  const f = path.join(outDir, `${sceneId || 'global'}_${lt.toFixed(3)}.png`);
  fs.writeFileSync(f, png); files.push(f);
  console.log(`${f}  (scene ${info.id} local ${info.t.toFixed(3)}s)`);
}
const errs = await errors(page);
if (errs.length) { console.error('ERRORS:\n' + [...new Set(errs)].slice(0, 10).join('\n')); }

if (opt('sheet')) {
  const cols = parseInt(opt('cols')) || 4;
  const rows = Math.ceil(files.length / cols);
  const sheet = path.join(outDir, `${sceneId || 'global'}_sheet.png`);
  const inputs = files.flatMap(f => ['-i', f]);
  const filt = files.map((_, i) => `[${i}:v]scale=640:360,drawtext=text='${times[i].toFixed(2)}s':x=8:y=8:fontsize=18:fontcolor=white:box=1:boxcolor=black@0.6[v${i}]`).join(';')
    + ';' + files.map((_, i) => `[v${i}]`).join('') + `xstack=inputs=${files.length}:layout=` +
    files.map((_, i) => `${(i % cols) * 640}_${Math.floor(i / cols) * 360}`).join('|') + `:fill=black[o]`;
  execFileSync('ffmpeg', ['-y', '-loglevel', 'error', ...inputs, '-filter_complex', filt, '-map', '[o]', sheet]);
  for (const f of files) fs.unlinkSync(f);
  console.log('sheet:', sheet, `(${cols}x${rows})`);
}
await browser.close(); srv.close();
