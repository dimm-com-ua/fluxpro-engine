use super::{Session, declarations::EditorTab, fields::*};
use crate::settings::process_settings;
use leptos::prelude::*;
use serde_json::{Value, json};

#[component]
pub(super) fn ProcessSettings(session: Session) -> impl IntoView {
    let current = session
        .state
        .with_untracked(|s| process_settings(&s.document));
    let draft = RwSignal::new(current.clone());
    let baseline = RwSignal::new(current);
    let field = Field::root(draft);
    Effect::new(move |_| {
        let current = session.state.with(|s| process_settings(&s.document));
        if draft.get_untracked() == baseline.get_untracked() {
            draft.set(current.clone());
            baseline.set(current);
        }
    });
    view! { <div class="fp-process-settings"><div class="fp-settings-heading"><div><span class="fp-eyebrow">"PROCESS DEFINITION"</span><h2>"General settings"</h2><p>"Identity, lifecycle and business stages for the entire process."</p></div>
        <DraftActions session=session draft=draft baseline=baseline node=None/>
    </div><div class="fp-settings-grid">
        <section class="fp-setting-card"><h3>"Identity and version"</h3>
            <TextField field=field.child("key") label="Process key"/>
            <TextField field=field.child("name") label="Process name"/>
            <TextField field=field.child("version") label="Version"/>
            <Select field=field.child("status") label="Status" options=fixed(&["draft","active","deprecated"]) optional=false/>
            <DateField field=field.child("effective_from") label="Effective from (UTC)"/>
            <DateField field=field.child("deprecated_at") label="Deprecated at (UTC)"/>
        </section>
        <section class="fp-setting-card"><h3>"Metadata"</h3>
            <TextField field=field.child("metadata").child("owner") label="Owner / team" optional=true/>
            <Notes field=field.child("metadata").child("comment") label="Description / version notes"/>
            <Toggle field=field.child("metadata").child("sla") label="Service-level target" initial=json!("PT30M")/>
            <Show when=move || !field.child("metadata").child("sla").get().is_null()><DurationField field=field.child("metadata").child("sla") label="Target duration"/><p>"Descriptive target; does not schedule a timeout."</p></Show>
        </section>
        <section class="fp-setting-card"><h3>"Lifecycle handlers"</h3><p>"Choose an existing handler or enter a registry key from your application."</p>
            <Toggle field=field.child("special_handlers") label="Configure lifecycle hooks" initial=json!({})/>
            <Show when=move || !field.child("special_handlers").get().is_null()>
                {[("on_stage_change","Stage changed"),("on_show_form","Show form"),("on_hide_form","Hide form"),("on_process_complete","Process completed")].into_iter().map(|(key,label)| view!{<Hook field=field.child("special_handlers").child(key) label=label session=session/>}).collect_view()}
            </Show>
        </section>
        <section class="fp-setting-card fp-stages-card"><h3>"Business stages"</h3><p>"Mark exactly one stage as initial. Update block assignments before renaming or removing a used stage."</p><Stages field=field.child("stages")/></section>
    </div></div> }
}
#[component]
fn Hook(field: Field, label: &'static str, session: Session) -> impl IntoView {
    let expanded = Memo::new(move |_| field.get().is_object());
    view! { <Toggle field=field label=label initial=json!("")/><Show when=move || !field.get().is_null()>{move || {
        let payload=if expanded.get(){field.child("handler")}else{field};
        view!{<HandlerField field=payload label=label session=session/>}
    }}</Show> }
}
#[component]
fn Stages(field: Field) -> impl IntoView {
    view! { <For each={move ||(0..field.get().as_array().map_or(0,Vec::len)).collect::<Vec<_>>()} key=|i|*i children=move |i| {
        let row=field.child(i); let compact=Memo::new(move |_|row.get().is_string());
        view!{<div class="fp-stage-row">
            {move || if compact.get(){view!{<TextField field=row label="Stage ID"/><button type="button" on:click=move |_|row.set(json!({"id":row.text(),"name":row.text()}))>"Add name and markers"</button>}.into_any()}else{view!{
                <TextField field=row.child("id") label="Stage ID"/><TextField field=row.child("name") label="Stage name"/>
                <Check field=row.child("is_initial") label="Initial stage"/><Check field=row.child("is_final") label="Final stage"/>
            }.into_any()}}
            <button type="button" on:click=move |_| {let mut values=field.get();if let Some(a)=values.as_array_mut(){a.remove(i);}field.set(values);}>"Remove stage"</button>
        </div>}
    }/><button type="button" on:click=move |_| {let mut values=field.get();if let Some(a)=values.as_array_mut(){let mut i=a.len()+1;while a.iter().any(|v|v.as_str().or_else(||v["id"].as_str())==Some(format!("stage_{i}").as_str())){i+=1;}a.push(json!({"id":format!("stage_{i}"),"name":"New stage"}));}field.set(values);}>"Add stage"</button> }
}

