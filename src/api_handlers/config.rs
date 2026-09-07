use crate::api_handlers::proc_def_methods::create_process_def;
use crate::api_handlers::proc_events_methods::post_signal;
use crate::api_handlers::proc_instance_methods::start_proc_instance::start_proc_instance;
use actix_web::web;

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
