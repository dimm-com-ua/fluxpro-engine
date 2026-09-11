use super::Session;
use crate::editor::declarations::DeclarationKind;
use leptos::{ev, prelude::*};
use serde_json::{Value, json};
use wasm_bindgen::JsCast;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EditorTab {
    Process,
    Settings,
    Forms,
    Signals,
    Escalations,
}

impl EditorTab {
    const ALL: [Self; 5] = [
        Self::Process,
        Self::Settings,
        Self::Forms,
        Self::Signals,
        Self::Escalations,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Process => "Process",
            Self::Settings => "Settings",
            Self::Forms => "Forms",
            Self::Signals => "Signals",
            Self::Escalations => "Escalations",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Settings => "settings",
            Self::Forms => "forms",
            Self::Signals => "signals",
            Self::Escalations => "escalations",
        }
    }
    fn count(self, session: Session) -> usize {
        session.state.with(|s| match self {
            Self::Process => s.document.definition.nodes.len(),
            Self::Settings => s.document.definition.stages.len(),
            Self::Forms => s.document.definition.forms.len(),
            Self::Signals => s.document.definition.signals.len(),
            Self::Escalations => s.document.definition.escalations.len(),
        })
    }
}

#[component]
pub(super) fn EditorTabs(session: Session, active: RwSignal<EditorTab>) -> AnyView {
    view! {
        <nav class="fp-tabs" role="tablist" aria-label="Process sections">
            {EditorTab::ALL.into_iter().enumerate().map(|(index, tab)| view! {
                <button type="button" role="tab" aria-label=tab.label() data-editor-tab=tab.key()
                    aria-selected=move || active.get() == tab tabindex=move || if active.get() == tab { 0 } else { -1 }
                    class:fp-tab-active=move || active.get() == tab
                    on:click=move |_| { active.set(tab); session.connecting.set(None); }
                    on:keydown=move |event: ev::KeyboardEvent| {
                        let next = match event.key().as_str() { "ArrowRight" => (index+1)%5, "ArrowLeft" => (index+4)%5, "Home" => 0, "End" => 4, _ => return };
                        event.prevent_default(); active.set(EditorTab::ALL[next]); session.connecting.set(None);
                        if let Some(parent) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).and_then(|el| el.parent_element()) {
                            if let Ok(Some(button)) = parent.query_selector(&format!("[data-editor-tab='{}']", EditorTab::ALL[next].key())) {
                                if let Ok(button) = button.dyn_into::<web_sys::HtmlElement>() { let _ = button.focus(); }
                            }
                        }
                    }>
                    {tab.label()}<span class="fp-tab-count">{move || tab.count(session)}</span>
                </button>
            }).collect_view()}
        </nav>
    }
    .into_any()
}

#[component]
pub(super) fn DeclarationPanel(
    session: Session,
    kind: DeclarationKind,
    active: RwSignal<EditorTab>,
) -> AnyView {
    let search = RwSignal::new(String::new());
    let role_input = RwSignal::new(String::new());
    let original = RwSignal::new(None::<String>);
    let draft = RwSignal::new(Value::Null);
    let baseline = RwSignal::new(Value::Null);
    let editing = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let pending_key = StoredValue::new(format!("declaration:{}", kind.singular()));
    Effect::new(move |_| {
        session.track_pending(
            pending_key.get_value(),
            editing.get() && draft.get() != baseline.get(),
        )
    });
    on_cleanup(move || session.track_pending(pending_key.get_value(), false));
    let guard_pending = Callback::new(move |_: ()| {
        if session.require_applied_changes
            && editing.get_untracked()
            && draft.get_untracked() != baseline.get_untracked()
        {
            error.set("Apply or cancel these changes before editing another declaration.".into());
            true
        } else {
            false
        }
    });
    let matching = Memo::new(move |_| {
        session.state.with(|s| {
            s.document
                .declaration_ids(kind)
                .into_iter()
                .filter(|id| id.to_lowercase().contains(&search.get().to_lowercase()))
                .collect::<Vec<_>>()
        })
    });
    // Refresh clean forms after undo/import; protect unfinished edits from being overwritten.
    Effect::new(move |_| {
        let id = original.get();
        let current = session.state.with(|s| {
            id.as_deref()
                .and_then(|id| s.document.declaration(kind, id))
        });
        if let Some(current) = current {
            if draft.get_untracked() == baseline.get_untracked() {
                role_input.set(roles_text(&current));
                draft.set(current.clone());
                baseline.set(current);
            }
        } else if id.is_some() {
            editing.set(false);
            original.set(None);
        }
    });
    let load = Callback::new(move |id: String| {
        if guard_pending.run(()) {
            return;
        }
        if let Some(value) = session
            .state
            .with_untracked(|s| s.document.declaration(kind, &id))
        {
            role_input.set(roles_text(&value));
            draft.set(value.clone());
            baseline.set(value);
            original.set(Some(id));
            editing.set(true);
            error.set(String::new());
        }
    });
    Effect::new(move |_| {
        if let Some((requested_kind, id)) = session.reveal_declaration.get() {
            if requested_kind == kind {
                load.run(id);
                session.reveal_declaration.set(None);
            }
        }
    });
    view! {
        <div class="fp-declarations">
            <DeclarationBoard
                session kind search role_input original draft baseline editing error matching
                guard_pending load
            />
            <DeclarationInspector
                session kind active role_input original draft baseline editing error load
            />
        </div>
    }
    .into_any()
}

