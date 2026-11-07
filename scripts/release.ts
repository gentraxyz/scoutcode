/**
 * Publish the desktop app: build it for x64 and ARM64, sign both and put them where every installed copy will find them.
 *
 *   npm run release -- --summary "One line" --changelog "- What changed"   # builds Cargo.toml's version and publishes it
 *   npm run release -- --skip-build --summary "…" --changelog-file CHANGES.md   # publishes the builds already in target
 *
 * The summary is one line on what the release is about, shown beside "Restart to update". The changelog lists what
 * changed, shown in Settings > Updates and in "What's new" after the update. Both go in latest.json, and in
 * releases.json, which keeps every release's notes for /download/releases.json. --notes still works as the summary.
 *   npm run release -- --notes-from CHANGELOG.md          # takes the summary and changelog from its "## <version>" section
 *   npm run release -- check                              # says whether Cargo.toml's version is newer than what's out
 *   npm run release -- keygen                    # makes the signing key, once; prints the public half
 *
 * To try a release against a local server, write it to a folder instead of uploading:
 *   npm run release -- --skip-build --out <dir> --url-base http://127.0.0.1:8765 --version 9.9.9
 *
 * Bump `version` in Cargo.toml first. Installed apps only take a higher version, so to undo a bad release, publish
 * the old code under a new number.
 *
 * Building needs rustup's x86_64-pc-windows-gnullvm and aarch64-pc-windows-gnullvm targets and llvm-mingw on the PATH.
 * Publishing needs SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY (the environment, or .env here) and the private signing key:
 * ~/.scoutcode/desktop-release-key.pem, or SCOUT_RELEASE_KEY for another path. Keep a backup of that key somewhere
 * safe and out of the repo. Installed apps only accept builds signed with it, so without it they can't be updated.
 */
import { spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseEnv } from 'node:util';
import { createClient } from '@supabase/supabase-js';

/**
 * Where releases live: a public Supabase Storage bucket. latest.json says what's newest and releases.json lists every
 * release's summary and changelog, newest first. The site serves both at /download/*, and its src/lib/desktop.ts has
 * these same names and shapes, so change them together.
 */
const RELEASE_BUCKET = 'desktop-releases';
const MANIFEST_PATH = 'latest.json';
const HISTORY_PATH = 'releases.json';
/** The keys in `platforms`: Rust's OS and architecture names, which the app looks itself up by. */
const WINDOWS_X64 = 'windows-x86_64';
const WINDOWS_ARM = 'windows-aarch64';

interface DesktopBuild {
  url: string;
  sha256: string;
  size: number;
  /** Ed25519 over "scout-desktop:<platform>:<version>:<sha256>", checked by the app against the key built into it. */
  signature: string;
}

interface DesktopRelease {
  version: string;
  /** The summary and changelog together, for apps from before they were separate. */
  notes: string;
  summary?: string;
  changelog?: string;
  published: string;
  platforms: Record<string, DesktopBuild>;
}

interface DesktopReleaseNote {
  version: string;
  summary: string;
  changelog: string;
  published: string;
}

/** The app's folder, wherever this is run from. */
const GUI = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const KEY_FILE = process.env.SCOUT_RELEASE_KEY ?? path.join(os.homedir(), '.scoutcode', 'desktop-release-key.pem');
/** Supabase's limit per file on the free plan. */
const MAX_BYTES = 50 * 1024 * 1024;

/** `machine` is the architecture field in a Windows exe's header. */
const TARGETS = [
  { platform: WINDOWS_X64, target: 'x86_64-pc-windows-gnullvm', machine: 0x8664 },
  { platform: WINDOWS_ARM, target: 'aarch64-pc-windows-gnullvm', machine: 0xaa64 },
];

const TOOLCHAIN_DLLS = ['libunwind.dll', 'libc++.dll', 'libgcc_s', 'libwinpthread'];

interface Built {
  platform: string;
  exe: Buffer;
}

function fail(message: string): never {
  console.error(`release: ${message}`);
  process.exit(1);
}

/** Fills in what isn't already set from .env beside Cargo.toml (or the repo root), if there is one. */
function loadEnv() {
  for (const file of [path.join(GUI, '.env'), path.join(GUI, '..', '.env')]) {
    if (!fs.existsSync(file)) continue;
    for (const [k, v] of Object.entries(parseEnv(fs.readFileSync(file, 'utf8')))) process.env[k] ??= v;
  }
}

function option(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  if (i < 0) return undefined;
  return process.argv[i + 1] ?? fail(`--${name} needs a value`);
}

