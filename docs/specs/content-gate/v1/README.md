# Content gate protocol, version 1

Status: draft. This is the version 1 contract between a **gate runner** and a
**content validator**. Neither side needs the other to work: any runner can
call any validator that speaks this protocol, and any validator can be called
by a CI job, a hook or a script.

Version 1 defines two documents:

| Document | Identifier | Written by | Read by | Schema |
|---|---|---|---|---|
| Change document | `content-gate.change/v1` | runner | validator | [`change.schema.json`](change.schema.json) |
| Verdict document | `content-gate.verdict/v1` | validator | runner | [`verdict.schema.json`](verdict.schema.json) |

The key words MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119.

## 1. Roles and flow

1. The runner detects a content change from any source (a Git diff, a CMS
   write, an API call) and writes one **change document** describing it.
2. The runner invokes the validator (section 4) with the paths of the change
   document and of the verdict file to write.
3. The validator evaluates the change and writes one **verdict document**.
4. The runner reads the verdict and applies it: for a required gate, only a
   verified `pass` lets the change proceed (section 5).

The protocol says nothing about how a runner is configured, when it runs a
gate or what a validator checks. Those belong to each tool.

## 2. Change document (`content-gate.change/v1`)

A change document describes one set of content changes that are evaluated, and
later published, together.

| Field | Required | Meaning |
|---|---|---|
| `schema` | yes | Always `content-gate.change/v1`. |
| `change_id` | yes | Opaque identifier chosen by the runner. It MUST be unique for each distinct set of entry contents: when any entry changes, the runner issues a new `change_id`. |
| `created_at` | yes | RFC 3339 timestamp. |
| `origin` | yes | Where the change came from: `kind` is `git`, `mcp`, `api` or `manual`; `system` optionally names the source (`git`, a CMS name, an MCP server); `ref` optionally carries a source-specific reference such as a branch, session or request id. |
| `base` | yes | The state the change was made against: `kind` (`git` or `opaque`) and `revision`. For `git`, `revision` is the full commit object id. |
| `content_root` | when any `file` reference is used | Absolute local path that `file` references are relative to. |
| `site` | no | `id` and/or `base_url` of the site the content belongs to. |
| `author` | no | `id`, `display_name`, and `kind` (`human`, `agent` or `system`). |
| `entries` | yes | One or more changed entries (section 2.1). |
| `extensions` | no | Tool-specific data, see section 6. |

### 2.1 Entries

| Field | Required | Meaning |
|---|---|---|
| `entity_id` | yes | Stable identifier of the content item, unique within the document. It MUST NOT change when the content is edited. |
| `operation` | yes | `create`, `update`, `delete` or `rename`. |
| `previous_entity_id` | for `rename` | The entity's identifier before the rename. |
| `media_type` | yes | Format of the content, for example `text/markdown`, `text/html`, or a source-specific type such as `application/vnd.portable-text+json`. |
| `public_path` | no | URL path where the entry is or will be served, for example `/blog/launch/`. Validators that check links need it. |
| `metadata` | no | Object of string fields supplied outside the body, for example CMS title and description. Markdown front matter stays in the body. |
| `before` | no | The entry's state at `base`. Omitted for `create`. |
| `after` | for every operation except `delete` | The proposed state. Omitted for `delete`. |

`before` and `after` are **content states**:

| Field | Required | Meaning |
|---|---|---|
| `revision` | no | Source revision identifier. For `git` it is the blob object id. |
| `sha256` | yes | Lowercase hex SHA-256 of the exact content bytes. It identifies the content for verdict binding (section 3.2), whatever the source. |
| `inline` | exactly one of `inline` / `file` / `uri` | The content as a UTF-8 string. |
| `file` | | Path relative to `content_root`, using `/` separators, without `..` segments. |
| `uri` | | A URI the validator can dereference. A validator that cannot dereference it MUST report the entry as unavailable rather than skip it. |

The validator MUST check that the bytes it reads hash to `sha256`; a mismatch
is an `error` verdict. Version 1 covers text content; binary media are out of
scope.

## 3. Verdict document (`content-gate.verdict/v1`)

| Field | Required | Meaning |
|---|---|---|
| `schema` | yes | Always `content-gate.verdict/v1`. |
| `status` | yes | Overall result: `pass`, `fail`, `not_checked` or `error` (section 3.1). |
| `subject` | yes | What was evaluated (section 3.2). |
| `validator` | yes | `name` and `version`; optional `profile` (the rule set or preset applied) and `profile_sha256` (digest of its configuration). |
| `evaluated_at` | yes | RFC 3339 timestamp. |
| `reason` | for `not_checked` and `error` | Human-readable explanation. |
| `summary` | yes | Counts: `checks`, `passed`, `failed`, `not_checked`, `blocking_failed`, `blocking_not_checked`. |
| `checks` | yes | One result per check the validator ran: `id`, `status` (`passed`, `failed`, `not_checked`), `blocking`, optional `reason`. |
| `findings` | yes | Problems found, possibly empty (section 3.3). |
| `findings_omitted` | no | Number of findings left out of `findings` because of output limits. Defaults to 0. |
| `extensions` | no | Tool-specific data, see section 6. |

