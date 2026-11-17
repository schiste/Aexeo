# Reviewable editorial patches

`editorial-patch` binds explicitly authored edits to a UTF-8 snapshot. It does not generate edits or establish that an edit preserves meaning, facts, citations, accessibility, or style. A human or authorised agent must review each proposal against its context before applying it. The proposal and review output contain the document's private text: handle them accordingly.

Create an array of edits as JSON. Positions are zero-based UTF-8 byte offsets into the **original**, with an exclusive end, and must be sorted and non-overlapping. Every range must be nonempty, match `expected` exactly, and include a nonempty `reason`. Insertions can be represented as a replacement of an adjacent nonempty range. Aexeo rejects stale snapshots, duplicate JSON keys, invalid boundaries, oversized inputs, overlaps, and unsupported patch versions.

```json
[{"start":0,"end":3,"expected":"One","replacement":"A","reason":"Editorial review"}]
```

```sh
aexeo editorial-patch propose --source draft.txt --edits edits.json > proposal.json
aexeo editorial-patch review --source draft.txt --patch proposal.json > candidate.txt
diff -u draft.txt candidate.txt
# Copy the proposal SHA-256 printed during review.
aexeo editorial-patch apply --source draft.txt --patch proposal.json --confirm SHA256 --output approved-candidate.txt
aexeo content approved-candidate.txt --preset article
```

`review` prints the complete candidate to stdout and the proposal hash to stderr. Review the diff and context; `--confirm` binds application to those exact proposal bytes, while the source hash binds it to the exact document. Aexeo checks the source again immediately before creating the output. The source can still change after that check: the new file represents the reviewed snapshot, not a lock on the original. Application exclusively creates a new file; it never overwrites the source or publishes to a CMS. A failed write can leave an incomplete newly created file; remove it before retrying. Re-run the appropriate content gates on the candidate and have the host manage any later CMS update or publication.

This byte-level tool does not parse Markdown or HTML. It does not automatically protect quoted passages, code, links, or markup. The author and reviewer must avoid changing those regions unless the change is deliberate and appropriately checked. No clause, list item, transition, or punctuation is removed automatically. This preserves editorial choice, but an explicitly supplied replacement can still change meaning.

Optional `--telemetry` emits operation and edit count only, without source or replacement content. Proposal and review output are not telemetry and can contain sensitive text.

## Connecting transition findings to proposals

The `sentence_opening_terms` finding from the editorial style gates is advisory, not a patch. Its sentence-start line and column describe projected paragraph text; they do not supply an exact source byte range for the matched expression. Normalized comparisons can also differ in length from the original UTF-8. The existing check for a quote at a sentence opening does not certify that the match is outside every quote, code span, link, HTML element, or CMS component. Do not infer an edit by searching the document for the reported term or by converting that position directly into a byte offset.

A future opt-in English/French proposal generator needs all of the following before it can connect that finding to this patch format:

- A finding bound to the exact source snapshot and identifying the exact original UTF-8 range and matched bytes, including the delimiter and whitespace it proposes to change; no reverse-search or ambiguous projected offsets.
- A complete, adapter-supplied source map proving that the entire range belongs to ordinary editable prose and is outside quotation, code, links and markup; unknown or incomplete context must yield no proposal. Each adapter must establish this capability independently, including nested Markdown/HTML and CMS components.
- An explicit language-specific policy for punctuation and capitalization, with conservative abstention for ambiguous sentence boundaries and contexts. No assertion that removing a connective preserves meaning; each proposal remains advisory and requires review.
- Snapshot binding and exact-byte validation through this module, followed by a fresh evaluation of the candidate with the same resolved preset and adapter. Results must identify unavailable checks and cannot certify factual or semantic equivalence.
- Tests covering duplicate expressions, NFC/decomposed Unicode, multibyte offsets, nested quotations, inline and fenced code, links and their labels, malformed markup, incomplete source maps, mixed CMS blocks, both locales, stale sources, and idempotent proposal generation.

Until those contracts exist, agents can manually author and review source-bound edits, while the transition gate only reports observations.
