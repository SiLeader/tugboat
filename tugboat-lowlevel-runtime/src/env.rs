use crate::RuntimeArgs;
use resources::manifests::core::v1::CpuSpec;

const TUGBOAT_RUNTIME_VM_ID: &str = "TUGBOAT_RUNTIME_VM_ID";
const TUGBOAT_RUNTIME_IMAGE: &str = "TUGBOAT_RUNTIME_IMAGE";
// CPU
const TUGBOAT_RUNTIME_CPU_ARCH: &str = "TUGBOAT_RUNTIME_CPU_ARCH";
const TUGBOAT_RUNTIME_CPU_CORES: &str = "TUGBOAT_RUNTIME_CPU_CORES";
const TUGBOAT_RUNTIME_CPU_SOCKETS: &str = "TUGBOAT_RUNTIME_CPU_SOCKETS";
const TUGBOAT_RUNTIME_CPU_DIES: &str = "TUGBOAT_RUNTIME_CPU_DIES";
const TUGBOAT_RUNTIME_CPU_THREADS: &str = "TUGBOAT_RUNTIME_CPU_THREADS";

// Memory
const TUGBOAT_RUNTIME_MEMORY_SIZE: &str = "TUGBOAT_RUNTIME_MEMORY_SIZE";

impl RuntimeArgs {
    pub fn from_env() -> crate::Result<RuntimeArgs> {
        Ok(RuntimeArgs {
            image: read_env(TUGBOAT_RUNTIME_IMAGE)?,
            cpu: CpuSpec {
                architecture: read_env(TUGBOAT_RUNTIME_CPU_ARCH)?,
                cores: parse_env(TUGBOAT_RUNTIME_CPU_CORES)?,
                sockets: parse_env(TUGBOAT_RUNTIME_CPU_SOCKETS)?,
                dies: parse_env(TUGBOAT_RUNTIME_CPU_DIES)?,
                threads_per_core: parse_env(TUGBOAT_RUNTIME_CPU_THREADS)?,
            },
            memory: parse_env(TUGBOAT_RUNTIME_MEMORY_SIZE)?,
            id: read_env(TUGBOAT_RUNTIME_VM_ID)?,
        })
    }
}

fn read_env(name: &str) -> crate::Result<String> {
    std::env::var(name).map_err(|e| crate::Error::Environment(name.to_string(), e))
}

fn parse_env<T>(name: &str) -> crate::Result<T>
where
    T: std::str::FromStr,
    T::Err: ToString,
{
    read_env(name)?
        .parse()
        .map_err(|e: <T as std::str::FromStr>::Err| {
            crate::Error::EnvironmentParseError(name.to_string(), e.to_string())
        })
}
