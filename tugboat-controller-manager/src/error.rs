use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ControllerError {
    #[error("client error: {0}")]
    Client(#[from] tugboat_client::Error),
    #[error("CSI driver error: {0}")]
    Csi(#[from] tugboat_csi_operator::Error),
    #[error("{0} metadata.name is missing")]
    MissingName(&'static str),
    #[error("{0} metadata.namespace is missing")]
    MissingNamespace(&'static str),
    #[error("PersistentVolumeClaim '{namespace}/{name}' is missing spec")]
    MissingPersistentVolumeClaimSpec { namespace: String, name: String },
    #[error("PersistentVolume '{name}' is missing spec")]
    MissingPersistentVolumeSpec { name: String },
    #[error("PersistentVolume '{name}' does not have a CSI source")]
    MissingPersistentVolumeCsi { name: String },
    #[error("Managed PersistentVolume '{name}' has an empty CSI volume handle")]
    MissingVolumeHandle { name: String },
    #[error("StorageClass '{name}' is missing spec")]
    MissingStorageClassSpec { name: String },
    #[error("StorageClass '{name}' has an empty provisioner")]
    MissingProvisioner { name: String },
    #[error("PersistentVolumeClaim '{namespace}/{name}' is missing access modes")]
    MissingAccessModes { namespace: String, name: String },
    #[error("PersistentVolumeClaim '{namespace}/{name}' has unsupported access mode '{mode}'")]
    UnsupportedAccessMode {
        namespace: String,
        name: String,
        mode: String,
    },
    #[error("PersistentVolumeClaim '{namespace}/{name}' has unsupported volume mode '{mode}'")]
    UnsupportedVolumeMode {
        namespace: String,
        name: String,
        mode: String,
    },
    #[error(
        "PersistentVolume '{name}' already exists but does not match managed claim '{namespace}/{claim}'"
    )]
    ExistingVolumeConflict {
        name: String,
        namespace: String,
        claim: String,
    },
    #[error("finalizer error: {0}")]
    Finalizer(String),
}
