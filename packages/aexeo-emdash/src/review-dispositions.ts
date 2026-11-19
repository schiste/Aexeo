import type { ContentReport } from "./content.js";
import type { KvNamespace } from "./plugin.js";

export type ReviewDecision = "retain_with_explanation" | "needs_revision" | "not_applicable";
export interface ReviewRequest {
  audit_id: string;
  gate_id: string;
  finding_index: number;
  decision: ReviewDecision;
  explanation: string;
}
interface StoredAudit {
  status: "evaluated";
  audit_id: string;
  collection: string;
  id: string;
  revision: string;
  report: ContentReport;
}
const key = (request: Pick<ReviewRequest, "audit_id" | "gate_id" | "finding_index">) =>
  `revision-review:${request.audit_id}:${encodeURIComponent(request.gate_id)}:${request.finding_index}`;

type ReviewKey = Pick<ReviewRequest, "audit_id" | "gate_id" | "finding_index">;
const historyPrefix = (request: ReviewKey) =>
  `revision-review-history:${request.audit_id}:${encodeURIComponent(request.gate_id)}:${request.finding_index}:`;

type ReviewActor = { kind: "authenticated_account"; id: string } | { kind: "unattributed" };

/** Review records annotate an audit finding; they never suppress findings. */
export async function setReviewDisposition(kv: KvNamespace, request: ReviewRequest, user?: { id?: unknown }) {
  const explanation = request.explanation.trim();
  if (!explanation || explanation.length > 2000 || !Number.isSafeInteger(request.finding_index) || request.finding_index < 0 ||
      !["retain_with_explanation", "needs_revision", "not_applicable"].includes(request.decision)) {
    return { status: "not_recorded", reason: "invalid_review" } as const;
  }
  const audit = await kv.get<StoredAudit>(`revision-audit:${request.audit_id}`);
  const gates = audit?.report?.gates?.filter(gate => gate.gate_id === request.gate_id) ?? [];
  if (!audit || audit.status !== "evaluated" || audit.audit_id !== request.audit_id || gates.length !== 1 ||
      !gates[0]?.findings[request.finding_index]) {
    return { status: "not_recorded", reason: "finding_unavailable" } as const;
  }
  // Only the host context can provide this identity; request data is never trusted.
  const actor: ReviewActor = typeof user?.id === "string" && user.id.trim()
    ? { kind: "authenticated_account", id: user.id } : { kind: "unattributed" };
  const record = { ...request, explanation, actor, collection: audit.collection, id: audit.id, revision: audit.revision,
    recorded_at: new Date().toISOString() };
  // These writes are not transactional. History captures a stored submission,
  // including one whose subsequent current-record update might have failed.
  const event_id = crypto.randomUUID();
  await kv.set(`${historyPrefix(request)}${event_id}`, { ...record, event_id });
  await kv.set(key(request), record);
  return { status: "recorded", review: record } as const;
}

export async function getReviewDisposition(kv: KvNamespace, request: Pick<ReviewRequest, "audit_id" | "gate_id" | "finding_index">) {
  const review = await kv.get(key(request));
  return review ? { status: "recorded", review } as const : { status: "not_recorded" } as const;
}

/** History is advisory: the host KV is mutable and earlier overwrites are unavailable. */
export async function getReviewHistory(kv: KvNamespace, request: ReviewKey) {
  if (!Number.isSafeInteger(request.finding_index) || request.finding_index < 0) {
    return { status: "not_available", reason: "invalid_finding" } as const;
  }
  const audit = await kv.get<StoredAudit>(`revision-audit:${request.audit_id}`);
  const gates = audit?.report?.gates?.filter(gate => gate.gate_id === request.gate_id) ?? [];
  if (!audit || audit.status !== "evaluated" || audit.audit_id !== request.audit_id || gates.length !== 1 ||
      !gates[0]?.findings[request.finding_index]) {
    return { status: "not_available", reason: "finding_unavailable" } as const;
  }
  const events = (await kv.list<Record<string, unknown>>(historyPrefix(request)))
    .map(entry => entry.value)
    .sort((a, b) => String(a.recorded_at).localeCompare(String(b.recorded_at)) ||
      String(a.event_id).localeCompare(String(b.event_id)));
  return { status: "available", completeness: "not_guaranteed", events } as const;
}
