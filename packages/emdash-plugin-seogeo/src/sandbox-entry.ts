import { handleAfterSave } from "./plugin.js";
import { tools as mcpTools } from "./mcp.js";

// Stand-in for @emdash-cms/core's definePlugin. The real implementation
// is identity-returning for the sandboxed shape (hooks + routes); this
// shim lets the plugin typecheck and ship before the peer dependency
// is on public npm. Replace the import once @emdash-cms/core publishes.
function definePlugin<T>(plugin: T): T {
  return plugin;
}

// Interaction protocol the host POSTs to /_emdash/api/plugins/<id>/admin.
// Mirrors @emdash-cms/blocks BlockInteraction; redeclared locally so
// the plugin can typecheck without that package installed.
export type BlockInteraction =
  | { type: "page_load"; page: string }
  | {
      type: "block_action";
      action_id: string;
      block_id?: string;
      value?: unknown;
    }
  | {
      type: "form_submit";
      action_id: string;
      block_id?: string;
      values: Record<string, unknown>;
    };

export interface RouteContext {
  request: Request;
  body: BlockInteraction;
}

export interface BlockResponse {
  blocks: unknown[];
  toast?: { message: string; type: "success" | "error" | "info" };
}

// Placeholder until 6.3 (findings page), 6.4 (score widget), and
// 6.5 (document panel) replace each branch with real Block Kit output.
async function handleAdminRoute({
  body,
}: RouteContext): Promise<BlockResponse> {
  const page = "page" in body ? body.page : "(non-page interaction)";
  return {
    blocks: [
      { type: "header", text: "seogeo" },
      {
        type: "context",
        text: `placeholder: received type=${body.type} page=${page}`,
      },
    ],
  };
}

export default definePlugin({
  hooks: {
    "content:afterSave": handleAfterSave,
  },
  routes: {
    admin: { handler: handleAdminRoute },
  },
  // emdash's MCP server picks tools up from this field. The exact
  // host-side spec for plugin-contributed tools is still being mapped;
  // the field is harmless if unrecognized.
  mcpTools,
});
