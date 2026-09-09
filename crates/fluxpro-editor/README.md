# FluxPro Editor

Embeddable Leptos 0.8 components for authoring and monitoring FluxPro processes.
The editor depends on the engine's models, with no database, HTTP server,
router, or application shell. Requires Rust 1.88 or newer.

## Embed

Choose the Leptos mode matching the host application (`csr`, `hydrate`, or
`ssr`); enable only one mode per build target. During local development:

```toml
[dependencies]
fluxpro-editor = { path = "path/to/fluxpro-engine/crates/fluxpro-editor", features = ["csr"] }
leptos = { version = "0.8.20", features = ["csr"] }
```

```rust,no_run
use fluxpro_editor::{EditorDocument, ProcessEditor};
use leptos::prelude::*;

#[component]
fn ProcessDesigner() -> impl IntoView {
    // Or EditorDocument::new(existing_process_definition)? for engine models.
    let document = EditorDocument::default();
    view! {
        <div style="height: 760px; width: 100%">
            <ProcessEditor
                document=document
                on_change=Callback::new(|document: EditorDocument| {
                    // Optional: keep a draft in your application state.
                    let _definition = document.definition;
                })
                on_save=Callback::new(|document: EditorDocument| {
                    // Persist through your application's existing API.
                    let _project_yaml = document.to_project_yaml().unwrap();
                    let _engine_yaml = document.to_process_yaml().unwrap();
                })
            />
        </div>
    }
}
```

`ProcessEditor` is the authoring component. Its props are all optional.
The independent `ProcessMonitor` component is described in [MONITOR.md](MONITOR.md):

| Prop | Meaning |
| --- | --- |
| `document: EditorDocument` | Initial process and coordinates; defaults to Start → Finish |
| `on_change: Callback<EditorDocument>` | Current draft after edits and on initial mount |
| `on_save: Callback<EditorDocument>` | Save button; without it, Save opens YAML export |

`document` initializes a mounted instance. Remount with a new key to switch to
another process from the host; the editor also provides its own YAML import.
Callbacks include drafts with validation issues. They do not deploy, persist,
or send data over the network. Avoid expensive synchronous work in `on_change`,
which also runs during dragging; debounce autosave in the host if needed.

Styles are included and scoped to `.fluxpro-editor`. The component fills the
host container and has a 480px minimum height. A width of 900px or more is
recommended for the three-panel layout. No global CSS reset is required.
Multiple editor instances have independent selection, history, and documents.

## Authoring

- Drag Start, Service task, User task, Condition, Wait, or Finish from the palette.
  Clicking a palette block also adds it; a second Start is prevented.
- Drag a block to move it. Focus a block and use arrow keys for 8px steps, or
  Shift + arrows for 40px steps.
- Pinch with two fingers on the trackpad to zoom around the cursor (25–250%).
  Native Safari/WebKit gestures and Ctrl+wheel pinch events are supported.
  The `−` / `+` buttons zoom around the viewport center; click the percentage
  to reset to 100%. Node coordinates and exported YAML do not change with zoom.
- Scroll naturally in both directions. Command + trackpad scroll pans the
  canvas and suppresses the browser's default gesture on that canvas;
  Command + drag also pans. `Start` returns to the entry node.
- Use the bottom `+` port and then click a destination block or its upper port.
  This sets the default route. Use Properties to add ordered conditional routes.
  Existing conditions are preserved when replacing the default path.
- Smooth SVG Bézier arrows follow blocks as they move. Conditions, timeouts,
  errors, compensation, and backward/cyclic routes are displayed.
- Configure every node through structured Properties: handler, typed arguments,
  form and signal selectors, stage and reason, duration/date timeout, retries,
  ordered paths, error destination and compensation. `Apply settings` commits a
  form as one undoable change. `Reload settings` discards its pending edits.
  Concurrent changes to the same settings are detected before applying.
  Changing Block ID updates incoming routes, self loops and canvas coordinates.
- `Arrange` applies deterministic vertical layers, with Start above and all
  Finish blocks below. Imported definitions receive this layout automatically;
  saved coordinates take precedence. Manual placement remains unrestricted.
- Undo/redo restores definitions and coordinates together (100 steps). Keyboard:
  Command/Ctrl + Z and Command/Ctrl + Shift + Z outside text fields.
- Incoming references must be removed or reconnected before deleting a block.
  Start's required successor must be reconnected, rather than removed.
- Review lists structural errors, unreachable blocks, missing connections, and
  placeholder handlers. Host handlers and Rhai expressions need host validation.

## Forms, signals, and escalations

Five tabs live inside the same `ProcessEditor`: Process, Settings, Forms,
Signals, and Escalations. Switching tabs preserves the canvas viewport and unfinished resource
form edits. Tab buttons support arrow keys and Home/End. Each resource tab has
searchable cards, a declaration count, and an explicit Apply changes button.

- Forms: identifier and role IDs. These are engine declarations; the host
  application supplies the actual form fields and rendering.
- Signals: named events used by waiting nodes and escalation actions.
- Escalations: topic and operator actions, including action ID, operator action,
  button label, help text, presentation kind, and an optional declared signal.

Cards show usage counts. The properties panel lists references and can jump to
an executable node on the Process tab. Renaming updates typed references in
user tasks, wait signal lists, escalation actions, and `create_support_ticket`
topic arguments. Signal names that occur as text in routing conditions must be
updated in those conditions first; the editor does not rewrite Rhai expressions.
Runtime/custom handler references outside the engine schema remain the host's
responsibility.

