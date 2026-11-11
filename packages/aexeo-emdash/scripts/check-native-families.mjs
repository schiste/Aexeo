// Run in Node: the plugin test host replaces the filesystem with a sandbox.
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/cms-coverage.ts", import.meta.url), "utf8");
const registry = readFileSync(new URL("../../../crates/aexeo-core/src/registry.rs", import.meta.url), "utf8");
const pattern = source.match(/const NATIVE_RULE_ID = \/([^\n/]+)\//)?.[1];
if (!pattern) throw new Error("CMS native rule pattern not found");
const nativeRule = new RegExp(pattern);
const ids = [...registry.matchAll(/rule_id:\s*"([A-Z][A-Z0-9]*[0-9]{3})"/g)].map((match) => match[1]);
if (ids.length < 100) throw new Error("Native rule registry not found or unexpectedly incomplete");
const missing = ids.filter((id) => !nativeRule.test(id));
if (missing.length) throw new Error(`CMS native rule filter does not recognize: ${missing.join(", ")}`);
console.log(`CMS native rule filter covers ${ids.length} registered rules`);
