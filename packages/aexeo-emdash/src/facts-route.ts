// HTTP route handler for the /facts admin page.
//
// Four operations multiplexed onto one POST endpoint via a "kind" field
// in the JSON body:
//
//   { kind: "data" }                       — read current manifest from KV
//   { kind: "prompt" }                     — generate the LLM authoring prompt
//   { kind: "validate", manifest_json }    — validate a pasted manifest
//   { kind: "save",     manifest_json }    — validate then persist on success
//
// Multiplexing onto one route keeps emdash's plugin route surface small
// (each top-level handler costs a route declaration in the descriptor).
// A four-route fan-out would also work but adds churn for marginal benefit.

import type { SandboxCtx } from "./plugin.js";
import { FACTS_KEY, readAllDocuments, readStoredFacts } from "./plugin.js";
import {
  generateFactsPrompt,
  validateFactsManifest,
} from "./wasm-init.js";

interface FactsBody {
  kind?: string;
  manifest_json?: string;
}

interface RouteContext extends SandboxCtx {
  input?: unknown;
}

export async function handleFactsRoute(ctx: RouteContext): Promise<unknown> {
  const body = (ctx.input ?? {}) as FactsBody;
  switch (body.kind) {
    case "data":
      return await handleData(ctx);
    case "prompt":
      return await handlePrompt(ctx);
    case "validate":
      return await handleValidate(ctx, body);
    case "save":
      return await handleSave(ctx, body);
    default:
      return {
        error: {
          code: "unknown_kind",
          message: `unknown facts route kind: ${String(body.kind)}`,
        },
      };
  }
}

async function handleData(ctx: SandboxCtx): Promise<unknown> {
  const manifest = await readStoredFacts(ctx.kv);
  return {
    data: {
      manifest,
      // The presence flag lets the React component branch on
      // "show authoring CTA" vs "show 'manifest stored' state" without
      // having to introspect the manifest shape.
      present: manifest !== null,
    },
  };
}

async function handlePrompt(ctx: SandboxCtx): Promise<unknown> {
  const documents = await readAllDocuments(ctx.kv);
  if (documents.length === 0) {
    return {
      error: {
        code: "no_documents",
        message:
          "no documents indexed yet — click Refresh on the Findings page first, then come back to author the manifest",
      },
    };
  }
  const prompt = await generateFactsPrompt(JSON.stringify(documents));
  return { data: { prompt } };
}

async function handleValidate(
  ctx: SandboxCtx,
  body: FactsBody,
): Promise<unknown> {
  if (typeof body.manifest_json !== "string" || body.manifest_json.length === 0) {
    return {
      error: {
        code: "missing_manifest",
        message: "manifest_json field is required",
      },
    };
  }
  const documents = await readAllDocuments(ctx.kv);
  try {
    const raw = await validateFactsManifest(
      body.manifest_json,
      JSON.stringify(documents),
    );
    return { data: JSON.parse(raw) };
  } catch (error) {
    return {
      error: {
        code: "validate_failed",
        message: error instanceof Error ? error.message : String(error),
      },
    };
  }
}

async function handleSave(
  ctx: SandboxCtx,
  body: FactsBody,
): Promise<unknown> {
  if (typeof body.manifest_json !== "string" || body.manifest_json.length === 0) {
    return {
      error: {
        code: "missing_manifest",
        message: "manifest_json field is required",
      },
    };
  }
  // Re-validate at save time so the editor can't accidentally persist a
  // candidate that was edited after the last validate click. This is
  // also the gate that prevents shape-invalid JSON from reaching KV.
  const documents = await readAllDocuments(ctx.kv);
  let validation: { valid: boolean; errors: string[] };
  try {
    const raw = await validateFactsManifest(
      body.manifest_json,
      JSON.stringify(documents),
    );
    const parsed = JSON.parse(raw) as {
      validation: { valid: boolean; errors: string[] };
    };
    validation = parsed.validation;
  } catch (error) {
    return {
      error: {
        code: "validate_failed",
        message: error instanceof Error ? error.message : String(error),
      },
    };
  }
  if (!validation.valid || validation.errors.length > 0) {
    return {
      error: {
        code: "validation_failed",
        message: `manifest does not pass shape validation: ${validation.errors.slice(0, 3).join("; ")}`,
      },
    };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(body.manifest_json);
  } catch (error) {
    return {
      error: {
        code: "parse_failed",
        message: error instanceof Error ? error.message : String(error),
      },
    };
  }
  await ctx.kv.set(FACTS_KEY, parsed);
  return { data: { saved: true } };
}
