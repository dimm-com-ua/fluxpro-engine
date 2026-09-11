use super::Session;
use crate::editor::settings::{duration_parts, duration_value};
use leptos::prelude::*;
use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub(super) struct Field {
    pub draft: RwSignal<Value>,
    path: StoredValue<String>,
}
impl Field {
    pub fn root(draft: RwSignal<Value>) -> Self {
        Self {
            draft,
            path: StoredValue::new(String::new()),
        }
    }
    pub fn child(self, key: impl ToString) -> Self {
        Self {
            draft: self.draft,
            path: StoredValue::new(format!(
                "{}/{}",
                self.path.get_value(),
                key.to_string().replace('~', "~0").replace('/', "~1")
            )),
        }
    }
    pub fn get(self) -> Value {
        self.draft.with(|v| {
            v.pointer(&self.path.get_value())
                .cloned()
                .unwrap_or(Value::Null)
        })
    }
    pub fn text(self) -> String {
        match self.get() {
            Value::String(v) => v,
            Value::Null => String::new(),
            v => v.to_string(),
        }
    }
    pub fn set(self, value: Value) {
        self.draft.update(|v| {
            set_path(v, &self.path.get_value(), value);
        });
    }
}
fn set_path(root: &mut Value, path: &str, value: Value) {
    if path.is_empty() {
        *root = value;
        return;
    }
    let (parent, key) = path.rsplit_once('/').unwrap();
    let key = key.replace("~1", "/").replace("~0", "~");
    if root.pointer(parent).is_none_or(Value::is_null) {
        set_path(root, parent, json!({}));
    }
    if let Some(target) = root.pointer_mut(parent) {
        match target {
            Value::Array(items) => {
                if let Ok(i) = key.parse::<usize>() {
                    if let Some(item) = items.get_mut(i) {
                        *item = value;
                    }
                }
            }
            _ => target[&key] = value,
        }
    }
}

