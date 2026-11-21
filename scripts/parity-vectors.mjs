#!/usr/bin/env node
// SPDX-License-Identifier: LGPL-2.1-or-later
// parity-vectors.mjs — (re)generates the shared EDID vectors in tests/fixtures/parity/edid/ from
// redacted EDIDs in AudioCaps v1.0.0's committed fixtures, plus synthetic variants that cover every
// branch of the sink-key algorithm. The synthetic serials are made up, not real devices.
// Usage: node scripts/parity-vectors.mjs <audiocaps-v1.0.0-export-dir>
import fs from 'node:fs';
import path from 'node:path';

const src = process.argv[2];
if (!src) { console.error('usage: parity-vectors.mjs <audiocaps v1.0.0 export>'); process.exit(2); }
const out = 'tests/fixtures/parity/edid';
const drm = (fx, dev) => path.join(src, 'tests/fixtures', fx, 'root/sys/devices', dev, 'edid');
const real = {
  'hisense-55qd5n': drm('htpc', 'pci0000:00/0000:00:03.1/0000:07:00.0/0000:08:00.0/0000:09:00.0/drm/card1/card1-HDMI-A-2'),
  'qemu-virtual': drm('lab-alsa-only', 'pci0000:00/0000:00:01.0/drm/card0/card0-Virtual-1'),
  'samsung-lc34g55t': drm('workstation', 'pci0000:00/0000:00:03.1/0000:0a:00.0/drm/card1/card1-DP-2'),
  'lg-fhd': drm('workstation', 'pci0000:00/0000:00:03.1/0000:0a:00.0/drm/card1/card1-DP-3'),
};
const fix = (b) => { let s = 0; for (let i = 0; i < 127; i++) s += b[i]; b[127] = (256 - (s % 256)) % 256; return b; };
const descriptor = (b, tag, text) => { // replace the first non-name display descriptor (or the last slot) with `tag`
  let off = [54, 72, 90, 108].find((o) => b[o] === 0 && b[o + 1] === 0 && b[o + 3] !== 0xfc && b[o + 3] !== 0xfd) ?? 108;
  const t = Buffer.alloc(13, 0x20); Buffer.from(text + '\n').copy(t);
  b[off] = b[off + 1] = b[off + 2] = 0; b[off + 3] = tag; b[off + 4] = 0; t.copy(b, off + 5);
};
fs.mkdirSync(out, { recursive: true });
for (const [n, f] of Object.entries(real)) fs.writeFileSync(path.join(out, `${n}.bin`), fs.readFileSync(f));
const base = () => Buffer.from(fs.readFileSync(real['hisense-55qd5n']));
const synth = {
  'synthetic-serial-number': (b) => { b.writeUInt32LE(0x00bc614e, 12); },
  'synthetic-serial-string': (b) => { descriptor(b, 0xff, 'SYNTH0001'); },
  'synthetic-serial-both': (b) => { b.writeUInt32LE(4242, 12); descriptor(b, 0xff, 'SYNTH0002'); },
  'synthetic-invalid-pnp': (b) => { b[8] = 0; b[9] = 0; b.writeUInt32LE(7, 12); },
  'synthetic-no-name': (b) => { for (const o of [54, 72, 90, 108]) if (b[o] === 0 && b[o + 3] === 0xfc) b[o + 3] = 0x10; },
};
for (const [n, f] of Object.entries(synth)) { const b = base(); f(b); fix(b); fs.writeFileSync(path.join(out, `${n}.bin`), b); }
console.log(`parity-vectors: ${Object.keys(real).length} real (redacted) + ${Object.keys(synth).length} synthetic → ${out}`);
