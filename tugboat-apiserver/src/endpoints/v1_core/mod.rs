use utoipa_actix_web::service_config::ServiceConfig;

mod namespace_create;
mod namespace_list;
mod namespace_read;
mod ship_create;
mod ship_list;
mod ship_read;
mod shipclass_create;
mod shipclass_list;
mod shipclass_read;

pub(super) fn register_v1_core(service: &mut ServiceConfig) {
    service
        .service(namespace_create::handle_namespace_create)
        .service(namespace_list::handle_namespace_list)
        .service(namespace_read::handle_namespace_read)
        .service(ship_create::handle_ship_create)
        .service(ship_list::handle_ship_list)
        .service(ship_read::handle_ship_read)
        .service(shipclass_create::handle_shipclass_create)
        .service(shipclass_list::handle_shipclass_list)
        .service(shipclass_read::handle_shipclass_read);
}
