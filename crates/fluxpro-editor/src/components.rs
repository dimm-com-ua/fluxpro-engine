mod declarations;
mod fields;
mod settings;
use crate::EDITOR_CSS;
use crate::declarations::DeclarationKind;
use crate::document::{BlockKind, EditorDocument, EditorState, Position};
use crate::viewport::{MAX_ZOOM, MIN_ZOOM, canvas_point, clamp_zoom, zoom_scroll};
use declarations::{DeclarationPanel, EditorTab, EditorTabs};
use leptos::{ev, html, prelude::*};
use settings::{ProcessInspector, ProcessSettings};
use wasm_bindgen::JsCast;

#[derive(Clone)]
struct Drag {
    id: Option<String>,
    pointer: i32,
    client: Position,
    origin: Position,
    scroll: Position,
}

#[derive(Clone, Copy)]
struct Session {
    state: RwSignal<EditorState>,
    selected: RwSignal<Option<String>>,
    connecting: RwSignal<Option<String>>,
    message: RwSignal<String>,
    reset_view: RwSignal<u64>,
    reveal_node: RwSignal<Option<String>>,
    reveal_declaration: RwSignal<Option<(DeclarationKind, String)>>,
}

impl Session {
    fn edit(self, edit: impl FnOnce(&mut EditorDocument) -> Result<(), String>) {
        let mut result = Ok(());
        self.state.update(|state| result = state.edit(edit));
        self.message.set(result.err().unwrap_or_default());
    }

    fn add(self, kind: BlockKind, position: Position) {
        let mut result = Err(String::new());
        self.state
            .update(|state| result = state.edit(|doc| doc.add_block(kind, position)));
        match result {
            Ok(id) => {
                self.selected.set(Some(id));
                self.message.set(String::new());
            }
            Err(error) => self.message.set(error),
        }
    }

    fn select(self, id: String) {
        if let Some(source) = self.connecting.get_untracked() {
            self.edit(|doc| doc.connect(&source, &id, None));
            self.connecting.set(None);
        }
        self.selected.set(Some(id));
    }
}

