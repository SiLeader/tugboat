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

use super::{
    ControllerCapability, CsiAccessMode, CsiAccessType, CsiVolumeContentSource, NodeVolumeStats,
    TugboatCsiOperator, VolumeHealthCondition, VolumeUsageStats, VolumeUsageUnit,
    normalize_socket_path, volume_capability,
};
use crate::error::Error;
use crate::proto::csi::v1::controller_server::{Controller as CsiController, ControllerServer};
use crate::proto::csi::v1::controller_service_capability;
use crate::proto::csi::v1::controller_service_capability::rpc::Type as ControllerCapabilityType;
use crate::proto::csi::v1::node_server::{Node, NodeServer};
use crate::proto::csi::v1::volume_capability::AccessType;
use crate::proto::csi::v1::volume_content_source;
use crate::proto::csi::v1::volume_usage::Unit as VolumeUsageProtoUnit;
use crate::proto::csi::v1::{
    ControllerExpandVolumeRequest, ControllerExpandVolumeResponse,
    ControllerGetCapabilitiesRequest, ControllerGetCapabilitiesResponse,
    ControllerGetVolumeRequest, ControllerGetVolumeResponse, ControllerModifyVolumeRequest,
    ControllerModifyVolumeResponse, ControllerPublishVolumeRequest,
    ControllerPublishVolumeResponse, ControllerServiceCapability, ControllerUnpublishVolumeRequest,
    ControllerUnpublishVolumeResponse, CreateSnapshotRequest, CreateSnapshotResponse,
    CreateVolumeRequest, CreateVolumeResponse, DeleteSnapshotRequest, DeleteSnapshotResponse,
    DeleteVolumeRequest, DeleteVolumeResponse, GetCapacityRequest, GetCapacityResponse,
    GetSnapshotRequest, GetSnapshotResponse, ListSnapshotsRequest, ListSnapshotsResponse,
    ListVolumesRequest, ListVolumesResponse, NodeExpandVolumeRequest, NodeExpandVolumeResponse,
    NodeGetCapabilitiesRequest, NodeGetCapabilitiesResponse, NodeGetInfoRequest,
    NodeGetInfoResponse, NodeGetVolumeStatsRequest, NodeGetVolumeStatsResponse,
    NodePublishVolumeRequest, NodePublishVolumeResponse, NodeStageVolumeRequest,
    NodeStageVolumeResponse, NodeUnpublishVolumeRequest, NodeUnpublishVolumeResponse,
    NodeUnstageVolumeRequest, NodeUnstageVolumeResponse, Snapshot,
    ValidateVolumeCapabilitiesRequest, ValidateVolumeCapabilitiesResponse, Volume, VolumeCondition,
    VolumeUsage,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::UnixListener;
use tokio::time::{Duration, sleep};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{Code, Request, Response, Status};

#[derive(Debug, Clone)]
enum RecordedCall {
    Stage(NodeStageVolumeRequest),
    Publish(NodePublishVolumeRequest),
    GetVolumeStats(NodeGetVolumeStatsRequest),
    Expand(NodeExpandVolumeRequest),
    Unpublish(NodeUnpublishVolumeRequest),
    Unstage(NodeUnstageVolumeRequest),
    CreateVolume(CreateVolumeRequest),
    CreateSnapshot(CreateSnapshotRequest),
    DeleteSnapshot(DeleteSnapshotRequest),
    ListSnapshots(ListSnapshotsRequest),
}

#[derive(Clone)]
struct FakeNodeService {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    volume_stats_response: NodeGetVolumeStatsResponse,
    publish_delay: Option<Duration>,
    node_capabilities_error: Option<Code>,
}

#[tonic::async_trait]
impl Node for FakeNodeService {
    async fn node_stage_volume(
        &self,
        request: Request<NodeStageVolumeRequest>,
    ) -> Result<Response<NodeStageVolumeResponse>, Status> {
        self.calls
            .lock()
            .expect("lock should be available")
            .push(RecordedCall::Stage(request.into_inner()));
        Ok(Response::new(NodeStageVolumeResponse {}))
    }

    async fn node_unstage_volume(
        &self,
        request: Request<NodeUnstageVolumeRequest>,
    ) -> Result<Response<NodeUnstageVolumeResponse>, Status> {
        self.calls
            .lock()
            .expect("lock should be available")
            .push(RecordedCall::Unstage(request.into_inner()));
        Ok(Response::new(NodeUnstageVolumeResponse {}))
    }

    async fn node_publish_volume(
        &self,
        request: Request<NodePublishVolumeRequest>,
    ) -> Result<Response<NodePublishVolumeResponse>, Status> {
        self.calls
            .lock()
            .expect("lock should be available")
            .push(RecordedCall::Publish(request.into_inner()));
        if let Some(delay) = self.publish_delay {
            sleep(delay).await;
        }
        Ok(Response::new(NodePublishVolumeResponse {}))
    }

    async fn node_unpublish_volume(
        &self,
        request: Request<NodeUnpublishVolumeRequest>,
    ) -> Result<Response<NodeUnpublishVolumeResponse>, Status> {
        self.calls
            .lock()
            .expect("lock should be available")
            .push(RecordedCall::Unpublish(request.into_inner()));
        Ok(Response::new(NodeUnpublishVolumeResponse {}))
    }

    async fn node_get_capabilities(
        &self,
        _request: Request<NodeGetCapabilitiesRequest>,
    ) -> Result<Response<NodeGetCapabilitiesResponse>, Status> {
        if let Some(code) = self.node_capabilities_error {
            return Err(Status::new(code, "injected failure"));
        }
        Ok(Response::new(NodeGetCapabilitiesResponse::default()))
    }

    async fn node_get_info(
        &self,
        _request: Request<NodeGetInfoRequest>,
    ) -> Result<Response<NodeGetInfoResponse>, Status> {
        Ok(Response::new(NodeGetInfoResponse::default()))
    }

    async fn node_get_volume_stats(
        &self,
        request: Request<NodeGetVolumeStatsRequest>,
    ) -> Result<Response<NodeGetVolumeStatsResponse>, Status> {
        self.calls
            .lock()
            .expect("lock should be available")
            .push(RecordedCall::GetVolumeStats(request.into_inner()));
        Ok(Response::new(self.volume_stats_response.clone()))
    }

    async fn node_expand_volume(
        &self,
        request: Request<NodeExpandVolumeRequest>,
    ) -> Result<Response<NodeExpandVolumeResponse>, Status> {
        self.calls
            .lock()
            .expect("lock should be available")
            .push(RecordedCall::Expand(request.into_inner()));
        Ok(Response::new(NodeExpandVolumeResponse {
            capacity_bytes: 4096,
        }))
    }
}

#[derive(Clone)]
struct FakeControllerService {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    capabilities: Vec<ControllerCapabilityType>,
}

#[tonic::async_trait]
impl CsiController for FakeControllerService {
    async fn create_volume(
        &self,
        request: Request<CreateVolumeRequest>,
    ) -> Result<Response<CreateVolumeResponse>, Status> {
        let request = request.into_inner();
        self.calls
            .lock()
            .expect("lock should be available")
            .push(RecordedCall::CreateVolume(request.clone()));
        Ok(Response::new(CreateVolumeResponse {
            volume: Some(Volume {
                volume_id: "volume-created".to_string(),
                capacity_bytes: request
                    .capacity_range
                    .as_ref()
                    .map(|range| range.required_bytes)
                    .unwrap_or_default(),
                volume_context: HashMap::new(),
                accessible_topology: Vec::new(),
                content_source: None,
            }),
        }))
    }

    async fn delete_volume(
        &self,
        _request: Request<DeleteVolumeRequest>,
    ) -> Result<Response<DeleteVolumeResponse>, Status> {
        Ok(Response::new(DeleteVolumeResponse {}))
    }

    async fn controller_publish_volume(
        &self,
        _request: Request<ControllerPublishVolumeRequest>,
    ) -> Result<Response<ControllerPublishVolumeResponse>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    async fn controller_unpublish_volume(
        &self,
        _request: Request<ControllerUnpublishVolumeRequest>,
    ) -> Result<Response<ControllerUnpublishVolumeResponse>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    async fn validate_volume_capabilities(
        &self,
        _request: Request<ValidateVolumeCapabilitiesRequest>,
    ) -> Result<Response<ValidateVolumeCapabilitiesResponse>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    async fn list_volumes(
        &self,
        _request: Request<ListVolumesRequest>,
    ) -> Result<Response<ListVolumesResponse>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    async fn get_capacity(
        &self,
        _request: Request<GetCapacityRequest>,
    ) -> Result<Response<GetCapacityResponse>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    async fn controller_get_capabilities(
        &self,
        _request: Request<ControllerGetCapabilitiesRequest>,
    ) -> Result<Response<ControllerGetCapabilitiesResponse>, Status> {
        Ok(Response::new(ControllerGetCapabilitiesResponse {
            capabilities: self
                .capabilities
                .iter()
                .map(|capability| ControllerServiceCapability {
                    r#type: Some(controller_service_capability::Type::Rpc(
                        controller_service_capability::Rpc {
                            r#type: *capability as i32,
                        },
                    )),
                })
                .collect(),
        }))
    }

    async fn create_snapshot(
        &self,
        request: Request<CreateSnapshotRequest>,
    ) -> Result<Response<CreateSnapshotResponse>, Status> {
        let request = request.into_inner();
        self.calls
            .lock()
            .expect("lock should be available")
            .push(RecordedCall::CreateSnapshot(request.clone()));
        Ok(Response::new(CreateSnapshotResponse {
            snapshot: Some(Snapshot {
                size_bytes: 4096,
                snapshot_id: "snapshot-created".to_string(),
                source_volume_id: request.source_volume_id,
                creation_time: Some(prost_types::Timestamp {
                    seconds: 123,
                    nanos: 456,
                }),
                ready_to_use: true,
                group_snapshot_id: String::new(),
            }),
        }))
    }

    async fn delete_snapshot(
        &self,
        request: Request<DeleteSnapshotRequest>,
    ) -> Result<Response<DeleteSnapshotResponse>, Status> {
        self.calls
            .lock()
            .expect("lock should be available")
            .push(RecordedCall::DeleteSnapshot(request.into_inner()));
        Ok(Response::new(DeleteSnapshotResponse {}))
    }

    async fn list_snapshots(
        &self,
        request: Request<ListSnapshotsRequest>,
    ) -> Result<Response<ListSnapshotsResponse>, Status> {
        self.calls
            .lock()
            .expect("lock should be available")
            .push(RecordedCall::ListSnapshots(request.into_inner()));
        Ok(Response::new(ListSnapshotsResponse {
            entries: vec![crate::proto::csi::v1::list_snapshots_response::Entry {
                snapshot: Some(Snapshot {
                    size_bytes: 1024,
                    snapshot_id: "snapshot-listed".to_string(),
                    source_volume_id: "volume-1".to_string(),
                    creation_time: Some(prost_types::Timestamp {
                        seconds: 100,
                        nanos: 0,
                    }),
                    ready_to_use: true,
                    group_snapshot_id: String::new(),
                }),
            }],
            next_token: "next".to_string(),
        }))
    }

    async fn get_snapshot(
        &self,
        _request: Request<GetSnapshotRequest>,
    ) -> Result<Response<GetSnapshotResponse>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    async fn controller_expand_volume(
        &self,
        _request: Request<ControllerExpandVolumeRequest>,
    ) -> Result<Response<ControllerExpandVolumeResponse>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    async fn controller_get_volume(
        &self,
        _request: Request<ControllerGetVolumeRequest>,
    ) -> Result<Response<ControllerGetVolumeResponse>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    async fn controller_modify_volume(
        &self,
        _request: Request<ControllerModifyVolumeRequest>,
    ) -> Result<Response<ControllerModifyVolumeResponse>, Status> {
        Err(Status::unimplemented("not implemented"))
    }
}

