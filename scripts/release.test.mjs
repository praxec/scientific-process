// release.test.mjs — fast, offline behavioral tests for the scientific-process
// release scripts. Each test makes exactly one behavioral assertion about a
// public interface (check-version-sync.sh, release-manifest.sh, mcp-smoke.mjs,
// install.sh, install.ps1). No network, no machine-config changes.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const ROOT = import.meta.dirname;
const BIN = 'scientific-process';
const TAG = 'v1.2.3';
const SIX_TARGETS = [
  ['x86_64-unknown-linux-gnu', 'tar.gz'],
  ['aarch64-unknown-linux-gnu', 'tar.gz'],
  ['x86_64-apple-darwin', 'tar.gz'],
  ['aarch64-apple-darwin', 'tar.gz'],
  ['x86_64-pc-windows-msvc', 'zip'],
  ['aarch64-pc-windows-msvc', 'zip'],
];

function run(cmd, args, opts = {}) {
  const res = spawnSync(cmd, args, {
    encoding: 'utf8',
    timeout: 60000,
    cwd: opts.cwd ?? ROOT,
    // Fixtures are served over file://, so allow insecure by default; tests of the https rule override it.
    env: { ...process.env, PRAXEC_ALLOW_INSECURE: '1', ...(opts.env ?? {}) },
  });
  return { status: res.status, stdout: res.stdout ?? '', stderr: res.stderr ?? '' };
}

function sha256(file) {
  return createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function tempDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'scientific-process-test-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

function rewriteChecksums(releaseDir) {
  const lines = fs.readdirSync(releaseDir)
    .filter((f) => f.endsWith('.tar.gz'))
    .sort()
    .map((f) => `${sha256(path.join(releaseDir, f))}  ${f}`);
  fs.writeFileSync(path.join(releaseDir, 'checksums.sha256'), lines.join('\n') + '\n');
}

// A directory tree shaped like a GitHub release download page, served to the
// installer over file:// so tests stay fully offline.
function makeReleaseFixture(assets, mutate) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'scientific-process-release-'));
  const releaseDir = path.join(root, 'releases', 'download', TAG);
  fs.mkdirSync(releaseDir, { recursive: true });
  for (const asset of assets) {
    const stage = path.join(root, 'stage', asset.target);
    fs.mkdirSync(stage, { recursive: true });
    const binary = path.join(stage, BIN);
    fs.writeFileSync(binary, asset.payload, { mode: 0o755 });
    execFileSync('tar', ['-czf', path.join(releaseDir, `${BIN}-${asset.target}.tar.gz`), '-C', stage, BIN]);
  }
  if (mutate) mutate({ root, releaseDir });
  rewriteChecksums(releaseDir);
  return { root, releaseDir, baseUrl: `file://${path.join(root, 'releases')}` };
}

function installArgs(fixture, dest) {
  return ['--base-url', fixture.baseUrl, '--version', TAG, '--install-dir', dest];
}

// ---------------------------------------------------------------------------
// check-version-sync.sh
// ---------------------------------------------------------------------------
function versionSync(t, { cargo = '1.2.3', server = '1.2.3', ociTag = server, ociVersion, tag } = {}) {
  const dir = tempDir(t);
  const cargoToml = path.join(dir, 'Cargo.toml');
  const serverJson = path.join(dir, 'server.json');
  fs.writeFileSync(cargoToml, `[package]\nname = "${BIN}"\nversion = "${cargo}"\n`);
  const oci = { registryType: 'oci', identifier: `ghcr.io/praxec/${BIN}:${ociTag}` };
  if (ociVersion !== undefined) oci.version = ociVersion;
  fs.writeFileSync(serverJson, JSON.stringify({ version: server, packages: [oci] }));
  const args = [path.join(ROOT, 'check-version-sync.sh')];
  if (tag) args.push('--tag', tag);
  args.push(cargoToml, serverJson);
  return run('sh', args);
}

test('check-version-sync.sh reports success when Cargo.toml and server.json agree', (t) => {
  assert.equal(versionSync(t).status, 0);
});

test('check-version-sync.sh rejects a server.json version that differs from Cargo.toml', (t) => {
  assert.equal(versionSync(t, { server: '1.2.4' }).status, 1);
});

