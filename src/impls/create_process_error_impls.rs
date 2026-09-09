//! Conversion of engine errors to JSON HTTP error responses.

use crate::models::process_def_error::CreateProcessError;
use actix_web::HttpResponse;
use serde_json::json;

/// HTTP response adapter for engine operation errors.
pub struct HttpResponseWrapper {
    /// Constructed Actix Web HTTP response.
    pub response: HttpResponse,
}
impl HttpResponseWrapper {
    /// Converts an engine error into an HTTP 500 response with a JSON body.
    pub fn create_response(error: CreateProcessError) -> HttpResponseWrapper {
        HttpResponseWrapper {
            response: HttpResponse::InternalServerError().json(json!(error)),
        }
    }
}
