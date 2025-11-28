// HAL promo — CRT post-processing (WebGL2).
// content canvas -> multi-level bloom -> composite (curvature, aberration, scanlines,
// vignette, grain, glitch, power on/off) -> screen.
(() => {
  const VS = `#version 300 es
  in vec2 aPos; out vec2 vUv;
  void main(){ vUv = aPos * 0.5 + 0.5; gl_Position = vec4(aPos, 0.0, 1.0); }`;

  // downsample with a 13-tap filter (CoD style); level 0 applies a soft threshold
  const FS_DOWN = `#version 300 es
  precision highp float;
  uniform sampler2D uTex; uniform vec2 uTexel; uniform float uThreshold; uniform int uPrefilter;
  in vec2 vUv; out vec4 o;
  vec3 s(vec2 off){ return texture(uTex, vUv + off * uTexel).rgb; }
  void main(){
    vec3 a = s(vec2(-2, 2)), b = s(vec2(0, 2)), c = s(vec2(2, 2));
    vec3 d = s(vec2(-2, 0)), e = s(vec2(0, 0)), f = s(vec2(2, 0));
    vec3 g = s(vec2(-2,-2)), h = s(vec2(0,-2)), i = s(vec2(2,-2));
    vec3 j = s(vec2(-1, 1)), k = s(vec2(1, 1)), l = s(vec2(-1,-1)), m = s(vec2(1,-1));
    vec3 col = e*0.125 + (a+c+g+i)*0.03125 + (b+d+f+h)*0.0625 + (j+k+l+m)*0.125;
    if (uPrefilter == 1) {
      float br = max(col.r, max(col.g, col.b));
      float knee = 0.18;
      float soft = clamp(br - uThreshold + knee, 0.0, 2.0 * knee);
      soft = soft * soft / (4.0 * knee + 1e-4);
      float contrib = max(soft, br - uThreshold) / max(br, 1e-4);
      col *= contrib;
    }
    o = vec4(col, 1.0);
  }`;

  // separable gaussian (9 taps via linear sampling)
  const FS_BLUR = `#version 300 es
  precision highp float;
  uniform sampler2D uTex; uniform vec2 uDir; in vec2 vUv; out vec4 o;
  void main(){
    vec3 c = texture(uTex, vUv).rgb * 0.2270270270;
    c += texture(uTex, vUv + uDir * 1.3846153846).rgb * 0.3162162162;
    c += texture(uTex, vUv - uDir * 1.3846153846).rgb * 0.3162162162;
    c += texture(uTex, vUv + uDir * 3.2307692308).rgb * 0.0702702703;
    c += texture(uTex, vUv - uDir * 3.2307692308).rgb * 0.0702702703;
    o = vec4(c, 1.0);
  }`;

  const FS_FINAL = `#version 300 es
  precision highp float;
  uniform sampler2D uSrc, uB0, uB1, uB2, uB3, uB4;
  uniform vec2 uRes; uniform float uTime, uFrame;
  uniform float uGlitch, uFlash, uBloom, uAb, uCurv, uNoise, uScan, uVig, uExposure;
  uniform vec3 uCrt; uniform vec2 uShake;
  in vec2 vUv; out vec4 o;

  float h1(float n){ return fract(sin(n * 12.9898) * 43758.5453); }
  uint pcg(uint v){ uint st = v * 747796405u + 2891336453u; uint w = ((st >> ((st >> 28u) + 4u)) ^ st) * 277803737u; return (w >> 22u) ^ w; }
  float h2(uvec2 p, uint f){ return float(pcg(p.x + pcg(p.y + pcg(f)))) / 4294967295.0; }
  float sdRound(vec2 p, vec2 b, float r){ vec2 q = abs(p) - b + r; return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r; }

  void main(){
    vec2 uv = vUv;
    uv.y = 1.0 - uv.y; // canvas is top-down
    uv -= uShake / uRes;

    // CRT power on/off: squash around the centre
    vec2 cc = uv - 0.5;
    cc.x /= max(uCrt.x, 1e-3);
    cc.y /= max(uCrt.y, 1e-3);
    uv = cc + 0.5;

    // gentle barrel curvature
    vec2 c = uv - 0.5;
    vec2 ca = c * vec2(uRes.x / uRes.y, 1.0);
    float r2 = dot(ca, ca);
    uv = 0.5 + c * (1.0 + uCurv * 0.028 * r2);

    // glitch: horizontal tearing bands
    float gt = floor(uFrame / 2.0);
    float band = floor(uv.y * 36.0);
    float bn = h1(band * 7.13 + gt * 3.7);
    float tear = 0.0;
    if (bn > 1.0 - uGlitch * 0.55) tear = (h1(band + gt * 11.1) - 0.5) * 0.09 * uGlitch;
    float fine = (h1(floor(uv.y * uRes.y / 3.0) + gt) - 0.5) * 0.004 * uGlitch;
    uv.x += tear + fine;

    // chromatic aberration grows toward the edges (and with glitch)
    vec2 dir = c * (0.0016 * uAb) + vec2(0.006 * uGlitch, 0.0);
    vec3 col;
    col.r = texture(uSrc, uv + dir).r;
    col.g = texture(uSrc, uv).g;
    col.b = texture(uSrc, uv - dir).b;

    // scanlines (applied to the sharp image only, bloom fills the gaps like a real CRT)
    float py = uv.y * uRes.y;
    float sl = 0.5 + 0.5 * cos(py * 6.2831853 / 3.0);
    col *= 1.0 - uScan * 0.20 * sl;
    // faint vertical phosphor triad
    float px = uv.x * uRes.x;
    vec3 mask = vec3(1.0) - uScan * 0.045 * vec3(
      step(0.66, fract(px / 3.0)), step(fract(px / 3.0), 0.33), step(0.33, fract(px / 3.0)) * step(fract(px / 3.0), 0.66));
    col *= mask;

    // bloom: sum of blurred mips (wide, soft, colourful)
    vec3 b = texture(uB0, uv).rgb * 0.55
           + texture(uB1, uv).rgb * 0.65
           + texture(uB2, uv).rgb * 0.80
           + texture(uB3, uv).rgb * 0.95
           + texture(uB4, uv).rgb * 1.10;
    vec3 bb = b * 0.40 * uBloom;
    col += bb * (1.0 - 0.8 * clamp(col, 0.0, 1.0));   // glow hugs edges, doesn't bleach bright areas

    // flash
    col += uFlash * (0.8 * vec3(1.0) + b);

    col *= uExposure;
    // subtle flicker
    col *= 1.0 + 0.010 * sin(uTime * 120.0) + 0.006 * (h1(gt) - 0.5);

    // vignette
    float v = smoothstep(1.15, 0.25, length(ca * vec2(0.82, 1.05)));
    col *= mix(1.0, v, 0.38 * uVig);

    // power line white
    col = mix(col, vec3(1.0) * (0.6 + 0.4 * b.g), uCrt.z * smoothstep(0.5, 0.0, abs(cc.y) * uCrt.y * 40.0));

    // hue-preserving shoulder: compress the peak channel, keep ratios
    float pk = max(col.r, max(col.g, col.b));
    if (pk > 0.85) { float np = 0.85 + 0.15 * (1.0 - exp(-(pk - 0.85) / 0.15)); col *= np / pk; }

    // rounded screen edge
    vec2 pp = (uv - 0.5) * uRes;
    float d = sdRound(pp, uRes * 0.5 - vec2(2.0), 34.0);
    float edge = smoothstep(1.5, -1.5, d);
    // outside power-on squash region is dark
    float inside = step(abs(cc.x), 0.5) * step(abs(cc.y), 0.5);
    col *= edge * inside;
    col += vec3(0.008, 0.009, 0.011) * (1.0 - edge * inside);

    // grain + dither (keeps bloom gradients from banding in H.264)
    float n = h2(uvec2(gl_FragCoord.xy), uint(uFrame)) - 0.5;
    col += n * (0.028 * uNoise + 1.5 / 255.0);

    o = vec4(clamp(col, 0.0, 1.0), 1.0);
  }`;

  function Post(canvas, W, H) {
    const gl = canvas.getContext('webgl2', { preserveDrawingBuffer: true, antialias: false, premultipliedAlpha: false, alpha: false });
    if (!gl) throw new Error('no webgl2');
    const half = !gl.getExtension('EXT_color_buffer_float');

    function sh(type, src) {
      const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
      return s;
    }
    function prog(fs) {
      const p = gl.createProgram();
      gl.attachShader(p, sh(gl.VERTEX_SHADER, VS)); gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
      gl.bindAttribLocation(p, 0, 'aPos'); gl.linkProgram(p);
      if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
      const u = {}; const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
      for (let i = 0; i < n; i++) { const a = gl.getActiveUniform(p, i); u[a.name] = gl.getUniformLocation(p, a.name); }
      return { p, u };
    }
    const P = { down: prog(FS_DOWN), blur: prog(FS_BLUR), fin: prog(FS_FINAL) };

    const vb = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, vb);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

    function tex(w, h, float) {
      const t = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, t);
      if (float && !half) gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
      else gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      return t;
    }
    function target(w, h) {
      const t = tex(w, h, true), fb = gl.createFramebuffer();
      gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t, 0);
      return { t, fb, w, h };
    }
    const src = tex(W, H, false);
    gl.bindTexture(gl.TEXTURE_2D, src);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
    const LEVELS = 5;
    const mips = [];
    for (let i = 0; i < LEVELS; i++) {
      const w = Math.max(1, Math.round(W / Math.pow(2, i + 1))), h = Math.max(1, Math.round(H / Math.pow(2, i + 1)));
      mips.push({ a: target(w, h), b: target(w, h) });
    }

    function draw(fb, w, h) { gl.bindFramebuffer(gl.FRAMEBUFFER, fb); gl.viewport(0, 0, w, h); gl.drawArrays(gl.TRIANGLES, 0, 3); }

    this.render = (content, fx, T, frame) => {
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, src);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, content);

      // downsample chain + blur each level
      let inTex = src, inW = W, inH = H;
      for (let i = 0; i < LEVELS; i++) {
        const m = mips[i];
        gl.useProgram(P.down.p);
        gl.uniform1i(P.down.u.uTex, 0);
        gl.uniform2f(P.down.u.uTexel, 1 / inW, 1 / inH);
        gl.uniform1f(P.down.u.uThreshold, 0.12);
        gl.uniform1i(P.down.u.uPrefilter, i === 0 ? 1 : 0);
        gl.bindTexture(gl.TEXTURE_2D, inTex);
        draw(m.a.fb, m.a.w, m.a.h);
        gl.useProgram(P.blur.p);
        gl.uniform1i(P.blur.u.uTex, 0);
        gl.uniform2f(P.blur.u.uDir, 1 / m.a.w, 0);
        gl.bindTexture(gl.TEXTURE_2D, m.a.t);
        draw(m.b.fb, m.b.w, m.b.h);
        gl.uniform2f(P.blur.u.uDir, 0, 1 / m.a.h);
        gl.bindTexture(gl.TEXTURE_2D, m.b.t);
        draw(m.a.fb, m.a.w, m.a.h);
        inTex = m.a.t; inW = m.a.w; inH = m.a.h;
      }

      // composite
      const u = P.fin.u;
      gl.useProgram(P.fin.p);
      const units = [src, mips[0].a.t, mips[1].a.t, mips[2].a.t, mips[3].a.t, mips[4].a.t];
      const names = ['uSrc', 'uB0', 'uB1', 'uB2', 'uB3', 'uB4'];
      units.forEach((t, i) => { gl.activeTexture(gl.TEXTURE0 + i); gl.bindTexture(gl.TEXTURE_2D, t); gl.uniform1i(u[names[i]], i); });
      gl.uniform2f(u.uRes, W, H);
      gl.uniform1f(u.uTime, T);
      gl.uniform1f(u.uFrame, frame);
      gl.uniform1f(u.uGlitch, fx.glitch);
      gl.uniform1f(u.uFlash, fx.flash);
      gl.uniform1f(u.uBloom, fx.bloom);
      gl.uniform1f(u.uAb, fx.aberration);
      gl.uniform1f(u.uCurv, fx.curvature);
      gl.uniform1f(u.uNoise, fx.noise);
      gl.uniform1f(u.uScan, fx.scan);
      gl.uniform1f(u.uVig, fx.vignette);
      gl.uniform1f(u.uExposure, fx.exposure);
      gl.uniform3f(u.uCrt, fx.crt[0], fx.crt[1], fx.crt[2]);
      gl.uniform2f(u.uShake, fx.shake[0], fx.shake[1]);
      draw(null, W, H);
      gl.activeTexture(gl.TEXTURE0);
      gl.finish();
    };
  }
  window.HAL_Post = Post;
})();
