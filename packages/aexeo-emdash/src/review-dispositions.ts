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

/** Review records annotate one immutable audit finding; they never suppress findings. */
export async function setReviewDisposition(kv: KvNamespace, request: ReviewRequest) {
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
  const record = { ...request, explanation, collection: audit.collection, id: audit.id, revision: audit.revision,
    recorded_at: new Date().toISOString() };
  await kv.set(key(request), record);
  return { status: "recorded", review: record } as const;
}

export async function getReviewDisposition(kv: KvNamespace, request: Pick<ReviewRequest, "audit_id" | "gate_id" | "finding_index">) {
  const review = await kv.get(key(request));
  return review ? { status: "recorded", review } as const : { status: "not_recorded" } as const;
}
