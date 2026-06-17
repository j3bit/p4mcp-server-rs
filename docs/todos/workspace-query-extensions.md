# Workspace Query Extensions

Status: deferred from PR #2 to keep the Rust port aligned with upstream `perforce/p4mcp-server`.

## Reason

The write approval gate is an approved Rust-port extension because it is a safety measure. The previous `query_workspaces` actions `opened`, `changes`, and `where` were functional extensions. They should not be shipped accidentally inside the upstream-parity porting PR.

## Future Design

### `opened`

- Keep the action read-only.
- If `workspace_name` is present, run `p4 opened -C <workspace>`.
- If `workspace_name` is absent, run `p4 opened` for the current client.
- Tests must cover both the current-client and explicit-workspace forms.

### `changes`

- Prefer guiding callers to `query_changelists` with `action: "list"` and `workspace_name` set. That tool already owns `p4 changes` query behavior.
- If `changes` remains in `query_workspaces`, require `workspace_name` for workspace-scoped behavior and run `p4 changes -c <workspace>`.
- Preserve `max_results` with `-m <n>` if this action is reintroduced.
- Tests must prove that `workspace_name` is never silently ignored.

### `where`

- Treat this as a current-client mapping query only.
- Require `file_path`.
- Do not support `workspace_name`; `p4 where` resolves through the active client context.
- If a future implementation receives both `where` and `workspace_name`, return an invalid-params error rather than pretending the named workspace is used.

### `status` Client Scoping

- Keep the current PR aligned with upstream: `query_workspaces.status` validates and reads the named workspace spec, but the status probes run in the active client context.
- Treat stricter per-request scoping as a deliberate Rust-port extension, not a parity bug fix.
- If this extension is added, run every status probe under `P4CLIENT=<workspace_name>` or an equivalent temporary client context.
- Cover `opened`, `sync -n`, `resolve -n`, and `changes -m1 #have` so the returned status cannot mix a named workspace spec with another active client's state.

## Reintroduction Checklist

- Add explicit action documentation before exposing the action.
- Add direct builder tests for every generated `p4` argument list.
- Add server-to-executor tests for every parameter that changes command scope.
- Verify the action does not duplicate an existing tool unless the user-facing workflow is clearer in `query_workspaces`.
- Reply to the relevant PR or issue explaining that this is a deliberate functional extension, not upstream-parity work.
