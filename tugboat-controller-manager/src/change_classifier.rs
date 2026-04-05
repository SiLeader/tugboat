use tugboat_resources::manifests::apps::v1::ShipTemplateSpec;

#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TemplateChangeKind {
    NoChange,
    InPlace,
    RequiresRotation,
}

#[allow(dead_code)]
pub(crate) fn classify_template_change(
    old: &ShipTemplateSpec,
    new: &ShipTemplateSpec,
) -> TemplateChangeKind {
    if old == new {
        return TemplateChangeKind::NoChange;
    }

    let old_spec = old.spec.as_ref();
    let new_spec = new.spec.as_ref();

    let rotation_required = old_spec.map(|spec| &spec.image) != new_spec.map(|spec| &spec.image)
        || old_spec.map(|spec| &spec.uefi) != new_spec.map(|spec| &spec.uefi)
        || old_spec.map(|spec| &spec.network_class_ref)
            != new_spec.map(|spec| &spec.network_class_ref)
        || old_spec.map(|spec| &spec.volume_claim_ref)
            != new_spec.map(|spec| &spec.volume_claim_ref);

    if rotation_required {
        TemplateChangeKind::RequiresRotation
    } else {
        TemplateChangeKind::InPlace
    }
}

#[cfg(test)]
mod tests {
    use super::{TemplateChangeKind, classify_template_change};
    use tugboat_resources::manifests::apps::v1::ShipTemplateSpec;
    use tugboat_resources::manifests::core::v1::{
        ConfigMapVolumeSource, KeyToPath, ShipSpec, ShipUefi, ShipVolume, ShipVolumeClaimReference,
    };

    fn base_template() -> ShipTemplateSpec {
        ShipTemplateSpec {
            spec: Some(ShipSpec {
                image: "example.com/images/demo:latest".to_string(),
                ship_class: "standard".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn ship_class_change_is_in_place() {
        let old = base_template();
        let mut new = base_template();
        new.spec.as_mut().unwrap().ship_class = "large".to_string();

        assert_eq!(
            classify_template_change(&old, &new),
            TemplateChangeKind::InPlace
        );
    }

    #[test]
    fn image_change_requires_rotation() {
        let old = base_template();
        let mut new = base_template();
        new.spec.as_mut().unwrap().image = "example.com/images/demo:v2".to_string();

        assert_eq!(
            classify_template_change(&old, &new),
            TemplateChangeKind::RequiresRotation
        );
    }

    #[test]
    fn configmap_volume_change_is_in_place() {
        let old = base_template();
        let mut new = base_template();
        new.spec.as_mut().unwrap().volumes.push(ShipVolume {
            name: "config".to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: "app-config".to_string(),
                items: vec![KeyToPath {
                    key: "config.toml".to_string(),
                    path: "config.toml".to_string(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        });

        assert_eq!(
            classify_template_change(&old, &new),
            TemplateChangeKind::InPlace
        );
    }

    #[test]
    fn pvc_change_requires_rotation() {
        let old = base_template();
        let mut new = base_template();
        new.spec
            .as_mut()
            .unwrap()
            .volume_claim_ref
            .push(ShipVolumeClaimReference {
                name: "data".to_string(),
            });

        assert_eq!(
            classify_template_change(&old, &new),
            TemplateChangeKind::RequiresRotation
        );
    }

    #[test]
    fn mixed_change_requires_rotation() {
        let old = base_template();
        let mut new = base_template();
        let spec = new.spec.as_mut().unwrap();
        spec.ship_class = "large".to_string();
        spec.image = "example.com/images/demo:v2".to_string();

        assert_eq!(
            classify_template_change(&old, &new),
            TemplateChangeKind::RequiresRotation
        );
    }

    #[test]
    fn no_changes_returns_no_change() {
        let old = base_template();
        let new = base_template();

        assert_eq!(
            classify_template_change(&old, &new),
            TemplateChangeKind::NoChange
        );
    }

    #[test]
    fn uefi_change_requires_rotation() {
        let old = base_template();
        let mut new = base_template();
        new.spec.as_mut().unwrap().uefi = Some(ShipUefi { enabled: true });

        assert_eq!(
            classify_template_change(&old, &new),
            TemplateChangeKind::RequiresRotation
        );
    }
}