#[component]
fn DeclarationBoard(
    session: Session,
    kind: DeclarationKind,
    search: RwSignal<String>,
    role_input: RwSignal<String>,
    original: RwSignal<Option<String>>,
    draft: RwSignal<Value>,
    baseline: RwSignal<Value>,
    editing: RwSignal<bool>,
    error: RwSignal<String>,
    matching: Memo<Vec<String>>,
    guard_pending: Callback<(), bool>,
    load: Callback<String>,
) -> AnyView {
    view! {
        <main class="fp-declaration-board">
            <header class="fp-declaration-heading">
                <div><span class="fp-eyebrow">"PROCESS RESOURCES"</span><h2>{kind.label()}</h2><p>{kind.hint()}</p></div>
                <button type="button" class="fp-primary" on:click=move |_| {
                    if guard_pending.run(()){return;}
                    original.set(None); role_input.set(String::new()); draft.set(session.state.with_untracked(|s| s.document.new_declaration(kind))); baseline.set(Value::Null); editing.set(true); error.set(String::new());
                }>{format!("+ Add {}", kind.singular())}</button>
            </header>
            <label class="fp-resource-search"><span>"⌕"</span><input type="search" aria-label=format!("Search {}", kind.label().to_lowercase()) placeholder=format!("Find a {}…", kind.singular()) prop:value=move || search.get() on:input=move |event| search.set(event_target_value(&event))/></label>
            <DeclarationCards session kind matching editing original load />
            <Show when=move || matching.get().is_empty()>
                <div class="fp-resource-empty"><span>{kind.symbol()}</span><h3>{move || if search.get().is_empty() { format!("Your {} live here", kind.label().to_lowercase()) } else { "No matching declarations".into() }}</h3><p>"Add a declaration above, or import an existing process from YAML."</p></div>
            </Show>
        </main>
    }
    .into_any()
}

#[component]
fn DeclarationCards(
    session: Session,
    kind: DeclarationKind,
    matching: Memo<Vec<String>>,
    editing: RwSignal<bool>,
    original: RwSignal<Option<String>>,
    load: Callback<String>,
) -> AnyView {
    view! {
        <div class="fp-resource-grid">
            <For each=move || matching.get() key=|id| id.clone() children=move |id| {
                view! { <DeclarationCard session kind id editing original load /> }
            }/>
        </div>
    }
    .into_any()
}

#[component]
fn DeclarationCard(
    session: Session,
    kind: DeclarationKind,
    id: String,
    editing: RwSignal<bool>,
    original: RwSignal<Option<String>>,
    load: Callback<String>,
) -> AnyView {
    let id = StoredValue::new(id);
    let resource = Memo::new(move |_| {
        session.state.with(|s| {
            s.document
                .declaration(kind, &id.get_value())
                .unwrap_or(Value::Null)
        })
    });
    let uses = Memo::new(move |_| {
        session
            .state
            .with(|s| s.document.declaration_uses(kind, &id.get_value()).len())
    });
    view! {
        <button type="button" class="fp-resource-card" data-resource-kind=kind.singular()
            class:fp-resource-selected=move || editing.get() && original.get().as_deref() == Some(id.get_value().as_str())
            aria-label=format!("Edit {} {}", kind.singular(), id.get_value()) on:click=move |_| load.run(id.get_value())>
            <span class="fp-resource-top"><span class="fp-resource-icon">{kind.symbol()}</span><span class="fp-resource-tag">{kind.singular()}</span><span class="fp-resource-edit">"↗"</span></span>
            <strong>{id.get_value()}</strong>
            <span class="fp-resource-summary">{move || match kind {
                DeclarationKind::Form => {
                    let roles = resource.get()["roles"].as_array().cloned().unwrap_or_default();
                    if roles.is_empty() { "No roles specified".into() } else { roles.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" · ") }
                }
                DeclarationKind::Signal => "Resumes a waiting step".into(),
                DeclarationKind::Escalation => { let count = resource.get()["actions"].as_array().map_or(0, Vec::len); format!("{count} operator action{}", if count == 1 { "" } else { "s" }) },
            }}</span>
            <Show when=move || kind == DeclarationKind::Escalation>
                <span class="fp-action-previews">{move || resource.get()["actions"].as_array().cloned().unwrap_or_default().into_iter().take(3).map(|action| view! {
                    <span data-intent=action["kind"].as_str().unwrap_or("Secondary").to_string()>{action["label"].as_str().unwrap_or("Action").to_string()}</span>
                }).collect_view()}</span>
            </Show>
            <span class="fp-resource-usage">{move || if uses.get() == 0 { "Not used yet".into() } else { format!("Used in {} place{}", uses.get(), if uses.get() == 1 { "" } else { "s" }) }}</span>
        </button>
    }
    .into_any()
}

