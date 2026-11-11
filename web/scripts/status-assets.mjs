// Writes the status-page assets the server embeds (crates/uwustatus-server/assets/status/):
// Nyu per mood, the Suite font and the Nyu CSS. Run after bumping @uwusuite/design:
//   node scripts/status-assets.mjs
import { copyFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { nyuSvg } from '@uwusuite/design/nyu-svg';

const out = join(
  dirname(fileURLToPath(import.meta.url)),
  '../../crates/uwustatus-server/assets/status',
);
const design = dirname(createRequire(import.meta.url).resolve('@uwusuite/design/package.json'));
mkdirSync(out, { recursive: true });
// "puzzled" stands in for "worried": the Suite has no worried Nyu yet.
for (const mood of ['happy', 'puzzled', 'sad', 'sleepy']) {
  writeFileSync(join(out, `nyu-${mood}.svg`), nyuSvg({ shell: 'box', mood, label: '' }) + '\n');
}
copyFileSync(join(design, 'fonts/UwUSans[wght].woff2'), join(out, 'uwu-sans.woff2'));
copyFileSync(join(design, 'fonts/OFL.txt'), join(out, 'OFL.txt'));
copyFileSync(join(design, 'dist/css/nyu.css'), join(out, 'nyu.css'));
