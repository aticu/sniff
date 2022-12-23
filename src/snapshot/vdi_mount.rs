//! Allows transparent mounting of VDI files.

use anyhow::Context as _;
use tempfile::TempDir;

use std::{path::Path, process::Command};

use crate::fs::OsStrExt as _;

/// Represents an actively mounted VDI image.
pub(crate) struct VDIMount {
    /// The path where the partition devices are created.
    dev_path: TempDir,
    /// The path where the largest partition of the VDI image is mounted.
    mounted_path: TempDir,
}

impl VDIMount {
    /// Creates a new mounted VDI image.
    pub(crate) fn new(path: impl AsRef<Path>) -> anyhow::Result<VDIMount> {
        let path = path.as_ref();
        let dev_path = TempDir::new().context("Could not create a temp dir")?;

        let vboxmount_result = Command::new("vboximg-mount")
            .arg("--image")
            .arg(path)
            .arg(dev_path.path())
            .stderr(std::process::Stdio::piped())
            .output()
            .context("Failed starting `vboximg-mount`")?;

        if !vboxmount_result.status.success() {
            anyhow::bail!(
                "vdi partitions could not be loaded: {}",
                std::str::from_utf8(&vboxmount_result.stderr).unwrap_or("unknown_error")
            );
        }

        let mut max_partition_name = None;
        let mut max_partition_size = 0;
        for entry in std::fs::read_dir(&dev_path)
            .with_context(|| format!("Could not read directory `{}`", dev_path.path().display()))?
        {
            let entry = entry.with_context(|| {
                format!("Could not read entry in `{}`", dev_path.path().display())
            })?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("vol") {
                let metadata = entry
                    .metadata()
                    .with_context(|| format!("Could not read metadata of `{}`", name.display()))?;
                if metadata.len() > max_partition_size {
                    max_partition_size = metadata.len();
                    max_partition_name = Some(name);
                }
            }
        }

        let mut max_partition_path = dev_path.path().to_path_buf();
        if let Some(max_partition_name) = max_partition_name {
            max_partition_path.push(&max_partition_name);
        } else {
            anyhow::bail!("could not find a suitable partition");
        }

        let mounted_path = TempDir::new().context("Could not create a temp dir")?;

        let ntfs3g_result = Command::new("ntfs-3g")
            .arg("-o")
            .arg("no_def_opts,ro,show_sys_files,silent")
            .arg(&max_partition_path)
            .arg(mounted_path.path())
            .stderr(std::process::Stdio::piped())
            .output()
            .context("Could not start `ntfs-3g`")?;

        if !ntfs3g_result.status.success() {
            // Try to unmount the previously mounted device dir to clean up, ignore errors
            Command::new("umount").arg(dev_path.path()).output().ok();

            anyhow::bail!(
                "vdi partition could not be mounted: {}",
                std::str::from_utf8(&ntfs3g_result.stderr).unwrap_or("unknown_error")
            );
        }

        Ok(VDIMount {
            dev_path,
            mounted_path,
        })
    }

    /// Returns the path to the mounted partition.
    pub(crate) fn path(&self) -> &Path {
        self.mounted_path.path()
    }
}

impl Drop for VDIMount {
    fn drop(&mut self) {
        // Try to unmount both mounts in order, ignoring errors
        Command::new("umount")
            .arg(self.mounted_path.path())
            .output()
            .ok();
        Command::new("umount")
            .arg(self.dev_path.path())
            .output()
            .ok();
        // The temporary directories are unmounted after this in their own `Drop` impl.
    }
}
