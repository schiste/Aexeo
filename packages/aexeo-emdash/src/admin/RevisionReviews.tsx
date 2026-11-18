import * as React from "react";
import { useState } from "react";
import type { ContentReport } from "../content.js";
import type { ReviewDecision } from "../review-dispositions.js";

interface Audit { status: "evaluated"; audit_id: string; revision: string; revision_status: string; preset: string; limitation: string; report: ContentReport }
type Response = Audit | { status: "not_checked"; reason: string };

async function call<T>(route: string, input: unknown): Promise<T> {
  const response = await fetch(`/_emdash/api/plugins/aexeo-emdash/${route}`, {
    method: "POST", headers: { "Content-Type": "application/json", "X-EmDash-Request": "1" }, body: JSON.stringify(input),
  });
  const envelope = await response.json() as { data?: T; error?: { message: string } };
  if (!response.ok || envelope.error || envelope.data === undefined) throw new Error(envelope.error?.message ?? `HTTP ${response.status}`);
  return envelope.data;
}

export function RevisionReviewInstructions(): React.JSX.Element {
  return <><h2 className="font-semibold">Revision audit and editorial review</h2>
    <p>Audit a specific revision or retrieve an existing report. Reviews annotate findings; they never suppress them or carry over to another audit.</p></>;
}

/** Review an exact audit without implying that its conclusions apply to later revisions. */
export function RevisionReviews(): React.JSX.Element {
  const [collection, setCollection] = useState("");
  const [id, setId] = useState("");
  const [revision, setRevision] = useState("");
  const [auditId, setAuditId] = useState("");
  const [audit, setAudit] = useState<Audit | null>(null);
  const [selected, setSelected] = useState("");
  const [decision, setDecision] = useState<ReviewDecision>("retain_with_explanation");
  const [explanation, setExplanation] = useState("");
  const [message, setMessage] = useState("");
  const choices = audit?.report.gates.flatMap(g => g.findings.map((f, index) => ({ gate: g.gate_id, index, text: f.message }))) ?? [];
  const choice = selected === "" ? undefined : choices[Number(selected)];
  const show = (result: Response) => {
    setSelected(""); setExplanation("");
    if (result.status === "evaluated") { setAudit(result); setAuditId(result.audit_id); setMessage(""); }
    else { setAudit(null); setMessage(`Not checked: ${result.reason}`); }
  };
  const run = async () => { try { show(await call<Response>("audit-revision", { collection, id, revision })); } catch (error) { setAudit(null); setMessage(String(error)); } };
  const load = async () => { try { show(await call<Response>("get-revision-audit", { audit_id: auditId })); } catch (error) { setAudit(null); setMessage(String(error)); } };
  const review = async (save: boolean) => {
    if (!audit || !choice) return;
    const input = { audit_id: audit.audit_id, gate_id: choice.gate, finding_index: choice.index };
    try {
      const result = await call<{ status: string; reason?: string; review?: { decision: ReviewDecision; explanation: string } }>(save ? "set-revision-review" : "get-revision-review", save ? { ...input, decision, explanation } : input);
      if (result.review) { setDecision(result.review.decision); setExplanation(result.review.explanation); }
      setMessage(result.status === "recorded" ? (save ? "Review saved for this audit only." : "Review loaded for this audit.") : `Review not recorded${result.reason ? `: ${result.reason}` : "."}`);
    } catch (error) { setMessage(String(error)); }
  };
  return <section className="rounded border border-kumo-line p-3 text-sm">
    <RevisionReviewInstructions />
    <div className="flex flex-wrap gap-2">
      <label>Collection <input value={collection} onChange={e => setCollection(e.target.value)} /></label>
      <label>Content ID <input value={id} onChange={e => setId(e.target.value)} /></label>
      <label>Revision ID <input value={revision} onChange={e => setRevision(e.target.value)} /></label>
      <button type="button" disabled={!collection || !id || !revision} onClick={() => void run()}>Audit revision</button>
    </div>
    <div className="flex gap-2"><label>Audit ID <input value={auditId} onChange={e => setAuditId(e.target.value)} /></label><button type="button" disabled={!auditId} onClick={() => void load()}>Load audit</button></div>
    {message && <p role="status">{message}</p>}
    {audit && <div>
      <p>Revision {audit.revision} — {audit.revision_status} at last load. Preset: {audit.preset}. Audit ID: {audit.audit_id}</p>
      <p>{audit.limitation}</p>
      <ul>{audit.report.gates.map(g => <li key={g.gate_id}>{g.gate_id}: {g.status}{g.reason ? ` — ${g.reason}` : ""}</li>)}</ul>
      {choices.length > 0 && <div>
        <label>Finding <select value={selected} onChange={e => { setSelected(e.target.value); setExplanation(""); }}><option value="">Select a finding</option>{choices.map((item, index) => <option key={`${item.gate}:${item.index}`} value={index}>{item.gate}: {item.text}</option>)}</select></label>
        {choice && <div>
          <label>Decision <select value={decision} onChange={e => setDecision(e.target.value as ReviewDecision)}><option value="retain_with_explanation">Retain with explanation</option><option value="needs_revision">Needs revision</option><option value="not_applicable">Not applicable</option></select></label>
          <label>Explanation <textarea value={explanation} maxLength={2000} onChange={e => setExplanation(e.target.value)} /></label>
          <button type="button" onClick={() => void review(false)}>Load review</button>
          <button type="button" disabled={!explanation.trim()} onClick={() => void review(true)}>Save review</button>
        </div>}
      </div>}
    </div>}
  </section>;
}
