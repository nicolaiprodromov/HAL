// Render the full film: parallel headless workers -> lossless chunks -> final H.264.
//   node render.mjs [--workers 8] [--from 0] [--to 30] [--out out/hal-promo.mp4]
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawn, execFileSync } from 'node:child_process';
import { ROOT, serve, launch, openPage, frame, errors } from './lib.mjs';

const args = process.argv.slice(2);
const opt = (k, d) => { const i = args.indexOf('--' + k); return i < 0 ? d : args[i + 1]; };
const FPS = 60;
const workers = parseInt(opt('workers', Math.max(2, Math.min(10, os.cpus().length - 4))));
const out = path.resolve(ROOT, opt('out', 'out/hal-promo.mp4'));
const tmp = path.join(ROOT, 'out/chunks');
fs.rmSync(tmp, { recursive: true, force: true }); fs.mkdirSync(tmp, { recursive: true });

const srv = await serve();
const port = srv.address().port;
const probe = await launch();
const pp = await openPage(probe, port);
const total = (await pp.evaluate(() => HAL.timelineInfo())).total;
await probe.close();
const from = parseFloat(opt('from', 0)), to = Math.min(total, parseFloat(opt('to', total)));
const f0 = Math.round(from * FPS), f1 = Math.round(to * FPS);
const N = f1 - f0;
console.log(`rendering ${N} frames (${from}s..${to}s) with ${workers} workers`);

const per = Math.ceil(N / workers);
let doneFrames = 0; const t0 = Date.now();
const allErrs = new Set();
await Promise.all(Array.from({ length: workers }, async (_, w) => {
  const a = f0 + w * per, b = Math.min(f1, a + per);
  if (a >= b) return;
  const browser = await launch();
  const page = await openPage(browser, port);
  const chunk = path.join(tmp, `chunk_${String(w).padStart(2, '0')}.mkv`);
  const ff = spawn('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'image2pipe', '-framerate', String(FPS), '-c:v', 'png', '-i', '-',
    '-c:v', 'libx264rgb', '-qp', '0', '-preset', 'ultrafast', chunk], { stdio: ['pipe', 'inherit', 'inherit'] });
  for (let f = a; f < b; f++) {
    const { png } = await frame(page, f / FPS + 1e-6);
    if (!ff.stdin.write(png)) await new Promise(r => ff.stdin.once('drain', r));
    doneFrames++;
    if (doneFrames % 60 === 0) {
      const el = (Date.now() - t0) / 1000;
      process.stdout.write(`  ${doneFrames}/${N} frames  ${(doneFrames / el).toFixed(1)} fps  eta ${((N - doneFrames) / (doneFrames / el)).toFixed(0)}s\n`);
    }
  }
  for (const e of await errors(page)) allErrs.add(e.split('\n')[0]);
  ff.stdin.end();
  await new Promise(r => ff.on('close', r));
  await browser.close();
}));
srv.close();
if (allErrs.size) console.error('scene errors:\n' + [...allErrs].slice(0, 20).join('\n'));

const list = path.join(tmp, 'list.txt');
fs.writeFileSync(list, fs.readdirSync(tmp).filter(f => f.endsWith('.mkv')).sort().map(f => `file '${path.join(tmp, f)}'`).join('\n'));
fs.mkdirSync(path.dirname(out), { recursive: true });
const enc = (file, crf, maxrate) => execFileSync('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', list,
  '-c:v', 'libx264', '-preset', 'slow', '-crf', String(crf), '-maxrate', maxrate, '-bufsize', '120M',
  '-vf', 'scale=out_color_matrix=bt709:out_range=tv,format=yuv420p',
  '-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-movflags', '+faststart', file], { stdio: 'inherit' });
enc(out, 16, '80M');                                   // master
enc(out.replace(/\.mp4$/, '-web.mp4'), 20, '30M');    // share / upload size
console.log('wrote', out, `in ${((Date.now() - t0) / 1000).toFixed(0)}s`);