test('check-version-sync.sh rejects an OCI identifier tag that differs from Cargo.toml', (t) => {
  assert.equal(versionSync(t, { ociTag: '1.2.4' }).status, 1);
});

test('check-version-sync.sh rejects an OCI package that repeats the version field', (t) => {
  assert.equal(versionSync(t, { ociVersion: '1.2.3' }).status, 1);
});

test('check-version-sync.sh rejects a release tag that differs from Cargo.toml', (t) => {
  assert.equal(versionSync(t, { tag: 'v1.2.4' }).status, 1);
});

// ---------------------------------------------------------------------------
// release-manifest.sh
// ---------------------------------------------------------------------------
function makeManifestFixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'scientific-process-manifest-'));
  const dist = path.join(root, 'dist');
  const out = path.join(root, 'out');
  fs.mkdirSync(dist, { recursive: true });
  for (const [target, ext] of SIX_TARGETS) {
    fs.writeFileSync(path.join(dist, `${BIN}-${target}.${ext}`), `payload ${target}\n`);
  }
  return { root, dist, out };
}

function runManifest(fixture) {
  return run('sh', [
    path.join(ROOT, 'release-manifest.sh'),
    '--assets-dir', fixture.dist,
    '--out-dir', fixture.out,
    '--version', '1.2.3',
    '--source-sha', 'abcdef0123456789',
    '--binary', BIN,
  ]);
}

test('release-manifest.sh emits one manifest entry for every native target', (t) => {
  const fixture = makeManifestFixture();
  t.after(() => fs.rmSync(fixture.root, { recursive: true, force: true }));
  runManifest(fixture);
  const manifest = JSON.parse(fs.readFileSync(path.join(fixture.out, 'release-manifest.json'), 'utf8'));
  assert.equal(manifest.targets.length, 6);
});

test('release-manifest.sh lists every packaged asset in checksums.sha256', (t) => {
  const fixture = makeManifestFixture();
  t.after(() => fs.rmSync(fixture.root, { recursive: true, force: true }));
  runManifest(fixture);
  const lines = fs.readFileSync(path.join(fixture.out, 'checksums.sha256'), 'utf8').trim().split('\n');
  assert.equal(lines.length, 6);
});

test('release-manifest.sh refuses to emit metadata for an incomplete matrix', (t) => {
  const fixture = makeManifestFixture();
  t.after(() => fs.rmSync(fixture.root, { recursive: true, force: true }));
  fs.rmSync(path.join(fixture.dist, `${BIN}-aarch64-pc-windows-msvc.zip`));
  assert.notEqual(runManifest(fixture).status, 0);
});

// ---------------------------------------------------------------------------
// mcp-smoke.mjs
// ---------------------------------------------------------------------------
const GOOD_SERVER = `#!/usr/bin/env node
import process from 'node:process';
let buf = '';
const toolNames = ['session.open', 'append', 'state.get'];
const reply = (m) => {
  if (m.method === 'initialize') {
    process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'fake', version: '1' } } }) + '\\n');
  } else if (m.method === 'tools/list') {
    process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result: { tools: toolNames.map((name) => ({ name, description: name, inputSchema: { type: 'object' } })) } }) + '\\n');
  }
};
process.stdin.on('data', (d) => {
  buf += d.toString();
  let i;
  while ((i = buf.indexOf('\\n')) >= 0) {
    const line = buf.slice(0, i);
    buf = buf.slice(i + 1);
    if (!line.trim()) continue;
    try { const m = JSON.parse(line); if (m.id != null) reply(m); } catch {}
  }
});
`;

const EMPTY_SERVER = GOOD_SERVER.replace(
  'result: { tools: toolNames.map((name) => ({ name, description: name, inputSchema: { type: \'object\' } })) }',
  'result: { tools: [] }',
);

function writeServer(t, source) {
  const dir = tempDir(t);
  const file = path.join(dir, 'fake-mcp.mjs');
  fs.writeFileSync(file, source, { mode: 0o755 });
  return file;
}

