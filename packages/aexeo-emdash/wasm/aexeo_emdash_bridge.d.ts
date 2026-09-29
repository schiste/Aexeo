/* tslint:disable */
/* eslint-disable */

export function evaluateDocuments(documents_json: string, config_json?: string | null): string;

/**
 * Generate the LLM authoring prompt for a truth manifest. The plugin calls
 * this when the editor opens "Generate authoring prompt" — the returned
 * string is what the editor pastes into Claude/GPT/etc.
 */
export function generateFactsPrompt(documents_json: string): string;

export function scoreIntelligence(documents_json: string, manifest_json?: string | null): string;

/**
 * Validate a candidate `facts.json`. Returns a JSON object with two top-level
 * fields: `validation` (shape / schema check) and `assessment` (truth-layer
 * audit including mismatches against the site). The plugin renders this in
 * the validate-paste UI.
 */
export function validateFactsManifest(manifest_json: string, documents_json: string): string;
