# Workspace Modify Existence Policy

Status: deferred follow-up from PR #2. The review finding is a real
create/update semantics risk, but the current Rust behavior matches upstream
`perforce/p4mcp-server` at `a64efb07511b2a62db41aeed110ab96744c4076a`
(`v2026.2.2955897`). Rechecked against upstream `origin/main` on 2026-06-18.

## Reviewer Claim

`modify_workspaces.create` and `modify_workspaces.update` both fetch a client
form, patch it, and save it without checking the expected existence state.
Because Perforce client form operations are create-or-edit shaped, a `create`
request can mutate an existing workspace and an `update` request with a typo can
create a new workspace from a template.

## Current Rust Port

- `execute_workspace_create` runs `p4 client -o <workspace>`, patches supplied
  fields, and saves with `p4 client -i`.
- `execute_workspace_update` follows the same fetch-patch-save shape after an
  `info` call.
- The Rust server has a workspace existence helper for query/read paths, but
  create/update do not use an existence-state policy before saving.

## Upstream Behavior

Upstream distinguishes read and write paths:

- `get_workspace` checks `p4 clients -e <workspace>` before reading a named
  workspace.
- `create_workspace` calls `p4.fetch_client(Name)`, patches supplied fields,
  and saves with `p4.save_client`.
- `update_workspace` calls `p4.fetch_client(workspace_name)`, patches supplied
  fields, and saves with `p4.save_client`.
- Neither upstream create nor update first checks `p4 clients -e <workspace>` to
  enforce duplicate-create or missing-update semantics.

## Actual Threat

The write approval gate prevents silent writes, but it does not prove that the
approved action has the requested create/update meaning. The risk is semantic
drift between the MCP action and the Perforce form operation:

- Duplicate create can modify an existing workspace's root, view, options, or
  line-end fields while the caller believes a new workspace is being created.
- Missing update can create an unintended workspace from a default/template
  form when the name is misspelled.
- A changed workspace view or root can affect later sync/edit/submit decisions
  made by an agent or operator.
- The failure mode is especially easy to miss because the command can complete
  successfully and report the requested action name.

Severity: medium. This can alter workspace metadata and misdirect later P4
operations, but it is still gated by write approval and does not directly submit
or delete depot content.

## Follow-Up Design

- Treat the current behavior as upstream parity, not as a Rust-port bug.
- For a stricter Rust extension, run `p4 clients -e <workspace>` before any
  create/update save.
- Reject `modify_workspaces.create` when the workspace already exists.
- Reject `modify_workspaces.update` when the workspace does not exist.
- Keep approval behavior unchanged, but ensure the final execution path repeats
  the existence check before `p4 client -i`.

## Verification Needed

- Test duplicate create rejection.
- Test missing update rejection.
- Test that existence validation runs before `client -o` and before `client -i`.
- Document the behavior as an intentional upstream divergence if implemented.
