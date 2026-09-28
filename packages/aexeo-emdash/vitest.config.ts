import { emdashPluginTest } from "@emdash-cms/plugin-test/config";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [emdashPluginTest({ dir: "./tests/fixtures/plugin-host" })],
  test: {
    // `createPluginRuntimeTestHost` compiles the fixture plugin twice (the
    // runtime bundle and the type probe) before a single assertion runs, and
    // it does that on a cold rolldown cache. On a clean CI runner or a slow
    // filesystem that comfortably exceeds vitest's 5s default, which fails
    // the test before any behavior is exercised.
    testTimeout: 60_000,
    // The host owns a workerd instance; the default single fork is correct,
    // but the suite must not reuse a process between files either.
    pool: "forks",
    exclude: [
      "**/node_modules/**",
      "**/dist/**",
      // AppleDouble resource-fork sidecars (`._<name>`) are created by macOS
      // when a checkout lands on a non-HFS filesystem such as an external
      // exFAT volume. Vitest would otherwise collect `._foo.test.ts` as a
      // second copy of the real suite, which boots a competing workerd
      // instance and makes both files fail with confusing D1 errors.
      "**/._*",
    ],
  },
});
