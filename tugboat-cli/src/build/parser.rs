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

use crate::build::error::BuildError;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tracing::debug;
use tugboat_vm_image::{Arch, Format};

#[derive(Debug)]
pub(super) struct Imagefile {
    from: String,
    pub arch: Arch,
    pub format: Format,
}

impl FromStr for Imagefile {
    type Err = BuildError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let joined = s.replace("\\\n", "");

        let mut image: &str = "";
        let mut arch = Arch::X64;
        let mut format = Format::Qcow2;
        for raw_line in joined.lines() {
            let line = raw_line
                .split_once('#')
                .map_or(raw_line, |(before_comment, _)| before_comment)
                .trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with("FROM ") {
                image = line.trim_start_matches("FROM ").trim();
            } else if line.starts_with("ARCH ") {
                let arch_str = line.trim_start_matches("ARCH ").trim();
                arch = match arch_str {
                    "x64" => Arch::X64,
                    _ => return Err(BuildError::InvalidArch(arch_str.to_string())),
                };
            } else if line.starts_with("FORMAT ") {
                let format_str = line.trim_start_matches("FORMAT ").trim();
                format = Format::from_str(format_str)
                    .map_err(|_| BuildError::InvalidFormat(format_str.to_string()))?;
            }
        }
        if image.is_empty() {
            return Err(BuildError::MissingFrom);
        }
        Ok(Self {
            from: image.to_string(),
            arch,
            format,
        })
    }
}

impl Imagefile {
    pub(super) fn disk_path(&self, context: impl AsRef<Path>) -> PathBuf {
        context.as_ref().join(self.from.as_str())
    }

    pub(super) async fn read_disk(
        &self,
        context: impl AsRef<Path>,
    ) -> Result<Vec<u8>, std::io::Error> {
        debug!("Reading disk image from: {}", self.from);
        let image = self.disk_path(context);
        tokio::fs::read(image).await
    }
}

#[cfg(test)]
mod tests {
    use super::Imagefile;
    use crate::build::error::BuildError;
    use std::str::FromStr;
    use tugboat_vm_image::Format;

    #[test]
    fn parses_raw_format() {
        let imagefile =
            Imagefile::from_str("FROM ./disk.raw\nARCH x64\nFORMAT raw\n").expect("imagefile");

        assert_eq!(imagefile.format, Format::Raw);
    }

    #[test]
    fn missing_format_defaults_to_qcow2() {
        let imagefile = Imagefile::from_str("FROM ./disk.qcow2\nARCH x64\n").expect("imagefile");

        assert_eq!(imagefile.format, Format::Qcow2);
    }

    #[test]
    fn invalid_format_errors() {
        let err = Imagefile::from_str("FROM ./disk.vmdk\nFORMAT vmdk\n")
            .expect_err("invalid format should fail");

        assert!(matches!(err, BuildError::InvalidFormat(format) if format == "vmdk"));
    }
}