/** The key as the app holds it: the 32 raw bytes, in base64. */
function rawPublicKey(key: crypto.KeyObject): string {
  const der = crypto.createPublicKey(key).export({ type: 'spki', format: 'der' });
  return der.subarray(-32).toString('base64');
}

function keygen() {
  if (fs.existsSync(KEY_FILE)) fail(`${KEY_FILE} already exists. Installed apps trust it, so it isn't replaced.`);
  const { privateKey } = crypto.generateKeyPairSync('ed25519');
  fs.mkdirSync(path.dirname(KEY_FILE), { recursive: true });
  fs.writeFileSync(KEY_FILE, privateKey.export({ type: 'pkcs8', format: 'pem' }), { mode: 0o600 });
  console.log(`Saved the private key to ${KEY_FILE}. Back it up, and never commit it.`);
  console.log(`Public key, for PUBLIC_KEY in src/update.rs:\n${rawPublicKey(privateKey)}`);
}

function signingKey(): crypto.KeyObject {
  if (!fs.existsSync(KEY_FILE)) fail(`no signing key at ${KEY_FILE}. Restore your backup, or set SCOUT_RELEASE_KEY.`);
  const key = crypto.createPrivateKey(fs.readFileSync(KEY_FILE));
  // A build signed with another key would be refused by every installed copy, so catch it here.
  const source = fs.readFileSync(path.join(GUI, 'src', 'update.rs'), 'utf8');
  const built = /const PUBLIC_KEY: &str = "([^"]+)"/.exec(source)?.[1];
  if (built !== rawPublicKey(key)) fail(`${KEY_FILE} doesn't match PUBLIC_KEY in src/update.rs.`);
  return key;
}

function cargoVersion(): string {
  const toml = fs.readFileSync(path.join(GUI, 'Cargo.toml'), 'utf8');
  return /^version = "([^"]+)"/m.exec(toml)?.[1] ?? fail('no version in Cargo.toml');
}

/** a.b.c as numbers, for comparing. */
function parts(version: string): number[] {
  const p = version.split('.').map(Number);
  if (p.length !== 3 || p.some((n) => !Number.isInteger(n) || n < 0)) fail(`"${version}" isn't a version like 1.2.3`);
  return p;
}

function isNewer(a: string, b: string): boolean {
  const [x, y] = [parts(a), parts(b)];
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
}

/** The DLLs a 64-bit Windows exe needs to start, from its import table. */
function importedDlls(exe: Buffer): string[] {
  const pe = exe.readUInt32LE(0x3c);
  const optional = pe + 24;
  const sections = optional + exe.readUInt16LE(pe + 20);
  const count = exe.readUInt16LE(pe + 6);
  const offset = (rva: number) => {
    for (let i = 0; i < count; i++) {
      const s = sections + i * 40;
      const start = exe.readUInt32LE(s + 12);
      if (rva >= start && rva < start + exe.readUInt32LE(s + 16)) return rva - start + exe.readUInt32LE(s + 20);
    }
    return fail('an import in the exe is outside its sections');
  };
  const names: string[] = [];
  // In the PE32+ optional header, the data directories start at 112 and imports are the second.
  for (let d = offset(exe.readUInt32LE(optional + 120)); exe.readUInt32LE(d + 12) !== 0; d += 20) {
    const at = offset(exe.readUInt32LE(d + 12));
    names.push(exe.toString('latin1', at, exe.indexOf(0, at)));
  }
  return names;
}

/** The exe for each architecture. An exe built for the wrong one would leave people with an app that can't start. */
function loadBuilds(): Built[] {
  return TARGETS.map(({ platform, target, machine }) => {
    const file = path.join(GUI, 'target', target, 'release', 'scout-desktop.exe');
    if (!fs.existsSync(file)) fail(`${file} doesn't exist. Run this without --skip-build.`);
    const exe = fs.readFileSync(file);
    if (exe.length > MAX_BYTES) fail(`${file} is ${exe.length} bytes, over Supabase's limit of ${MAX_BYTES}.`);
    if (exe.readUInt16LE(exe.readUInt32LE(0x3c) + 4) !== machine) fail(`${file} isn't a ${platform} build.`);
    // These come with the compiler, not with Windows, so the app wouldn't start on anyone else's PC.
    for (const dll of TOOLCHAIN_DLLS) if (exe.includes(dll)) fail(`${file} needs ${dll}. Check .cargo/config.toml.`);
    // Updates only replace the exe, so it can't need this beside it. See src/webview2.rs, and don't look for the
    // name in the whole file as above: the exe carries the DLL itself.
    if (importedDlls(exe).some((d) => d.toLowerCase().startsWith('webview2loader'))) fail(`${file} imports WebView2Loader.dll. Check src/webview2.rs.`);
    return { platform, exe };
  });
}

