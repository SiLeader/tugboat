use crate::manifests::meta::v1::{ObjectMeta, TypeMeta};

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
            api_version: if Self::group() == "core" || Self::group().is_empty() {
                Self::version().to_string()
            } else {
                format!("{}/{}", Self::group(), Self::version())
            },
            kind: Self::kind().to_string(),
        }
    }
}

#[macro_export]
macro_rules! apply_resource {
    ($ty:ident, $group:literal, $version:literal, $plural:literal, $singular:literal, $cluster_scoped:literal) => {
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
                $cluster_scoped
            }
        }

        impl $crate::ObjectMetaResource for $ty {
            fn object_meta(&self) -> &Option<$crate::manifests::meta::v1::ObjectMeta> {
                &self.object_meta
            }
        }
    };

    ($ty:ident, $group:literal, $version:literal, $plural:literal, $singular:literal, namespaced) => {
        apply_resource!($ty, $group, $version, $plural, $singular, false);
    };

    ($ty:ident, $group:literal, $version:literal, $plural:literal, $singular:literal, cluster) => {
        apply_resource!($ty, $group, $version, $plural, $singular, true);
    };
}