#[component]
fn DraftActions(
    session: Session,
    draft: RwSignal<Value>,
    baseline: RwSignal<Value>,
    node: Option<String>,
) -> impl IntoView {
    let pending_key = StoredValue::new(
        node.as_ref()
            .map(|id| format!("node:{id}"))
            .unwrap_or_else(|| "process".into()),
    );
    Effect::new(move |_| {
        session.track_pending(pending_key.get_value(), draft.get() != baseline.get())
    });
    on_cleanup(move || session.track_pending(pending_key.get_value(), false));
    let node = StoredValue::new(node);
    let read = move || {
        session.state.with_untracked(|s| match node.get_value() {
            Some(id) => s
                .document
                .node(&id)
                .and_then(|n| serde_json::to_value(n).ok())
                .unwrap_or(Value::Null),
            None => process_settings(&s.document),
        })
    };
    view! {<div class="fp-settings-actions">
        <span class="fp-fine-print">{move ||if draft.get()!=baseline.get(){"Unapplied changes"}else{"Up to date"}}</span>
        <button type="button" on:click=move |_|{let current=read();draft.set(current.clone());baseline.set(current);session.message.set(String::new());}>"Reload settings"</button>
        <button type="button" class="fp-primary" on:click=move |_|{
            let mut renamed=None;
            session.edit(|doc| match node.get_value(){Some(id)=>{renamed=Some(doc.save_node_settings(&id,&baseline.get_untracked(),draft.get_untracked())?);Ok(())},None=>doc.save_process_settings(&baseline.get_untracked(),draft.get_untracked())});
            if session.message.get_untracked().is_empty(){if let Some(id)=renamed{node.set_value(Some(id.clone()));session.selected.set(Some(id));}let current=read();draft.set(current.clone());baseline.set(current);}
        }>"Apply settings"</button>
    </div>}
}