#[component]
pub(super) fn TextField(
    field: Field,
    label: &'static str,
    #[prop(default = "text")] kind: &'static str,
    #[prop(default = false)] optional: bool,
) -> AnyView {
    view! { <label class="fp-field">{label}<input type=kind step="any" prop:value=move || field.text() on:input=move |ev| {
        let value = event_target_value(&ev);
        field.set(if optional && value.is_empty() { Value::Null } else if kind == "number" { serde_json::from_str::<Value>(&value).ok().filter(Value::is_number).unwrap_or(json!(value)) } else { json!(value) });
    }/></label> }
    .into_any()
}
#[component]
pub(super) fn Notes(field: Field, label: &'static str) -> AnyView {
    view! { <label class="fp-field">{label}<textarea rows="3" prop:value=move || field.text() on:input=move |ev| field.set(json!(event_target_value(&ev)))></textarea></label> }.into_any()
}
#[component]
pub(super) fn Check(field: Field, label: &'static str) -> AnyView {
    view! { <label class="fp-check"><input type="checkbox" prop:checked=move || field.get().as_bool().unwrap_or(false) on:change=move |ev| field.set(json!(event_target_checked(&ev)))/>{label}</label> }.into_any()
}
#[component]
pub(super) fn Select(
    field: Field,
    label: &'static str,
    options: Signal<Vec<String>>,
    #[prop(default = true)] optional: bool,
) -> AnyView {
    // Retain unresolved imported references so opening a form never drops them.
    let choices = Memo::new(move |_| {
        let mut values = options.get();
        let current = field.text();
        if !current.is_empty() && !values.contains(&current) {
            values.insert(0, current);
        }
        values
    });
    view! { <label class="fp-field">{label}<select prop:value=move || field.text() on:change=move |ev| { let value=event_target_value(&ev); field.set(if value.is_empty() {Value::Null} else {json!(value)}); }>
        <option value="">{if optional { "None" } else { "Choose…" }}</option>
        {move || choices.get().into_iter().map(|id| view! { <option value=id.clone()>{id.clone()}</option> }).collect_view()}
    </select></label> }
    .into_any()
}
pub(super) fn fixed(values: &[&str]) -> Signal<Vec<String>> {
    Signal::stored(values.iter().map(|s| s.to_string()).collect())
}
pub(super) fn catalog(session: Session, kind: &'static str) -> Signal<Vec<String>> {
    Signal::derive(move || {
        session.state.with(|s| match kind {
            "forms" => s
                .document
                .definition
                .forms
                .iter()
                .map(|f| f.id.to_string())
                .collect(),
            "signals" => s
                .document
                .definition
                .signals
                .iter()
                .map(|f| f.name().to_owned())
                .collect(),
            "stages" => s
                .document
                .definition
                .stages
                .iter()
                .map(|f| f.id().to_string())
                .collect(),
            "topics" => s
                .document
                .definition
                .escalations
                .iter()
                .map(|f| f.topic.to_string())
                .collect(),
            _ => s
                .document
                .definition
                .nodes
                .iter()
                .filter(|n| !n.is_start())
                .map(|n| n.id().to_string())
                .collect(),
        })
    })
}
#[component]
pub(super) fn Toggle(field: Field, label: &'static str, initial: Value) -> AnyView {
    let initial = StoredValue::new(initial);
    view! { <label class="fp-check"><input type="checkbox" prop:checked=move || !field.get().is_null() on:change=move |ev| field.set(if event_target_checked(&ev) { initial.get_value() } else { Value::Null })/>{label}</label> }.into_any()
}
#[component]
pub(super) fn DurationField(field: Field, label: &'static str) -> AnyView {
    let initial_value = untrack(move || field.text());
    let initial = duration_parts(&initial_value);
    let amount = RwSignal::new(initial.0);
    let unit = RwSignal::new(initial.1);
    let written = RwSignal::new(initial_value);
    Effect::new(move |_| {
        let current = field.text();
        if current != written.get_untracked() {
            let parts = duration_parts(&current);
            amount.set(parts.0);
            unit.set(parts.1);
            written.set(current);
        }
    });
    let write = move || {
        let value = duration_value(&amount.get_untracked(), &unit.get_untracked());
        written.set(value.clone());
        field.set(json!(value));
    };
    view! { <div class="fp-duration"><label class="fp-field">{label}<input prop:value=move || amount.get() on:input=move |ev| { amount.set(event_target_value(&ev)); write(); }/></label>
        <label class="fp-field">"Unit"<select aria-label=format!("{label} unit") prop:value=move || unit.get() on:change=move |ev| {
            let next=event_target_value(&ev);
            if next == "custom" { amount.set(field.text()); } else if unit.get_untracked() == "custom" { amount.set("1".into()); }
            unit.set(next); write();
        }><option value="seconds">"Seconds"</option><option value="minutes">"Minutes"</option><option value="hours">"Hours"</option><option value="days">"Days"</option><option value="weeks">"Weeks"</option><option value="custom">"ISO duration"</option></select></label></div>
    }
    .into_any()
}
#[component]
pub(super) fn DateField(field: Field, label: &'static str) -> AnyView {
    view! { <label class="fp-field">{label}<input type="datetime-local" step="any" prop:value=move || {
        chrono::DateTime::parse_from_rfc3339(&field.text()).map(|v| v.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%S%.f").to_string()).unwrap_or_else(|_| field.text().trim_end_matches('Z').to_owned())
    } on:input=move |ev| { let value=event_target_value(&ev); field.set(if value.is_empty() { Value::Null } else { json!(format!("{}Z", if value.len()==16 {format!("{value}:00")} else {value})) }); }/></label> }.into_any()
}

#[component]
pub(super) fn HandlerField(field: Field, label: &'static str, session: Session) -> AnyView {
    let list_id = format!("fp-handlers-{}", field.path.get_value().replace('/', "-"));
    let options = move || {
        session.state.with(|s| {
            let root = serde_json::to_value(&s.document.definition).unwrap_or_default();
            let mut values = root["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|n| n["handler"].as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            if let Some(hooks) = root["special_handlers"].as_object() {
                for hook in hooks.values() {
                    if let Some(id) = hook.as_str().or_else(|| hook["handler"].as_str()) {
                        values.push(id.into());
                    }
                }
            }
            values.sort();
            values.dedup();
            values
        })
    };
    view! { <label class="fp-field">{label}<input list=list_id.clone() prop:value=move || field.text() on:input=move |ev| field.set(json!(event_target_value(&ev)))/></label>
    <datalist id=list_id>{move || options().into_iter().map(|id| view!{ <option value=id/> }).collect_view()}</datalist> }
    .into_any()
}
/// Recursive editor for ordinary JSON and the engine's typed ContextValue wrappers.
#[component]
pub(super) fn ValueEditor(
    field: Field,
    #[prop(default = false)] typed: bool,
    session: Session,
) -> AnyView {
    let kind = Memo::new(move |_| value_kind(&field.get(), typed));
    let options = if typed {
        fixed(&[
            "string", "id_field", "number", "float", "date", "datetime", "boolean", "array",
            "object",
        ])
    } else {
        fixed(&["string", "number", "boolean", "array", "object", "null"])
    };
    view! { <div class="fp-value-editor">
        <label class="fp-field">"Value type"<select prop:value=move || kind.get() on:change=move |ev| {
            let k=event_target_value(&ev); let value=default_value(&k);
            field.set(if typed {json!({k:value})} else {value});
        }>{move || options.get().into_iter().map(|k| view!{<option value=k.clone()>{k.clone()}</option>}).collect_view()}</select></label>
        {move || {
            let k=kind.get(); let payload=if typed {field.child(&k)} else {field};
            match k.as_str() {
                "object" => view!{<ObjectEditor field=payload typed=false session=session/>}.into_any(),
                "array" => view!{<ArrayEditor field=payload typed=typed session=session/>}.into_any(),
                "boolean" => view!{<Check field=payload label="Enabled"/>}.into_any(),
                "datetime" => view!{<DateField field=payload label="Date and time (UTC)"/>}.into_any(),
                "date" => view!{<TextField field=payload label="Date" kind="date"/>}.into_any(),
                "number" | "float" => view!{<TextField field=payload label="Number" kind="number"/>}.into_any(),
                "null" => view!{<p>"Empty value"</p>}.into_any(),
                _ => view!{<TextField field=payload label="Value"/>}.into_any(),
            }
        }}
    </div> }
    .into_any()
}
fn value_kind(value: &Value, typed: bool) -> String {
    if typed {
        return value
            .as_object()
            .and_then(|v| v.keys().next())
            .cloned()
            .unwrap_or("string".into());
    }
    match value {
        Value::Object(_) => "object",
        Value::Array(_) => "array",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::Null => "null",
        _ => "string",
    }
    .into()
}
fn default_value(kind: &str) -> Value {
    match kind {
        "object" => json!({}),
        "array" => json!([]),
        "boolean" => json!(false),
        "number" => json!(0),
        "float" => json!(0.0),
        "null" => Value::Null,
        "date" => json!("2026-01-01"),
        "datetime" => json!("2026-01-01T00:00:00Z"),
        _ => json!(""),
    }
}
#[component]
pub(super) fn ObjectEditor(
    field: Field,
    #[prop(default = true)] typed: bool,
    session: Session,
) -> AnyView {
    let key = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());
    view! { <div class="fp-object-editor">
        <For each={move || field.get().as_object().map(|v| v.keys().cloned().collect::<Vec<_>>()).unwrap_or_default()} key=|key|key.clone() let:name>
            <ObjectEditorField name field typed session error />
        </For>
        <ObjectEditorAddField field typed key error />
    </div> }
    .into_any()
}

