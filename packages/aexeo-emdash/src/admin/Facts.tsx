// Truth-manifest authoring + validation page.
//
// Three things on one page, in editorial order:
//
//   1. Status — is a facts.json stored? When was it last touched?
//   2. Generate prompt — copy a populated LLM authoring prompt to the
//      clipboard so the editor can paste it into Claude/GPT/etc.
//   3. Paste & validate — textarea for the LLM's JSON output. The
//      "Validate" button runs validateFactsManifest via the bridge;
//      the "Save" button persists to KV (FACTS_KEY) after a successful
//      validation.
//
// The generate path is intentionally clipboard-only — the editor takes
// the prompt away to a separate LLM tab. The plugin owns validation,
// not generation.

import * as React from "react";
import { useCallback, useEffect, useState } from "react";

const PLUGIN_ID = "aexeo-seogeo";
const API_BASE = "/_emdash/api";

interface ManifestData {
  manifest: unknown | null;
  present: boolean;
}

interface ValidationResult {
  validation: {
    valid: boolean;
    errors: string[];
    warnings: string[];
    organization_present: boolean;
    product_count: number;
    preferred_term_count: number;
    forbidden_term_count: number;
  };
  assessment: {
    score: number;
    score_ceiling: number;
    pages_analyzed: number;
    pages_with_schema: number;
    structured_truth_source: string;
    mismatches: Array<{
      route: string;
      field: string;
      expected: string;
      observed: string;
      source: string;
      severity: "error" | "warning";
    }>;
  };
}

