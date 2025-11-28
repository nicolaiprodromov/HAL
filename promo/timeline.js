// HAL promo — 30.0 s at 60 fps. [scene id, duration in seconds]. Scene code lives in scenes/<id>.js
window.HAL_TIMELINE = [
  ['boot',        2.2],  // CRT power-on, prompt types `hal run fizzbuzz.hal`
  ['source',      1.8],  // the .hal file: plain English in
  ['interpreter', 1.6],  // agent 1: what?   -> logic.md
  ['architect',   1.6],  // agent 2: how?    -> artifacts/ + runtime.yaml
  ['conductor',   1.6],  // agent 3: docker run -> stdout
  ['reflector',   1.6],  // agent 4: observe -> memory
  ['tui',         4.0],  // the live TUI dashboard
  ['ex_aquarium', 1.5],  // examples montage
  ['ex_quakes',   1.5],
  ['ex_pet',      1.5],
  ['ex_gitstory', 1.5],
  ['ex_synth',    1.5],
  ['ex_vague',    1.5],
  ['loop',        3.0],  // the memory loop (header.png, animated)
  ['outro',       3.6],  // wordmark, install line, CRT power-off
];