async fn spawn_node_server_with_volume_stats(
    volume_stats_response: NodeGetVolumeStatsResponse,
    publish_delay: Option<Duration>,
    node_capabilities_error: Option<Code>,
) -> (String, Arc<Mutex<Vec<RecordedCall>>>) {
    let socket_path = std::env::temp_dir().join(format!(
        "tugboat-csi-operator-{}.sock",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).expect("listener should bind");
    let incoming = UnixListenerStream::new(listener);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let service = FakeNodeService {
        calls: calls.clone(),
        volume_stats_response,
        publish_delay,
        node_capabilities_error,
    };
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(NodeServer::new(service))
            .serve_with_incoming(incoming)
            .await
            .expect("server should run");
    });
    sleep(Duration::from_millis(50)).await;
    (socket_path.display().to_string(), calls)
}

async fn spawn_node_server() -> (String, Arc<Mutex<Vec<RecordedCall>>>) {
    spawn_node_server_with_volume_stats(NodeGetVolumeStatsResponse::default(), None, None).await
}

async fn spawn_controller_server(
    capabilities: Vec<ControllerCapabilityType>,
) -> (String, Arc<Mutex<Vec<RecordedCall>>>) {
    let socket_path = std::env::temp_dir().join(format!(
        "tugboat-csi-controller-{}.sock",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).expect("listener should bind");
    let incoming = UnixListenerStream::new(listener);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let service = FakeControllerService {
        calls: calls.clone(),
        capabilities,
    };
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(ControllerServer::new(service))
            .serve_with_incoming(incoming)
            .await
            .expect("server should run");
    });
    sleep(Duration::from_millis(50)).await;
    (socket_path.display().to_string(), calls)
}