#[component]
pub(super) fn ProcessInspector(session: Session, active: RwSignal<EditorTab>) -> impl IntoView {
    view! {<aside class="fp-inspector" aria-label="Process properties"><div class="fp-panel-heading"><span class="fp-eyebrow">"MAKE IT WORK"</span><h2>"Properties"</h2></div>
        <Show when=move ||session.selected.get().is_none()><div class="fp-empty-inspector"><h3>"Select a block"</h3><p>"Configure its signals, stages, timeouts and paths here."</p></div></Show>
        <button type="button" class="fp-wide" on:click=move |_|active.set(EditorTab::Settings)>"General process settings"</button>
        <For each={move ||session.selected.get().into_iter().collect::<Vec<_>>()} key=|id|id.clone() children=move |id|view!{<NodeSettings session=session id=id/>}/>
        <details class="fp-diagnostics"><summary>{move ||session.state.with(|s|format!("Review · {}",s.document.diagnostics().len()))}</summary><ul>{move ||session.state.with(|s|s.document.diagnostics().into_iter().map(|m|view!{<li>{m}</li>}).collect_view())}</ul><p>"Your application validates registered handlers and Rhai expressions."</p></details>
    </aside>}
}
#[component]
fn NodeSettings(session: Session, id: String) -> impl IntoView {
    let id = StoredValue::new(id);
    let read = move || {
        session.state.with(|s| {
            s.document
                .node(&id.get_value())
                .and_then(|n| serde_json::to_value(n).ok())
                .unwrap_or(Value::Null)
        })
    };
    let current = untrack(read);
    let kind = current["type"].as_str().unwrap_or("").to_owned();
    let draft = RwSignal::new(current.clone());
    let baseline = RwSignal::new(current);
    let field = Field::root(draft);
    Effect::new(move |_| {
        let current = read();
        if draft.get_untracked() == baseline.get_untracked() {
            draft.set(current.clone());
            baseline.set(current);
        }
    });
    let kind = StoredValue::new(kind);
    view! {<div class="fp-node-settings"><div class="fp-selection-title"><span class="fp-draft">{kind.get_value()}</span><h3>{id.get_value()}</h3></div>
        <TextField field=field.child("id") label="Block ID"/>
        <Show when=move ||kind.get_value()=="ServiceTask"><HandlerField field=field.child("handler") label="Service handler" session=session/>
            <details class="fp-settings-section" open><summary>"Arguments"</summary><Toggle field=field.child("args") label="Pass arguments" initial=json!({})/><Show when=move ||!field.child("args").get().is_null()><ObjectEditor field=field.child("args") session=session/></Show></details>
            <details class="fp-settings-section" open><summary>"Retries"</summary><Toggle field=field.child("retries") label="Retry on failure" initial=json!({"max":3,"backoff":"PT5S"})/><Show when=move ||!field.child("retries").get().is_null()><TextField field=field.child("retries").child("max") label="Maximum retries" kind="number"/><DurationField field=field.child("retries").child("backoff") label="Delay between attempts"/></Show></details>
        </Show>
        <Show when=move ||kind.get_value()=="UserTask"><Select field=field.child("form") label="Form" options=catalog(session,"forms") optional=false/></Show>
        <Show when=move ||matches!(kind.get_value().as_str(),"UserTask"|"Wait")>
            <details class="fp-settings-section" open><summary>"Signals"</summary>
                <Show when=move ||kind.get_value()=="UserTask"><Toggle field=field.child("wait_for") label="Wait for signals" initial=json!({"signals":[]})/></Show>
                <Show when=move ||!field.child("wait_for").get().is_null()><SignalPicker field=field.child("wait_for") session=session/></Show>
            </details>
            <details class="fp-settings-section" open><summary>"Timeout"</summary>
                <Toggle field=field.child("timeout") label="Enable timeout" initial=json!({"after":"PT1H","at":null,"on_timeout":""})/>
                <Show when=move ||!field.child("timeout").get().is_null()>
                    <Toggle field=field.child("timeout").child("after") label="After a duration" initial=json!("PT1H")/>
                    <Show when=move ||!field.child("timeout").child("after").get().is_null()><DurationField field=field.child("timeout").child("after") label="Wait duration"/></Show>
                    <Toggle field=field.child("timeout").child("at") label="At a specific date" initial=json!("2026-12-31T12:00:00Z")/>
                    <Show when=move ||!field.child("timeout").child("at").get().is_null()><DateField field=field.child("timeout").child("at") label="Deadline (UTC)"/><p>"An absolute date takes priority over the duration."</p></Show>
                    <Select field=field.child("timeout").child("on_timeout") label="On timeout, go to" options=catalog(session,"nodes") optional=false/>
                </Show>
            </details>
        </Show>
        <details class="fp-settings-section" open><summary>"Business stage"</summary><StageAssignment field=field.child("set_stage") session=session/></details>
        <Show when=move ||kind.get_value()!="End"><details class="fp-settings-section" open><summary>"Paths"</summary><Routes field=field session=session kind=kind.get_value()/></details></Show>
        <Show when=move ||matches!(kind.get_value().as_str(),"ServiceTask"|"UserTask")><details class="fp-settings-section" open><summary>"Error handling"</summary>
            <Toggle field=field.child("on_error") label="Configure error routes" initial=json!({"next":null,"compensate":null})/>
            <Show when=move ||!field.child("on_error").get().is_null()>
                <Select field=field.child("on_error").child("next") label="On error, go to" options=catalog(session,"nodes")/>
                <Select field=field.child("on_error").child("compensate") label="Compensation block" options=catalog(session,"nodes")/>
                <p>"Compensation and user-task error routes are reserved by the engine; only service-task error transitions execute."</p>
            </Show>
        </details></Show>
        <DraftActions session=session draft=draft baseline=baseline node=Some(id.get_value())/>
        <button type="button" class="fp-danger" on:click=move |_|{session.edit(|doc|doc.remove_node(&id.get_value()));if !session.state.with_untracked(|s|s.document.node(&id.get_value()).is_some()){session.selected.set(None);}}>"Delete block"</button>
    </div>}
}
#[component]
fn StageAssignment(field: Field, session: Session) -> impl IntoView {
    view! {<Toggle field=field label="Assign a stage on entry" initial=json!("")/><Show when=move ||!field.get().is_null()>
        {move ||{let stage=if field.get().is_object(){field.child("stage")}else{field};view!{<Select field=stage label="Stage" options=catalog(session,"stages") optional=false/>}}}
        <label class="fp-check"><input type="checkbox" prop:checked=move ||field.get().is_object() on:change=move |ev|{
            let current=field.get();field.set(if event_target_checked(&ev){json!({"stage":current,"reason":""})}else{current["stage"].clone()});
        }/>"Include stage reason"</label><Show when=move ||field.get().is_object()><Notes field=field.child("reason") label="Stage reason"/></Show>
    </Show>}
}
#[component]
fn SignalPicker(field: Field, session: Session) -> impl IntoView {
    let selected = move || {
        let v = field.get();
        if let Some(s) = v["signal"].as_str() {
            vec![s.to_owned()]
        } else {
            v["signals"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        }
    };
    let available = catalog(session, "signals");
    let options = Memo::new(move |_| {
        let mut values = available.get();
        for s in selected() {
            if !values.contains(&s) {
                values.push(s);
            }
        }
        values
    });
    view! {<p>"Continue when any selected signal arrives. Manage available signals in the Signals tab."</p><div class="fp-signal-picker">
    <For each=move ||options.get() key=|id|id.clone() children=move |id|{let id=StoredValue::new(id);view!{
        <label class="fp-check"><input type="checkbox" prop:checked=move ||selected().contains(&id.get_value()) on:change=move |ev|{let mut values=selected();if event_target_checked(&ev){if !values.contains(&id.get_value()){values.push(id.get_value());}}else{values.retain(|s|*s!=id.get_value());}field.set(json!({"signals":values}));}/>{id.get_value()}</label>
    }}/></div>}
}
#[component]
fn Routes(field: Field, session: Session, kind: String) -> impl IntoView {
    let kind = StoredValue::new(kind);
    let next = field.child("next");
    view! {<Show when=move ||kind.get_value()=="Gateway"><Select field=field.child("gateway") label="Gateway rule" options=fixed(&["XOR"]) optional=false/></Show>
        {move ||{let target=if next.get().is_object(){next.child("default")}else{next};view!{<Select field=target label="Default destination" options=catalog(session,"nodes") optional=kind.get_value()!="Start"/>}}}
        <Show when=move ||kind.get_value()!="Start"><p>"Conditions are Rhai expressions. The first matching path wins; otherwise use the default destination."</p>
            <Branches field=field session=session gateway=kind.get_value()=="Gateway"/>
        </Show>
    }
}
#[component]
fn Branches(field: Field, session: Session, gateway: bool) -> impl IntoView {
    let rows = if gateway {
        field.child("branches")
    } else {
        field.child("next").child("branches")
    };
    view! {<For each={move ||(0..rows.get().as_array().map_or(0,Vec::len)).collect::<Vec<_>>()} key=|i|*i children=move |i|view!{
        <div class="fp-setting-card"><Notes field=rows.child(i).child("when") label="Condition"/><Select field=rows.child(i).child("next") label="Then go to" options=catalog(session,"nodes") optional=false/>
            <div class="fp-inline"><button type="button" aria-label="Move condition up" disabled=i==0 on:click=move |_|{let mut v=rows.get();if let Some(a)=v.as_array_mut(){a.swap(i,i-1);}rows.set(v);}>"↑"</button>
            <button type="button" aria-label="Move condition down" disabled={move ||i+1>=rows.get().as_array().map_or(0,Vec::len)} on:click=move |_|{let mut v=rows.get();if let Some(a)=v.as_array_mut(){a.swap(i,i+1);}rows.set(v);}>"↓"</button>
            <button type="button" on:click=move |_|{let mut v=rows.get();if let Some(a)=v.as_array_mut(){a.remove(i);}rows.set(v);}>"Remove condition"</button></div>
        </div>
    }/><button type="button" on:click=move |_|{
        if !gateway && !field.child("next").get().is_object(){let default=field.child("next").get();field.child("next").set(json!({"default":default,"branches":[]}));}
        let mut v=rows.get();if !v.is_array(){v=json!([]);}v.as_array_mut().unwrap().push(json!({"when":"","next":""}));rows.set(v);
    }>"Add condition"</button>}
}

#[cfg(all(test, feature = "ssr"))]
mod render_tests {
    use super::*;
    use crate::{EditorDocument, document::EditorState};

    #[test]
    fn every_node_variant_renders_structured_fields_without_yaml() {
        for (id, labels) in [
            ("start", vec!["Default destination", "Stage"]),
            (
                "prepare",
                vec![
                    "Service handler",
                    "Value type",
                    "Maximum retries",
                    "Delay between attempts",
                    "On error, go to",
                    "Compensation block",
                ],
            ),
            (
                "review",
                vec![
                    "Form",
                    "Wait for signals",
                    "Wait duration",
                    "On timeout, go to",
                    "Include stage reason",
                ],
            ),
            (
                "decision",
                vec![
                    "Gateway rule",
                    "Condition",
                    "Then go to",
                    "Move condition up",
                    "Move condition down",
                ],
            ),
            (
                "await_confirmation",
                vec!["confirmed", "Wait duration", "Default destination"],
            ),
            ("completed_end", vec!["Business stage", "Block ID"]),
        ] {
            let owner = Owner::new();
            let html = owner.with(|| {
                let document = EditorDocument::from_yaml(include_str!(
                    "../../../../examples/definitions/approval.yaml"
                ))
                .unwrap();
                let session = Session {
                    state: RwSignal::new(EditorState::new(document)),
                    pending_forms: RwSignal::new(Default::default()),
                    require_applied_changes: false,
                    selected: RwSignal::new(Some(id.to_owned())),
                    connecting: RwSignal::new(None),
                    message: RwSignal::new(String::new()),
                    reset_view: RwSignal::new(0),
                    reveal_node: RwSignal::new(None),
                    reveal_declaration: RwSignal::new(None),
                };
                view! { <NodeSettings session=session id=id.to_owned()/> }.to_html()
            });
            for label in labels {
                assert!(html.contains(label), "{id} missing {label}");
            }
            assert!(!html.contains("YAML"));
            assert!(!html.contains("collect::"));
            assert!(!html.contains("kind.get_value"));
        }
    }
}