function describe(platform: string, version: string, url: string, exe: Buffer, key: crypto.KeyObject): DesktopBuild {
  const sha256 = crypto.createHash('sha256').update(exe).digest('hex');
  const message = `scout-desktop:${platform}:${version}:${sha256}`;
  return { url, sha256, size: exe.length, signature: crypto.sign(null, Buffer.from(message), key).toString('base64') };
}

interface Notes {
  summary: string;
  changelog: string;
}

/** What latest.json says, for this release. */
function manifest(version: string, notes: Notes, published: string, platforms: Record<string, DesktopBuild>): DesktopRelease {
  // Apps from before the summary and changelog were separate show `notes` alone.
  const joined = [notes.summary, notes.changelog].filter(Boolean).join('\n\n');
  return { version, notes: joined, summary: notes.summary, changelog: notes.changelog, published, platforms };
}

/** releases.json with this release first, replacing an earlier entry for the same version. */
function withRelease(history: DesktopReleaseNote[], version: string, notes: Notes, published: string): DesktopReleaseNote[] {
  return [{ version, ...notes, published }, ...history.filter((r) => r.version !== version)];
}

async function publish(version: string, notes: Notes, builds: Built[], key: crypto.KeyObject) {
  loadEnv();
  const { SUPABASE_URL: url, SUPABASE_SERVICE_ROLE_KEY: secret } = process.env;
  if (!url || !secret) fail('SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY must be set (the environment, .env here, or the release environment on GitHub).');
  const db = createClient(url, secret, { auth: { persistSession: false, autoRefreshToken: false } });

  const { data: bucket } = await db.storage.getBucket(RELEASE_BUCKET);
  if (!bucket) {
    const { error } = await db.storage.createBucket(RELEASE_BUCKET, { public: true });
    if (error) fail(`could not create the ${RELEASE_BUCKET} bucket: ${error.message}`);
  }

  const res = await fetch(`${url}/storage/v1/object/public/${RELEASE_BUCKET}/${MANIFEST_PATH}?t=${Date.now()}`);
  if (res.ok) {
    const current = (await res.json()) as DesktopRelease;
    if (!isNewer(version, current.version)) fail(`${current.version} is already out. Bump the version in Cargo.toml to something higher than that.`);
  }

  const platforms: Record<string, DesktopBuild> = {};
  for (const { platform, exe } of builds) {
    const exePath = `${platform}/${version}/Scout.exe`;
    console.log(`Uploading ${exePath} (${(exe.length / 1024 / 1024).toFixed(1)} MB)…`);
    const up = await db.storage.from(RELEASE_BUCKET).upload(exePath, exe, { upsert: true, contentType: 'application/octet-stream', cacheControl: '31536000' });
    if (up.error) fail(`upload failed: ${up.error.message}`);
    platforms[platform] = describe(platform, version, db.storage.from(RELEASE_BUCKET).getPublicUrl(exePath).data.publicUrl, exe, key);
  }

  const published = new Date().toISOString();
  // Before latest.json, so an app that just updated finds the notes for the version it updated to.
  const old = await fetch(`${url}/storage/v1/object/public/${RELEASE_BUCKET}/${HISTORY_PATH}?t=${Date.now()}`);
  const history: DesktopReleaseNote[] = old.ok ? await old.json() : [];
  const hist = await db.storage.from(RELEASE_BUCKET).upload(HISTORY_PATH, JSON.stringify(withRelease(history, version, notes, published), null, 2), { upsert: true, contentType: 'application/json', cacheControl: '60' });
  if (hist.error) fail(`could not publish ${HISTORY_PATH}: ${hist.error.message}`);

  // Last, so nobody is pointed at a file that isn't there yet.
  const body = manifest(version, notes, published, platforms);
  const man = await db.storage.from(RELEASE_BUCKET).upload(MANIFEST_PATH, JSON.stringify(body, null, 2), { upsert: true, contentType: 'application/json', cacheControl: '60' });
  if (man.error) fail(`could not publish ${MANIFEST_PATH}: ${man.error.message}`);
  console.log(`Published Scout ${version}. Installed apps pick it up within 6 hours, or at their next start.`);
}