#[tokio::test]
async fn create_snapshot_requires_controller_capability_and_sends_request() {
    let operator = TugboatCsiOperator::default();
    let (socket_path, calls) =
        spawn_controller_server(vec![ControllerCapabilityType::CreateDeleteSnapshot]).await;

    let snapshot = operator
        .create_snapshot(
            &socket_path,
            "volume-1".to_string(),
            "snapshot-1".to_string(),
            HashMap::from([("policy".to_string(), "daily".to_string())]),
            HashMap::from([("token".to_string(), "secret".to_string())]),
        )
        .await
        .expect("snapshot creation should succeed");

    assert_eq!(snapshot.snapshot_id, "snapshot-created");
    assert_eq!(snapshot.source_volume_id, "volume-1");
    assert_eq!(snapshot.size_bytes, Some(4096));
    assert_eq!(snapshot.creation_time_seconds, 123);
    assert!(snapshot.ready_to_use);

    let calls = calls.lock().expect("lock should be available").clone();
    let Some(RecordedCall::CreateSnapshot(request)) = calls.last() else {
        panic!("last call should be create snapshot");
    };
    assert_eq!(request.source_volume_id, "volume-1");
    assert_eq!(request.name, "snapshot-1");
    assert_eq!(request.parameters.get("policy"), Some(&"daily".to_string()));
    assert_eq!(request.secrets.get("token"), Some(&"secret".to_string()));
}