/// A complete embeddable workflow editor: palette, vertical canvas, properties,
/// YAML import/export, and undo/redo. Styles are scoped to `.fluxpro-editor`.
///
/// `document` initializes this mounted editor. To load another document later,
/// use its import UI or remount with a new key in the host. Callbacks contain
/// the full definition and coordinates, including incomplete drafts. The
/// component makes no network requests and does not persist data by itself.
///
/// ```rust,no_run
/// use fluxpro_editor::{EditorDocument, ProcessEditor};
/// use leptos::prelude::*;
///
/// # fn example() -> impl IntoView {
/// let document = EditorDocument::default();
/// view! {
///     <div style="height: 760px">
///         <ProcessEditor document=document
///             on_save=Callback::new(|document: EditorDocument| {
///                 let yaml = document.to_project_yaml().unwrap();
///                 // Persist through your application's existing API.
///                 leptos::logging::log!("{}", yaml);
///             })
///         />
///     </div>
/// }
/// # }
/// ```
#[component]
pub fn ProcessEditor(
    /// Initial definition and layout; defaults to a Start → Finish draft.
    #[prop(default = EditorDocument::default())]
    document: EditorDocument,
    /// Called after document changes, including manual layout changes, and once
    /// after the component mounts. No persistence is performed automatically.
    #[prop(optional)]
    on_change: Option<Callback<EditorDocument>>,
    /// Called when Save is clicked. If omitted, Save opens the YAML export panel.
    #[prop(optional)]
    on_save: Option<Callback<EditorDocument>>,
) -> impl IntoView {
    let session = Session {
        state: RwSignal::new(EditorState::new(document)),
        selected: RwSignal::new(None),
        connecting: RwSignal::new(None),
        message: RwSignal::new(String::new()),
        reset_view: RwSignal::new(0),
        reveal_node: RwSignal::new(None),
        reveal_declaration: RwSignal::new(None),
    };
    let active_tab = RwSignal::new(EditorTab::Process);
    let yaml_open = RwSignal::new(false);
    let yaml = RwSignal::new(String::new());
    let yaml_error = RwSignal::new(String::new());
    let yaml_input = NodeRef::<html::Textarea>::new();
    Effect::new(move |_| {
        if yaml_open.get() {
            if let Some(input) = yaml_input.get() {
                let _ = input.focus();
            }
        }
    });
    let diagnostics = Memo::new(move |_| session.state.with(|s| s.document.diagnostics()));
    Effect::new(move |_| {
        let document = session.state.with(|state| state.document.clone());
        if let Some(callback) = on_change {
            callback.run(document);
        }
    });
    let open_yaml = move || match session
        .state
        .with_untracked(|s| s.document.to_project_yaml())
    {
        Ok(value) => {
            yaml.set(value);
            yaml_error.set(String::new());
            yaml_open.set(true);
        }
        Err(error) => session.message.set(error),
    };
    view! {
        <section class="fluxpro-editor" aria-label="FluxPro process editor"
            on:keydown=move |event: ev::KeyboardEvent| {
                if event.key() == "Escape" { session.connecting.set(None); yaml_open.set(false); }
                let editing_text = event.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                    .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT") || el.closest("[contenteditable=true]").ok().flatten().is_some());
                if !editing_text && !yaml_open.get_untracked() && (event.meta_key() || event.ctrl_key()) && event.key().eq_ignore_ascii_case("z") {
                    event.prevent_default();
                    session.state.update(|s| if event.shift_key() { s.redo() } else { s.undo() });
                }
            }>
            <style>{EDITOR_CSS}</style>
            <header class="fp-toolbar" inert=move || yaml_open.get()>
                <div class="fp-brand"><span class="fp-brand-mark">"ƒ"</span><div><strong>"Process studio"</strong><span>"FLUXPRO"</span></div></div>
                <div class="fp-process-name">{move || session.state.with(|s| s.document.definition.name.clone())}<span class="fp-draft">{move || session.state.with(|s| s.document.definition.status.to_string())}</span></div>
                <div class="fp-actions">
                    <button type="button" title="Undo · ⌘Z" aria-label="Undo" disabled=move || !session.state.with(|s| s.can_undo()) on:click=move |_| session.state.update(EditorState::undo)>"↶"</button>
                    <button type="button" title="Redo · ⌘⇧Z" aria-label="Redo" disabled=move || !session.state.with(|s| s.can_redo()) on:click=move |_| session.state.update(EditorState::redo)>"↷"</button>
                    <button type="button" disabled=move || active_tab.get() != EditorTab::Process on:click=move |_| { session.reveal_node.set(None); session.edit(|doc| { doc.auto_layout(); Ok(()) }); session.reset_view.update(|revision| *revision += 1); }>"↓ Arrange"</button>
                    <button type="button" on:click=move |_| open_yaml()>"YAML"</button>
                    <button type="button" class="fp-primary" on:click=move |_| {
                        if let Some(callback) = on_save { callback.run(session.state.with_untracked(|s| s.document.clone())); }
                        else { open_yaml(); }
                    }>"Save"</button>
                </div>
            </header>
            <div inert=move || yaml_open.get()><EditorTabs session=session active=active_tab/></div>
            <div class="fp-editor-body" inert=move || yaml_open.get()>
                <div class="fp-workspace fp-tab-panel" role="tabpanel" aria-label="Process" hidden=move || active_tab.get() != EditorTab::Process>
                    <BlockPalette session=session/>
                    <ProcessCanvas session=session/>
                    <ProcessInspector session=session active=active_tab/>
                </div>
                <div class="fp-tab-panel" role="tabpanel" aria-label="Settings" hidden=move || active_tab.get() != EditorTab::Settings><ProcessSettings session=session/></div>
                <div class="fp-tab-panel" role="tabpanel" aria-label="Forms" hidden=move || active_tab.get() != EditorTab::Forms><DeclarationPanel session=session kind=DeclarationKind::Form active=active_tab/></div>
                <div class="fp-tab-panel" role="tabpanel" aria-label="Signals" hidden=move || active_tab.get() != EditorTab::Signals><DeclarationPanel session=session kind=DeclarationKind::Signal active=active_tab/></div>
                <div class="fp-tab-panel" role="tabpanel" aria-label="Escalations" hidden=move || active_tab.get() != EditorTab::Escalations><DeclarationPanel session=session kind=DeclarationKind::Escalation active=active_tab/></div>
            </div>
            <footer class="fp-status">
                <span class="fp-status-count">{move || session.state.with(|s| format!("{} blocks · {} connections", s.document.definition.nodes.len(), s.document.connections().len()))}</span>
                <span class="fp-gesture-hint" hidden=move || active_tab.get() != EditorTab::Process>"Pinch to zoom · Scroll to explore · ⌘ + drag to pan"</span>
                <span role="status" class="fp-feedback">{move || {
                    let message = session.message.get();
                    if !message.is_empty() { message }
                    else if let Some(id) = session.connecting.get() { format!("Connect {id}: choose a destination · Esc to cancel") }
                    else if diagnostics.get().is_empty() { "No structural issues".into() }
                    else { format!("{} items to review", diagnostics.get().len()) }
                }}</span>
            </footer>
            <Show when=move || yaml_open.get()>
                <div class="fp-modal-backdrop">
                    <section class="fp-yaml-dialog" role="dialog" aria-modal="true" aria-label="Import and export process YAML">
                        <header><div><h2>"Process YAML"</h2><p>"Import a definition or save your process with its layout."</p></div><button type="button" aria-label="Close YAML panel" on:click=move |_| yaml_open.set(false)>"✕"</button></header>
                        <div class="fp-yaml-tools">
                            <label class="fp-file">"Open .yaml file"<input type="file" accept=".yaml,.yml,application/yaml,text/yaml" on:change=move |event| {
                                let input = event_target::<web_sys::HtmlInputElement>(&event);
                                if let Some(file) = input.files().and_then(|files| files.get(0)) {
                                    leptos::task::spawn_local(async move {
                                        match wasm_bindgen_futures::JsFuture::from(file.text()).await {
                                            Ok(text) => { yaml.set(text.as_string().unwrap_or_default()); yaml_error.set(String::new()); }
                                            Err(_) => yaml_error.set("Could not read this file.".into()),
                                        }
                                    });
                                }
                                input.set_value("");
                            }/></label>
                            <button type="button" on:click=move |_| {
                                match session.state.with_untracked(|s| s.document.to_process_yaml()) {
                                    Ok(text) => yaml.set(text), Err(error) => yaml_error.set(error),
                                }
                            }>"Process only"</button>
                            <button type="button" on:click=move |_| open_yaml()>"Include layout"</button>
                        </div>
                        <textarea class="fp-yaml-input" node_ref=yaml_input aria-label="Process YAML" spellcheck="false" prop:value=move || yaml.get() on:input=move |event| yaml.set(event_target_value(&event))></textarea>
                        <p class="fp-error" role="alert">{move || yaml_error.get()}</p>
                        <footer><a class="fp-button" download="process.yaml" href=move || format!("data:application/yaml;charset=utf-8,{}", percent_encode(&yaml.get()))>"Download YAML"</a>
                        <button type="button" class="fp-primary" on:click=move |_| {
                            match EditorDocument::from_yaml(&yaml.get_untracked()) {
                                Ok(document) => {
                                    session.edit(|doc| { *doc = document; Ok(()) });
                                    session.selected.set(None); session.connecting.set(None); session.reveal_node.set(None); session.reveal_declaration.set(None); active_tab.set(EditorTab::Process); yaml_open.set(false); session.reset_view.update(|revision| *revision += 1);
                                }
                                Err(error) => yaml_error.set(error),
                            }
                        }>"Import process"</button></footer>
                    </section>
                </div>
            </Show>
        </section>
    }
}

