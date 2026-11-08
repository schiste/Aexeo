import { describe, expect, it } from "vitest";

import { evaluateViaSidecar, type SidecarHttp } from "../src/sidecar.js";
import { PACKAGE_VERSION } from "../src/version.js";

function run(version: string | null, withHeaders = true) {
  const warnings: string[] = [];
  const http: SidecarHttp = {
    fetch: async () => ({
      status: 200,
      ok: true,
      text: async () => "[]",
      ...(withHeaders ? { headers: new Headers(version === null ? {} : { "x-aexeo-version": version }) } : {}),
    }),
  };
  return evaluateViaSidecar(
    http,
    {
      url: "https://evaluator.test",
      authToken: "token",
      log: { warn: (message) => warnings.push(message) },
    },
    [],
  ).then((result) => ({ result, warnings }));
}

describe("sidecar version check", () => {
  it("stays quiet when the versions match", async () => {
    const { result, warnings } = await run(PACKAGE_VERSION);
    expect(result).toEqual({ ok: true, findings: [] });
    expect(warnings).toEqual([]);
  });

  it("warns on a mismatch and still uses the findings", async () => {
    const { result, warnings } = await run("0.0.1");
    expect(result).toEqual({ ok: true, findings: [] });
    expect(warnings).toHaveLength(1);
    expect(warnings[0]).toContain("0.0.1");
    expect(warnings[0]).toContain(PACKAGE_VERSION);
  });

  it.each([
    ["a worker that predates the header", null, true],
    ["a host that exposes no headers", null, false],
  ])("warns for %s", async (_name, version, withHeaders) => {
    const { warnings } = await run(version, withHeaders);
    expect(warnings[0]).toContain("unknown");
  });
});
