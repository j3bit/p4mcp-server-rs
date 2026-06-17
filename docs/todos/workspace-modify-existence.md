# Workspace Modify Existence Policy

Status: deferred from PR #2 to keep the Rust port aligned with upstream
`perforce/p4mcp-server`.

## Reason

The Rust port currently mirrors upstream workspace create/update behavior:

- Upstream `create_workspace` calls `p4.fetch_client(workspace_spec["Name"])`,
  patches supplied fields, and saves the client with `p4.save_client`.
- Upstream `update_workspace` calls `p4.fetch_client(workspace_name)`, patches
  supplied fields, and saves the client with `p4.save_client`.
- Neither upstream path first checks `p4 clients -e <workspace>` to distinguish
  duplicate create from missing update.
- The Rust port maps the same workflow to `p4 client -o <workspace>`, form
  patching, and `p4 client -i`.

The PR review feedback is valid as a safety and product-semantics issue, but it
is not a Rust-port parity bug. Perforce's client fetch/edit flow can make
`create` mutate an existing workspace and `update` create a workspace from a
template when the name is wrong. Changing that behavior in this PR would be an
intentional divergence from the upstream baseline.

## Future Design

- Add an explicit workspace-existence probe before any create/update form save:
  `p4 clients -e <workspace>`.
- For `modify_workspaces.create`, reject an existing workspace before fetching
  and saving the form.
- For `modify_workspaces.update`, reject a missing workspace before fetching
  and saving the form.
- Keep approval semantics unchanged: the approval preview should disclose the
  planned write, but existence validation should still run after approval before
  `p4 client -i`.
- Decide whether this stricter policy should also be proposed upstream.

## Reintroduction Checklist

- Add server-to-executor tests for duplicate create and missing update.
- Add direct tests proving the existence probe happens before `client -o`.
- Document this as an intentional upstream divergence in the commit or PR notes.
