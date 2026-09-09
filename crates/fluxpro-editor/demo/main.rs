#[cfg(target_arch = "wasm32")]
mod monitor_data;

#[cfg(target_arch = "wasm32")]
fn main() {
    use fluxpro_editor::{
        EditorDocument, MonitorRequest, MonitorSnapshot, ProcessEditor, ProcessMonitor,
    };
    use leptos::prelude::*;
    leptos::mount::mount_to_body(move || {
        let document = RwSignal::new(
            EditorDocument::from_yaml(include_str!("../../../examples/definitions/approval.yaml"))
                .expect("valid example process"),
        );
        let snapshot = RwSignal::new(MonitorSnapshot::default());
        let monitoring = RwSignal::new(false);
        let on_request = Callback::new(move |request: MonitorRequest| {
            snapshot.set(monitor_data::snapshot(request))
        });
        view! {
            <div style="height:100dvh;padding:16px 24px 24px;background:#edf1f6;box-sizing:border-box;display:flex;flex-direction:column;gap:12px">
                <style>".fp-demo-nav button { padding:5px 12px;border:1px solid #d5deeb;background:#fff;color:#526782;border-radius:6px;font:inherit;cursor:pointer; }.fp-demo-nav button:hover { background:#edf3ff; }"</style>
                <nav class="fp-demo-nav" style="display:flex;gap:8px;align-items:center;font:12px -apple-system,sans-serif;color:#68788f">
                    <button on:click=move |_|monitoring.set(false)>"Process editor"</button>
                    <button on:click=move |_|monitoring.set(true)>"Live monitor demo"</button>
                    <span>{move ||if monitoring.get(){"SIMULATED DATA · 67 instances · updates every 5 seconds"}else{"Embeddable FluxPro components"}}</span>
                </nav>
                <div style="flex:1;min-height:0" hidden=move ||monitoring.get()><ProcessEditor document=document.get_untracked() on_change=Callback::new(move |doc|document.set(doc))/></div>
                <Show when=move ||monitoring.get()><div style="flex:1;min-height:0"><ProcessMonitor document=document snapshot=snapshot on_request=on_request/></div></Show>
            </div>
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("Run: cd crates/fluxpro-editor && trunk serve --open");
}
