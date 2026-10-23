// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

// Puts uclog-hub binaries into bin/<platform>-<arch>/ and packages the VSIX (plan 03 §6, task 7).
//
//   node scripts/package.mjs --local            this machine's native/target/{release,debug}/uclog-hub
//   node scripts/package.mjs --from DIR         release archives (uclog-<tag>-<name>.tar.gz|zip and
//                                              their .sha256), checksums verified
//   --require-all                              fail when a target is missing
//   --no-package                               only fill bin/

import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(here, '../../..');
const args = process.argv.slice(2);
const opt = (n) => args.includes(n);
const val = (n) => (args.includes(n) ? args[args.indexOf(n) + 1] : undefined);

// Release archive name → VS Code platform-arch.
const TARGETS = {
  'linux-x64': 'linux-x64',
  'linux-arm64': 'linux-arm64',
  'linux-armv7': 'linux-arm',
  'macos-arm64': 'darwin-arm64',
  'macos-x64': 'darwin-x64',
  'windows-x64': 'win32-x64',
  'windows-arm64': 'win32-arm64',
};

const bin = join(here, 'bin');
rmSync(bin, { recursive: true, force: true });

function place(src, target) {
  const exe = target.startsWith('win32') ? 'uclog-hub.exe' : 'uclog-hub';
  mkdirSync(join(bin, target), { recursive: true });
  copyFileSync(src, join(bin, target, exe));
  if (!target.startsWith('win32')) chmodSync(join(bin, target, exe), 0o755);
  console.log(`bin/${target}/${exe}  ${(statSync(src).size / 1e6).toFixed(1)} MB`);
}

function find(dir, names) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) {
      const r = find(p, names);
      if (r) return r;
    } else if (names.includes(e.name)) return p;
  }
  return null;
}

if (opt('--local')) {
  const exe = process.platform === 'win32' ? 'uclog-hub.exe' : 'uclog-hub';
  const src = ['release', 'debug'].map((p) => join(repo, 'native/target', p, exe)).find((p) => existsSync(p));
  if (!src) throw new Error('native/target/release/uclog-hub is missing: cargo build --release -p uclog-hub');
  place(src, `${process.platform}-${process.arch}`);
} else if (val('--from')) {
  const dir = resolve(val('--from'));
  const missing = [];
  for (const [name, target] of Object.entries(TARGETS)) {
    const archive = readdirSync(dir).find((f) => f.startsWith('uclog-') && (f.endsWith(`-${name}.tar.gz`) || f.endsWith(`-${name}.zip`)));
    if (!archive) {
      missing.push(name);
      continue;
    }
    const a = join(dir, archive);
    const sumFile = `${a}.sha256`;
    if (!existsSync(sumFile)) throw new Error(`${archive}: no .sha256`);
    const want = readFileSync(sumFile, 'utf8').trim().split(/\s+/)[0].toLowerCase();
    const got = createHash('sha256').update(readFileSync(a)).digest('hex');
    if (want !== got) throw new Error(`${archive}: checksum mismatch`);
    const tmp = mkdtempSync(join(tmpdir(), 'uclog-vsix-'));
    try {
      if (archive.endsWith('.zip')) execFileSync(process.platform === 'win32' ? 'tar' : 'unzip', process.platform === 'win32' ? ['-xf', a, '-C', tmp] : ['-q', a, '-d', tmp]);
      else execFileSync('tar', ['xzf', a, '-C', tmp]);
      const hub = find(tmp, ['uclog-hub', 'uclog-hub.exe']);
      if (!hub) throw new Error(`${archive}: no uclog-hub inside`);
      place(hub, target);
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  }
  if (missing.length) {
    const msg = `no archive for ${missing.join(', ')}`;
    if (opt('--require-all')) throw new Error(msg);
    console.warn(`warning: ${msg}`);
  }
} else {
  console.error('usage: node scripts/package.mjs --local | --from DIR [--require-all] [--no-package]');
  process.exit(2);
}

if (!opt('--no-package')) {
  copyFileSync(join(repo, 'LICENSE'), join(here, 'LICENSE'));
  const version = JSON.parse(readFileSync(join(here, 'package.json'), 'utf8')).version;
  execFileSync('npx', ['-y', '@vscode/vsce@3.9.2', 'package', '--no-dependencies', '--allow-missing-repository', '-o', `uclog-${version}.vsix`], {
    cwd: here,
    stdio: 'inherit',
    shell: process.platform === 'win32',
  });
}
