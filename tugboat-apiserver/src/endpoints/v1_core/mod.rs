use crate::endpoints::v1_core::namespace_create::handle_namespace_create;
use utoipa_actix_web::service_config::ServiceConfig;

mod namespace_create;

pub(super) fn register_v1_core(service: &mut ServiceConfig) {
    service.service(handle_namespace_create);
}
