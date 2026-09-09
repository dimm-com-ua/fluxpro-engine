# Embedding the live monitor

`ProcessMonitor` is the second public UI component in `fluxpro_engine::editor`. It is
independent of `ProcessEditor`, read-only, and uses the same saved node positions,
vertical layout, smooth connectors, trackpad zoom and Command + drag navigation.

```rust,no_run
use fluxpro_engine::editor::{EditorDocument, MonitorRequest, MonitorSnapshot, ProcessMonitor};
use leptos::prelude::*;

// In your host component:
let document = RwSignal::new(EditorDocument::default());
let snapshot = RwSignal::new(MonitorSnapshot::default());
let on_request = Callback::new(move |request: MonitorRequest| {
    // Spawn your authenticated API call here.
    // Set snapshot to the response, echoing the entire request unchanged.
});
let monitor = view! {
    <div style="height: 800px">
        <ProcessMonitor document=document snapshot=snapshot
            on_request=on_request refresh_interval_ms=5000/>
    </div>
};
```

The component does not assume API URLs, tokens, a database connection or an auth
mechanism. The host supplies these through its existing transport. It can use
polling, SSE or WebSocket: for push updates set `refresh_interval_ms=0`, subscribe
to the latest request and keep publishing snapshots with that request. Polling
stops on unmount; Pause stops automatic requests. Refresh and search still work.
No processes are started, resumed, cancelled or modified by this component.

## Request and response contract

Every callback receives `MonitorRequest`:

- `scope`: process key, exact normalized version, optional persisted definition
  UUID from `document.definition.uuid`. Use UUID when available. If it is absent,
  resolve the **exact key/version** on the server, never the latest version.
- `revision`: request identity. Echo it unchanged; late responses are ignored.
- `active_counts_only`: set by the optional component prop of the same name.
  When true, graph counts exclude completed instances; suspended instances still
  count as active. The default is false, including for older serialized requests.
- `query`: search string, exact node/state filters, offset and page size (25).
  Search covers application business key (`process_id`), runtime `token`, process
  key and UUID. Apply filters **before** counting and paginating.
- `instances`: UUIDs for open tabs plus execution/signal history prefix limits.
  Initially each history requests 50 records; Load more increases its own limit
  by 100. Return newest-first prefixes from offset zero and full totals. When
  fetching through an API limited to 250 rows, combine enough pages to satisfy
  the prefix request. Closing a tab removes its UUID from future requests.

Return `MonitorSnapshot` with the echoed request, server observation timestamp,
version-wide `node_counts`, the filtered instance page, and requested details.
Supply a count entry for **every** definition node, including zero counts.
Missing node counts render as unknown (`—`), not zero. Node counts are independent
of search, page and state filters. They count instances currently assigned to the
node, including persisted completed instances at terminal blocks unless
`active_counts_only=true`. This option does not change the instance-list filters.
Nodes with a positive count receive a green highlight; zero and unknown counts
remain neutral. Error borders and selected-instance outlines stay distinguishable.

Each detail contains the instance summary, current node, current context, scoped
variables, stage history with historical context snapshots, execution logs and
admitted signal events. Context uses a recursive, typed read-only tree. Scoped
variables remain separate so equal names from different scopes are not lost.
Execution events retain source, handler, queue/attempt, error fields and arbitrary
diagnostic details. Signal history describes admission; execution logs provide
the subsequent delivery outcome.

Populate `issues` with **unresolved** errors and active support escalations.
Provide `errors` and `escalations` as per-node counts of affected instances across
the complete version. The viewer does not guess live incidents from historical
error logs, node names or `create_support_ticket` alone. It never clears an issue
just because a new snapshot arrives: absence in a successful snapshot is the
host's authoritative indication that it was resolved.

Errors in the whole request go in `snapshot.error`; detail-specific failures go
in `details[n].error`. Echo the request even on failure. The last accepted snapshot
remains visible with an update-failed/stale notice. A list for a different query
is hidden while the new result loads. Rows/details from another definition
version are rejected, and changing the document's version closes its tabs.

## Mapping existing FluxPro administration data

The UI crate keeps the engine's backend features disabled. Serialize admin data
on your server and adapt it to the monitor DTOs without shipping SQLx to WASM.

| Monitor data | Engine source |
| --- | --- |
| Exact definition and layout | `get_process_definition(uuid)` + host-stored editor positions |
| Node assignment counts | `get_process_node_instance_counts(definition_uuid)` |
| Instance page | `list_process_instances` filtered by `process_definition_uuid` |
| Summary/context/scoped variables/stage history/current node | `get_process_instance(uuid)` |
| Execution history | `list_process_instance_logs(uuid, filter, page)` |
| Admitted signals | `list_process_instance_signals(uuid, page)` |
| Unresolved execution incident | `get_open_incident(token)` |
| Active escalation/ticket state | Host application's support-ticket source |

`MonitorInstance`, `MonitorStageEntry`, `MonitorLogEntry` and
`MonitorSignalEntry` accept the corresponding admin JSON fields. Unknown summary
and log fields are retained in the metadata/diagnostics maps. Admin instance
details flatten the summary; the monitor detail object instead puts it in
`instance`, alongside context and the separately fetched histories.

The existing admin list query supports business key/token/process key search and
state/definition filters. Its current API does not include a node filter or UUID
search; add these to your host query **before pagination**, not as a filter over
one returned page. Existing assignment-count queries do not aggregate open
incidents or support tickets; compute those two issue counts from their live
sources. Never derive version totals from the displayed page.

## Demo and verification

The demo has separate **Process editor** and **Live monitor demo** buttons.
Monitor data are explicitly simulated: 67 instances, active errors/escalations,
140 execution events, stage snapshots and scoped context. Updates occur every
five seconds through the same public callback contract used by a real host.
`demo/monitor_data.rs` is a working, in-memory response adapter.

Tests cover version isolation, outdated request rejection, searching, filtered
pagination with independent counts, history pagination, context snapshots,
UUID-based closable tabs, read-only diagram rendering and remount identities.

## Console embedding and server adapter

`open_instance` is an optional reactive `Signal<Option<MonitorInstance>>`. It
opens/selects a tab only when its exact scope matches the document. `instance_actions`
is an optional `Callback<Signal<Option<MonitorInstanceDetails>>, AnyView>` rendered
in the Events section. Hosts can implement authorized signal actions using the
current detail signal and refresh their latest request on completion. Closing or
switching versions disposes these action views. At most 25 tabs are opened.

Set `show_instances=false` for a diagram-only presentation. On the server, enable
`admin` and reuse `admin::monitor_document`, `admin::load_monitor_snapshot`, or
`admin::load_definition_snapshot`. The last method never loads instance identities
or context. Authorization remains the host's responsibility; choose the server
method after checking the appropriate permission, never by trusting client UI.
The adapter requires a persisted definition UUID and verifies key/version, filters
before pagination, loads history in chunks up to 250, echoes errors and scopes,
and maps unresolved incidents. History prefixes are bounded at 10000 entries.
`EditorDocument` serialization preserves that UUID across host transport; portable
process YAML exported by `to_project_yaml` continues to omit database identity.

`escalation_counts_available` is false when support counts are unavailable; the
health bar then labels those counts unknown. A host that enriches the snapshot
with complete support counts can set it to true. Incident counts always use the
engine's unresolved incident table, independently of the displayed page.
