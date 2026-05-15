use crate::proto::csi::v1::controller_client::ControllerClient;
use crate::proto::csi::v1::controller_service_capability;
use crate::proto::csi::v1::controller_service_capability::rpc::Type as ControllerServiceCapabilityType;
use crate::proto::csi::v1::node_client::NodeClient;
use crate::proto::csi::v1::node_service_capability;
use crate::proto::csi::v1::node_service_capability::rpc::Type as NodeServiceCapabilityType;
use crate::proto::csi::v1::volume_capability::access_mode::Mode;
use crate::proto::csi::v1::volume_capability::{AccessMode, AccessType, BlockVolume, MountVolume};
use crate::proto::csi::v1::volume_content_source;
use crate::proto::csi::v1::volume_usage::Unit as VolumeUsageProtoUnit;
use crate::proto::csi::v1::{
    CapacityRange, ControllerExpandVolumeRequest, ControllerGetCapabilitiesRequest,
    ControllerPublishVolumeRequest, ControllerUnpublishVolumeRequest, CreateSnapshotRequest,
    CreateVolumeRequest, DeleteSnapshotRequest, DeleteVolumeRequest, ListSnapshotsRequest,
    NodeExpandVolumeRequest, NodeGetCapabilitiesRequest, NodeGetVolumeStatsRequest,
    NodePublishVolumeRequest, NodeStageVolumeRequest, NodeUnpublishVolumeRequest,
    NodeUnstageVolumeRequest, Topology, TopologyRequirement, VolumeCapability, VolumeContentSource,
};
pub use error::Error;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Arc;
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio::time::{Duration, timeout};
use tonic::Code;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

const DEFAULT_SOCKET_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(not(test))]
const DEFAULT_RPC_CALL_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const DEFAULT_RPC_CALL_TIMEOUT: Duration = Duration::from_millis(200);

mod error;
mod proto;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsiTimeouts {
    pub socket_connect_timeout: Duration,
    pub rpc_call_timeout: Duration,
}

impl Default for CsiTimeouts {
    fn default() -> Self {
        Self {
            socket_connect_timeout: DEFAULT_SOCKET_CONNECT_TIMEOUT,
            rpc_call_timeout: DEFAULT_RPC_CALL_TIMEOUT,
        }
    }
}

#[derive(Clone)]
pub struct TugboatCsiOperator {
    timeouts: CsiTimeouts,
    channel_cache: Arc<RwLock<HashMap<String, Channel>>>,
}

