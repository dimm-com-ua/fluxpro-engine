use super::MonitorSession;
use crate::monitor::*;
use leptos::prelude::*;
use serde_json::Value;

#[component]
pub(super) fn InstanceDetails(session: MonitorSession, instance: MonitorInstance) -> impl IntoView {
    let id = StoredValue::new(instance.uuid.clone());
    let initial = StoredValue::new(instance);
    let tab = RwSignal::new("Overview");
    let detail = Memo::new(move |_| {
        session.accepted.with(|s| {
            s.as_ref()?
                .details
                .iter()
                .find(|d| d.instance.uuid == id.get_value())
                .cloned()
        })
    });
    let summary = Memo::new(move |_| {
        detail
            .get()
            .map(|d| d.instance)
            .unwrap_or_else(|| initial.get_value())
    });
    view! {<section class="fm-detail"><div class="fm-section-heading"><span class="fp-eyebrow">"INSTANCE"</span><h2>{move ||summary.get().process_id}</h2><code class="fm-token">{move ||summary.get().token}</code></div>
        <div class="fm-detail-state"><span class="fm-state">{move ||summary.get().state}</span><span>"Block: "{move ||summary.get().current_node_id.unwrap_or_else(||"Not assigned".into())}</span><span>"Stage: "{move ||summary.get().current_stage_name.or(summary.get().current_stage_id).unwrap_or_else(||"Not assigned".into())}</span></div>
        <nav class="fm-detail-nav" aria-label="Instance detail sections">{["Overview","History","Stages","Events","Context"].into_iter().map(|label|view!{<button type="button" class:fm-active=move ||tab.get()==label aria-pressed=move ||tab.get()==label on:click=move |_|tab.set(label)>{label}</button>}).collect_view()}</nav>
        <Show when=move ||detail.get().is_none()><p role="status" class="fm-empty">"Loading instance state and histories…"</p></Show>
        <Show when=move ||detail.get().is_some_and(|d|d.error.is_some())><p class="fm-error" role="alert">{move ||detail.get().and_then(|d|d.error).unwrap_or_default()}</p></Show>
        <div class="fm-detail-content">
            <Show when=move ||tab.get()=="Overview">
                <dl class="fm-facts"><dt>"Instance UUID"</dt><dd>{id.get_value()}</dd><dt>"Business key"</dt><dd>{move ||summary.get().process_id}</dd><dt>"Runtime token"</dt><dd>{move ||summary.get().token}</dd><dt>"Created"</dt><dd>{move ||summary.get().created_at}</dd><dt>"Process version"</dt><dd>{move ||format!("{} / {}",summary.get().process_key,summary.get().process_version)}</dd><dt>"Stage reason"</dt><dd>{move ||summary.get().current_stage_reason.unwrap_or_else(||"—".into())}</dd></dl>
                <h3>"Active issues"</h3><Show when=move ||summary.get().issues.is_empty()><p>"No active issues reported."</p></Show>
                <For each=move ||summary.get().issues key=|issue|serde_json::to_string(issue).unwrap_or_default() children=move |issue|view!{<article class=if issue.kind=="error"{"fm-incident fm-incident-error"}else{"fm-incident fm-incident-escalation"}><strong>{format!("{} · {}",issue.kind,issue.topic.unwrap_or_default())}</strong><p>{issue.message}</p><small>{issue.created_at.unwrap_or_default()}</small><JsonTree label="Issue details" value=issue.details/></article>}/>
                <details><summary>"Runtime metadata"</summary><JsonTree label="Metadata" value=Signal::derive(move ||serde_json::to_value(summary.get().metadata).unwrap_or_default())/></details>
                <Show when=move ||detail.get().is_some_and(|d|d.current_node.is_some())><JsonTree label="Current node configuration" value=Signal::derive(move ||detail.get().and_then(|d|d.current_node).unwrap_or_default())/></Show>
            </Show>
            <Show when=move ||tab.get()=="History"><h3>"Execution history"</h3><p>"Newest first · Node visits, transitions, attempts and errors."</p>
                <Show when=move ||detail.get().is_some_and(|d|d.logs.items.is_empty())><p class="fm-empty">"No execution history recorded."</p></Show>
                <For each=move ||detail.get().map(|d|d.logs.items).unwrap_or_default() key=|entry|entry.uuid.clone() children=move |entry|view!{<article class="fm-history-entry" class:fm-history-error=matches!(entry.level.as_str(),"error"|"critical")><div><span class="fm-state">{entry.level.clone()}</span><time>{entry.created_at}</time></div><strong>{entry.event_type}</strong><p>{entry.message}</p><small>{entry.node_id.unwrap_or_default()}</small><JsonTree label="Event diagnostics" value=serde_json::to_value(entry.data).unwrap_or_default()/></article>}/>
                <button type="button" class="fp-wide" disabled=move ||detail.get().is_none_or(|d|d.logs.items.len() as u64>=d.logs.total) on:click=move |_|session.change(|r|{if let Some(i)=r.instances.iter_mut().find(|i|i.uuid==id.get_value()){i.log_limit=i.log_limit.saturating_add(100);}})>{move ||detail.get().map(|d|format!("Load more history · {} of {}",d.logs.items.len(),d.logs.total)).unwrap_or("Load history".into())}</button>
            </Show>
            <Show when=move ||tab.get()=="Stages"><h3>"Stage history"</h3><p>"Persisted business-stage changes and the context at each transition."</p><Show when=move ||detail.get().is_some_and(|d|d.stage_history.is_empty())><p class="fm-empty">"No stage changes recorded."</p></Show>
                <For each=move ||detail.get().map(|d|d.stage_history).unwrap_or_default() key=|entry|entry.uuid.clone() children=move |entry|view!{<article class="fm-history-entry"><time>{entry.created_at}</time><strong>{entry.stage_name.or(entry.stage_id).unwrap_or_else(||"Removed stage".into())}</strong><p>{entry.reason.unwrap_or_default()}</p><JsonTree label="Context at this stage" value=entry.context/></article>}/>
            </Show>
            <Show when=move ||tab.get()=="Events"><h3>"Signal events"</h3><p>"Newest admitted signals first. Admission does not imply successful delivery."</p><Show when=move ||detail.get().is_some_and(|d|d.signals.items.is_empty())><p class="fm-empty">"No signal events recorded."</p></Show>
                <For each=move ||detail.get().map(|d|d.signals.items).unwrap_or_default() key=|entry|entry.uuid.clone() children=move |entry|view!{<article class="fm-history-entry"><time>{entry.created_at}</time><strong>{entry.signal_name}</strong><small>{entry.uuid}</small><JsonTree label="Signal payload" value=entry.payload/></article>}/>
                <button type="button" class="fp-wide" disabled=move ||detail.get().is_none_or(|d|d.signals.items.len() as u64>=d.signals.total) on:click=move |_|session.change(|r|{if let Some(i)=r.instances.iter_mut().find(|i|i.uuid==id.get_value()){i.signal_limit=i.signal_limit.saturating_add(100);}})>{move ||detail.get().map(|d|format!("Load more events · {} of {}",d.signals.items.len(),d.signals.total)).unwrap_or("Load events".into())}</button>
            </Show>
            <Show when=move ||tab.get()=="Context"><h3>"Current context"</h3><p>"Typed values supplied by the runtime."</p><JsonTree label="Context" value=Signal::derive(move ||detail.get().map(|d|d.context).unwrap_or_default())/>
                <h3>"Scoped variables"</h3><JsonTree label="Variables by scope" value=Signal::derive(move ||Value::Array(detail.get().map(|d|d.context_variables).unwrap_or_default()))/>
            </Show>
        </div>
    </section>}
}
#[component]
fn JsonTree(#[prop(into)] label: String, #[prop(into)] value: Signal<Value>) -> impl IntoView {
    let label = StoredValue::new(label);
    let kind = Memo::new(move |_| match value.get() {
        Value::Object(_) => 0,
        Value::Array(_) => 1,
        _ => 2,
    });
    view! { {move || match kind.get() {
        0 => view!{<details class="fm-json"><summary>{move ||format!("{} · {} fields",label.get_value(),value.get().as_object().map_or(0,|v|v.len()))}</summary>
            <For each={move ||value.get().as_object().map(|v|v.keys().cloned().collect::<Vec<_>>()).unwrap_or_default()} key=|key|key.clone() children=move |key|{
                let name=key.clone();view!{<JsonTree label=key value=Signal::derive(move ||value.get().get(&name).cloned().unwrap_or_default())/>}
            }/>
        </details>}.into_any(),
        1 => view!{<details class="fm-json"><summary>{move ||format!("{} · {} items",label.get_value(),value.get().as_array().map_or(0,Vec::len))}</summary>
            <For each={move ||(0..value.get().as_array().map_or(0,Vec::len)).collect::<Vec<_>>()} key=|i|*i children=move |i|view!{<JsonTree label=i.to_string() value=Signal::derive(move ||value.get().get(i).cloned().unwrap_or_default())/>}/>
        </details>}.into_any(),
        _=>view!{<div class="fm-json-value"><strong>{label.get_value()}</strong><code>{move ||match value.get(){Value::String(s)=>s,v=>v.to_string()}}</code></div>}.into_any(),
    } } }
}
