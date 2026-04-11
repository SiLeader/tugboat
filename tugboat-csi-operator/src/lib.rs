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
use tokio::time::{Duration, timeout};
use tonic::Code;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

const SOCKET_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(not(test))]
const RPC_CALL_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const RPC_CALL_TIMEOUT: Duration = Duration::from_millis(200);

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
    #[allow(clippy::too_many_arguments)]
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
        let response = timeout(RPC_CALL_TIMEOUT, client.create_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
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
        timeout(RPC_CALL_TIMEOUT, client.delete_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
            .map(|_| ())
            .map_err(map_controller_grpc_error)
    }

    pub async fn controller_capabilities(
        &self,
        socket_path: &str,
    ) -> Result<Vec<ControllerCapability>, error::Error> {
        let mut client = connect_controller_client(socket_path).await?;
        let response = match timeout(
            RPC_CALL_TIMEOUT,
            client.controller_get_capabilities(ControllerGetCapabilitiesRequest {}),
        )
        .await
        .map_err(|_| error::Error::RpcTimeout)?
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
        let response = timeout(RPC_CALL_TIMEOUT, client.controller_publish_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
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
        timeout(RPC_CALL_TIMEOUT, client.controller_unpublish_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
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
        let response = timeout(RPC_CALL_TIMEOUT, client.controller_expand_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
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
        let response = timeout(
            RPC_CALL_TIMEOUT,
            client.node_get_capabilities(NodeGetCapabilitiesRequest {}),
        )
        .await
        .map_err(|_| error::Error::RpcTimeout)?
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
        timeout(RPC_CALL_TIMEOUT, client.node_publish_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
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
        timeout(RPC_CALL_TIMEOUT, client.node_stage_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
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
        timeout(RPC_CALL_TIMEOUT, client.node_unpublish_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
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
        timeout(RPC_CALL_TIMEOUT, client.node_unstage_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
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
        let response = timeout(RPC_CALL_TIMEOUT, client.node_get_volume_stats(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
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
        let response = timeout(RPC_CALL_TIMEOUT, client.node_expand_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
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
    let socket_path = normalize_socket_path(socket_path)?;
    let endpoint =
        Endpoint::try_from("http://localhost").expect("static tonic endpoint should be valid");
    let connect_fut = endpoint.connect_with_connector(service_fn(move |_: Uri| {
        let socket_path = socket_path.clone();
        async move {
            let stream = UnixStream::connect(socket_path).await?;
            Ok::<_, io::Error>(TokioIo::new(stream))
        }
    }));
    let channel = timeout(SOCKET_CONNECT_TIMEOUT, connect_fut)
        .await
        .map_err(|_| error::Error::SocketConnectionTimeout)?
        .map_err(error::Error::GrpcTransport)?;
    Ok(channel)
}

fn normalize_socket_path(socket_path: &str) -> Result<String, error::Error> {
    let normalized = socket_path
        .strip_prefix("unix://")
        .unwrap_or(socket_path)
        .trim();

    if normalized.is_empty() {
        return Err(error::Error::InvalidSocketPath(socket_path.to_string()));
    }

    Ok(normalized.to_string())
}

fn map_grpc_error(error: tonic::Status) -> error::Error {
    match error.code() {
        Code::DeadlineExceeded => error::Error::RpcTimeout,
        Code::AlreadyExists => error::Error::TargetPathAlreadyExists,
        Code::NotFound => error::Error::TargetPathNotFound,
        Code::FailedPrecondition => error::Error::FailedPrecondition,
        _ => error::Error::Grpc(error),
    }
}

fn map_controller_grpc_error(error: tonic::Status) -> error::Error {
    match error.code() {
        Code::DeadlineExceeded => error::Error::RpcTimeout,
        Code::AlreadyExists => error::Error::VolumeAlreadyExists,
        Code::NotFound => error::Error::VolumeNotFound,
        Code::FailedPrecondition => error::Error::FailedPrecondition,
        _ => error::Error::Grpc(error),
    }
}

#[cfg(test)]
mod tests;
