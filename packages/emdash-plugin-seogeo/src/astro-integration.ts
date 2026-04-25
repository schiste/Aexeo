import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

// Minimal AstroIntegration shape the integration uses. The real types
// from "astro" take over once it is installed as a peer dependency.
export interface AstroBuildDoneContext {
  dir: URL;
  logger: { info(message: string): void; error(message: string): void };
}

export interface AstroIntegration {
  name: string;
  hooks: {
    "astro:build:done": (context: AstroBuildDoneContext) => Promise<void>;
  };
}

export interface SeogeoIntegrationOptions {
  // Path (or PATH-resolvable name) of the seogeo CLI. Defaults to the
  // binary name the install script drops into the user's environment.
  command?: string;
  // Extra flags to forward to `seogeo-cli check`. Use this to scope
  // severities, add --regressions-only, or point at a non-default config.
  extraArgs?: readonly string[];
}

// Run seogeo-cli check against Astro's built output. The CLI already
// exits 1 when blocking findings are present, so the gate inherits the
// stable exit-code contract; no JSON parsing required.
export function seogeoIntegration(
  options: SeogeoIntegrationOptions = {},
): AstroIntegration {
  const command = options.command ?? "seogeo-cli";
  const extraArgs = options.extraArgs ?? [];
  return {
    name: "aexeo-seogeo",
    hooks: {
      "astro:build:done": async ({ dir, logger }) => {
        const distPath = fileURLToPath(dir);
        logger.info(`aexeo-seogeo: running ${command} check ${distPath}`);
        const exitCode = await runCommand(command, [
          "check",
          distPath,
          ...extraArgs,
        ]);
        if (exitCode === 0) {
          return;
        }
        const message =
          `aexeo-seogeo: blocking findings detected in ${distPath} ` +
          `(${command} exited ${exitCode})`;
        logger.error(message);
        throw new Error(message);
      },
    },
  };
}

function runCommand(command: string, args: readonly string[]): Promise<number> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, [...args], { stdio: "inherit" });
    child.once("error", reject);
    child.once("close", (code: number | null) => {
      resolve(code ?? 1);
    });
  });
}
