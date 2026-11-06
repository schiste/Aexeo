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

async function sync(path, constant) {
  const next = `export const ${constant} = ${JSON.stringify(version)};\n`;
  const current = await readFile(path, "utf8").catch(() => "");
  if (current !== next) {
    await writeFile(path, next);
    console.log(`synced ${path} -> ${version}`);
  }
}

await sync(versionFilePath, "PACKAGE_VERSION");
// The sidecar reports this back, so the plugin can warn when the two differ.
await sync(resolve(root, "../aexeo-crawl-worker/src/version.ts"), "WORKER_VERSION");
