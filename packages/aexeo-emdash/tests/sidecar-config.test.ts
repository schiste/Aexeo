import { describe, expect, it } from "vitest";

import {
  CONFIG_TOKEN_KEY,
  CONFIG_URL_KEY,
  PLUGIN_SETTINGS_SCHEMA,
  SETTINGS_TOKEN_KEY,
  type KvNamespace,
  type PluginSettingsAccess,
  readSidecarConfig,
  writeSidecarConfig,
} from "../src/plugin.js";
import { validateSidecarUrl } from "../src/sidecar-url.js";
import { aexeoPluginSandboxed } from "../src/sandbox.js";

// --- SSRF allowlist on the Setup form -------------------------------------
//
// The URL pasted on the Setup page is the destination of a credentialed
// POST: sidecar.ts appends /evaluate and sends `Authorization: Bearer
// <EVAL_TOKEN>`. These lock in the three properties that has to hold.

describe("validateSidecarUrl", () => {
  const ALLOWED = ["aexeo-crawl-worker.example.workers.dev"];

  it("accepts a public https host on the allowlist", () => {
    expect(
      validateSidecarUrl("https://aexeo-crawl-worker.example.workers.dev", ALLOWED),
    ).toEqual({
      ok: true,
      url: "https://aexeo-crawl-worker.example.workers.dev",
    });
  });

  it("accepts a port, a path, and surrounding whitespace", () => {
    expect(
      validateSidecarUrl(
        "  https://aexeo-crawl-worker.example.workers.dev:8787/  ",
        ALLOWED,
      ),
    ).toEqual({
      ok: true,
      url: "https://aexeo-crawl-worker.example.workers.dev:8787/",
    });
  });

  it("matches the allowlist case-insensitively", () => {
    expect(
      validateSidecarUrl("https://AEXEO-Crawl-Worker.example.workers.dev", ALLOWED)
        .ok,
    ).toBe(true);
  });

  it("skips the allowlist check when no allowlist could be resolved", () => {
    expect(
      validateSidecarUrl("https://aexeo-crawl-worker.example.workers.dev").ok,
    ).toBe(true);
  });

  it("requires https so the bearer token never crosses the wire in cleartext", () => {
    const result = validateSidecarUrl("http://aexeo-crawl-worker.example.workers.dev", ALLOWED);
    expect(result.ok).toBe(false);
    expect(result.ok === false && result.error).toMatch(/https/i);
  });

  it("rejects non-http schemes outright", () => {
    for (const raw of [
      "file:///etc/passwd",
      "ftp://aexeo-crawl-worker.example.workers.dev",
      "javascript:alert(1)",
      "data:text/plain,hi",
    ]) {
      expect(validateSidecarUrl(raw, ALLOWED).ok, raw).toBe(false);
    }
  });

  it("rejects an unparseable or empty URL", () => {
    expect(validateSidecarUrl("", ALLOWED).ok).toBe(false);
    expect(validateSidecarUrl("   ", ALLOWED).ok).toBe(false);
    expect(validateSidecarUrl("not a url", ALLOWED).ok).toBe(false);
  });

  it("rejects loopback, unspecified, and localhost names", () => {
    for (const raw of [
      "https://localhost",
      "https://localhost:8787",
      "https://app.localhost",
      "https://0.0.0.0",
      "https://127.0.0.1",
      "https://127.1.2.3",
      // Non-canonical IPv4 spellings; `new URL()` normalizes all of them
      // to 127.0.0.1 before we see them.
      "https://2130706433",
      "https://0x7f.1",
      "https://0177.0.0.1",
    ]) {
      expect(validateSidecarUrl(raw, ALLOWED).ok, raw).toBe(false);
    }
  });

  it("rejects the RFC1918 private ranges", () => {
    for (const raw of [
      "https://10.0.0.5",
      "https://10.255.255.254",
      "https://192.168.1.1",
      "https://172.16.0.1",
      "https://172.20.10.1",
      "https://172.31.255.254",
    ]) {
      expect(validateSidecarUrl(raw, ALLOWED).ok, raw).toBe(false);
    }
  });

  it("keeps 172.15/172.32 public — they are outside RFC1918", () => {
    // No allowlist here: the point is the range arithmetic, and every
    // non-allowlisted host fails the host check regardless.
    for (const raw of ["https://172.15.0.1", "https://172.32.0.1"]) {
      expect(validateSidecarUrl(raw).ok, raw).toBe(true);
    }
  });

  it("rejects the cloud instance metadata endpoint and the rest of link-local", () => {
    for (const raw of [
      "https://169.254.169.254",
      "https://169.254.0.1",
      "https://100.100.100.200",
    ]) {
      expect(validateSidecarUrl(raw, ALLOWED).ok, raw).toBe(false);
    }
  });

  it("rejects IPv6 loopback, unspecified, mapped, and private addresses", () => {
    for (const raw of [
      "https://[::1]",
      "https://[::]",
      // URL normalizes this to [::ffff:7f00:1]; both spellings must fail.
      "https://[::ffff:127.0.0.1]",
      "https://[::ffff:7f00:1]",
      "https://[fd00::1]",
      "https://[fe80::1]",
    ]) {
      expect(validateSidecarUrl(raw, ALLOWED).ok, raw).toBe(false);
    }
  });

  it("rejects a host that is not on the plugin's outbound allowlist", () => {
    const result = validateSidecarUrl("https://attacker.example.net", ALLOWED);
    expect(result.ok).toBe(false);
    expect(result.ok === false && result.error).toContain(
      "aexeo-crawl-worker.example.workers.dev",
    );
  });

  it("does not let an allowlisted host be smuggled in via userinfo or a query", () => {
    // `new URL().hostname` is the authority after the userinfo separator,
    // so both of these resolve to attacker.example.net.
    for (const raw of [
      "https://aexeo-crawl-worker.example.workers.dev@attacker.example.net/",
      "https://attacker.example.net/?next=https://aexeo-crawl-worker.example.workers.dev",
    ]) {
      expect(validateSidecarUrl(raw, ALLOWED).ok, raw).toBe(false);
    }
  });

  it("honors a *. wildcard in the allowlist, as the host does", () => {
    expect(
      validateSidecarUrl("https://a.b.example.com", ["*.example.com"]).ok,
    ).toBe(true);
    // The wildcard must not match the bare parent domain.
    expect(validateSidecarUrl("https://example.com", ["*.example.com"]).ok).toBe(
      false,
    );
    expect(validateSidecarUrl("https://notexample.com", ["*.example.com"]).ok).toBe(
      false,
    );
  });

  it("rejects a non-https URL before it ever consults the allowlist", () => {
    // Ordering matters: an http:// URL on an allowed host is still a
    // cleartext credential leak, and an http:// URL to a private host
    // should not be reported as merely "not on the allowlist".
    const result = validateSidecarUrl("http://127.0.0.1", ALLOWED);
    expect(result.ok === false && result.error).toMatch(/https/i);
  });
});