#[component]
fn ObjectEditorField(
    name: String,
    field: Field,
    typed: bool,
    session: Session,
    error: RwSignal<String>,
) -> AnyView {
    let name = StoredValue::new(name);
    let value = field.child(name.get_value());
    let rename = RwSignal::new(name.get_value());

    view! {
        <div class="fp-setting-card">
            <ObjectEditorFieldActions name field rename error />
            <ObjectEditorFieldValue name field value typed session />
        </div>
    }
    .into_any()
}

#[component]
fn ObjectEditorFieldActions(
    name: StoredValue<String>,
    field: Field,
    rename: RwSignal<String>,
    error: RwSignal<String>,
) -> AnyView {
    view! {
        <div class="fp-inline">
            <input aria-label="Field name" prop:value=move || rename.get() on:input=move |ev| rename.set(event_target_value(&ev))/>
            <button type="button" on:click=move |_| {
                let new=rename.get_untracked();
                let mut object=field.get();
                if new.is_empty() || (new != name.get_value() && object.get(&new).is_some()) {
                    error.set("Use a nonempty, unique field name.".into());
                    return;
                }
                if let Some(object)=object.as_object_mut()
                    && let Some(value)=object.remove(&name.get_value())
                {
                    object.insert(new,value);
                }
                field.set(object);
                error.set(String::new());
            }>"Rename"</button>
            <button type="button" aria-label=move || format!("Remove field {}",name.get_value()) on:click=move |_| {
                let mut object=field.get();
                if let Some(object)=object.as_object_mut(){object.remove(&name.get_value());}
                field.set(object);
            }>"×"</button>
        </div>
    }
    .into_any()
}