#[component]
fn DeclarationInspector(
    session: Session,
    kind: DeclarationKind,
    active: RwSignal<EditorTab>,
    role_input: RwSignal<String>,
    original: RwSignal<Option<String>>,
    draft: RwSignal<Value>,
    baseline: RwSignal<Value>,
    editing: RwSignal<bool>,
    error: RwSignal<String>,
    load: Callback<String>,
) -> AnyView {
    view! {
        <aside class="fp-resource-inspector" aria-label=format!("{} properties", kind.label())>
            <Show when=move || editing.get() fallback=move || view! {
                <div class="fp-empty-inspector"><span>{kind.symbol()}</span><h3>{format!("Select a {}", kind.singular())}</h3><p>"Open a card to edit its settings and see where it is used."</p></div>
            }>
                <header class="fp-resource-form-heading"><span class="fp-eyebrow">{move || if original.get().is_some() { "EDIT DECLARATION" } else { "NEW DECLARATION" }}</span><h2>{kind.label()}</h2></header>
                <JsonField draft=draft pointer=format!("/{}", kind.id_field()) label=if kind == DeclarationKind::Escalation { "Topic ID" } else { "Identifier" }/>
                <Show when=move || kind == DeclarationKind::Form>
                    <label class="fp-field">"Roles"<input aria-label="Roles" placeholder="reviewer, manager" prop:value=move || role_input.get()
                        on:input=move |event| { let value = event_target_value(&event); role_input.set(value.clone()); draft.update(|v| v["roles"] = json!(value.split(',').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>())); }/></label>
                    <p class="fp-fine-print">"Comma-separated role IDs. The application provides the form fields and rendering."</p>
                </Show>
                <Show when=move || kind == DeclarationKind::Signal><p class="fp-fine-print">"Use this identifier in a Wait or User task, or emit it from an escalation action."</p></Show>
                <Show when=move || kind == DeclarationKind::Escalation><EscalationActions draft=draft session=session/></Show>
                <p class="fp-resource-error" role="alert">{move || error.get()}</p>
                <div class="fp-resource-save">
                    <button type="button" class="fp-primary" on:click=move |_| {
                        let mut result = Err(String::new());
                        session.state.update(|state| result = state.edit(|doc| {
                            if let Some(id) = original.get_untracked() {
                                if doc.declaration(kind, &id).as_ref() != Some(&baseline.get_untracked()) { return Err("This declaration changed. Reopen the card to load its latest settings.".into()); }
                            }
                            doc.save_declaration(kind, original.get_untracked().as_deref(), draft.get_untracked())
                        }));
                        match result {
                            Ok(id) => { load.run(id); session.message.set(format!("{} saved", kind.singular())); }
                            Err(message) => error.set(message),
                        }
                    }>"Apply changes"</button>
                    <button type="button" on:click=move |_| { editing.set(false); original.set(None); error.set(String::new()); }>"Cancel"</button>
                </div>
                <Show when=move || original.get().is_some()>
                    <section class="fp-resource-references"><h3>"Used by"</h3>
                        {move || session.state.with(|s| {
                            let uses = original.get().map(|id| s.document.declaration_uses(kind, &id)).unwrap_or_default();
                            if uses.is_empty() { view! { <p>"Not referenced by this process yet."</p> }.into_any() }
                            else { uses.into_iter().map(|usage| {
                                let node = usage.node;
                                let escalation = usage.escalation;
                                let title = if escalation.is_some() { "Show escalation" } else { "Show block in process" };
                                view! { <button type="button" class="fp-reference-link" title=title on:click=move |_| {
                                    if let Some(id) = &node { active.set(EditorTab::Process); session.select(id.clone()); session.reveal_node.set(Some(id.clone())); }
                                    else if let Some(id) = &escalation { active.set(EditorTab::Escalations); session.reveal_declaration.set(Some((DeclarationKind::Escalation, id.clone()))); }
                                }>{usage.label}<span>"↗"</span></button> }
                            }).collect_view().into_any() }
                        })}
                    </section>
                    <button type="button" class="fp-danger" on:click=move |_| {
                        if let Some(id) = original.get_untracked() {
                            let mut result = Ok(());
                            session.state.update(|s| result = s.edit(|doc| doc.remove_declaration(kind, &id)));
                            match result { Ok(()) => { editing.set(false); original.set(None); error.set(String::new()); session.message.set(format!("{} deleted", kind.singular())); }, Err(message) => error.set(message) }
                        }
                    }>{format!("Delete {}", kind.singular())}</button>
                </Show>
            </Show>
        </aside>
    }
    .into_any()
}