#[tokio::test]
async fn create_snapshot_fails_before_rpc_without_capability() {
    let operator = TugboatCsiOperator::default();
    let (socket_path, calls) = spawn_controller_server(Vec::new()).await;

    let err = operator
        .create_snapshot(
            &socket_path,
            "volume-1".to_string(),
            "snapshot-1".to_string(),
            HashMap::new(),
            HashMap::new(),
        )
        .await
        .expect_err("missing capability should fail");

    assert!(matches!(
        err,
        Error::UnsupportedControllerCapability(ControllerCapability::CreateDeleteSnapshot)
    ));
    assert!(calls.lock().expect("lock should be available").is_empty());
}

#[tokio::test]
async fn delete_and_list_snapshot_send_requests() {
    let operator = TugboatCsiOperator::default();
    let (socket_path, calls) = spawn_controller_server(vec![
        ControllerCapabilityType::CreateDeleteSnapshot,
        ControllerCapabilityType::ListSnapshots,
    ])
    .await;

    operator
        .delete_snapshot(
            &socket_path,
            "snapshot-1".to_string(),
            HashMap::from([("token".to_string(), "secret".to_string())]),
        )
        .await
        .expect("snapshot deletion should succeed");
    let listed = operator
        .list_snapshots(
            &socket_path,
            Some("snapshot-1".to_string()),
            Some("volume-1".to_string()),
            None,
            HashMap::new(),
        )
        .await
        .expect("list snapshots should succeed");

    assert_eq!(listed.next_token, "next");
    assert_eq!(listed.entries[0].snapshot_id, "snapshot-listed");
    let calls = calls.lock().expect("lock should be available").clone();
    let RecordedCall::DeleteSnapshot(delete_request) = &calls[0] else {
        panic!("first call should be delete snapshot");
    };
    assert_eq!(delete_request.snapshot_id, "snapshot-1");
    let RecordedCall::ListSnapshots(list_request) = &calls[1] else {
        panic!("second call should be list snapshots");
    };
    assert_eq!(list_request.snapshot_id, "snapshot-1");
    assert_eq!(list_request.source_volume_id, "volume-1");
}