#[component]
fn ObjectEditorFieldValue(
    name: StoredValue<String>,
    field: Field,
    value: Field,
    typed: bool,
    session: Session,
) -> AnyView {
    view! {
        <Show when=move || typed && name.get_value()=="topic" && value.get().get("id_field").is_some() && field.draft.with(|v|v["handler"]=="create_support_ticket") fallback=move || view!{<ValueEditor field=value typed=typed session=session/>}>
            <Select field=value.child("id_field") label="Escalation topic" options=catalog(session,"topics") optional=false/>
        </Show>
    }
    .into_any()
}

#[component]
fn ObjectEditorAddField(
    field: Field,
    typed: bool,
    key: RwSignal<String>,
    error: RwSignal<String>,
) -> AnyView {
    view! {
        <div class="fp-inline">
            <input aria-label=if typed {"New argument name"} else {"New field name"} placeholder="Field name" prop:value=move ||key.get() on:input=move |ev|key.set(event_target_value(&ev))/>
            <button type="button" on:click=move |_| {
                let name=key.get_untracked();
                let mut object=field.get();
                if name.is_empty() || object.get(&name).is_some(){
                    error.set("Use a nonempty, unique field name.".into());
                    return;
                }
                object[&name]=if typed{json!({"string":""})}else{json!("")};
                field.set(object);
                key.set(String::new());
                error.set(String::new());
            }>"Add field"</button>
        </div>
        <p role="status">{move ||error.get()}</p>
    }
    .into_any()
}
#[component]
fn ArrayEditor(field: Field, typed: bool, session: Session) -> AnyView {
    view! {
        <div>
            <For each={move || (0..field.get().as_array().map_or(0,Vec::len)).collect::<Vec<_>>()} key=|index|*index let:index>
                <ArrayEditorItem field index typed session />
            </For>
            <button type="button" on:click=move |_| {
                let mut value=field.get();
                if let Some(array)=value.as_array_mut(){
                    array.push(if typed{json!({"string":""})}else{json!("")});
                }
                field.set(value);
            }>"Add item"</button>
        </div>
    }
    .into_any()
}

#[component]
fn ArrayEditorItem(field: Field, index: usize, typed: bool, session: Session) -> AnyView {
    view! {
        <div class="fp-setting-card">
            <ValueEditor field=field.child(index) typed=typed session=session />
            <button type="button" on:click=move |_| {
                let mut value=field.get();
                if let Some(array)=value.as_array_mut(){array.remove(index);}
                field.set(value);
            }>"Remove item"</button>
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structured_fields_edit_nested_objects_arrays_and_escaped_keys() {
        let mut value = json!({"args":{"a/b~c":{"array":[{"object":{"flag":false}}]}}});
        set_path(&mut value, "/args/a~1b~0c/array/0/object/flag", json!(true));
        set_path(&mut value, "/metadata/owner", json!("team"));
        assert_eq!(value["args"]["a/b~c"]["array"][0]["object"]["flag"], true);
        assert_eq!(value["metadata"]["owner"], "team");
    }
}
