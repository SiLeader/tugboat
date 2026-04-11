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
    CsiAccessMode, CsiAccessType, NodeVolumeStats, TugboatCsiOperator, VolumeHealthCondition,
    VolumeUsageStats, VolumeUsageUnit, volume_capability,
};
use crate::error::Error;
use crate::proto::csi::v1::node_server::{Node, NodeServer};
use crate::proto::csi::v1::volume_capability::AccessType;
use crate::proto::csi::v1::volume_usage::Unit as VolumeUsageProtoUnit;
use crate::proto::csi::v1::{
    NodeExpandVolumeRequest, NodeExpandVolumeResponse, NodeGetCapabilitiesRequest,
    NodeGetCapabilitiesResponse, NodeGetInfoRequest, NodeGetInfoResponse,
    NodeGetVolumeStatsRequest, NodeGetVolumeStatsResponse, NodePublishVolumeRequest,
    NodePublishVolumeResponse, NodeStageVolumeRequest, NodeStageVolumeResponse,
    NodeUnpublishVolumeRequest, NodeUnpublishVolumeResponse, NodeUnstageVolumeRequest,
    NodeUnstageVolumeResponse, VolumeCondition, VolumeUsage,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::UnixListener;
use tokio::time::{Duration, sleep};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{Request, Response, Status};

#[derive(Debug, Clone)]
enum RecordedCall {
    Stage(NodeStageVolumeRequest),
    Publish(NodePublishVolumeRequest),
    GetVolumeStats(NodeGetVolumeStatsRequest),
    Expand(NodeExpandVolumeRequest),
    Unpublish(NodeUnpublishVolumeRequest),
    Unstage(NodeUnstageVolumeRequest),
}

#[derive(Clone)]
struct FakeNodeService {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    volume_stats_response: NodeGetVolumeStatsResponse,
    publish_delay: Option<Duration>,
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

async fn spawn_node_server_with_volume_stats(
    volume_stats_response: NodeGetVolumeStatsResponse,
    publish_delay: Option<Duration>,
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
    spawn_node_server_with_volume_stats(NodeGetVolumeStatsResponse::default(), None).await
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