// --- Descriptor wiring -----------------------------------------------------

describe("aexeoPluginSandboxed", () => {
  it("declares the EVAL_TOKEN setting as a secret so the host encrypts it", () => {
    const descriptor = aexeoPluginSandboxed({
      evaluatorHost: "aexeo-crawl-worker.example.workers.dev",
    });
    expect(descriptor.settingsSchema[SETTINGS_TOKEN_KEY]).toEqual({
      type: "secret",
      label: "EVAL_TOKEN",
      description: expect.any(String),
    });
  });

  it("keeps the same schema the token is written under", () => {
    // The key in settingsSchema and the key the plugin calls
    // `ctx.settings.set()` with have to be the same string, or the host
    // will not encrypt the write. Pin both sides.
    const descriptor = aexeoPluginSandboxed({ evaluatorHost: "evaluator.test" });
    expect(Object.keys(descriptor.settingsSchema)).toEqual([SETTINGS_TOKEN_KEY]);
    expect(PLUGIN_SETTINGS_SCHEMA[SETTINGS_TOKEN_KEY]?.type).toBe("secret");
  });
});

// --- Token storage ---------------------------------------------------------

interface Harness {
  store: { kv: KvNamespace; settings?: PluginSettingsAccess };
  kv: Map<string, unknown>;
  settings: Map<string, unknown>;
  warn: string[];
}

function harness(options: { withSettings: boolean }): Harness {
  const kv = new Map<string, unknown>();
  const settings = new Map<string, unknown>();
  const warn: string[] = [];
  const kvNamespace: KvNamespace = {
    get: async <T>(key: string) => (kv.get(key) as T | undefined) ?? null,
    set: async (key, value) => {
      kv.set(key, value);
    },
    delete: async (key) => kv.delete(key),
    list: async <T>() =>
      [...kv.entries()].map(([key, value]) => ({ key, value: value as T })),
  };
  const settingsAccess: PluginSettingsAccess = {
    // Models emdash's createSettingsAccess closely enough to prove the
    // point: a key declared `type: "secret"` is handed to `set` in
    // plaintext and lands in the store as an opaque envelope, and `get`
    // decrypts it back. No production code path ever sees the envelope.
    get: async <T>(key: string) => {
      const stored = settings.get(key);
      if (typeof stored === "string") {
        return stored as T;
      }
      if (stored !== null && typeof stored === "object" && "ciphertext" in stored) {
        return String(stored.ciphertext) as T;
      }
      return null;
    },
    set: async (key, value) => {
      settings.set(key, { $emdash: "plugin-setting", ciphertext: value });
    },
  };
  return {
    kv,
    settings,
    warn,
    store: {
      kv: kvNamespace,
      ...(options.withSettings ? { settings: settingsAccess } : {}),
    },
  };
}

