//! Registration of HTTP routes under `/fluxpro`.

use crate::api_handlers::proc_def_methods::create_process_def;
use crate::api_handlers::proc_events_methods::post_signal;
use crate::api_handlers::proc_instance_methods::start_proc_instance::start_proc_instance;
use actix_web::web;

/// Registers definition, process, and instance routes under `/fluxpro`.
///
/// The host must supply both `web::Data<Arc<FluxProEngine>>` and
/// `web::Data<FluxProEngine>` for the current endpoint extractors.
///
/// # Examples
///
/// ```no_run
/// use actix_web::{web, App};
/// use fluxpro_engine::engine::fluxpro_engine::FluxProEngine;
/// use std::sync::Arc;
///
/// fn configure_app(engine: Arc<FluxProEngine>) {
///     let _app = App::new()
///         .app_data(web::Data::new(engine.clone()))
///         .app_data(web::Data::<FluxProEngine>::from(engine))
///         .configure(fluxpro_engine::api_handlers::config::config);
/// }
/// ```
pub fn config(conf: &mut web::ServiceConfig) {
    let process_def_scope = web::scope("/process_definitions").service(create_process_def);
    let process_scope = web::scope("/process").service(start_proc_instance);
    let instance_scope = web::scope("/instance").service(post_signal);
    let flux_pro_scope = web::scope("/fluxpro")
        .service(process_def_scope)
        .service(process_scope)
        .service(instance_scope);

    conf.service(flux_pro_scope);
}
