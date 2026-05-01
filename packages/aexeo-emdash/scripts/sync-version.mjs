import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const packageJsonPath = resolve(root, "package.json");
const versionFilePath = resolve(root, "src/version.ts");

const packageJson = JSON.parse(await readFile(packageJsonPath, "utf8"));
const version = packageJson.version;

if (typeof version !== "string" || version.length === 0) {
  throw new Error("package.json is missing a string version");
}

const next = `export const PACKAGE_VERSION = ${JSON.stringify(version)};\n`;
const current = await readFile(versionFilePath, "utf8").catch(() => "");

if (current !== next) {
  await writeFile(versionFilePath, next);
  console.log(`synced src/version.ts -> ${version}`);
}
