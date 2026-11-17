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