export function Facts(): React.JSX.Element {
  const [data, setData] = useState<ManifestData | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [draft, setDraft] = useState<string>("");
  const [validation, setValidation] = useState<ValidationResult | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  const loadCurrent = useCallback(async () => {
    setLoadError(null);
    try {
      const res = await callRoute("data", {});
      setData(res as ManifestData);
      // Pre-populate the textarea with the stored manifest so editing
      // an existing manifest is a one-click flow, not a re-paste.
      const stored = (res as ManifestData).manifest;
      if (stored !== null) {
        setDraft(JSON.stringify(stored, null, 2));
      }
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    void loadCurrent();
  }, [loadCurrent]);

  const onCopyPrompt = useCallback(async () => {
    setBusy("prompt");
    setToast(null);
    try {
      const res = (await callRoute("prompt", {})) as { prompt: string };
      await navigator.clipboard.writeText(res.prompt);
      setToast(
        "Prompt copied to clipboard — paste it into Claude/GPT, answer the LLM's questions, then come back and paste the resulting JSON below.",
      );
    } catch (err) {
      setToast(
        `Failed to generate prompt: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setBusy(null);
    }
  }, []);

  const onValidate = useCallback(async () => {
    setBusy("validate");
    setToast(null);
    setValidation(null);
    try {
      const res = (await callRoute("validate", {
        manifest_json: draft,
      })) as ValidationResult;
      setValidation(res);
    } catch (err) {
      setToast(
        `Validation failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setBusy(null);
    }
  }, [draft]);

  const onSave = useCallback(async () => {
    setBusy("save");
    setToast(null);
    try {
      await callRoute("save", { manifest_json: draft });
      setToast("Manifest saved. Site truth score will reflect it on next refresh.");
      await loadCurrent();
    } catch (err) {
      setToast(
        `Save failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setBusy(null);
    }
  }, [draft, loadCurrent]);

  const validationOk =
    validation !== null &&
    validation.validation.valid &&
    validation.validation.errors.length === 0 &&
    validation.assessment.mismatches.filter((m) => m.severity === "error")
      .length === 0;

  return (
    <div className="space-y-6 p-4">
      <header>
        <h1 className="text-xl font-semibold">Truth manifest</h1>
        <p className="mt-1 text-sm text-kumo-subtle">
          Author a <code>facts.json</code> using your LLM of choice.
          The plugin frames the question, validates the answer, and stores
          the result. Read{" "}
          <a
            href="https://github.com/schiste/Aexeo/blob/main/docs/facts-manifest.md"
            target="_blank"
            rel="noopener noreferrer"
            className="text-kumo-link hover:underline"
          >
            the authoring guide
          </a>{" "}
          for the philosophy behind this flow.
        </p>
      </header>

      {loadError !== null && (
        <div className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-700">
          Failed to load manifest state: {loadError}
        </div>
      )}

      <section className="rounded border border-kumo-line p-3">
        <h2 className="text-sm font-medium">Status</h2>
        <p className="mt-2 text-sm">
          {data === null
            ? "Loading…"
            : data.present
              ? "✓ A truth manifest is stored. Editing the JSON below replaces it on Save."
              : "No truth manifest stored. Generate the prompt below to author one."}
        </p>
      </section>

      <section className="space-y-3 rounded border border-kumo-line p-3">
        <h2 className="text-sm font-medium">1. Generate authoring prompt</h2>
        <p className="text-sm text-kumo-subtle">
          Copies a populated prompt to your clipboard. Paste it into
          Claude/GPT/etc., answer the LLM's questions, and bring the
          resulting JSON back here.
        </p>
        <button
          type="button"
          onClick={() => void onCopyPrompt()}
          disabled={busy !== null}
          className="rounded bg-kumo-brand px-3 py-1.5 text-sm font-medium text-white hover:bg-kumo-brand/90 disabled:opacity-50"
        >
          {busy === "prompt" ? "Copying…" : "Copy prompt"}
        </button>
      </section>

      <section className="space-y-3 rounded border border-kumo-line p-3">
        <h2 className="text-sm font-medium">2. Paste &amp; validate</h2>
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          spellCheck={false}
          rows={16}
          placeholder='{ "version": 1, "organization": { ... }, ... }'
          className="w-full rounded border border-kumo-line bg-kumo-tint p-2 font-mono text-xs"
        />
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => void onValidate()}
            disabled={busy !== null || draft.trim().length === 0}
            className="rounded border border-kumo-line px-3 py-1.5 text-sm hover:bg-kumo-tint disabled:opacity-50"
          >
            {busy === "validate" ? "Validating…" : "Validate"}
          </button>
          <button
            type="button"
            onClick={() => void onSave()}
            disabled={busy !== null || !validationOk}
            title={
              validationOk ? "Save to KV" : "Run Validate first; only valid manifests can be saved."
            }
            className="rounded bg-kumo-brand px-3 py-1.5 text-sm font-medium text-white hover:bg-kumo-brand/90 disabled:opacity-50"
          >
            {busy === "save" ? "Saving…" : "Save"}
          </button>
        </div>
      </section>

      {toast !== null && (
        <div className="rounded border border-kumo-line bg-kumo-tint p-3 text-sm">
          {toast}
        </div>
      )}

      {validation !== null && (
        <ValidationReport result={validation} />
      )}
    </div>
  );
}

function ValidationReport({
  result,
}: {
  result: ValidationResult;
}): React.JSX.Element {
  const errMismatches = result.assessment.mismatches.filter(
    (m) => m.severity === "error",
  );
  const warnMismatches = result.assessment.mismatches.filter(
    (m) => m.severity === "warning",
  );
  return (
    <section className="space-y-3 rounded border border-kumo-line p-3">
      <h2 className="text-sm font-medium">Validation result</h2>
      <div className="text-sm">
        <div>
          Shape:{" "}
          <strong>
            {result.validation.valid ? "valid" : "invalid"}
          </strong>{" "}
          · org{" "}
          {result.validation.organization_present ? "present" : "missing"} ·{" "}
          {result.validation.product_count} products ·{" "}
          {result.validation.preferred_term_count} preferred terms ·{" "}
          {result.validation.forbidden_term_count} forbidden terms
        </div>
        <div>
          Score: <strong>{result.assessment.score}</strong>/
          {result.assessment.score_ceiling} · source ={" "}
          <code>{result.assessment.structured_truth_source}</code> ·{" "}
          {result.assessment.pages_analyzed} pages,{" "}
          {result.assessment.pages_with_schema} with schema.org
        </div>
      </div>
      {result.validation.errors.length > 0 && (
        <div className="rounded border border-red-500/40 bg-red-500/10 p-2 text-sm">
          <strong>Schema errors:</strong>
          <ul className="mt-1 list-disc pl-5">
            {result.validation.errors.map((e, i) => (
              <li key={i}>{e}</li>
            ))}
          </ul>
        </div>
      )}
      {result.validation.warnings.length > 0 && (
        <div className="rounded border border-yellow-500/40 bg-yellow-500/10 p-2 text-sm">
          <strong>Schema warnings:</strong>
          <ul className="mt-1 list-disc pl-5">
            {result.validation.warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </div>
      )}
      {errMismatches.length > 0 && (
        <div className="rounded border border-red-500/40 bg-red-500/10 p-2 text-sm">
          <strong>Mismatches with site (errors):</strong>
          <MismatchList items={errMismatches} />
        </div>
      )}
      {warnMismatches.length > 0 && (
        <div className="rounded border border-yellow-500/40 bg-yellow-500/10 p-2 text-sm">
          <strong>Mismatches with site (warnings):</strong>
          <MismatchList items={warnMismatches} />
        </div>
      )}
      {result.validation.errors.length === 0 &&
        result.assessment.mismatches.length === 0 && (
          <div className="rounded border border-green-500/40 bg-green-500/10 p-2 text-sm">
            ✓ Manifest is valid and agrees with on-page schema.org. Click{" "}
            Save to persist.
          </div>
        )}
    </section>
  );
}

function MismatchList({
  items,
}: {
  items: ValidationResult["assessment"]["mismatches"];
}): React.JSX.Element {
  return (
    <ul className="mt-1 list-disc pl-5">
      {items.map((m, i) => (
        <li key={i}>
          <code className="text-xs">{m.route}</code> · {m.field}: expected{" "}
          <code className="text-xs">{m.expected}</code>, observed{" "}
          <code className="text-xs">{m.observed}</code>{" "}
          <span className="text-xs text-kumo-subtle">({m.source})</span>
        </li>
      ))}
    </ul>
  );
}

interface ApiEnvelope<T> {
  data?: T;
  error?: { code: string; message: string };
}

async function callRoute(
  kind: string,
  extra: Record<string, unknown>,
): Promise<unknown> {
  const res = await fetch(`${API_BASE}/plugins/${PLUGIN_ID}/facts`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-EmDash-Request": "1",
    },
    body: JSON.stringify({ kind, ...extra }),
  });
  const body = (await res.json()) as ApiEnvelope<unknown>;
  if (body.error !== undefined) {
    throw new Error(`${body.error.code}: ${body.error.message}`);
  }
  if (body.data === undefined) {
    throw new Error(`Empty response (HTTP ${res.status})`);
  }
  return body.data;
}
