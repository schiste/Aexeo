import { describe, expect, it, vi } from "vitest";

// Uses the real definePlugin, so this also proves the host's normalization
// keeps what it needs for buildRouteMeta. Only the WASM is out of the picture.
vi.mock("../src/wasm-init.js", () => ({}));

const { createPlugin } = await import("../src/configured.js");

describe("configured plugin declaration", () => {
  const plugin = createPlugin() as unknown as {
    capabilities: string[];
    allowedHosts: string[];
    hooks: Record<string, unknown>;
    routes: Record<string, { permission?: string }>;
  };

  it("lets editors read findings and keeps every writing route admin", () => {
    const permissions = Object.fromEntries(
      Object.entries(plugin.routes).map(([name, route]) => [
        name,
        route.permission,
      ]),
    );
    expect(permissions).toEqual({
      admin: "plugins:read",
      data: "plugins:read",
      refresh: "plugins:manage",
      facts: "plugins:manage",
      presence: "plugins:manage",
    });
  });

  it("declares the network access the presence route uses", () => {
    expect(plugin.capabilities).toContain("network:request");
    expect(plugin.allowedHosts).toEqual(
      expect.arrayContaining(["rdap.org", "index.commoncrawl.org"]),
    );
  });

  it("registers the delete hook", () => {
    expect(plugin.hooks["content:afterDelete"]).toBeDefined();
  });
});
