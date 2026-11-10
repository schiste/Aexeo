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

`subject` repeats exactly what was evaluated, so that a runner can tell
whether a verdict still applies:

- `change_id` and `base`, copied from the change document;
- `entries`: one item per change-document entry, with `entity_id` and
  `sha256` (that is, `after.sha256`; for `delete` the value is `null`).

A verdict applies only to the exact content it names. When any entry's content
or the base changes, the verdict is stale.

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
5. the content about to be published still has those digests.

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

Aexeo does not yet read a change document or write a verdict file. Adding that
does not change its existing commands.