describe("sidecar config storage", () => {
  const CONFIG = { url: "https://evaluator.test", token: "super-secret-token" };

  it("stores the token in settings and never writes it to KV", async () => {
    const h = harness({ withSettings: true });
    await writeSidecarConfig(h.store, CONFIG);

    expect(h.settings.get(SETTINGS_TOKEN_KEY)).toEqual({
      $emdash: "plugin-setting",
      ciphertext: CONFIG.token,
    });
    // The URL is not sensitive and stays in KV; the token must not be
    // anywhere in the KV namespace.
    expect(h.kv.get(CONFIG_URL_KEY)).toBe(CONFIG.url);
    expect([...h.kv.keys()]).toEqual([CONFIG_URL_KEY]);
    expect(JSON.stringify([...h.kv.entries()])).not.toContain(CONFIG.token);
    expect(h.warn).toEqual([]);
  });

  it("reads the token back out of settings", async () => {
    const h = harness({ withSettings: true });
    await writeSidecarConfig(h.store, CONFIG);
    expect(await readSidecarConfig(h.store)).toEqual(CONFIG);
  });

  it("deletes the legacy plaintext KV copy when the token is re-saved", async () => {
    const h = harness({ withSettings: true });
    h.kv.set(CONFIG_URL_KEY, CONFIG.url);
    h.kv.set(CONFIG_TOKEN_KEY, "old-plaintext-token");
    await writeSidecarConfig(h.store, CONFIG);
    expect(h.kv.has(CONFIG_TOKEN_KEY)).toBe(false);
  });

  it("falls back to KV on a host that has no settings accessor", async () => {
    const h = harness({ withSettings: false });
    await writeSidecarConfig(h.store, CONFIG);
    expect(h.kv.get(CONFIG_TOKEN_KEY)).toBe(CONFIG.token);
    expect(await readSidecarConfig(h.store)).toEqual(CONFIG);
  });

  it("reads a pre-migration install whose token only exists in KV", async () => {
    const h = harness({ withSettings: true });
    h.kv.set(CONFIG_URL_KEY, CONFIG.url);
    h.kv.set(CONFIG_TOKEN_KEY, CONFIG.token);
    expect(await readSidecarConfig(h.store)).toEqual(CONFIG);
  });

  it("prefers settings over a stale KV copy", async () => {
    const h = harness({ withSettings: true });
    h.kv.set(CONFIG_URL_KEY, CONFIG.url);
    h.kv.set(CONFIG_TOKEN_KEY, "stale");
    h.settings.set(SETTINGS_TOKEN_KEY, "fresh");
    expect(await readSidecarConfig(h.store)).toEqual({
      url: CONFIG.url,
      token: "fresh",
    });
  });

  it("warns and falls back to KV when the host cannot encrypt the secret", async () => {
    // What emdash does when EMDASH_ENCRYPTION_KEY is unset: the write
    // throws PluginSettingEncryptionError. The Setup page must still work.
    const h = harness({ withSettings: true });
    const failing: PluginSettingsAccess = {
      get: async () => null,
      set: async () => {
        throw new Error("Plugin secret setting could not be encrypted");
      },
    };
    await writeSidecarConfig({ ...h.store, settings: failing, log: { warn: (m) => h.warn.push(m) } }, CONFIG);

    expect(h.kv.get(CONFIG_TOKEN_KEY)).toBe(CONFIG.token);
    expect(h.warn).toHaveLength(1);
    expect(h.warn[0]).toMatch(/EMDASH_ENCRYPTION_KEY/);
  });

  it("never puts the token in a warning, even if the error quotes it", async () => {
    const h = harness({ withSettings: true });
    const leaky: PluginSettingsAccess = {
      get: async () => null,
      set: async (_key, value) => {
        throw new Error(`failed to encrypt "${String(value)}"`);
      },
    };
    await writeSidecarConfig({ ...h.store, settings: leaky, log: { warn: (m) => h.warn.push(m) } }, CONFIG);

    expect(h.warn).toHaveLength(1);
    expect(h.warn[0]).not.toContain(CONFIG.token);
    expect(h.warn[0]).toContain("[redacted]");
  });

  it("falls back to KV when the encrypted value cannot be decoded", async () => {
    const h = harness({ withSettings: true });
    h.kv.set(CONFIG_URL_KEY, CONFIG.url);
    h.kv.set(CONFIG_TOKEN_KEY, "legacy-token");
    const throwing: PluginSettingsAccess = {
      get: async () => {
        throw new Error("Plugin secret setting has an invalid encrypted envelope");
      },
      set: async () => {},
    };
    expect(
      await readSidecarConfig({
        ...h.store,
        settings: throwing,
        log: { warn: (m) => h.warn.push(m) },
      }),
    ).toEqual({ url: CONFIG.url, token: "legacy-token" });
    expect(h.warn).toHaveLength(1);
  });

  it("reports no configuration until both halves are present", async () => {
    const h = harness({ withSettings: true });
    expect(await readSidecarConfig(h.store)).toBeNull();

    h.kv.set(CONFIG_URL_KEY, CONFIG.url);
    expect(await readSidecarConfig(h.store)).toBeNull();

    h.settings.set(SETTINGS_TOKEN_KEY, CONFIG.token);
    expect(await readSidecarConfig(h.store)).toEqual(CONFIG);

    h.settings.set(SETTINGS_TOKEN_KEY, "");
    expect(await readSidecarConfig(h.store)).toBeNull();
  });
});