fn percent_encode(text: &str) -> String {
    text.bytes().map(|byte| format!("%{byte:02X}")).collect()
}

#[component]
fn BlockPalette(session: Session) -> impl IntoView {
    view! {
        <aside class="fp-palette" aria-label="Block palette">
            <div class="fp-panel-heading"><span class="fp-eyebrow">"BUILD YOUR FLOW"</span><h2>"Blocks"</h2><p>"Drag onto the canvas, or click to add."</p></div>
            <div class="fp-palette-list">
                {BlockKind::ALL.into_iter().map(|kind| view! {
                    <button type="button" class="fp-palette-block" data-kind=kind.name() aria-label=kind.label() title=kind.hint() draggable="true"
                        disabled=move || kind == BlockKind::Start && session.state.with(|s| s.document.definition.nodes.iter().any(|n| n.is_start()))
                        on:dragstart=move |event: ev::DragEvent| {
                            if let Some(data) = event.data_transfer() { let _ = data.set_data("application/x-fluxpro-block", kind.name()); data.set_effect_allowed("copy"); }
                        }
                        on:click=move |_| {
                            let position = session.state.with_untracked(|s| {
                                session.selected.get_untracked().and_then(|id| s.document.positions.get(&id).copied())
                                    .map(|p| Position::new(p.x + 272.0, p.y))
                                    .unwrap_or_else(|| Position::new(80.0, s.document.positions.values().map(|p| p.y).fold(0.0, f64::max) + 160.0))
                            });
                            session.add(kind, position);
                        }>
                        <span class="fp-kind-icon" aria-hidden="true">{kind.symbol()}</span><span><strong>{kind.label()}</strong><small>{kind.hint()}</small></span><span class="fp-drag-grip" aria-hidden="true">"⠿"</span>
                    </button>
                }).collect_view()}
            </div>
            <div class="fp-palette-tip"><span>"A little guidance"</span><p>"Start at the top. Finish below. Use a condition when your process needs a choice."</p></div>
        </aside>
    }
}

