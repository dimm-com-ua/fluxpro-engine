//! HTTP startup of an instance using a process definition key.

use crate::engine::fluxpro_engine::FluxProEngine;
use crate::impls::create_process_error_impls::HttpResponseWrapper;
use crate::models::commands::start_process_instance::{
    StartProcessInstance, StartProcessInstanceResponse,
};
use crate::models::id_field::IdField;
use actix_web::web::Json;
use actix_web::{HttpResponse, Responder, post, web};
use log::{error, info};
use std::sync::Arc;

/// Starts an instance using the definition key in the URL and JSON request body.
#[post("/{process_id}/start")]
pub async fn start_proc_instance(
    process_id: web::Path<IdField>,
    start_process: Json<StartProcessInstance>,
    fxp_engine: web::Data<Arc<FluxProEngine>>,
) -> impl Responder {
    match fxp_engine
        .service
        .start_process_instance(process_id.into_inner(), start_process.into_inner())
        .await
    {
        Ok(process_ins_uuid) => {
            info!("Process instance started with id: {process_ins_uuid}");
            HttpResponse::Ok().json(StartProcessInstanceResponse {
                process_instance_id: process_ins_uuid,
            })
        }
        Err(err) => {
            error!("error: {:?}", err);
            HttpResponseWrapper::create_response(err).response
        }
    }
}
