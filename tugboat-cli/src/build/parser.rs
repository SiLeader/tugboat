use crate::build::error::BuildError;
use regex::Regex;
use serde::Serialize;
use std::path::Path;
use std::str::FromStr;
use std::sync::LazyLock;
use tracing::debug;

#[derive(Debug)]
pub(super) enum Arch {
    X86_64,
}

#[derive(Debug)]
pub(super) enum Format {
    Qcow2,
}

#[derive(Debug)]
pub(super) struct Imagefile {
    from: String,
    arch: Arch,
    format: Format,
}

static COMMENT_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(#.*$|\s+$)").unwrap());
static WRAP_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\\n").unwrap());
static MULTILINE_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n+").unwrap());

impl FromStr for Imagefile {
    type Err = BuildError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = COMMENT_REGEX.replace_all(s, "");
        let s = WRAP_REGEX.replace_all(s.as_ref(), "");
        let s = MULTILINE_REGEX.replace_all(s.as_ref(), "\n");

        let mut image: &str = "";
        let mut arch = Arch::X86_64;
        let mut format = Format::Qcow2;
        for line in s.lines() {
            if line.starts_with("FROM ") {
                image = line.trim_start_matches("FROM ").trim();
            } else if line.starts_with("ARCH ") {
                let arch_str = line.trim_start_matches("ARCH ").trim();
                arch = match arch_str {
                    "x86_64" => Arch::X86_64,
                    _ => return Err(BuildError::InvalidArch(arch_str.to_string())),
                };
            } else if line.starts_with("FORMAT ") {
                let format_str = line.trim_start_matches("FORMAT ").trim();
                format = match format_str {
                    "qcow2" => Format::Qcow2,
                    _ => return Err(BuildError::InvalidFormat(format_str.to_string())),
                };
            }
        }
        Ok(Self {
            from: image.to_string(),
            arch,
            format,
        })
    }
}

#[derive(Serialize)]
struct TugboatImageMetadata {
    format: String,
    arch: String,
}

impl Imagefile {
    pub(super) async fn read_disk(
        &self,
        context: impl AsRef<Path>,
    ) -> Result<Vec<u8>, std::io::Error> {
        debug!("Reading disk image from: {}", self.from);
        let image = context.as_ref().join(self.from.as_str());
        tokio::fs::read(image).await
    }

    pub(super) fn metadata(&self) -> Result<Vec<u8>, serde_json::Error> {
        let metadata = TugboatImageMetadata {
            format: match self.format {
                Format::Qcow2 => "qcow2".to_string(),
            },
            arch: match self.arch {
                Arch::X86_64 => "x86_64".to_string(),
            },
        };

        serde_json::to_vec(&metadata)
    }
}
