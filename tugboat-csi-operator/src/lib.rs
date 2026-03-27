use crate::proto::csi::v1::controller_client::ControllerClient;
use crate::proto::csi::v1::controller_service_capability;
use crate::proto::csi::v1::controller_service_capability::rpc::Type as ControllerServiceCapabilityType;
use crate::proto::csi::v1::node_client::NodeClient;
use crate::proto::csi::v1::node_service_capability;
use crate::proto::csi::v1::node_service_capability::rpc::Type as NodeServiceCapabilityType;
use crate::proto::csi::v1::volume_capability::access_mode::Mode;
use crate::proto::csi::v1::volume_capability::{AccessMode, AccessType, BlockVolume, MountVolume};
use crate::proto::csi::v1::volume_usage::Unit as VolumeUsageProtoUnit;
use crate::proto::csi::v1::{
    CapacityRange, ControllerExpandVolumeRequest, ControllerGetCapabilitiesRequest,
    ControllerPublishVolumeRequest, ControllerUnpublishVolumeRequest, CreateVolumeRequest,
    DeleteVolumeRequest, NodeExpandVolumeRequest, NodeGetCapabilitiesRequest,
    NodeGetVolumeStatsRequest, NodePublishVolumeRequest, NodeStageVolumeRequest,
    NodeUnpublishVolumeRequest, NodeUnstageVolumeRequest, VolumeCapability,
};
pub use error::Error;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::io;
use tokio::net::UnixStream;
use tonic::Code;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

mod error;
mod proto;