#[tokio::test]
async fn create_volume_with_snapshot_source_sets_content_source() {
    let operator = TugboatCsiOperator::default();
    let (socket_path, calls) =
        spawn_controller_server(vec![ControllerCapabilityType::CreateDeleteSnapshot]).await;

    operator
        .create_volume_with_source(
            &socket_path,
            "volume-from-snapshot".to_string(),
            Some(1024),
            HashMap::new(),
            vec![CsiAccessMode::ReadWriteOnce],
            CsiAccessType::Filesystem,
            HashMap::new(),
            Vec::new(),
            Vec::new(),
            Some(CsiVolumeContentSource::Snapshot {
                snapshot_id: "snapshot-1".to_string(),
            }),
        )
        .await
        .expect("volume creation from snapshot should succeed");

    let calls = calls.lock().expect("lock should be available").clone();
    let Some(RecordedCall::CreateVolume(request)) = calls.last() else {
        panic!("last call should be create volume");
    };
    assert!(matches!(
        request
            .volume_content_source
            .as_ref()
            .and_then(|source| source.r#type.as_ref()),
        Some(volume_content_source::Type::Snapshot(source)) if source.snapshot_id == "snapshot-1"
    ));
}

#[test]
fn filesystem_access_type_is_encoded_as_mount_volume() {
    let capability = volume_capability(
        CsiAccessMode::ReadWriteOnce,
        CsiAccessType::Filesystem,
        None,
        Vec::new(),
    );
    assert!(matches!(capability.access_type, Some(AccessType::Mount(_))));
}

#[test]
fn block_access_type_is_encoded_as_block_volume() {
    let capability = volume_capability(
        CsiAccessMode::ReadWriteOnce,
        CsiAccessType::Block,
        None,
        vec!["ignored".to_string()],
    );
    assert!(matches!(capability.access_type, Some(AccessType::Block(_))));
}

#[test]
fn filesystem_mount_flags_are_encoded() {
    let capability = volume_capability(
        CsiAccessMode::ReadWriteOnce,
        CsiAccessType::Filesystem,
        Some("xfs".to_string()),
        vec!["noatime".to_string(), "nodiratime".to_string()],
    );
    let Some(AccessType::Mount(mount)) = capability.access_type else {
        panic!("expected mount access type");
    };
    assert_eq!(mount.mount_flags, vec!["noatime", "nodiratime"]);
}

#[tokio::test]
async fn can_stage_and_publish_volume_over_uds() {
    let operator = TugboatCsiOperator::default();
    let (socket_path, calls) = spawn_node_server().await;
    let publish_context = HashMap::from([("published".to_string(), "yes".to_string())]);
    let volume_context = HashMap::from([("volume".to_string(), "ctx".to_string())]);

    operator
        .stage(
            &socket_path,
            "volume-1".to_string(),
            "/staging/volume-1".to_string(),
            CsiAccessMode::ReadWriteOnce,
            CsiAccessType::Filesystem,
            Some("xfs".to_string()),
            vec!["noatime".to_string()],
            HashMap::from([("token".to_string(), "secret".to_string())]),
            volume_context.clone(),
            publish_context.clone(),
        )
        .await
        .expect("stage should succeed");
    operator
        .publish(
            &socket_path,
            "volume-1".to_string(),
            "/publish/volume-1".to_string(),
            false,
            CsiAccessMode::ReadWriteOnce,
            CsiAccessType::Filesystem,
            Some("xfs".to_string()),
            vec!["noatime".to_string()],
            Some("/staging/volume-1".to_string()),
            HashMap::from([("token".to_string(), "secret".to_string())]),
            volume_context.clone(),
            publish_context.clone(),
        )
        .await
        .expect("publish should succeed");

    let calls = calls.lock().expect("lock should be available").clone();
    assert_eq!(calls.len(), 2);

    let RecordedCall::Stage(stage_request) = &calls[0] else {
        panic!("first call should be stage");
    };
    assert_eq!(stage_request.staging_target_path, "/staging/volume-1");
    assert_eq!(
        stage_request.secrets.get("token"),
        Some(&"secret".to_string())
    );
    assert_eq!(stage_request.publish_context, publish_context);
    assert_eq!(stage_request.volume_context, volume_context);
    assert!(matches!(
        stage_request
            .volume_capability
            .as_ref()
            .and_then(|capability| capability.access_type.clone()),
        Some(AccessType::Mount(_))
    ));
    let Some(AccessType::Mount(stage_mount)) = stage_request
        .volume_capability
        .as_ref()
        .and_then(|capability| capability.access_type.clone())
    else {
        panic!("stage capability should use mount access type");
    };
    assert_eq!(stage_mount.fs_type, "xfs");
    assert_eq!(stage_mount.mount_flags, vec!["noatime"]);

    let RecordedCall::Publish(publish_request) = &calls[1] else {
        panic!("second call should be publish");
    };
    assert_eq!(publish_request.target_path, "/publish/volume-1");
    assert_eq!(publish_request.staging_target_path, "/staging/volume-1");
    assert_eq!(
        publish_request.secrets.get("token"),
        Some(&"secret".to_string())
    );
    let Some(AccessType::Mount(publish_mount)) = publish_request
        .volume_capability
        .as_ref()
        .and_then(|capability| capability.access_type.clone())
    else {
        panic!("publish capability should use mount access type");
    };
    assert_eq!(publish_mount.fs_type, "xfs");
    assert_eq!(publish_mount.mount_flags, vec!["noatime"]);
}

#[tokio::test]
async fn can_query_volume_stats_over_uds() {
    let operator = TugboatCsiOperator::default();
    let (socket_path, calls) = spawn_node_server_with_volume_stats(
        NodeGetVolumeStatsResponse {
            usage: vec![
                VolumeUsage {
                    available: 3072,
                    total: 4096,
                    used: 1024,
                    unit: VolumeUsageProtoUnit::Bytes as i32,
                },
                VolumeUsage {
                    available: 90,
                    total: 100,
                    used: 10,
                    unit: VolumeUsageProtoUnit::Inodes as i32,
                },
            ],
            volume_condition: Some(VolumeCondition {
                abnormal: true,
                message: "filesystem is read-only".to_string(),
            }),
        },
        None,
        None,
    )
    .await;

    let stats = operator
        .node_volume_stats(
            &socket_path,
            "volume-1".to_string(),
            "/publish/volume-1".to_string(),
            Some("/staging/volume-1".to_string()),
        )
        .await
        .expect("stats query should succeed");

    assert_eq!(
        stats,
        NodeVolumeStats {
            usage: vec![
                VolumeUsageStats {
                    available: Some(3072),
                    total: 4096,
                    used: Some(1024),
                    unit: VolumeUsageUnit::Bytes,
                },
                VolumeUsageStats {
                    available: Some(90),
                    total: 100,
                    used: Some(10),
                    unit: VolumeUsageUnit::Inodes,
                },
            ],
            condition: Some(VolumeHealthCondition {
                abnormal: true,
                message: "filesystem is read-only".to_string(),
            }),
        }
    );

    let calls = calls.lock().expect("lock should be available").clone();
    assert_eq!(calls.len(), 1);
    let RecordedCall::GetVolumeStats(stats_request) = &calls[0] else {
        panic!("first call should be get volume stats");
    };
    assert_eq!(stats_request.volume_id, "volume-1");
    assert_eq!(stats_request.volume_path, "/publish/volume-1");
    assert_eq!(stats_request.staging_target_path, "/staging/volume-1");
}

#[tokio::test]
async fn publish_times_out_when_driver_stalls() {
    let operator = TugboatCsiOperator::default();
    let (socket_path, _calls) = spawn_node_server_with_volume_stats(
        NodeGetVolumeStatsResponse::default(),
        Some(Duration::from_secs(1)),
        None,
    )
    .await;

    let result = operator
        .publish(
            &socket_path,
            "volume-1".to_string(),
            "/publish/volume-1".to_string(),
            false,
            CsiAccessMode::ReadWriteOnce,
            CsiAccessType::Filesystem,
            Some("xfs".to_string()),
            vec!["noatime".to_string()],
            Some("/staging/volume-1".to_string()),
            HashMap::from([("token".to_string(), "secret".to_string())]),
            HashMap::new(),
            HashMap::new(),
        )
        .await;

    assert!(matches!(result, Err(Error::RpcTimeout)));
}

#[test]
fn normalize_socket_path_rejects_empty_path() {
    let result = normalize_socket_path("unix://");

    assert!(matches!(result, Err(Error::InvalidSocketPath(_))));
}

#[test]
fn normalize_socket_path_strips_unix_prefix() {
    let result = normalize_socket_path("unix:///var/run/csi.sock");

    assert_eq!(result.expect("path should normalize"), "/var/run/csi.sock");
}

#[test]
fn normalize_socket_path_rejects_relative_path() {
    let result = normalize_socket_path("./csi.sock");

    assert!(matches!(result, Err(Error::InvalidSocketPath(_))));
}

#[test]
fn normalize_socket_path_rejects_unsupported_scheme() {
    let result = normalize_socket_path("tcp://127.0.0.1:9000");

    assert!(matches!(result, Err(Error::InvalidSocketPath(_))));
}

#[tokio::test]
async fn create_volume_rejects_empty_name() {
    let operator = TugboatCsiOperator::default();
    let err = operator
        .create_volume(
            "/var/run/non-existent-csi.sock",
            "   ".to_string(),
            Some(1024),
            HashMap::new(),
            vec![CsiAccessMode::ReadWriteOnce],
            CsiAccessType::Filesystem,
            HashMap::new(),
            Vec::new(),
            Vec::new(),
        )
        .await
        .expect_err("empty volume name should fail before RPC");

    assert!(matches!(err, Error::InvalidVolumeName(_)));
}

#[tokio::test]
async fn create_volume_rejects_empty_access_modes() {
    let operator = TugboatCsiOperator::default();
    let err = operator
        .create_volume(
            "/var/run/non-existent-csi.sock",
            "volume-1".to_string(),
            Some(1024),
            HashMap::new(),
            Vec::new(),
            CsiAccessType::Filesystem,
            HashMap::new(),
            Vec::new(),
            Vec::new(),
        )
        .await
        .expect_err("missing access modes should fail before RPC");

    assert!(matches!(err, Error::MissingAccessModes));
}

#[tokio::test]
async fn create_volume_rejects_non_positive_capacity() {
    let operator = TugboatCsiOperator::default();
    let err = operator
        .create_volume(
            "/var/run/non-existent-csi.sock",
            "volume-1".to_string(),
            Some(0),
            HashMap::new(),
            vec![CsiAccessMode::ReadWriteOnce],
            CsiAccessType::Filesystem,
            HashMap::new(),
            Vec::new(),
            Vec::new(),
        )
        .await
        .expect_err("non-positive capacity should fail before RPC");

    assert!(matches!(err, Error::InvalidCapacityBytes(0)));
}

#[tokio::test]
async fn node_expand_rejects_non_positive_capacity() {
    let operator = TugboatCsiOperator::default();
    let err = operator
        .node_expand(
            "/var/run/non-existent-csi.sock",
            "volume-1".to_string(),
            "/publish/volume-1".to_string(),
            0,
            None,
            CsiAccessMode::ReadWriteOnce,
            CsiAccessType::Filesystem,
            None,
            HashMap::new(),
        )
        .await
        .expect_err("non-positive expansion capacity should fail before RPC");

    assert!(matches!(err, Error::InvalidCapacityBytes(0)));
}

#[tokio::test]
async fn can_unpublish_and_unstage_volume_over_uds() {
    let operator = TugboatCsiOperator::default();
    let (socket_path, calls) = spawn_node_server().await;

    operator
        .unpublish(
            &socket_path,
            "volume-1".to_string(),
            "/publish/volume-1".to_string(),
        )
        .await
        .expect("unpublish should succeed");
    operator
        .unstage(
            &socket_path,
            "volume-1".to_string(),
            "/staging/volume-1".to_string(),
        )
        .await
        .expect("unstage should succeed");

    let calls = calls.lock().expect("lock should be available").clone();
    assert_eq!(calls.len(), 2);

    let RecordedCall::Unpublish(unpublish_request) = &calls[0] else {
        panic!("first call should be unpublish");
    };
    assert_eq!(unpublish_request.target_path, "/publish/volume-1");

    let RecordedCall::Unstage(unstage_request) = &calls[1] else {
        panic!("second call should be unstage");
    };
    assert_eq!(unstage_request.staging_target_path, "/staging/volume-1");
}

#[tokio::test]
async fn can_expand_volume_over_uds() {
    let operator = TugboatCsiOperator::default();
    let (socket_path, calls) = spawn_node_server().await;

    let capacity = operator
        .node_expand(
            &socket_path,
            "volume-1".to_string(),
            "/publish/volume-1".to_string(),
            4096,
            Some("/staging/volume-1".to_string()),
            CsiAccessMode::ReadWriteOnce,
            CsiAccessType::Filesystem,
            Some("xfs".to_string()),
            HashMap::from([("token".to_string(), "secret".to_string())]),
        )
        .await
        .expect("node expand should succeed");

    assert_eq!(capacity, 4096);
    let calls = calls.lock().expect("lock should be available").clone();
    let Some(RecordedCall::Expand(request)) = calls.last() else {
        panic!("last call should be node expand");
    };
    assert_eq!(request.volume_path, "/publish/volume-1");
    assert_eq!(request.staging_target_path, "/staging/volume-1");
    assert_eq!(
        request
            .capacity_range
            .as_ref()
            .map(|range| range.required_bytes),
        Some(4096)
    );
}

#[tokio::test]
async fn node_capabilities_returns_empty_on_unimplemented() {
    let operator = TugboatCsiOperator::default();
    let (socket_path, _calls) = spawn_node_server_with_volume_stats(
        NodeGetVolumeStatsResponse::default(),
        None,
        Some(Code::Unimplemented),
    )
    .await;

    let capabilities = operator
        .node_capabilities(&socket_path)
        .await
        .expect("unimplemented node capabilities should be treated as empty");

    assert!(capabilities.is_empty());
}