function writeToFolder(out: string, version: string, notes: Notes, urlBase: string, builds: Built[], key: crypto.KeyObject) {
  const platforms: Record<string, DesktopBuild> = {};
  for (const { platform, exe } of builds) {
    fs.mkdirSync(path.join(out, platform), { recursive: true });
    fs.writeFileSync(path.join(out, platform, 'Scout.exe'), exe);
    platforms[platform] = describe(platform, version, `${urlBase.replace(/\/$/, '')}/${platform}/Scout.exe`, exe, key);
  }
  const published = new Date().toISOString();
  const historyFile = path.join(out, HISTORY_PATH);
  const history: DesktopReleaseNote[] = fs.existsSync(historyFile) ? JSON.parse(fs.readFileSync(historyFile, 'utf8')) : [];
  fs.writeFileSync(historyFile, JSON.stringify(withRelease(history, version, notes, published), null, 2));
  fs.writeFileSync(path.join(out, MANIFEST_PATH), JSON.stringify(manifest(version, notes, published, platforms), null, 2));
  console.log(`Wrote ${version} to ${out}. Nothing was uploaded.`);
}

/** The notes for `version` from a changelog with a "## 1.2.3" heading per release. The line after it is the summary. */
function notesFrom(file: string, version: string): Notes {
  const text = fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n');
  const heading = new RegExp(`^## \\[?v?${version.replace(/\./g, '\\.')}\\]?[ \\t]*$`, 'm').exec(text);
  if (!heading) fail(`${file} has no "## ${version}" section. Add one, with a one-line summary under it.`);
  const body = text.slice(heading.index + heading[0].length).split(/^## /m)[0].trim();
  const [summary = '', ...rest] = body.split('\n');
  if (!summary.trim()) fail(`the "## ${version}" section of ${file} is empty.`);
  return { summary: summary.trim(), changelog: rest.join('\n').trim() };
}

/**
 * Whether Cargo.toml's version is newer than the live release, for CI to decide whether to build. It reads the manifest
 * installed apps read (SCOUT_UPDATE_URL overrides it, as in the app), so it needs no keys; publishing checks the bucket
 * itself again. Prints the answer, and adds `version=` and `publish=` to $GITHUB_OUTPUT when that's set.
 */
async function check() {
  const version = cargoVersion();
  parts(version);
  const manifest = process.env.SCOUT_UPDATE_URL || 'https://scoutcode.dev/download/latest.json';
  const res = await fetch(manifest, { cache: 'no-store' });
  // The site answers 404 before the first release. Anything else means we can't tell, so don't guess.
  if (!res.ok && res.status !== 404) fail(`could not read ${manifest}: HTTP ${res.status}`);
  const live = res.ok ? ((await res.json()) as DesktopRelease).version : null;
  const publish = !live || isNewer(version, live);
  console.log(publish ? `${version} is newer than ${live ?? 'nothing'}: publish it.` : `${version} is already out (live: ${live}). Nothing to publish.`);
  if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, `version=${version}\npublish=${publish}\n`);
}

async function main() {
  if (process.argv[2] === 'keygen') return keygen();
  if (process.argv[2] === 'check') return check();
  const out = option('out');
  const version = out ? (option('version') ?? cargoVersion()) : cargoVersion();
  parts(version);
  const key = signingKey();

  // Read before building, so missing notes don't waste a build.
  const file = option('changelog-file');
  const from = option('notes-from');
  const notes: Notes = from ? notesFrom(from, version) : {
    summary: (option('summary') ?? option('notes') ?? '').trim(),
    changelog: (file ? fs.readFileSync(file, 'utf8') : (option('changelog') ?? '')).trim(),
  };
  if (notes.summary.includes('\n')) fail('--summary is one line. Put the rest in --changelog.');

  if (!process.argv.includes('--skip-build')) {
    // The web UI goes into each exe, so it's built first.
    for (const args of [['--prefix', 'ui', 'ci'], ['--prefix', 'ui', 'run', 'build']]) {
      const build = spawnSync('npm', args, { cwd: GUI, stdio: 'inherit', shell: true });
      if (build.status !== 0) fail(`npm ${args.join(' ')} failed`);
    }
    for (const { target } of TARGETS) {
      const build = spawnSync('cargo', ['build', '--release', '--target', target], {
        cwd: GUI,
        stdio: 'inherit',
        shell: true,
        env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS || '2' },
      });
      if (build.status !== 0) fail(`cargo build failed for ${target}`);
    }
  }
  const builds = loadBuilds();

  if (out) return writeToFolder(out, version, notes, option('url-base') ?? fail('--out needs --url-base'), builds, key);
  await publish(version, notes, builds, key);
}

await main();