#[derive(Clone, Default)]
pub struct TugboatCsiOperator {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsiAccessMode {
    ReadOnlyMany,
    ReadWriteOnce,
    ReadWriteMany,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsiAccessType {
    Block,
    Filesystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCapability {
    StageUnstageVolume,
    GetVolumeStats,
    ExpandVolume,
    VolumeCondition,
    SingleNodeMultiWriter,
    VolumeMountGroup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerCapability {
    PublishUnpublishVolume,
    PublishReadonly,
    ExpandVolume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedVolume {
    pub volume_id: String,
    pub capacity_bytes: i64,
    pub volume_context: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerExpandedVolume {
    pub capacity_bytes: i64,
    pub node_expansion_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeUsageUnit {
    Unknown,
    Bytes,
    Inodes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeUsageStats {
    pub available: Option<i64>,
    pub total: i64,
    pub used: Option<i64>,
    pub unit: VolumeUsageUnit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeHealthCondition {
    pub abnormal: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeVolumeStats {
    pub usage: Vec<VolumeUsageStats>,
    pub condition: Option<VolumeHealthCondition>,
}

impl TugboatCsiOperator {
    pub async fn create_volume(
        &self,
        socket_path: &str,
        name: String,
        capacity_bytes: Option<i64>,
        parameters: HashMap<String, String>,
        access_modes: Vec<CsiAccessMode>,
        access_type: CsiAccessType,
        secrets: HashMap<String, String>,
        mount_flags: Vec<String>,
    ) -> Result<ProvisionedVolume, error::Error> {
        let req = CreateVolumeRequest {
            name,
            capacity_range: capacity_bytes.map(|required_bytes| CapacityRange {
                required_bytes,
                limit_bytes: 0,
            }),
            volume_capabilities: access_modes
                .into_iter()
                .map(|access_mode| {
                    volume_capability(access_mode, access_type, None, mount_flags.clone())
                })
                .collect(),
            parameters,
            secrets,
            volume_content_source: None,
            accessibility_requirements: None,
            mutable_parameters: Default::default(),
        };

        let mut client = connect_controller_client(socket_path).await?;
        let response = client
            .create_volume(req)
            .await
            .map_err(map_controller_grpc_error)?
            .into_inner();
        let volume = response.volume.ok_or(error::Error::MissingVolume)?;

        Ok(ProvisionedVolume {
            volume_id: volume.volume_id,
            capacity_bytes: volume.capacity_bytes,
            volume_context: volume.volume_context,
        })
    }

    pub async fn delete_volume(
        &self,
        socket_path: &str,
        volume_id: String,
        secrets: HashMap<String, String>,
    ) -> Result<(), error::Error> {
        let req = DeleteVolumeRequest { volume_id, secrets };

        let mut client = connect_controller_client(socket_path).await?;
        client
            .delete_volume(req)
            .await
            .map(|_| ())
            .map_err(map_controller_grpc_error)
    }

    pub async fn controller_capabilities(
        &self,
        socket_path: &str,
    ) -> Result<Vec<ControllerCapability>, error::Error> {
        let mut client = connect_controller_client(socket_path).await?;
        let response = match client
            .controller_get_capabilities(ControllerGetCapabilitiesRequest {})
            .await
        {
            Ok(response) => response.into_inner(),
            Err(status) if status.code() == Code::Unimplemented => return Ok(Vec::new()),
            Err(status) => return Err(map_controller_grpc_error(status)),
        };

        Ok(response
            .capabilities
            .into_iter()
            .filter_map(|capability| match capability.r#type {
                Some(controller_service_capability::Type::Rpc(rpc)) => {
                    match ControllerServiceCapabilityType::try_from(rpc.r#type).ok()? {
                        ControllerServiceCapabilityType::Unknown => None,
                        ControllerServiceCapabilityType::PublishUnpublishVolume => {
                            Some(ControllerCapability::PublishUnpublishVolume)
                        }
                        ControllerServiceCapabilityType::PublishReadonly => {
                            Some(ControllerCapability::PublishReadonly)
                        }
                        ControllerServiceCapabilityType::ExpandVolume => {
                            Some(ControllerCapability::ExpandVolume)
                        }
                        _ => None,
                    }
                }
                None => None,
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn controller_publish(
        &self,
        socket_path: &str,
        volume_id: String,
        node_id: String,
        read_only: bool,
        access_mode: CsiAccessMode,
        access_type: CsiAccessType,
        fs_type: Option<String>,
        secrets: HashMap<String, String>,
        volume_context: HashMap<String, String>,
    ) -> Result<HashMap<String, String>, error::Error> {
        let req = ControllerPublishVolumeRequest {
            volume_id,
            node_id,
            volume_capability: Some(volume_capability(
                access_mode,
                access_type,
                fs_type,
                Vec::new(),
            )),
            readonly: read_only,
            secrets,
            volume_context,
        };

        let mut client = connect_controller_client(socket_path).await?;
        let response = client
            .controller_publish_volume(req)
            .await
            .map_err(map_controller_grpc_error)?
            .into_inner();
        Ok(response.publish_context)
    }

    pub async fn controller_unpublish(
        &self,
        socket_path: &str,
        volume_id: String,
        node_id: String,
        secrets: HashMap<String, String>,
    ) -> Result<(), error::Error> {
        let req = ControllerUnpublishVolumeRequest {
            volume_id,
            node_id,
            secrets,
        };

        let mut client = connect_controller_client(socket_path).await?;
        client
            .controller_unpublish_volume(req)
            .await
            .map(|_| ())
            .map_err(map_controller_grpc_error)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn controller_expand(
        &self,
        socket_path: &str,
        volume_id: String,
        capacity_bytes: i64,
        access_mode: CsiAccessMode,
        access_type: CsiAccessType,
        fs_type: Option<String>,
        mount_flags: Vec<String>,
        secrets: HashMap<String, String>,
    ) -> Result<ControllerExpandedVolume, error::Error> {
        let req = ControllerExpandVolumeRequest {
            volume_id,
            capacity_range: Some(CapacityRange {
                required_bytes: capacity_bytes,
                limit_bytes: 0,
            }),
            secrets,
            volume_capability: Some(volume_capability(
                access_mode,
                access_type,
                fs_type,
                mount_flags,
            )),
        };

        let mut client = connect_controller_client(socket_path).await?;
        let response = client
            .controller_expand_volume(req)
            .await
            .map_err(map_controller_grpc_error)?
            .into_inner();
        Ok(ControllerExpandedVolume {
            capacity_bytes: response.capacity_bytes,
            node_expansion_required: response.node_expansion_required,
        })
    }

    pub async fn node_capabilities(
        &self,
        socket_path: &str,
    ) -> Result<Vec<NodeCapability>, error::Error> {
        let mut client = connect_node_client(socket_path).await?;
        let response = client
            .node_get_capabilities(NodeGetCapabilitiesRequest {})
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        Ok(response
            .capabilities
            .into_iter()
            .filter_map(|capability| match capability.r#type {
                Some(node_service_capability::Type::Rpc(rpc)) => {
                    match NodeServiceCapabilityType::try_from(rpc.r#type).ok()? {
                        NodeServiceCapabilityType::Unknown => None,
                        NodeServiceCapabilityType::StageUnstageVolume => {
                            Some(NodeCapability::StageUnstageVolume)
                        }
                        NodeServiceCapabilityType::GetVolumeStats => {
                            Some(NodeCapability::GetVolumeStats)
                        }
                        NodeServiceCapabilityType::ExpandVolume => {
                            Some(NodeCapability::ExpandVolume)
                        }
                        NodeServiceCapabilityType::VolumeCondition => {
                            Some(NodeCapability::VolumeCondition)
                        }
                        NodeServiceCapabilityType::SingleNodeMultiWriter => {
                            Some(NodeCapability::SingleNodeMultiWriter)
                        }
                        NodeServiceCapabilityType::VolumeMountGroup => {
                            Some(NodeCapability::VolumeMountGroup)
                        }
                    }
                }
                None => None,
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn publish(
        &self,
        socket_path: &str,
        volume_id: String,
        target_path: String,
        read_only: bool,
        access_mode: CsiAccessMode,
        access_type: CsiAccessType,
        fs_type: Option<String>,
        mount_flags: Vec<String>,
        staging_target_path: Option<String>,
        secrets: HashMap<String, String>,
        volume_context: HashMap<String, String>,
        publish_context: HashMap<String, String>,
    ) -> Result<(), error::Error> {
        let req = NodePublishVolumeRequest {
            volume_id,
            target_path,
            volume_capability: Some(volume_capability(
                access_mode,
                access_type,
                fs_type,
                mount_flags,
            )),
            readonly: read_only,
            secrets,
            volume_context,
            publish_context,
            staging_target_path: staging_target_path.unwrap_or_default(),
        };

        let mut client = connect_node_client(socket_path).await?;
        client
            .node_publish_volume(req)
            .await
            .map(|_| ())
            .map_err(map_grpc_error)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn stage(
        &self,
        socket_path: &str,
        volume_id: String,
        staging_target_path: String,
        access_mode: CsiAccessMode,
        access_type: CsiAccessType,
        fs_type: Option<String>,
        mount_flags: Vec<String>,
        secrets: HashMap<String, String>,
        volume_context: HashMap<String, String>,
        publish_context: HashMap<String, String>,
    ) -> Result<(), error::Error> {
        let req = NodeStageVolumeRequest {
            volume_id,
            publish_context,
            staging_target_path,
            volume_capability: Some(volume_capability(
                access_mode,
                access_type,
                fs_type,
                mount_flags,
            )),
            secrets,
            volume_context,
        };

        let mut client = connect_node_client(socket_path).await?;
        client
            .node_stage_volume(req)
            .await
            .map(|_| ())
            .map_err(map_grpc_error)
    }

    pub async fn unpublish(
        &self,
        socket_path: &str,
        volume_id: String,
        target_path: String,
    ) -> Result<(), error::Error> {
        let req = NodeUnpublishVolumeRequest {
            volume_id,
            target_path,
        };

        let mut client = connect_node_client(socket_path).await?;
        client
            .node_unpublish_volume(req)
            .await
            .map(|_| ())
            .map_err(map_grpc_error)
    }

    pub async fn unstage(
        &self,
        socket_path: &str,
        volume_id: String,
        staging_target_path: String,
    ) -> Result<(), error::Error> {
        let req = NodeUnstageVolumeRequest {
            volume_id,
            staging_target_path,
        };

        let mut client = connect_node_client(socket_path).await?;
        client
            .node_unstage_volume(req)
            .await
            .map(|_| ())
            .map_err(map_grpc_error)
    }

    pub async fn node_volume_stats(
        &self,
        socket_path: &str,
        volume_id: String,
        volume_path: String,
        staging_target_path: Option<String>,
    ) -> Result<NodeVolumeStats, error::Error> {
        let req = NodeGetVolumeStatsRequest {
            volume_id,
            volume_path,
            staging_target_path: staging_target_path.unwrap_or_default(),
        };

        let mut client = connect_node_client(socket_path).await?;
        let response = client
            .node_get_volume_stats(req)
            .await
            .map_err(map_grpc_error)?
            .into_inner();
        Ok(NodeVolumeStats {
            usage: response
                .usage
                .into_iter()
                .map(|usage| VolumeUsageStats {
                    available: Some(usage.available),
                    total: usage.total,
                    used: Some(usage.used),
                    unit: match VolumeUsageProtoUnit::try_from(usage.unit).ok() {
                        Some(VolumeUsageProtoUnit::Bytes) => VolumeUsageUnit::Bytes,
                        Some(VolumeUsageProtoUnit::Inodes) => VolumeUsageUnit::Inodes,
                        _ => VolumeUsageUnit::Unknown,
                    },
                })
                .collect(),
            condition: response
                .volume_condition
                .map(|condition| VolumeHealthCondition {
                    abnormal: condition.abnormal,
                    message: condition.message,
                }),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn node_expand(
        &self,
        socket_path: &str,
        volume_id: String,
        volume_path: String,
        capacity_bytes: i64,
        staging_target_path: Option<String>,
        access_mode: CsiAccessMode,
        access_type: CsiAccessType,
        fs_type: Option<String>,
        secrets: HashMap<String, String>,
    ) -> Result<i64, error::Error> {
        let req = NodeExpandVolumeRequest {
            volume_id,
            volume_path,
            capacity_range: Some(CapacityRange {
                required_bytes: capacity_bytes,
                limit_bytes: 0,
            }),
            staging_target_path: staging_target_path.unwrap_or_default(),
            volume_capability: Some(volume_capability(
                access_mode,
                access_type,
                fs_type,
                Vec::new(),
            )),
            secrets,
        };

        let mut client = connect_node_client(socket_path).await?;
        let response = client
            .node_expand_volume(req)
            .await
            .map_err(map_grpc_error)?
            .into_inner();
        Ok(response.capacity_bytes)
    }
}

fn volume_capability(
    access_mode: CsiAccessMode,
    access_type: CsiAccessType,
    fs_type: Option<String>,
    mount_flags: Vec<String>,
) -> VolumeCapability {
    VolumeCapability {
        access_mode: Some(AccessMode {
            mode: match access_mode {
                CsiAccessMode::ReadOnlyMany => Mode::MultiNodeReaderOnly,
                CsiAccessMode::ReadWriteOnce => Mode::SingleNodeWriter,
                CsiAccessMode::ReadWriteMany => Mode::MultiNodeMultiWriter,
            } as i32,
        }),
        access_type: Some(match access_type {
            CsiAccessType::Block => AccessType::Block(BlockVolume {}),
            CsiAccessType::Filesystem => AccessType::Mount(MountVolume {
                fs_type: fs_type.unwrap_or_default(),
                mount_flags,
                volume_mount_group: String::new(),
            }),
        }),
    }
}

async fn connect_node_client(socket_path: &str) -> Result<NodeClient<Channel>, error::Error> {
    Ok(NodeClient::new(connect_channel(socket_path).await?))
}

async fn connect_controller_client(
    socket_path: &str,
) -> Result<ControllerClient<Channel>, error::Error> {
    Ok(ControllerClient::new(connect_channel(socket_path).await?))
}

async fn connect_channel(socket_path: &str) -> Result<Channel, error::Error> {
    let socket_path = normalize_socket_path(socket_path);
    let channel = Endpoint::try_from("http://[::]:50051")
        .expect("static tonic endpoint should be valid")
        .connect_with_connector(service_fn(move |_: Uri| {
            let socket_path = socket_path.clone();
            async move {
                let stream = UnixStream::connect(socket_path).await?;
                Ok::<_, io::Error>(TokioIo::new(stream))
            }
        }))
        .await?;
    Ok(channel)
}

fn normalize_socket_path(socket_path: &str) -> String {
    socket_path
        .strip_prefix("unix://")
        .unwrap_or(socket_path)
        .to_string()
}

fn map_grpc_error(error: tonic::Status) -> error::Error {
    match error.code() {
        Code::Ok => unreachable!("successful responses are not routed through map_grpc_error"),
        Code::AlreadyExists => error::Error::TargetPathAlreadyExists,
        Code::NotFound => error::Error::TargetPathNotFound,
        Code::FailedPrecondition => error::Error::FailedPrecondition,
        _ => error::Error::Grpc(error),
    }
}

fn map_controller_grpc_error(error: tonic::Status) -> error::Error {
    match error.code() {
        Code::Ok => unreachable!("successful responses are not routed through map_grpc_error"),
        Code::AlreadyExists => error::Error::VolumeAlreadyExists,
        Code::NotFound => error::Error::VolumeNotFound,
        Code::FailedPrecondition => error::Error::FailedPrecondition,
        _ => error::Error::Grpc(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CsiAccessMode, CsiAccessType, NodeVolumeStats, TugboatCsiOperator, VolumeHealthCondition,
        VolumeUsageStats, VolumeUsageUnit, volume_capability,
    };
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
        spawn_node_server_with_volume_stats(NodeGetVolumeStatsResponse::default()).await
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
        let (socket_path, calls) =
            spawn_node_server_with_volume_stats(NodeGetVolumeStatsResponse {
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
            })
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
}