Referenced declarations cannot be deleted. Duplicate declaration/action IDs,
invalid identifiers, and undeclared escalation signals are rejected atomically.
Resource edits share canvas undo/redo, callbacks, and YAML import/export.
Create forms and signals in their tabs, then select them in node Properties.
Adding a new User task or Wait block still creates its initial declarations.
No extra component props are needed.

## Complete definition settings

Settings contains the process key, name, version, status, activation/deprecation
UTC date pickers, owner, description/version notes (`metadata.comment`), SLA,
business stages and all four lifecycle hooks. The toolbar reflects actual status.
Handler fields suggest keys already present in the definition and accept custom
registry keys. The editor cannot discover a host's handler registry automatically.

| Engine fields | UI |
| --- | --- |
| `key`, `name`, `version`, `status`, `effective_from`, `deprecated_at` | Settings → Identity and version |
| `metadata.owner`, `metadata.comment`, `metadata.sla` | Settings → Metadata |
| `stages` (compact or expanded), `id`, `name`, `is_initial`, `is_final` | Settings → Business stages |
| `special_handlers.on_stage_change`, `on_show_form`, `on_hide_form`, `on_process_complete` | Settings → Lifecycle handlers |
| `forms`, `signals`, `escalations` and their fields | Corresponding resource tabs |
| Node `id`, `type` | Block ID; type chosen in the block palette |
| `handler`, `args` | Service handler; recursive typed argument editor |
| `form`, `wait_for.signal`, `wait_for.signals` | Form selector; signal checkboxes (any selected signal) |
| `set_stage`, `set_stage.stage`, `set_stage.reason` | Business stage selector and optional reason |
| `timeout.after`, `timeout.at`, `timeout.on_timeout` | Duration amount/unit, UTC deadline, destination selector |
| `retries.max`, `retries.backoff` | Retry count and delay amount/unit |
| `next` (direct or routes), `gateway`, `branches`, `default`, `when` | Default destination, XOR rule, ordered conditions and target selectors |
| `on_error.next`, `on_error.compensate` | Error handling destination selectors |

Typed arguments support string, identifier, integer, float, date, datetime,
boolean, typed arrays and arbitrary JSON objects, including nested arrays and
objects. Each value has a type selector; objects offer field addition/removal
and renaming. Support-ticket topic arguments select declared escalations.
Rhai expressions remain text fields. Compound/calendar ISO durations retain
an explicit ISO duration input; simple durations use seconds/minutes/hours/days/weeks.
An absolute timeout date takes priority when both `after` and `at` are set.

Settings preserve compact/expanded representations unless that field is edited.
Database UUID is host-managed and preserved, not an editable YAML field. Used
stages cannot be removed or renamed until block assignments are updated. The
engine's Review diagnostics remain visible for incomplete drafts. SLA is
metadata; compensation and user-task error routes are reserved engine fields.

Regression tests apply every settings form to all seven approval/cash-loan/TK
example files (fixtures, submitted and reviewed) and check identical typed data
and coordinates. The node-specific YAML editor has been removed; YAML remains
available for importing/exporting a whole definition.

## Live process monitoring

The crate also exports a separate, read-only `<ProcessMonitor />`: live node
counts, error/escalation markers, instance search and pagination, closable
instance tabs, execution/stage/signal history and context inspection. It accepts
reactive snapshots and requests data through a host callback. See
[the monitoring integration guide](MONITOR.md) for the full transport contract
and mappings to FluxPro administration queries.

## YAML interchange

```rust
use fluxpro_editor::EditorDocument;

let yaml = "key: demo\nname: Demo\nversion: 1.0.0\nstatus: draft\nnodes:\n  - { id: start, type: Start, next: finish }\n  - { id: finish, type: End }\n";
let document = EditorDocument::from_yaml(yaml).unwrap();
let process_yaml = document.to_process_yaml().unwrap();
let project_yaml = document.to_project_yaml().unwrap();
```

The built-in YAML panel accepts pasted YAML or local `.yaml`/`.yml` files and
can download either representation. Import is one undoable action. Parse errors
leave the current document intact. Schema-valid incomplete drafts are accepted;
duplicate node, form, signal, or escalation IDs are rejected.

Project YAML uses the engine's normal root fields plus:

```yaml
editor:
  positions:
    start: { x: 224.0, y: 64.0 }
    finish: { x: 224.0, y: 432.0 }
```

`to_process_yaml()` omits `editor` metadata. All fields represented by the engine
models survive a round trip, including forms, signals, stages, metadata, retries,
conditions, errors, and timeouts. YAML comments, formatting, and unknown extension
fields are not preserved. Partial layouts are filled automatically; stale IDs
are ignored and coordinates are clamped to finite nonnegative values.

## Browser example

From the repository root, with Trunk installed:

```sh
rustup target add wasm32-unknown-unknown
cd crates/fluxpro-editor
trunk serve --open
```

The example opens the existing approval process and runs entirely in the browser.

## Checks

```sh
cargo test -p fluxpro-editor
cargo check -p fluxpro-editor --features ssr
cargo check -p fluxpro-editor --features hydrate --target wasm32-unknown-unknown
cargo check -p fluxpro-editor --features csr --target wasm32-unknown-unknown
```
