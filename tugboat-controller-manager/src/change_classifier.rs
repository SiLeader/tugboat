use tugboat_resources::manifests::apps::v1::ShipTemplateSpec;
use tugboat_resources::manifests::core::v1::RuntimeClass;

#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TemplateChangeKind {
    NoChange,
    InPlace,
    Hotplug,
    RequiresRotation,
}

#[allow(dead_code)]
pub(crate) fn classify_template_change(
    old: &ShipTemplateSpec,
    new: &ShipTemplateSpec,
    runtime_class: Option<&RuntimeClass>,
) -> TemplateChangeKind {
    if old == new {
        return TemplateChangeKind::NoChange;
    }

    let old_spec = old.spec.as_ref();
    let new_spec = new.spec.as_ref();

    if old_spec.map(|spec| &spec.image) != new_spec.map(|spec| &spec.image)
        || old_spec.map(|spec| &spec.uefi) != new_spec.map(|spec| &spec.uefi)
    {
        return TemplateChangeKind::RequiresRotation;
    }

    let network_changed = old_spec.map(|spec| &spec.network_class_ref)
        != new_spec.map(|spec| &spec.network_class_ref);
    let volume_changed =
        old_spec.map(|spec| &spec.volume_claim_ref) != new_spec.map(|spec| &spec.volume_claim_ref);

    let Some(runtime_class) = runtime_class else {
        return if network_changed || volume_changed {
            TemplateChangeKind::RequiresRotation
        } else {
            TemplateChangeKind::InPlace
        };
    };

    let hotplug = runtime_class
        .spec
        .as_ref()
        .and_then(|spec| spec.hotplug.as_ref());
    let mut requires_hotplug = false;

    if network_changed {
        match diff_direction(
            old_spec
                .map(|spec| spec.network_class_ref.as_slice())
                .unwrap_or(&[]),
            new_spec
                .map(|spec| spec.network_class_ref.as_slice())
                .unwrap_or(&[]),
        ) {
            ChangeDirection::Add => {
                if hotplug
                    .and_then(|caps| caps.nic.as_ref())
                    .map(|cap| cap.add)
                    .unwrap_or(false)
                {
                    requires_hotplug = true;
                } else {
                    return TemplateChangeKind::RequiresRotation;
                }
            }
            ChangeDirection::Remove => {
                if hotplug
                    .and_then(|caps| caps.nic.as_ref())
                    .map(|cap| cap.remove)
                    .unwrap_or(false)
                {
                    requires_hotplug = true;
                } else {
                    return TemplateChangeKind::RequiresRotation;
                }
            }
            ChangeDirection::Mixed => {
                let add_allowed = hotplug
                    .and_then(|caps| caps.nic.as_ref())
                    .map(|cap| cap.add)
                    .unwrap_or(false);
                let remove_allowed = hotplug
                    .and_then(|caps| caps.nic.as_ref())
                    .map(|cap| cap.remove)
                    .unwrap_or(false);
                if add_allowed && remove_allowed {
                    requires_hotplug = true;
                } else {
                    return TemplateChangeKind::RequiresRotation;
                }
            }
            ChangeDirection::Reordered => return TemplateChangeKind::RequiresRotation,
            ChangeDirection::NoChange => {}
        }
    }

    if volume_changed {
        match diff_direction(
            old_spec
                .map(|spec| spec.volume_claim_ref.as_slice())
                .unwrap_or(&[]),
            new_spec
                .map(|spec| spec.volume_claim_ref.as_slice())
                .unwrap_or(&[]),
        ) {
            ChangeDirection::Add => {
                if hotplug
                    .and_then(|caps| caps.storage.as_ref())
                    .map(|cap| cap.add)
                    .unwrap_or(false)
                {
                    requires_hotplug = true;
                } else {
                    return TemplateChangeKind::RequiresRotation;
                }
            }
            ChangeDirection::Remove => {
                if hotplug
                    .and_then(|caps| caps.storage.as_ref())
                    .map(|cap| cap.remove)
                    .unwrap_or(false)
                {
                    requires_hotplug = true;
                } else {
                    return TemplateChangeKind::RequiresRotation;
                }
            }
            ChangeDirection::Mixed => {
                let add_allowed = hotplug
                    .and_then(|caps| caps.storage.as_ref())
                    .map(|cap| cap.add)
                    .unwrap_or(false);
                let remove_allowed = hotplug
                    .and_then(|caps| caps.storage.as_ref())
                    .map(|cap| cap.remove)
                    .unwrap_or(false);
                if add_allowed && remove_allowed {
                    requires_hotplug = true;
                } else {
                    return TemplateChangeKind::RequiresRotation;
                }
            }
            ChangeDirection::Reordered => return TemplateChangeKind::RequiresRotation,
            ChangeDirection::NoChange => {}
        }
    }

    if has_other_changes(old_spec, new_spec) {
        if supports_generic_hotplug(runtime_class) {
            requires_hotplug = true;
        } else {
            return TemplateChangeKind::RequiresRotation;
        }
    }

    if requires_hotplug {
        TemplateChangeKind::Hotplug
    } else {
        TemplateChangeKind::InPlace
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeDirection {
    NoChange,
    Add,
    Remove,
    Mixed,
    Reordered,
}

fn diff_direction<T: PartialEq>(old: &[T], new: &[T]) -> ChangeDirection {
    let has_add = new.iter().any(|item| !old.contains(item));
    let has_remove = old.iter().any(|item| !new.contains(item));

    match (has_add, has_remove) {
        (false, false) => {
            if old == new {
                ChangeDirection::NoChange
            } else {
                ChangeDirection::Reordered
            }
        }
        (true, false) => ChangeDirection::Add,
        (false, true) => ChangeDirection::Remove,
        (true, true) => ChangeDirection::Mixed,
    }
}

fn has_other_changes(
    old_spec: Option<&tugboat_resources::manifests::core::v1::ShipSpec>,
    new_spec: Option<&tugboat_resources::manifests::core::v1::ShipSpec>,
) -> bool {
    let mut old = old_spec.cloned().unwrap_or_default();
    let mut new = new_spec.cloned().unwrap_or_default();

    old.image.clear();
    new.image.clear();
    old.uefi = None;
    new.uefi = None;
    old.network_class_ref.clear();
    new.network_class_ref.clear();
    old.volume_claim_ref.clear();
    new.volume_claim_ref.clear();

    old != new
}

fn supports_generic_hotplug(runtime_class: &RuntimeClass) -> bool {
    let hotplug = runtime_class
        .spec
        .as_ref()
        .and_then(|spec| spec.hotplug.as_ref());

    hotplug
        .and_then(|caps| caps.cpu.as_ref())
        .is_some_and(|cap| cap.add || cap.remove)
        || hotplug
            .and_then(|caps| caps.memory.as_ref())
            .is_some_and(|cap| cap.add || cap.remove)
}

#[cfg(test)]
mod tests {
    use super::{TemplateChangeKind, classify_template_change};
    use tugboat_resources::manifests::apps::v1::ShipTemplateSpec;
    use tugboat_resources::manifests::core::v1::{
        ConfigMapVolumeSource, HotplugCapabilities, KeyToPath, RuntimeClass, RuntimeClassSpec,
        RuntimeHotplug, ShipNetworkClassReference, ShipSpec, ShipUefi, ShipVolume,
        ShipVolumeClaimReference,
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

    fn runtime_class_with_hotplug(hotplug: RuntimeHotplug) -> RuntimeClass {
        RuntimeClass {
            spec: Some(RuntimeClassSpec {
                hotplug: Some(hotplug),
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
            classify_template_change(&old, &new, None),
            TemplateChangeKind::InPlace
        );
    }

    #[test]
    fn image_change_requires_rotation() {
        let old = base_template();
        let mut new = base_template();
        new.spec.as_mut().unwrap().image = "example.com/images/demo:v2".to_string();

        assert_eq!(
            classify_template_change(&old, &new, None),
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
            classify_template_change(&old, &new, None),
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
            classify_template_change(&old, &new, None),
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
            classify_template_change(&old, &new, None),
            TemplateChangeKind::RequiresRotation
        );
    }

    #[test]
    fn no_changes_returns_no_change() {
        let old = base_template();
        let new = base_template();

        assert_eq!(
            classify_template_change(&old, &new, None),
            TemplateChangeKind::NoChange
        );
    }

    #[test]
    fn uefi_change_requires_rotation() {
        let old = base_template();
        let mut new = base_template();
        new.spec.as_mut().unwrap().uefi = Some(ShipUefi { enabled: true });

        assert_eq!(
            classify_template_change(&old, &new, None),
            TemplateChangeKind::RequiresRotation
        );
    }

    #[test]
    fn ship_class_change_is_hotplug_when_cpu_add_supported() {
        let old = base_template();
        let mut new = base_template();
        new.spec.as_mut().unwrap().ship_class = "large".to_string();

        let runtime_class = runtime_class_with_hotplug(RuntimeHotplug {
            cpu: Some(HotplugCapabilities {
                add: true,
                remove: false,
            }),
            ..Default::default()
        });

        assert_eq!(
            classify_template_change(&old, &new, Some(&runtime_class)),
            TemplateChangeKind::Hotplug
        );
    }

    #[test]
    fn nic_add_is_hotplug_when_flag_enabled() {
        let old = base_template();
        let mut new = base_template();
        new.spec
            .as_mut()
            .unwrap()
            .network_class_ref
            .push(ShipNetworkClassReference {
                api_group: "core".to_string(),
                kind: "NetworkClass".to_string(),
                name: "secondary".to_string(),
            });

        let runtime_class = runtime_class_with_hotplug(RuntimeHotplug {
            nic: Some(HotplugCapabilities {
                add: true,
                remove: false,
            }),
            ..Default::default()
        });

        assert_eq!(
            classify_template_change(&old, &new, Some(&runtime_class)),
            TemplateChangeKind::Hotplug
        );
    }

    #[test]
    fn nic_add_requires_rotation_when_flag_disabled() {
        let old = base_template();
        let mut new = base_template();
        new.spec
            .as_mut()
            .unwrap()
            .network_class_ref
            .push(ShipNetworkClassReference {
                api_group: "core".to_string(),
                kind: "NetworkClass".to_string(),
                name: "secondary".to_string(),
            });

        let runtime_class = runtime_class_with_hotplug(RuntimeHotplug {
            nic: Some(HotplugCapabilities {
                add: false,
                remove: false,
            }),
            ..Default::default()
        });

        assert_eq!(
            classify_template_change(&old, &new, Some(&runtime_class)),
            TemplateChangeKind::RequiresRotation
        );
    }

    #[test]
    fn mixed_hotplug_and_non_hotplug_requires_rotation() {
        let old = base_template();
        let mut new = base_template();
        let spec = new.spec.as_mut().unwrap();
        spec.image = "example.com/images/demo:v2".to_string();
        spec.network_class_ref.push(ShipNetworkClassReference {
            api_group: "core".to_string(),
            kind: "NetworkClass".to_string(),
            name: "secondary".to_string(),
        });

        let runtime_class = runtime_class_with_hotplug(RuntimeHotplug {
            nic: Some(HotplugCapabilities {
                add: true,
                remove: false,
            }),
            ..Default::default()
        });

        assert_eq!(
            classify_template_change(&old, &new, Some(&runtime_class)),
            TemplateChangeKind::RequiresRotation
        );
    }
}