test('mcp-smoke.mjs accepts a packaged server that answers initialize and tools/list', (t) => {
  const server = writeServer(t, GOOD_SERVER);
  assert.equal(run('node', [path.join(ROOT, 'mcp-smoke.mjs'), server]).status, 0);
});

test('mcp-smoke.mjs rejects a packaged server that advertises no tools', (t) => {
  const server = writeServer(t, EMPTY_SERVER);
  assert.notEqual(run('node', [path.join(ROOT, 'mcp-smoke.mjs'), server]).status, 0);
});

// ---------------------------------------------------------------------------
// install.sh
// ---------------------------------------------------------------------------
test('install.sh resolves the native linux/x86_64 release target', () => {
  const r = run('sh', [path.join(ROOT, 'install.sh'), '--print-target'], {
    env: { PRAXEC_OS: 'linux', PRAXEC_ARCH: 'x86_64' },
  });
  assert.equal(r.stdout.trim(), 'x86_64-unknown-linux-gnu');
});

test('install.sh installs the linux/x86_64 asset into the managed directory', (t) => {
  const fixture = makeReleaseFixture([{ target: 'x86_64-unknown-linux-gnu', payload: 'NATIVE-X64\n' }]);
  t.after(() => fs.rmSync(fixture.root, { recursive: true, force: true }));
  const dest = tempDir(t);
  run('sh', [path.join(ROOT, 'install.sh'), ...installArgs(fixture, dest)], {
    env: { PRAXEC_OS: 'linux', PRAXEC_ARCH: 'x86_64' },
  });
  assert.equal(fs.readFileSync(path.join(dest, BIN), 'utf8'), 'NATIVE-X64\n');
});

test('install.sh refuses an asset whose checksum does not match', (t) => {
  const fixture = makeReleaseFixture([{ target: 'x86_64-unknown-linux-gnu', payload: 'PAYLOAD\n' }]);
  t.after(() => fs.rmSync(fixture.root, { recursive: true, force: true }));
  fs.writeFileSync(
    path.join(fixture.releaseDir, 'checksums.sha256'),
    `${'0'.repeat(64)}  ${BIN}-x86_64-unknown-linux-gnu.tar.gz\n`,
  );
  const dest = tempDir(t);
  const r = run('sh', [path.join(ROOT, 'install.sh'), ...installArgs(fixture, dest)], {
    env: { PRAXEC_OS: 'linux', PRAXEC_ARCH: 'x86_64' },
  });
  assert.notEqual(r.status, 0);
});

test('install.sh rejects a tar archive containing parent-directory traversal', (t) => {
  const fixture = makeReleaseFixture([{ target: 'x86_64-unknown-linux-gnu', payload: 'X\n' }], ({ releaseDir }) => {
    execFileSync('python3', ['-c', `
import io, sys, tarfile
with tarfile.open(sys.argv[1], 'w:gz') as t:
    data = b'escaped'
    info = tarfile.TarInfo('../escape.txt')
    info.size = len(data)
    t.addfile(info, io.BytesIO(data))
`, path.join(releaseDir, `${BIN}-x86_64-unknown-linux-gnu.tar.gz`)]);
  });
  t.after(() => fs.rmSync(fixture.root, { recursive: true, force: true }));
  const dest = tempDir(t);
  const r = run('sh', [path.join(ROOT, 'install.sh'), ...installArgs(fixture, dest)], {
    env: { PRAXEC_OS: 'linux', PRAXEC_ARCH: 'x86_64' },
  });
  assert.notEqual(r.status, 0);
});

// ---------------------------------------------------------------------------
// install.ps1 (only when PowerShell is available)
// ---------------------------------------------------------------------------
const hasPwsh = spawnSync('pwsh', ['-NoProfile', '-Command', 'exit 0']).status === 0;

test('install.ps1 resolves the native windows/arm64 target', { skip: !hasPwsh }, () => {
  const r = run('pwsh', ['-NoProfile', '-File', path.join(ROOT, 'install.ps1'), '-PrintTarget'], {
    env: { PRAXEC_OS: 'windows', PRAXEC_ARCH: 'aarch64' },
  });
  assert.equal(r.stdout.trim(), 'aarch64-pc-windows-msvc');
});
