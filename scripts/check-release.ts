import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dir, "..");
const pkg = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
const config = JSON.parse(readFileSync(resolve(root, "src-tauri/tauri.conf.json"), "utf8"));
const cargo = readFileSync(resolve(root, "src-tauri/Cargo.toml"), "utf8");
const lock = readFileSync(resolve(root, "Cargo.lock"), "utf8");
const cargoVersion = cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const lockVersion = lock.match(/\[\[package\]\]\s+name = "gitjuke"\s+version = "([^"]+)"/)?.[1];
const version = pkg.version;
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`Unsupported release version: ${version}`);
}
if ([config.version, cargoVersion, lockVersion].some((value) => value !== version)) {
  throw new Error("Versions must match in package.json, tauri.conf.json, src-tauri/Cargo.toml, and Cargo.lock");
}
const tag = process.argv[2];
if (tag && tag !== `v${version}`) {
  throw new Error(`Tag ${tag} does not match v${version}`);
}
console.log(`GitJuke ${version}${tag ? ` (${tag})` : ""}`);
