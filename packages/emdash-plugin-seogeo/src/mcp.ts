import type { EmdashDocument, Finding } from "./types.js";
import { evaluate, errorFindings, findingsByRoute } from "./evaluator.js";

// Shape of an MCP tool definition the plugin will hand to emdash's
// MCP server. The real MCP type from the host takes over once it is
// present; until then this shape keeps the definitions typechecked
// and stable across the handler file.
export interface McpToolDefinition<Input, Output> {
  name: string;
  description: string;
  inputSchema: unknown;
  handler: (input: Input) => Promise<Output>;
}

export interface CheckInput {
  documents: EmdashDocument[];
  configJson?: string;
}

export interface CheckOutput {
  findings: Finding[];
  errors: Finding[];
  byRoute: Record<string, Finding[]>;
  totals: {
    findings: number;
    errors: number;
    warnings: number;
  };
}

const checkTool: McpToolDefinition<CheckInput, CheckOutput> = {
  name: "seogeo.check",
  description:
    "Evaluate a list of EmdashDocument values against the seogeo rule engine and return the stable Finding contract. Mirrors the CLI's `seogeo-cli check` surface so agents, the admin UI, and the CI gate all see the same rule ids.",
  inputSchema: {
    type: "object",
    properties: {
      documents: {
        type: "array",
        description:
          "Array of EmdashDocument values to evaluate. Each entry carries route, title, optional description/canonical/lang, alternates, meta map, schema, and Portable Text body.",
      },
      configJson: {
        type: "string",
        description:
          "Optional JSON-serialized seogeo Config. When absent the evaluator uses Config::default(), matching a cold CLI run.",
      },
    },
    required: ["documents"],
  },
  handler: async ({ documents, configJson }) => {
    const options = configJson === undefined ? {} : { configJson };
    const findings = await evaluate(documents, options);
    const errors = errorFindings(findings);
    const grouped = findingsByRoute(findings);
    const byRoute: Record<string, Finding[]> = {};
    for (const [route, items] of grouped.entries()) {
      byRoute[route] = items;
    }
    return {
      findings,
      errors,
      byRoute,
      totals: {
        findings: findings.length,
        errors: errors.length,
        warnings: findings.length - errors.length,
      },
    };
  },
};

// Exposed as a readonly tuple so the plugin registers tools in a stable
// order. The next MCP tools (seogeo.intelligence.score,
// seogeo.generate.machine_bundle, seogeo.indexnow.submit) get added here
// as the bridge grows the WASM-side surface to back them.
export const tools = [checkTool] as const;
