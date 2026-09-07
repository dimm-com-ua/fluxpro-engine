use crate::models::process_def_error::CreateProcessError;
use actix_web::HttpResponse;
use serde_json::json;

pub struct HttpResponseWrapper {
    pub response: HttpResponse,
}
impl HttpResponseWrapper {
    pub fn create_response(error: CreateProcessError) -> HttpResponseWrapper {
        HttpResponseWrapper {
            response: HttpResponse::InternalServerError().json(json!(error)),
        }
    }
}
