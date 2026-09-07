use crate::engine::fluxpro_engine::FluxProEngine;
use crate::impls::create_process_error_impls::HttpResponseWrapper;
use crate::models::commands::post_signal::PostSignal;
use crate::models::id_field::IdField;
use actix_web::{HttpResponse, Responder, post, web};

#[post("/{process_id}/post_signal")]
pub async fn post_signal(
    process_id: web::Path<IdField>,
    post_signal: web::Json<PostSignal>,
    fxp_engine: web::Data<FluxProEngine>,
) -> impl Responder {
    match fxp_engine
        .service
        .post_signal(process_id.into_inner(), post_signal.into_inner())
        .await
    {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(e) => HttpResponseWrapper::create_response(e).response,
    }
}
