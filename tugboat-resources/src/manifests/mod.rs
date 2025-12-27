pub mod core {
    pub mod v1 {
        use crate::apply_resource;

        include!(concat!(env!("OUT_DIR"), "/tugboat.core.v1.rs"));

        apply_resource!(Namespace, "core", "v1", "namespaces", "namespace", cluster);
        apply_resource!(Node, "core", "v1", "nodes", "node", cluster);
        apply_resource!(Ship, "core", "v1", "ships", "ship", namespaced);
        apply_resource!(ShipClass, "core", "v1", "shipclasses", "shipclass", cluster);
    }
}

pub mod meta {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/tugboat.meta.v1.rs"));
    }
}
