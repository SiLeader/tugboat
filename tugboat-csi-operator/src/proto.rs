pub(crate) mod csi {
    pub mod v1 {
        use tonic::include_proto;

        include_proto!("csi.v1");
    }
}