#[component]
fn JsonField(draft: RwSignal<Value>, pointer: String, label: &'static str) -> AnyView {
    let pointer = StoredValue::new(pointer);
    view! { <label class="fp-field">{label}<input aria-label=label prop:value=move || draft.with(|v| v.pointer(&pointer.get_value()).and_then(Value::as_str).unwrap_or_default().to_string()) on:input=move |event| {
        let value = event_target_value(&event);
        draft.update(|v| { if let Some(field) = v.pointer_mut(&pointer.get_value()) { *field = json!(value); } });
    }/></label> }
    .into_any()
}

#[component]
fn EscalationActions(draft: RwSignal<Value>, session: Session) -> AnyView {
    view! {
        <section class="fp-escalation-actions"><h3>"Operator actions"</h3>
            <For each={move || (0..draft.with(|v| v["actions"].as_array().map_or(0, Vec::len))).collect::<Vec<_>>()} key=|index| *index children=move |index| view! {
                <fieldset class="fp-escalation-action"><legend>{format!("Action {}", index + 1)}</legend>
                    <JsonField draft=draft pointer=format!("/actions/{index}/id") label="Action ID"/>
                    <JsonField draft=draft pointer=format!("/actions/{index}/operator_action") label="Operator action"/>
                    <JsonField draft=draft pointer=format!("/actions/{index}/label") label="Button label"/>
                    <JsonField draft=draft pointer=format!("/actions/{index}/hint") label="Help text"/>
                    <label class="fp-field">"Button style"<select prop:value=move || draft.with(|v| v["actions"][index]["kind"].as_str().unwrap_or("Secondary").to_owned()) on:change=move |event| draft.update(|v| v["actions"][index]["kind"] = json!(event_target_value(&event)))>
                        <option value="Primary">"Primary"</option><option value="Secondary">"Secondary"</option><option value="Dangerous">"Dangerous"</option>
                    </select></label>
                    <label class="fp-field">"Emit signal"<select prop:value=move || draft.with(|v| v["actions"][index]["emit_signal"].as_str().unwrap_or_default().to_owned()) on:change=move |event| {
                        let value = event_target_value(&event); draft.update(|v| v["actions"][index]["emit_signal"] = if value.is_empty() { Value::Null } else { json!(value) });
                    }>
                        <option value="">"No signal"</option>
                        {move || session.state.with(|s| s.document.definition.signals.iter().map(|signal| { let id = signal.name().to_string(); view! { <option value=id.clone()>{id.clone()}</option> } }).collect_view())}
                    </select></label>
                    <button type="button" class="fp-remove-action" aria-label=format!("Remove action {}", index + 1) on:click=move |_| draft.update(|v| { if let Some(actions) = v["actions"].as_array_mut() { if index < actions.len() { actions.remove(index); } } })>"Remove action"</button>
                </fieldset>
            }/>
            <button type="button" class="fp-wide" on:click=move |_| draft.update(|v| {
                if let Some(actions) = v["actions"].as_array_mut() {
                    let id = (1..).map(|n| format!("action_{n}")).find(|id| !actions.iter().any(|a| a["id"].as_str() == Some(id.as_str()))).unwrap();
                    actions.push(json!({"id":id,"operator_action":id,"label":"New action","hint":"","kind":"Secondary","emit_signal":null}));
                }
            })>"+ Add action"</button>
        </section>
    }
    .into_any()
}

fn roles_text(value: &Value) -> String {
    value["roles"]
        .as_array()
        .map(|roles| {
            roles
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}
