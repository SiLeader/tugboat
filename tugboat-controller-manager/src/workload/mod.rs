use std::collections::HashMap;
use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference};
use tugboat_resources::{ObjectMetaResource, StaticResource};

pub(crate) const SHIP_TEMPLATE_HASH_LABEL: &str = "ship-template-hash";

pub(crate) fn matches_selector(
    labels: &HashMap<String, String>,
    selector: &HashMap<String, String>,
) -> bool {
    selector
        .iter()
        .all(|(key, value)| labels.get(key).is_some_and(|label| label == value))
}

pub(crate) fn owner_reference_for<T>(resource: &T) -> OwnerReference
where
    T: ObjectMetaResource + StaticResource,
{
    OwnerReference {
        api_version: T::type_meta().api_version.unwrap_or_default(),
        kind: T::kind().to_string(),
        name: resource.name().unwrap_or_default().to_string(),
        uid: resource
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.uid.as_deref())
            .unwrap_or_default()
            .to_string(),
        controller: Some(true),
    }
}

pub(crate) fn is_controlled_by<T>(meta: Option<&ObjectMeta>, owner: &T, owner_kind: &str) -> bool
where
    T: ObjectMetaResource,
{
    let owner_name = owner.name().unwrap_or_default();
    let owner_uid = owner
        .object_meta()
        .as_ref()
        .and_then(|meta| meta.uid.as_deref());

    meta.is_some_and(|meta| {
        meta.owner_references.iter().any(|owner_ref| {
            owner_ref.kind == owner_kind
                && owner_ref.name == owner_name
                && owner_uid
                    .map(|uid| owner_ref.uid == uid)
                    .unwrap_or_else(|| owner_ref.uid.is_empty())
        })
    })
}

pub(crate) fn creation_sort_key(
    meta: Option<&ObjectMeta>,
    name: Option<&str>,
) -> (i64, i32, String) {
    let (seconds, nanos) = meta
        .and_then(|meta| meta.creation_timestamp.as_ref())
        .map(|time| (time.seconds, time.nanos))
        .unwrap_or((0, 0));
    (seconds, nanos, name.unwrap_or_default().to_string())
}

#[cfg(test)]
mod tests {
    use super::{creation_sort_key, is_controlled_by, matches_selector, owner_reference_for};
    use std::collections::HashMap;
    use tugboat_resources::manifests::apps::v1::ReplicaSet;
    use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference, Time};

    #[test]
    fn selector_requires_all_entries_to_match() {
        let labels = HashMap::from([
            ("app".to_string(), "demo".to_string()),
            ("tier".to_string(), "backend".to_string()),
        ]);
        let selector = HashMap::from([
            ("app".to_string(), "demo".to_string()),
            ("tier".to_string(), "backend".to_string()),
        ]);

        assert!(matches_selector(&labels, &selector));
        assert!(!matches_selector(
            &HashMap::from([("app".to_string(), "demo".to_string())]),
            &selector
        ));
    }

    #[test]
    fn owner_reference_uses_static_resource_metadata() {
        let rs = ReplicaSet {
            object_meta: Some(ObjectMeta {
                name: Some("demo-rs".to_string()),
                uid: Some("rs-uid".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(
            owner_reference_for(&rs),
            OwnerReference {
                api_version: "apps/v1".to_string(),
                kind: "ReplicaSet".to_string(),
                name: "demo-rs".to_string(),
                uid: "rs-uid".to_string(),
                controller: Some(true),
            }
        );
    }

    #[test]
    fn controlled_by_accepts_name_with_empty_uid_owner() {
        let owner = ReplicaSet {
            object_meta: Some(ObjectMeta {
                name: Some("demo-rs".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let meta = ObjectMeta {
            owner_references: vec![OwnerReference {
                kind: "ReplicaSet".to_string(),
                name: "demo-rs".to_string(),
                uid: String::new(),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(is_controlled_by(Some(&meta), &owner, "ReplicaSet"));
    }

    #[test]
    fn creation_key_orders_by_timestamp_then_name() {
        let newer = ObjectMeta {
            name: Some("a".to_string()),
            creation_timestamp: Some(Time {
                seconds: 20,
                nanos: 0,
            }),
            ..Default::default()
        };
        let older = ObjectMeta {
            name: Some("z".to_string()),
            creation_timestamp: Some(Time {
                seconds: 10,
                nanos: 0,
            }),
            ..Default::default()
        };

        assert!(
            creation_sort_key(Some(&newer), newer.name.as_deref())
                > creation_sort_key(Some(&older), older.name.as_deref())
        );
    }
}
