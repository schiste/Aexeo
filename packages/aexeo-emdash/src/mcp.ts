import type { EmdashDocument, Finding } from "./types.js";
import { evaluate, errorFindings, findingsByRoute } from "./evaluator.js";
import {
  type IndexNowConfig,
  type IndexNowSubmission,
  submitIndexNow,
} from "./indexnow.js";

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
  name: "aexeo.check",
  description:
    "Evaluate a list of EmdashDocument values against the Aexeo rule engine and return the stable Finding contract. Mirrors the CLI's `aexeo-cli check` surface so agents, the admin UI, and the CI gate all see the same rule ids.",
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
          "Optional JSON-serialized Aexeo config. When absent the evaluator uses Config::default(), matching a cold CLI run.",
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

export interface IndexNowSubmitInput {
  config: IndexNowConfig;
  urls: string[];
}

const indexNowSubmitTool: McpToolDefinition<
  IndexNowSubmitInput,
  IndexNowSubmission
> = {
  name: "aexeo.indexnow.submit",
  description:
    "Submit a list of URLs to the IndexNow freshness endpoint. Mirrors the aexeo-cli `indexnow submit` contract: the IndexNow protocol requires every submitted URL to belong to the configured site host. URLs that do not match the host are returned in `rejected` and never sent over the wire.",
  inputSchema: {
    type: "object",
    properties: {
      config: {
        type: "object",
        description:
          "IndexNow configuration: siteUrl, key, and optional keyLocation override.",
        properties: {
          siteUrl: { type: "string" },
          key: { type: "string" },
          keyLocation: { type: "string" },
        },
        required: ["siteUrl", "key"],
      },
      urls: {
        type: "array",
        items: { type: "string" },
        description: "Absolute URLs to submit. Must share siteUrl's host.",
      },
    },
    required: ["config", "urls"],
  },
  handler: async ({ config, urls }) => submitIndexNow(config, urls),
};

// Exposed as a readonly tuple so the plugin registers tools in a stable
// order. aexeo.intelligence.score and aexeo.generate.machine_bundle
// will join here once the bridge grows the WASM-side surface to back
// them; submission and check are the two flows that work today.
export const tools = [checkTool, indexNowSubmitTool] as const;
