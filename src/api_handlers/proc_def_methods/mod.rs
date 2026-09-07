use crate::engine::fluxpro_engine::FluxProEngine;
use crate::models::process_def::ProcessDefinition;
use actix_web::{HttpResponse, put, web};
use log::info;
use serde_json::json;
use std::sync::Arc;

#[put("/create")]
pub async fn create_process_def(
    body: web::Bytes,
    fxp_engine: web::Data<Arc<FluxProEngine>>,
) -> HttpResponse {
    info!("Create process definition");
    let create_process = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(e) => {
            return HttpResponse::BadRequest().json(json!({
                "status": "Failed",
                "error": format!("Invalid UTF-8 in the request body: {}", e)
            }));
        }
    };

    match serde_yaml::from_str::<ProcessDefinition>(create_process) {
        Ok(create_process_def) => {
            match fxp_engine
                .service
                .create_process_def_from_source(&create_process_def, create_process)
                .await
            {
                Ok(process_id) => {
                    HttpResponse::Ok().json(json!({"status": "Success", "process_id": process_id}))
                }
                Err(e) => HttpResponse::BadRequest().json(json!({
                    "status": "Failed",
                    "error": format!("Engine error: {:?}", e)
                })),
            }
        }
        Err(err) => HttpResponse::BadRequest().json(json!({
            "status": "Failed",
            "error": err.to_string()
        })),
    }
}