#[component]
fn ProcessCanvas(session: Session) -> impl IntoView {
    let viewport = NodeRef::<html::Div>::new();
    let drag = RwSignal::new(None::<Drag>);
    let zoom = RwSignal::new(1.0_f64);
    let pending_scroll = RwSignal::new(None::<Position>);
    let gesture = RwSignal::new(None::<f64>);
    let zoom_at = Callback::new(move |(requested, anchor): (f64, Position)| {
        let Some(element) = viewport.get_untracked() else {
            return;
        };
        let old = zoom.get_untracked();
        let next = clamp_zoom(requested);
        if (old - next).abs() < f64::EPSILON {
            return;
        }
        drag.set(None);
        let scroll = pending_scroll.get_untracked().unwrap_or(Position {
            x: element.scroll_left() as f64,
            y: element.scroll_top() as f64,
        });
        pending_scroll.set(Some(zoom_scroll(scroll, anchor, old, next)));
        zoom.set(next);
        // Apply the accumulated offset after the scaled scroll area is rendered.
        leptos::leptos_dom::helpers::request_animation_frame(move || {
            if let Some(Some(scroll)) = pending_scroll.try_get_untracked() {
                pending_scroll.set(None);
                if let Some(Some(element)) = viewport.try_get_untracked() {
                    element.set_scroll_left(scroll.x.round() as i32);
                    element.set_scroll_top(scroll.y.round() as i32);
                }
            }
        });
    });
    let zoom_center = move |next: f64| {
        if let Some(element) = viewport.get_untracked() {
            zoom_at.run((
                next,
                Position::new(
                    element.client_width() as f64 / 2.0,
                    element.client_height() as f64 / 2.0,
                ),
            ));
        }
    };
    Effect::new(move |_| {
        session.reset_view.get();
        let reveal = session.reveal_node.get();
        if let Some(element) = viewport.get() {
            let position = session.state.with_untracked(|s| {
                s.document
                    .definition
                    .nodes
                    .iter()
                    .find(|n| {
                        reveal
                            .as_deref()
                            .map_or_else(|| n.is_start(), |id| n.id().get_id() == id)
                    })
                    .and_then(|n| s.document.positions.get(n.id().get_id()))
                    .copied()
                    .unwrap_or_default()
            });
            element.set_scroll_left(
                ((position.x + 112.0) * zoom.get_untracked() - element.client_width() as f64 / 2.0)
                    .max(0.0) as i32,
            );
            element.set_scroll_top((position.y * zoom.get_untracked() - 64.0).max(0.0) as i32);
        }
    });
    let start_drag = Callback::new(move |(id, event): (String, ev::PointerEvent)| {
        if event.button() != 0 || session.connecting.get_untracked().is_some() {
            return;
        }
        if let Some(element) = viewport.get_untracked() {
            event.prevent_default();
            event.stop_propagation();
            let scroll = Position {
                x: element.scroll_left() as f64,
                y: element.scroll_top() as f64,
            };
            let origin = session
                .state
                .with_untracked(|s| s.document.positions.get(&id).copied().unwrap_or_default());
            if !event.meta_key() {
                session.state.update(EditorState::checkpoint);
                session.selected.set(Some(id.clone()));
            }
            drag.set(Some(Drag {
                id: if event.meta_key() { None } else { Some(id) },
                pointer: event.pointer_id(),
                client: Position {
                    x: event.client_x() as f64,
                    y: event.client_y() as f64,
                },
                origin,
                scroll,
            }));
            let _ = element.set_pointer_capture(event.pointer_id());
        }
    });
    let edges = Memo::new(move |_| session.state.with(|s| s.document.connections()));
    view! {
        <main class="fp-canvas-wrap">
            <div class="fp-canvas-caption"><span class="fp-canvas-dot"></span>"PROCESS CANVAS"<span>"TOP → BOTTOM"</span></div>
            <div class="fp-canvas" node_ref=viewport tabindex="0" aria-label="Process canvas. Drag blocks to arrange. Command and drag to pan. Pinch to zoom."
                class:fp-panning=move || drag.with(|d| d.as_ref().is_some_and(|d| d.id.is_none()))
                on:wheel=move |event: ev::WheelEvent| {
                    if event.ctrl_key() {
                        event.prevent_default(); event.stop_propagation();
                        if gesture.get_untracked().is_none() {
                            if let Some(element) = viewport.get_untracked() {
                                let rect = element.get_bounding_client_rect();
                                let unit = match event.delta_mode() { 1 => 16.0, 2 => element.client_height() as f64, _ => 1.0 };
                                let factor = (-event.delta_y() * unit * 0.008).clamp(-1.0, 1.0).exp();
                                zoom_at.run((zoom.get_untracked() * factor, Position::new(event.client_x() as f64 - rect.left(), event.client_y() as f64 - rect.top())));
                            }
                        }
                    } else if event.meta_key() {
                        event.prevent_default(); event.stop_propagation();
                        if let Some(element) = viewport.get_untracked() {
                            let scale = match event.delta_mode() { 1 => 16.0, 2 => element.client_height() as f64, _ => 1.0 };
                            element.set_scroll_left(element.scroll_left() + (event.delta_x() * scale).round() as i32);
                            element.set_scroll_top(element.scroll_top() + (event.delta_y() * scale).round() as i32);
                        }
                    }
                }
                on:gesturestart=move |event: ev::Event| {
                    event.prevent_default(); event.stop_propagation();
                    gesture.set(Some(zoom.get_untracked()));
                }
                on:gesturechange=move |event: ev::Event| {
                    event.prevent_default(); event.stop_propagation();
                    if let (Some(initial), Some(element)) = (gesture.get_untracked(), viewport.get_untracked()) {
                        let rect = element.get_bounding_client_rect();
                        let scale = gesture_number(&event, "scale").unwrap_or(1.0);
                        let x = gesture_number(&event, "clientX").map(|x| x - rect.left()).unwrap_or(element.client_width() as f64 / 2.0);
                        let y = gesture_number(&event, "clientY").map(|y| y - rect.top()).unwrap_or(element.client_height() as f64 / 2.0);
                        zoom_at.run((initial * scale, Position::new(x, y)));
                    }
                }
                on:gestureend=move |event: ev::Event| {
                    event.prevent_default(); event.stop_propagation(); gesture.set(None);
                }
                on:dragover=move |event: ev::DragEvent| { event.prevent_default(); if let Some(data) = event.data_transfer() { data.set_drop_effect("copy"); } }
                on:drop=move |event: ev::DragEvent| {
                    event.prevent_default();
                    if let (Some(element), Some(data)) = (viewport.get_untracked(), event.data_transfer()) {
                        if let Ok(name) = data.get_data("application/x-fluxpro-block") {
                            if let Some(kind) = BlockKind::ALL.into_iter().find(|kind| kind.name() == name) {
                                let rect = element.get_bounding_client_rect();
                                let point = canvas_point(Position { x: event.client_x() as f64 - rect.left(), y: event.client_y() as f64 - rect.top() }, Position { x: element.scroll_left() as f64, y: element.scroll_top() as f64 }, zoom.get_untracked());
                                session.add(kind, Position::new(point.x - 112.0, point.y - 44.0));
                            }
                        }
                    }
                }
                on:pointerdown=move |event: ev::PointerEvent| {
                    if event.meta_key() && event.button() == 0 {
                        if let Some(element) = viewport.get_untracked() {
                            event.prevent_default();
                            drag.set(Some(Drag { id: None, pointer: event.pointer_id(), client: Position { x: event.client_x() as f64, y: event.client_y() as f64 }, origin: Position::default(), scroll: Position { x: element.scroll_left() as f64, y: element.scroll_top() as f64 } }));
                            let _ = element.set_pointer_capture(event.pointer_id());
                        }
                    }
                }
                on:pointermove=move |event: ev::PointerEvent| {
                    if let (Some(active), Some(element)) = (drag.get_untracked(), viewport.get_untracked()) {
                        if event.pointer_id() != active.pointer { return; }
                        let dx = event.client_x() as f64 - active.client.x;
                        let dy = event.client_y() as f64 - active.client.y;
                        if let Some(id) = active.id {
                            let position = Position::new(active.origin.x + (dx + element.scroll_left() as f64 - active.scroll.x) / zoom.get_untracked(), active.origin.y + (dy + element.scroll_top() as f64 - active.scroll.y) / zoom.get_untracked());
                            session.state.update(|s| s.document.move_node(&id, position));
                        } else {
                            element.set_scroll_left((active.scroll.x - dx).round() as i32);
                            element.set_scroll_top((active.scroll.y - dy).round() as i32);
                        }
                    }
                }
                on:pointerup=move |event: ev::PointerEvent| {
                    drag.set(None);
                    if let Some(element) = viewport.get_untracked() { let _ = element.release_pointer_capture(event.pointer_id()); }
                }
                on:pointercancel=move |_| drag.set(None)
                on:lostpointercapture=move |_| drag.set(None)>
                <div class="fp-world" style=move || session.state.with(|s| { let (w,h) = s.document.canvas_size(); let z = zoom.get(); format!("width:{}px;height:{}px;background-size:{}px {}px", w*z, h*z, 20.0*z, 20.0*z) })>
                    <div class="fp-scene" style=move || session.state.with(|s| { let (w,h) = s.document.canvas_size(); format!("width:{w}px;height:{h}px;transform:scale({})", zoom.get()) })>
                    <svg class="fp-connections" aria-label="Process connections" width=move || session.state.with(|s| s.document.canvas_size().0) height=move || session.state.with(|s| s.document.canvas_size().1)>
                        {move || edges.get().into_iter().filter_map(|edge| {
                            session.state.with(|s| {
                                let path = edge.path(&s.document.positions)?;
                                let target = s.document.positions.get(&edge.target)?;
                                let tip_x = target.x + 112.0;
                                let tip_y = target.y;
                                let source = s.document.positions.get(&edge.source)?;
                                let label_x = (source.x + target.x) / 2.0 + 124.0;
                                let label_y = (source.y + 88.0 + target.y) / 2.0;
                                Some(view! {
                                    <g class="fp-edge" class:fp-edge-special=matches!(edge.label.as_str(), "On error" | "Timeout" | "Compensate")>
                                        <title>{format!("{} → {} {}", edge.source, edge.target, edge.label)}</title>
                                        <path d=path fill="none"/>
                                        <path class="fp-arrow" d=format!("M {} {} L {tip_x} {tip_y} L {} {} Z", tip_x - 4.0, tip_y - 8.0, tip_x + 4.0, tip_y - 8.0)/>
                                        <text x=label_x y=label_y>{if edge.label.chars().count() > 28 { format!("{}…", edge.label.chars().take(27).collect::<String>()) } else { edge.label.clone() }}</text>
                                    </g>
                                })
                            })
                        }).collect_view()}
                    </svg>
                    <For each=move || session.state.with(|s| s.document.definition.nodes.iter().map(|n| n.id().to_string()).collect::<Vec<_>>()) key=|id| id.clone() children=move |id| {
                        let id = StoredValue::new(id);
                        let kind = Memo::new(move |_| session.state.with(|s| s.document.node(&id.get_value()).map(BlockKind::of).unwrap_or(BlockKind::End)));
                        view! {
                            <div class="fp-node" data-node-id=id.get_value() data-kind=move || kind.get().name()
                                class:fp-selected=move || session.selected.get().as_deref() == Some(id.get_value().as_str())
                                class:fp-connecting=move || session.connecting.get().as_deref() == Some(id.get_value().as_str())
                                style=move || session.state.with(|s| { let p = s.document.positions.get(&id.get_value()).copied().unwrap_or_default(); format!("transform:translate({}px,{}px)", p.x, p.y) })>
                                <button type="button" class="fp-node-body" on:pointerdown=move |event| start_drag.run((id.get_value(), event))
                                    on:click=move |_| session.select(id.get_value())
                                    on:keydown=move |event: ev::KeyboardEvent| {
                                        let (dx,dy) = match event.key().as_str() { "ArrowLeft" => (-1.0,0.0), "ArrowRight" => (1.0,0.0), "ArrowUp" => (0.0,-1.0), "ArrowDown" => (0.0,1.0), _ => return };
                                        event.prevent_default(); let step = if event.shift_key() { 40.0 } else { 8.0 };
                                        session.edit(|doc| { if let Some(p) = doc.positions.get(&id.get_value()).copied() { doc.move_node(&id.get_value(), Position::new(p.x+dx*step,p.y+dy*step)); } Ok(()) });
                                    }>
                                    <span class="fp-kind-icon" aria-hidden="true">{move || kind.get().symbol()}</span>
                                    <span class="fp-node-copy"><strong>{move || kind.get().label()}</strong><small>{id.get_value()}</small></span>
                                    <span class="fp-node-menu" aria-hidden="true">"⠿"</span>
                                </button>
                                <Show when=move || kind.get() != BlockKind::Start>
                                    <button type="button" class="fp-port fp-port-in" aria-label=format!("Connect to {}", id.get_value()) title="Connect here" on:click=move |_| session.select(id.get_value())></button>
                                </Show>
                                <Show when=move || kind.get() != BlockKind::End>
                                    <button type="button" class="fp-port fp-port-out" aria-label=format!("Connect from {}", id.get_value()) title="Connect to another block" on:click=move |_| { session.selected.set(Some(id.get_value())); session.connecting.set(Some(id.get_value())); }>"+"</button>
                                </Show>
                            </div>
                        }
                    }/>
                    </div>
                </div>
            </div>
            <div class="fp-canvas-navigation"><button type="button" title="Return to Start" on:click=move |_| {
                if let Some(element) = viewport.get_untracked() {
                    let p = session.state.with_untracked(|s| s.document.definition.nodes.iter().find(|n| n.is_start()).and_then(|n| s.document.positions.get(n.id().get_id())).copied().unwrap_or_default());
                    element.set_scroll_left(((p.x + 112.0) * zoom.get_untracked() - element.client_width() as f64 / 2.0).max(0.0) as i32);
                    element.set_scroll_top((p.y * zoom.get_untracked() - 64.0).max(0.0) as i32);
                }
            }>"⌖ Start"</button>
                <button type="button" aria-label="Zoom out" title="Zoom out" disabled={move || zoom.get() <= MIN_ZOOM} on:click=move |_| zoom_center(zoom.get_untracked() / 1.2)>"−"</button>
                <button type="button" class="fp-zoom-level" aria-label="Reset zoom to 100%" title="Reset zoom" on:click=move |_| zoom_center(1.0)>{move || format!("{:.0}%", zoom.get() * 100.0)}</button>
                <button type="button" aria-label="Zoom in" title="Zoom in" disabled={move || zoom.get() >= MAX_ZOOM} on:click=move |_| zoom_center(zoom.get_untracked() * 1.2)>"+"</button>
            </div>
        </main>
    }
}

// WebKit exposes native trackpad pinch via GestureEvent instead of ctrl+wheel.
fn gesture_number(event: &ev::Event, property: &str) -> Option<f64> {
    js_sys::Reflect::get(event.as_ref(), &property.into())
        .ok()?
        .as_f64()
        .filter(|value| value.is_finite())
}
