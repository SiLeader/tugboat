use crate::manifests::ObjectMeta;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistentVolume {
    pub metadata: ObjectMeta,
    pub spec: PersistentVolumeSpec,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistentVolumeSpec {
    pub access_modes: Vec<PersistentVolumeAccessMode>,
    pub storage_class_name: Option<String>,
    pub volume_mode: PersistentVolumeMode,
    #[serde(flatten)]
    pub options: VolumeOptions,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum VolumeOptions {
    Nbd { nbd: NbdPersistentVolume },
    Local { local: LocalPersistentVolume },
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NbdPersistentVolume {
    pub server: String,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPersistentVolume {
    pub file: String,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize)]
#[allow(clippy::enum_variant_names)]
pub enum PersistentVolumeAccessMode {
    ReadOnlyOnce,
    ReadOnlyMany,
    #[default]
    ReadWriteOnce,
    ReadWriteMany,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize)]
pub enum PersistentVolumeMode {
    Filesystem,
    #[default]
    Block,
}

impl Default for VolumeOptions {
    fn default() -> Self {
        Self::Local {
            local: Default::default(),
        }
    }
}
