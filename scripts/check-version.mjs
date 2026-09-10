import fs from 'node:fs';
import path from 'node:path';

const root = process.cwd();
const read = (relative) => fs.readFileSync(path.join(root, relative), 'utf8');
const packageJson = JSON.parse(read('package.json'));
const packageLock = JSON.parse(read('package-lock.json'));
const tauriConfig = JSON.parse(read('src-tauri/tauri.conf.json'));
const cargo = read('Cargo.toml');
const cargoVersion = cargo.match(
  /\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
)?.[1];

const versions = {
  'Cargo.toml [workspace.package]': cargoVersion,
  'package.json': packageJson.version,
  'package-lock.json': packageLock.packages?.['']?.version,
  'src-tauri/tauri.conf.json': tauriConfig.version,
};
const missing = Object.entries(versions).filter(([, version]) => !version);
const distinct = new Set(Object.values(versions));
if (missing.length || distinct.size !== 1) {
  throw new Error(`version mismatch: ${JSON.stringify(versions)}`);
}

console.log(`version consistency: ${[...distinct][0]}`);
