import type { ContentPresetOptions } from "./content-evaluator.js";

/** Only host-configured fields may be evaluated as fragments. */
export function selectFragmentPreset(options: ContentPresetOptions | undefined, collection: string, field: string): string | undefined {
  if (!options || (options.collections && !options.collections.includes(collection))) return undefined;
  const collections = options.fragmentFieldsByCollection;
  if (!collections || !Object.hasOwn(collections, collection)) return undefined;
  const fields = collections[collection];
  if (!fields || !Object.hasOwn(fields, field)) return undefined;
  const preset = fields[field];
  return typeof preset === "string" && preset.trim().length > 0 ? preset : undefined;
}
