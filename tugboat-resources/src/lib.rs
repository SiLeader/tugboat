use crate::manifests::meta::v1::{ObjectMeta, TypeMeta};
use std::fs::OpenOptions;

pub mod manifests;

pub trait Resource {
    fn type_meta() -> TypeMeta;
}

pub trait StaticResource: Resource {
    fn group() -> &'static str;
    fn version() -> &'static str;
    fn kind() -> &'static str;
    fn plural() -> &'static str;
    fn singular() -> &'static str;
    fn is_cluster_scoped() -> bool;
}

pub trait ObjectMetaResource {
    fn object_meta(&self) -> &Option<ObjectMeta>;
}

impl<T: StaticResource> Resource for T {
    fn type_meta() -> TypeMeta {
        TypeMeta {
            group: Self::group().to_string(),
            version: Self::version().to_string(),
            kind: Self::kind().to_string(),
        }
    }
}

#[macro_export]
macro_rules! apply_resource {
    ($ty:ident, $group:literal, $version:literal, $plural:literal, $singular:literal, $cluster:literal) => {
        impl $crate::StaticResource for $ty {
            fn group() -> &'static str {
                $group
            }

            fn version() -> &'static str {
                $version
            }

            fn kind() -> &'static str {
                stringify!($ty)
            }

            fn plural() -> &'static str {
                $plural
            }

            fn singular() -> &'static str {
                $singular
            }

            fn is_cluster_scoped() -> bool {
                $cluster
            }
        }

        impl $crate::ObjectMetaResource for $ty {
            fn object_meta(&self) -> &Option<$crate::manifests::meta::v1::ObjectMeta> {
                &self.object_meta
            }
        }
    };
}