impl Default for TugboatCsiOperator {
    fn default() -> Self {
        Self::new()
    }
}

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
    CreateDeleteSnapshot,
    ListSnapshots,
    CloneVolume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedVolume {
    pub volume_id: String,
    pub capacity_bytes: i64,
    pub volume_context: HashMap<String, String>,
    pub accessible_topology: Vec<HashMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedSnapshot {
    pub snapshot_id: String,
    pub source_volume_id: String,
    pub size_bytes: Option<i64>,
    pub creation_time_seconds: i64,
    pub creation_time_nanos: i32,
    pub ready_to_use: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedSnapshots {
    pub entries: Vec<ProvisionedSnapshot>,
    pub next_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListSnapshotsPaging {
    pub max_entries: i32,
    pub starting_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsiVolumeContentSource {
    Snapshot { snapshot_id: String },
    Volume { volume_id: String },
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
    pub fn new() -> Self {
        Self::with_timeouts(CsiTimeouts::default())
    }

    pub fn with_timeouts(timeouts: CsiTimeouts) -> Self {
        Self {
            timeouts,
            channel_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn connect_node_client(
        &self,
        socket_path: &str,
    ) -> Result<NodeClient<Channel>, error::Error> {
        Ok(NodeClient::new(self.connect_channel(socket_path).await?))
    }

    async fn connect_controller_client(
        &self,
        socket_path: &str,
    ) -> Result<ControllerClient<Channel>, error::Error> {
        Ok(ControllerClient::new(
            self.connect_channel(socket_path).await?,
        ))
    }

    async fn connect_channel(&self, socket_path: &str) -> Result<Channel, error::Error> {
        let socket_path = normalize_socket_path(socket_path)?;

        if let Some(cached) = self.channel_cache.read().await.get(&socket_path).cloned() {
            return Ok(cached);
        }

        let channel =
            connect_channel_once(&socket_path, self.timeouts.socket_connect_timeout).await?;
        let mut cache = self.channel_cache.write().await;
        let entry = cache.entry(socket_path).or_insert_with(|| channel.clone());
        Ok(entry.clone())
    }

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
        accessibility_topologies: Vec<HashMap<String, String>>,
    ) -> Result<ProvisionedVolume, error::Error> {
        self.create_volume_with_source(
            socket_path,
            name,
            capacity_bytes,
            parameters,
            access_modes,
            access_type,
            secrets,
            mount_flags,
            accessibility_topologies,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_volume_with_source(
        &self,
        socket_path: &str,
        name: String,
        capacity_bytes: Option<i64>,
        parameters: HashMap<String, String>,
        access_modes: Vec<CsiAccessMode>,
        access_type: CsiAccessType,
        secrets: HashMap<String, String>,
        mount_flags: Vec<String>,
        accessibility_topologies: Vec<HashMap<String, String>>,
        content_source: Option<CsiVolumeContentSource>,
    ) -> Result<ProvisionedVolume, error::Error> {
        validate_volume_name(&name)?;
        validate_optional_capacity_bytes(capacity_bytes)?;
        validate_access_modes(&access_modes)?;

        let volume_content_source = match content_source {
            Some(CsiVolumeContentSource::Snapshot { snapshot_id }) => {
                self.ensure_controller_capability(
                    socket_path,
                    ControllerCapability::CreateDeleteSnapshot,
                )
                .await?;
                Some(VolumeContentSource {
                    r#type: Some(volume_content_source::Type::Snapshot(
                        volume_content_source::SnapshotSource { snapshot_id },
                    )),
                })
            }
            Some(CsiVolumeContentSource::Volume { volume_id }) => {
                self.ensure_controller_capability(socket_path, ControllerCapability::CloneVolume)
                    .await?;
                Some(VolumeContentSource {
                    r#type: Some(volume_content_source::Type::Volume(
                        volume_content_source::VolumeSource { volume_id },
                    )),
                })
            }
            None => None,
        };

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
            volume_content_source,
            accessibility_requirements: topology_requirement(accessibility_topologies),
            mutable_parameters: Default::default(),
        };

        let mut client = self.connect_controller_client(socket_path).await?;
        let response = timeout(self.timeouts.rpc_call_timeout, client.create_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
            .map_err(map_controller_grpc_error)?
            .into_inner();
        let volume = response.volume.ok_or(error::Error::MissingVolume)?;
        if volume.volume_id.trim().is_empty() {
            return Err(error::Error::MissingVolumeId);
        }

        Ok(ProvisionedVolume {
            volume_id: volume.volume_id,
            capacity_bytes: volume.capacity_bytes,
            volume_context: volume.volume_context,
            accessible_topology: volume
                .accessible_topology
                .into_iter()
                .map(|topology| topology.segments)
                .collect(),
        })
    }

    pub async fn delete_volume(
        &self,
        socket_path: &str,
        volume_id: String,
        secrets: HashMap<String, String>,
    ) -> Result<(), error::Error> {
        let req = DeleteVolumeRequest { volume_id, secrets };

        let mut client = self.connect_controller_client(socket_path).await?;
        timeout(self.timeouts.rpc_call_timeout, client.delete_volume(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
            .map(|_| ())
            .map_err(map_controller_grpc_error)
    }

    pub async fn create_snapshot(
        &self,
        socket_path: &str,
        source_volume_id: String,
        name: String,
        parameters: HashMap<String, String>,
        secrets: HashMap<String, String>,
    ) -> Result<ProvisionedSnapshot, error::Error> {
        validate_snapshot_name(&name)?;
        validate_volume_id(&source_volume_id)?;
        self.ensure_controller_capability(socket_path, ControllerCapability::CreateDeleteSnapshot)
            .await?;

        let req = CreateSnapshotRequest {
            source_volume_id,
            name,
            secrets,
            parameters,
        };
        let mut client = self.connect_controller_client(socket_path).await?;
        let response = timeout(self.timeouts.rpc_call_timeout, client.create_snapshot(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
            .map_err(map_snapshot_grpc_error)?
            .into_inner();
        snapshot_from_proto(response.snapshot)
    }

    pub async fn delete_snapshot(
        &self,
        socket_path: &str,
        snapshot_id: String,
        secrets: HashMap<String, String>,
    ) -> Result<(), error::Error> {
        validate_snapshot_id(&snapshot_id)?;
        self.ensure_controller_capability(socket_path, ControllerCapability::CreateDeleteSnapshot)
            .await?;

        let req = DeleteSnapshotRequest {
            snapshot_id,
            secrets,
        };
        let mut client = self.connect_controller_client(socket_path).await?;
        timeout(self.timeouts.rpc_call_timeout, client.delete_snapshot(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
            .map(|_| ())
            .map_err(map_snapshot_grpc_error)
    }

    pub async fn list_snapshots(
        &self,
        socket_path: &str,
        snapshot_id: Option<String>,
        source_volume_id: Option<String>,
        paging: Option<ListSnapshotsPaging>,
        secrets: HashMap<String, String>,
    ) -> Result<ListedSnapshots, error::Error> {
        self.ensure_controller_capability(socket_path, ControllerCapability::ListSnapshots)
            .await?;
        let paging = paging.unwrap_or(ListSnapshotsPaging {
            max_entries: 0,
            starting_token: String::new(),
        });
        let req = ListSnapshotsRequest {
            max_entries: paging.max_entries,
            starting_token: paging.starting_token,
            source_volume_id: source_volume_id.unwrap_or_default(),
            snapshot_id: snapshot_id.unwrap_or_default(),
            secrets,
        };
        let mut client = self.connect_controller_client(socket_path).await?;
        let response = timeout(self.timeouts.rpc_call_timeout, client.list_snapshots(req))
            .await
            .map_err(|_| error::Error::RpcTimeout)?
            .map_err(map_snapshot_grpc_error)?
            .into_inner();
        let entries = response
            .entries
            .into_iter()
            .filter_map(|entry| snapshot_from_proto(entry.snapshot).ok())
            .collect();
        Ok(ListedSnapshots {
            entries,
            next_token: response.next_token,
        })
    }

    pub async fn controller_capabilities(
        &self,
        socket_path: &str,
    ) -> Result<Vec<ControllerCapability>, error::Error> {
        let mut client = self.connect_controller_client(socket_path).await?;
        let response = match timeout(
            self.timeouts.rpc_call_timeout,
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
                        ControllerServiceCapabilityType::CreateDeleteSnapshot => {
                            Some(ControllerCapability::CreateDeleteSnapshot)
                        }
                        ControllerServiceCapabilityType::ListSnapshots => {
                            Some(ControllerCapability::ListSnapshots)
                        }
                        ControllerServiceCapabilityType::CloneVolume => {
                            Some(ControllerCapability::CloneVolume)
                        }
                        _ => None,
                    }
                }
                None => None,
            })
            .collect())
    }

    async fn ensure_controller_capability(
        &self,
        socket_path: &str,
        capability: ControllerCapability,
    ) -> Result<(), error::Error> {
        let capabilities = self.controller_capabilities(socket_path).await?;
        if capabilities.contains(&capability) {
            Ok(())
        } else {
            Err(error::Error::UnsupportedControllerCapability(capability))
        }
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

        let mut client = self.connect_controller_client(socket_path).await?;
        let response = timeout(
            self.timeouts.rpc_call_timeout,
            client.controller_publish_volume(req),
        )
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

        let mut client = self.connect_controller_client(socket_path).await?;
        timeout(
            self.timeouts.rpc_call_timeout,
            client.controller_unpublish_volume(req),
        )
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
        validate_required_capacity_bytes(capacity_bytes)?;

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

        let mut client = self.connect_controller_client(socket_path).await?;
        let response = timeout(
            self.timeouts.rpc_call_timeout,
            client.controller_expand_volume(req),
        )
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
        let mut client = self.connect_node_client(socket_path).await?;
        let response = match timeout(
            self.timeouts.rpc_call_timeout,
            client.node_get_capabilities(NodeGetCapabilitiesRequest {}),
        )
        .await
        .map_err(|_| error::Error::RpcTimeout)?
        {
            Ok(response) => response.into_inner(),
            Err(status) if status.code() == Code::Unimplemented => return Ok(Vec::new()),
            Err(status) => return Err(map_grpc_error(status)),
        };

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

        let mut client = self.connect_node_client(socket_path).await?;
        timeout(
            self.timeouts.rpc_call_timeout,
            client.node_publish_volume(req),
        )
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

        let mut client = self.connect_node_client(socket_path).await?;
        timeout(
            self.timeouts.rpc_call_timeout,
            client.node_stage_volume(req),
        )
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

        let mut client = self.connect_node_client(socket_path).await?;
        timeout(
            self.timeouts.rpc_call_timeout,
            client.node_unpublish_volume(req),
        )
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

        let mut client = self.connect_node_client(socket_path).await?;
        timeout(
            self.timeouts.rpc_call_timeout,
            client.node_unstage_volume(req),
        )
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

        let mut client = self.connect_node_client(socket_path).await?;
        let response = timeout(
            self.timeouts.rpc_call_timeout,
            client.node_get_volume_stats(req),
        )
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
        validate_required_capacity_bytes(capacity_bytes)?;

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

        let mut client = self.connect_node_client(socket_path).await?;
        let response = timeout(
            self.timeouts.rpc_call_timeout,
            client.node_expand_volume(req),
        )
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

fn topology_requirement(topologies: Vec<HashMap<String, String>>) -> Option<TopologyRequirement> {
    let topologies = topologies
        .into_iter()
        .filter(|segments| !segments.is_empty())
        .map(|segments| Topology { segments })
        .collect::<Vec<_>>();
    if topologies.is_empty() {
        return None;
    }
    Some(TopologyRequirement {
        requisite: topologies.clone(),
        preferred: topologies,
    })
}

async fn connect_channel_once(
    socket_path: &str,
    socket_connect_timeout: Duration,
) -> Result<Channel, error::Error> {
    let socket_path = socket_path.to_string();
    let endpoint =
        Endpoint::try_from("http://localhost").expect("static tonic endpoint should be valid");
    let connect_fut = endpoint.connect_with_connector(service_fn(move |_: Uri| {
        let socket_path = socket_path.clone();
        async move {
            let stream = UnixStream::connect(socket_path).await?;
            Ok::<_, io::Error>(TokioIo::new(stream))
        }
    }));
    timeout(socket_connect_timeout, connect_fut)
        .await
        .map_err(|_| error::Error::SocketConnectionTimeout)?
        .map_err(error::Error::GrpcTransport)
}

fn normalize_socket_path(socket_path: &str) -> Result<String, error::Error> {
    let trimmed = socket_path.trim();
    let normalized = if let Some(path) = trimmed.strip_prefix("unix://") {
        path
    } else if trimmed.contains("://") {
        return Err(error::Error::InvalidSocketPath(socket_path.to_string()));
    } else {
        trimmed
    };

    if normalized.is_empty() {
        return Err(error::Error::InvalidSocketPath(socket_path.to_string()));
    }
    if !Path::new(normalized).is_absolute() {
        return Err(error::Error::InvalidSocketPath(socket_path.to_string()));
    }

    Ok(normalized.to_string())
}

fn validate_volume_name(name: &str) -> Result<(), error::Error> {
    if name.trim().is_empty() {
        return Err(error::Error::InvalidVolumeName(name.to_string()));
    }
    Ok(())
}

fn validate_volume_id(volume_id: &str) -> Result<(), error::Error> {
    if volume_id.trim().is_empty() {
        return Err(error::Error::MissingVolumeId);
    }
    Ok(())
}

fn validate_snapshot_name(name: &str) -> Result<(), error::Error> {
    if name.trim().is_empty() {
        return Err(error::Error::InvalidSnapshotName(name.to_string()));
    }
    Ok(())
}

fn validate_snapshot_id(snapshot_id: &str) -> Result<(), error::Error> {
    if snapshot_id.trim().is_empty() {
        return Err(error::Error::MissingSnapshotId);
    }
    Ok(())
}

fn validate_access_modes(access_modes: &[CsiAccessMode]) -> Result<(), error::Error> {
    if access_modes.is_empty() {
        return Err(error::Error::MissingAccessModes);
    }
    Ok(())
}

fn validate_optional_capacity_bytes(capacity_bytes: Option<i64>) -> Result<(), error::Error> {
    if let Some(value) = capacity_bytes {
        validate_required_capacity_bytes(value)?;
    }
    Ok(())
}

fn validate_required_capacity_bytes(capacity_bytes: i64) -> Result<(), error::Error> {
    if capacity_bytes <= 0 {
        return Err(error::Error::InvalidCapacityBytes(capacity_bytes));
    }
    Ok(())
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

fn map_snapshot_grpc_error(error: tonic::Status) -> error::Error {
    match error.code() {
        Code::DeadlineExceeded => error::Error::RpcTimeout,
        Code::AlreadyExists => error::Error::SnapshotAlreadyExists,
        Code::NotFound => error::Error::SnapshotNotFound,
        Code::FailedPrecondition => error::Error::FailedPrecondition,
        _ => error::Error::Grpc(error),
    }
}

fn snapshot_from_proto(
    snapshot: Option<crate::proto::csi::v1::Snapshot>,
) -> Result<ProvisionedSnapshot, error::Error> {
    let snapshot = snapshot.ok_or(error::Error::MissingSnapshot)?;
    if snapshot.snapshot_id.trim().is_empty() {
        return Err(error::Error::MissingSnapshotId);
    }
    let creation_time = snapshot.creation_time.unwrap_or_default();
    Ok(ProvisionedSnapshot {
        snapshot_id: snapshot.snapshot_id,
        source_volume_id: snapshot.source_volume_id,
        size_bytes: (snapshot.size_bytes > 0).then_some(snapshot.size_bytes),
        creation_time_seconds: creation_time.seconds,
        creation_time_nanos: creation_time.nanos,
        ready_to_use: snapshot.ready_to_use,
    })
}

#[cfg(test)]
mod tests;