### 3.1 Overall status

The validator derives `status` from its checks, in this order of precedence:

1. `error`: the evaluation could not be carried out, for example because the
   input was unreadable or invalid, a digest did not match, the configuration
   was invalid, or the validator failed internally.
2. `not_checked`: at least one blocking check could not be evaluated, for
   example because a required capability or input was missing. This takes
   precedence over `fail`, because the result is incomplete. The failed checks
   are still listed.
3. `fail`: at least one blocking check failed.
4. `pass`: every blocking check passed.

Non-blocking checks never change the overall status. `not_checked` is never
a pass.

### 3.2 Subject binding

`subject` identifies the change and content bytes evaluated:

- `change_id` and `base`, copied from the change document;
- `entries`: one item per change-document entry, with `entity_id` and
  `sha256` (that is, `after.sha256`; for `delete` the value is `null`).

A verdict applies only to the exact content it names. When any entry's content
or the base changes, the verdict is stale. These fields do **not** bind entry
metadata, operations, media types, collection snapshots, preset contents, host
options or external observations. A reused `change_id` does not prove these
inputs stayed unchanged. Metadata and collection gates can produce different
results for the same subject. A runner must independently retain and compare
all inputs and configuration that affected its decision; if it cannot establish
they are unchanged, it must treat the verdict as stale. The v1 verdict alone
cannot provide this assurance.

### 3.3 Findings

| Field | Required | Meaning |
|---|---|---|
| `rule_id` | yes | Identifier of the rule that produced the finding. |
| `severity` | yes | `error`, `warning` or `info`. |
| `message` | yes | Human-readable description. |
| `entity_id` | no | The content item concerned. It may be outside the change, such as an unchanged page whose link a rename breaks. Omitted for findings about the site as a whole. |
| `check_id` | no | The check that reported the finding. |
| `location` | no | `line` and `column` (1-based), relative to the text the validator evaluated. A validator SHOULD document whether positions refer to the source or to an extracted projection. |
| `subject` | no | Stable key that tells apart several findings of one rule on one entity, for example a link target. Together with `rule_id` and `entity_id` it identifies the finding across runs. |
| `suggestion` | no | A proposed fix, as text. |

## 4. Invocation

The runner MUST give the validator:

- the change document path, through the environment variable
  `CONTENT_GATE_CHANGE_FILE`;
- the path where the verdict must be written, through
  `CONTENT_GATE_VERDICT_FILE`.

The runner MAY also substitute these paths into the command line, for example
as `{change_file}` and `{verdict_file}`. The validator writes the verdict
document only to that file; standard output and error are free for
human-readable logs.

Exit codes:

| Exit code | Meaning |
|---|---|
| 0 | `pass` |
| 1 | `fail` |
| 2 | `not_checked`, or invalid input or configuration |
| any other | `error` |

The verdict document is authoritative. Exit codes let tools that do not read
the verdict react correctly, and let the runner cross-check the result.

## 5. Runner obligations (fail-closed)

For a gate the runner treats as required, the change MAY proceed only if all
of the following hold:

1. the validator exited with code 0 within the runner's time limit;
2. the verdict file exists and is a valid `content-gate.verdict/v1` document;
3. `status` is `pass`;
4. `subject.change_id` and `subject.base` equal those of the change document
   the runner supplied, and every entry's `entity_id` and `sha256` match it
   exactly, with none missing and none extra;
5. the content about to be published still has those digests;
6. every other evaluated input and the effective policy and host options are
   unchanged, including entry metadata and any supplied collection snapshot or
   external gate results. The runner must establish this independently: the v1
   subject does not carry their fingerprints.

If any of these fails, the gate is not passed. A missing file, a parse error,
a timeout, a disagreement between the exit code and `status`, or an unknown
`schema` major version all block. A runner MAY offer an explicit, recorded
override; it MUST NOT treat a missing or invalid verdict as a pass.

For a gate that is not required, the runner reports the verdict and does not
block.

## 6. Versioning and extensions

- The identifier carries the major version (`/v1`). Within version 1, new
  optional fields may be added. Consumers MUST ignore fields they do not know,
  except inside `schema`, `status` and `subject`, whose meaning MUST NOT change
  within a major version.
- Any change that removes a field, changes a field's meaning, or adds a value
  to `status`, `operation` or check `status` requires a new major version.
- `extensions` is an object keyed by a tool-specific namespace, for example
  `"extensions": {"aexeo": {...}}`. A consumer MUST NOT need an extension to
  apply the verdict correctly.

## 7. Profile: Markdown in Git

When `origin.kind` is `git`:

- `base.kind` is `git`, and `base.revision` is the full id of the commit the
  change is evaluated against (for a branch, its merge base with the target
  branch);
- `entity_id` is the repository-relative path with `/` separators, and
  `previous_entity_id` holds the old path for a rename;
