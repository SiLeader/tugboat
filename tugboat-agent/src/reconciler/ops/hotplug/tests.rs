// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use super::{HotplugBaseline, HotplugDesired, classify_hotplug_changes};
use tugboat_resources::manifests::core::v1::{
    HotplugCapabilities, RuntimeHotplug, ShipNetworkClassReference, ShipSpec,
};
use tugboat_vm_runtime_interface::run::{VmNetworkConfig, VmVolumeConfig};

fn ship_spec() -> ShipSpec {
    ShipSpec {
        image: "registry.example.com/test:1".to_string(),
        ship_class: "small".to_string(),
        node_name: Some("node-a".to_string()),
        network_class_ref: vec![],
        uefi: None,
        tolerations: vec![],
        scheduler_name: None,
        volume_claim_ref: vec![],
        volumes: vec![],
        target_node_name: None,
        runtime_class: None,
        service_account_name: None,
        automount_service_account_token: None,
    }
}

fn enabled_hotplug() -> RuntimeHotplug {
    RuntimeHotplug {
        cpu: Some(HotplugCapabilities {
            add: true,
            remove: true,
        }),
        memory: Some(HotplugCapabilities {
            add: true,
            remove: true,
        }),
        nic: Some(HotplugCapabilities {
            add: true,
            remove: true,
        }),
        storage: Some(HotplugCapabilities {
            add: true,
            remove: true,
        }),
    }
}

#[test]
fn cpu_increase_with_flag_enabled_is_hotpluggable() {
    let old_spec = ship_spec();
    let new_spec = ship_spec();
    let baseline = HotplugBaseline {
        current_cpu_cores: 2,
        current_memory_bytes: 1024,
        current_nic_ids: vec![],
        current_volume_ids: vec![],
    };
    let desired = HotplugDesired {
        desired_cpu_cores: 4,
        desired_memory_bytes: 1024,
        memory_size: "1024".to_string(),
        nics_added: vec![],
        volumes_added: vec![],
    };

    let plan = classify_hotplug_changes(
        "ship-1",
        &old_spec,
        &new_spec,
        &baseline,
        &desired,
        Some(&enabled_hotplug()),
        false,
    );

    let request = plan.hotplug_req.expect("hotplug request");
    assert_eq!(request.cpu.expect("cpu request").cores, 4);
    assert!(!plan.has_unsupported_changes);
}

#[test]
fn cpu_increase_with_flag_disabled_returns_empty_plan() {
    let old_spec = ship_spec();
    let new_spec = ship_spec();
    let baseline = HotplugBaseline {
        current_cpu_cores: 2,
        current_memory_bytes: 1024,
        current_nic_ids: vec![],
        current_volume_ids: vec![],
    };
    let desired = HotplugDesired {
        desired_cpu_cores: 4,
        desired_memory_bytes: 1024,
        memory_size: "1024".to_string(),
        nics_added: vec![],
        volumes_added: vec![],
    };

    let plan = classify_hotplug_changes(
        "ship-1",
        &old_spec,
        &new_spec,
        &baseline,
        &desired,
        Some(&RuntimeHotplug::default()),
        false,
    );

    assert!(plan.hotplug_req.is_none());
    assert!(plan.has_unsupported_changes);
}

#[test]
fn mixed_hotplug_and_unsupported_change() {
    let old_spec = ship_spec();
    let mut new_spec = ship_spec();
    new_spec.image = "registry.example.com/test:2".to_string();
    new_spec.network_class_ref.push(ShipNetworkClassReference {
        api_group: "core".to_string(),
        kind: "NetworkClass".to_string(),
        name: "frontend".to_string(),
    });
    let baseline = HotplugBaseline {
        current_cpu_cores: 2,
        current_memory_bytes: 1024,
        current_nic_ids: vec![],
        current_volume_ids: vec![],
    };
    let desired = HotplugDesired {
        desired_cpu_cores: 4,
        desired_memory_bytes: 1024,
        memory_size: "1024".to_string(),
        nics_added: vec![VmNetworkConfig {
            iface_name: "eth0".to_string(),
            mac_address: "52:54:00:00:00:01".to_string(),
        }],
        volumes_added: vec![VmVolumeConfig::block("/var/lib/test.img", "raw", false)],
    };

    let plan = classify_hotplug_changes(
        "ship-1",
        &old_spec,
        &new_spec,
        &baseline,
        &desired,
        Some(&enabled_hotplug()),
        true,
    );

    let request = plan.hotplug_req.expect("hotplug request");
    assert_eq!(request.cpu.expect("cpu request").cores, 4);
    assert!(request.nics_added.len() == 1);
    assert!(plan.has_unsupported_changes);
}