- `before.revision` and `after.revision` are Git blob object ids;
- `content_root` is the absolute path of the checkout holding the proposed
  content, and `after.file` is the entry's repository-relative path.

See [`examples/markdown-change.json`](examples/markdown-change.json) and
[`examples/verdict-fail.json`](examples/verdict-fail.json).

## 8. Out of scope for version 1

Identity of whole-site revision sets, impact queries between entries, binary
media, and authentication or signing of verdicts. These may come in later
documents or versions. A runner that needs to trust a verdict's origin must
control which validator it runs.

## Appendix A (informative): Aexeo mapping

Aexeo's content report already carries the elements the verdict needs:

| Aexeo | Verdict |
|---|---|
| `GateStatus::Passed / Failed / NotChecked` | check `status` `passed / failed / not_checked` |
| `ContentGateResult.blocking`, `reason` | check `blocking`, `reason` |
| `ContentSummary` | `summary` |
| `Finding.rule_id`, `severity`, `message`, `line`, `column`, `subject`, `suggestion` | finding fields of the same name, with `line`/`column` under `location` |
| `Finding.path` | `entity_id` |
| exit 1 (failed blocking gate), 2 (blocking gate not checked or invalid input), 70 (internal error) | exit codes 1, 2 and "any other" |

Aexeo implements this invocation as `aexeo-cli content-change --preset NAME`.
The runner supplies `CONTENT_GATE_CHANGE_FILE` and `CONTENT_GATE_VERDICT_FILE`;
choose a preset deliberately because a passing verdict only covers its gates.
Additional TOML presets can be loaded with `--preset-source NAME=PATH`.
Reachability gates do not access the network unless the host passes
`--check-link-reachability`. The total number of attempted unique checks per
entry is bounded by `--link-max-checks` across the change (default 20, maximum
100); each check including redirects uses `--link-timeout` (default 5 seconds,
maximum 30). Repeated destinations in different entries consume the budget
again. After exhaustion remaining external links are not checked. The optional
`--telemetry` writes aggregate counts and timings to stderr without URLs or
content. Results are advisory observations, not proof of source quality or safety.
The CLI validates known change fields, timestamps and URIs before evaluation,
including optional fields explicitly set to null, and accepts unknown fields for
forward compatibility. It does not use a generic JSON Schema validator; runners
remain responsible for validating their inputs and enforcing section 5.
Markdown finding locations refer to the original source when an exact mapping is
available; unmappable or non-positional findings omit `location`. Plain-text
prose locations refer directly to the original text.

Inline Markdown and plain text are supported. On Linux and macOS, file sources
require an explicit `--content-root` matching the change's absolute
`content_root`. The kernel must enforce `openat2` on Linux or
`O_RESOLVE_BENEATH` on macOS; unsupported kernels fail closed. Other platforms
cannot currently read file sources. URI sources are not fetched. Deletions and renames do not establish
site impact, so their blocking impact checks are reported as not checked.

Aexeo enforces a 64 MiB change-document limit, a 16 MiB per-item limit and a
64 MiB aggregate content limit. It writes a verdict via a temporary file and
rename. If parsing, digest verification, evaluation or output fails, the command
exits nonzero and may leave no new verdict; the runner must enforce all checks
in section 5, including the exit code, even if a previous verdict file exists.

### Plain-text paragraph capability

`content-change --plain-text-paragraphs` explicitly interprets `text/plain`
entries as blank-line-separated paragraphs, using the same bounded adapter as
`content --input-format paragraph-text`. Without this option the existing
raw-text interpretation is retained and paragraph-dependent checks remain
not checked. Source locations refer to the verified input snapshot. When the
paragraph limit is exceeded, content-dependent capabilities are unavailable;
no partial paragraph evaluation is reported as complete. This option does not
change Markdown or unsupported media types. With `--telemetry`, a local
`content_change_paragraph_adapter` event reports only the number of eligible
text entries, not the content or whether each entry was successfully adapted.

### Host-supplied custom gate results

`content-change --extension-results results.json` accepts an explicit JSON
array in the same format as `content --extension-results`. A batch can contain
one result per entity ID, extension and parameter set; it is limited to 100
results and 32 MiB in total. Each record must bind to the exact projected
`ContentInput`, including the entry's `entity_id` as `id`, text, source text,
headings and metadata. Missing or stale records are not checked. Invalid or
out-of-range diagnostics fail the invocation without replacing the verdict.

For an individual entry, `content --export-adapter-input` can provide the
projection using the same source, format, metadata and paragraph interpretation;
set the exported `id` to the change entry's `entity_id`. The result may contain
private text and metadata; retain it only in an appropriately protected file.
The input comparison detects stale data, not a dishonest or incorrect producer.
Aexeo does not execute external code. The core API also accepts a trusted
`ContentGateExtensions` host through `ChangeEvaluationHost`; host callbacks run
only after all content digests have been verified. With `--telemetry`, a local
`content_change_external_gates` event reports the number of configured custom
gates when a result file is supplied; it contains no content, IDs or outcomes.
