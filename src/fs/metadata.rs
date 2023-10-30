//! Retrival and representation of filesystem metadata.

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use std::{collections::BTreeMap, ffi::OsString, io, path::Path};

use crate::timestamp::Timestamp;

/// Stores filesystem metadata about objects.
///
/// See [this article](https://jp-andre.pagesperso-orange.fr/extend-attr.html) for details about
/// the values.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct Metadata {
    /// The size of the entry in bytes.
    pub(crate) size: u64,
    /// The time the entry was created.
    pub(crate) created: Option<Timestamp>,
    /// The time the entry was last modified.
    pub(crate) modified: Option<Timestamp>,
    /// The time the entry was last accessed.
    pub(crate) accessed: Option<Timestamp>,
    /// The time the MFT entry was last modified.
    ///
    /// This means any changes to the file including changes to metadata.
    pub(crate) mft_modified: Option<Timestamp>,
    /// The NTFS attributes of the entry.
    pub(crate) ntfs_attributes: Option<NtfsAttributes>,
    /// The UNIX permissions of the file.
    pub(crate) unix_permissions: Option<u32>,
    /// The number of hard links to this file.
    pub(crate) nlink: Option<u64>,
    /// The UNIX user ID of the file.
    pub(crate) uid: Option<u32>,
    /// The UNIX group ID of the file.
    pub(crate) gid: Option<u32>,
    /// The NTFS reparse data.
    pub(crate) reparse_data: Option<Vec<u8>>,
    /// The NTFS access control list.
    pub(crate) acl: Option<Vec<u8>>,
    /// The NTFS dos name.
    pub(crate) dos_name: Option<Vec<u8>>,
    /// The NTFS object id.
    pub(crate) object_id: Option<Vec<u8>>,
    /// The NTFS encryption information.
    pub(crate) efs_info: Option<Vec<u8>>,
    /// The NTFS extended attributes.
    pub(crate) ea: Option<Vec<u8>>,
    /// The NTFS alternate data streams.
    pub(crate) streams: Option<AlternateDataStreams>,
    /// The inode of the entry.
    pub(crate) inode: Option<u64>,
}

/// A common trait that all implementations of metadata should fulfill.
pub(crate) trait GenericMetadata:
    Serialize + DeserializeOwned + Clone + Sized + Send
{
    /// Reads the metadata for the given path.
    fn from_path(path: impl AsRef<Path>) -> io::Result<Self>;

    /// Returns meaningless metadata.
    ///
    /// This can be used as a replacement until the real metadata is computed.
    fn meaningless() -> Self;
}

impl GenericMetadata for Metadata {
    fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        firestorm::profile_fn!(meta_from_path);

        let path = path.as_ref();

        let metadata = std::fs::symlink_metadata(path)?;

        let size = metadata.len();

        let ntfs_attributes = xattr::get(path, "system.ntfs_attrib_be")
            .ok()
            .flatten()
            .map(|attr| {
                let as_u32 = u32::from_be_bytes(attr[0..4].try_into().unwrap());
                unsafe { NtfsAttributes::from_bits_unchecked(as_u32) }
            });

        let (created, modified, accessed, mft_modified);
        if let Ok(maybe_times) = xattr::get(path, "system.ntfs_times_be")
            && let Some(times) = maybe_times
            && times.len() >= 32 {
            created =
                Some(Timestamp::from_ntfs_timestamp(i64::from_be_bytes(times[0..8].try_into().unwrap())));
            modified =
                Some(Timestamp::from_ntfs_timestamp(i64::from_be_bytes(times[8..16].try_into().unwrap())));
            accessed =
                Some(Timestamp::from_ntfs_timestamp(i64::from_be_bytes(times[16..24].try_into().unwrap())));
            mft_modified =
                Some(Timestamp::from_ntfs_timestamp(i64::from_be_bytes(times[24..32].try_into().unwrap())));
        } else {
            created = metadata.created().ok().map(Timestamp::from);
            modified = metadata.modified().ok().map(Timestamp::from);
            accessed = metadata.accessed().ok().map(Timestamp::from);
            mft_modified = None;
        }

        let reparse_data = xattr::get(path, "system.ntfs_reparse_data").ok().flatten();
        let acl = xattr::get(path, "system.ntfs_acl").ok().flatten();
        let dos_name = xattr::get(path, "system.ntfs_dos_name").ok().flatten();
        let object_id = xattr::get(path, "system.ntfs_object_id").ok().flatten();
        let efs_info = xattr::get(path, "system.ntfs_efsinfo").ok().flatten();
        let ea = xattr::get(path, "system.ntfs_ea").ok().flatten();

        let streams = AlternateDataStreams::from_path(path).ok();

        let (unix_permissions, nlink, uid, gid, inode);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            unix_permissions = Some(metadata.mode());
            nlink = Some(metadata.nlink());
            uid = Some(metadata.uid());
            gid = Some(metadata.gid());
            inode = Some(metadata.ino());
        }
        #[cfg(not(unix))]
        {
            unix_permissions = None;
            nlink = None;
            uid = None;
            gid = None;
            inode = None;
        }

        Ok(Self {
            size,
            created,
            modified,
            accessed,
            mft_modified,
            ntfs_attributes,
            unix_permissions,
            nlink,
            uid,
            gid,
            reparse_data,
            acl,
            dos_name,
            object_id,
            efs_info,
            ea,
            streams,
            inode,
        })
    }

    fn meaningless() -> Self {
        Self {
            size: 0,
            created: None,
            modified: None,
            accessed: None,
            mft_modified: None,
            ntfs_attributes: None,
            unix_permissions: None,
            nlink: None,
            uid: None,
            gid: None,
            reparse_data: None,
            acl: None,
            dos_name: None,
            object_id: None,
            efs_info: None,
            ea: None,
            streams: None,
            inode: None,
        }
    }
}

bitflags::bitflags! {
    /// The attributes of a filesystem entry.
    ///
    /// Information about the flags if from
    /// [here](https://github.com/libyal/libfsntfs/blob/main/documentation/New%20Technologies%20File%20System%20(NTFS).asciidoc#file_attribute_flags).
    #[rustfmt::skip]
    #[derive(Deserialize, Serialize)]
    #[repr(transparent)]
    pub(crate) struct NtfsAttributes: u32 {
        const READONLY            = 0x0001;
        const HIDDEN              = 0x0002;
        const SYSTEM              = 0x0004;
        const VOLUME_LABEL        = 0x0008;
        const DIRECTORY           = 0x0010;
        const ARCHIVE             = 0x0020;
        const DEVICE              = 0x0040;
        const NORMAL              = 0x0080;
        const TEMPORARY           = 0x0100;
        const SPARSE              = 0x0200;
        const REPARSE_POINT       = 0x0400;
        const COMPRESSED          = 0x0800;
        const OFFLINE             = 0x1000;
        const NOT_CONTENT_INDEXED = 0x2000;
        const ENCRYPTED           = 0x4000;
    }
}

/// Alternate data streams of filesystem objects.
#[derive(Debug, Default, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct AlternateDataStreams {
    /// The alternate data streams associated with a filesystem object.
    pub(crate) streams: BTreeMap<OsString, Option<Vec<u8>>>,
}

impl AlternateDataStreams {
    /// Reads the alternate data stream for the specified path.
    pub(crate) fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();

        let mut streams = BTreeMap::new();

        let iter = xattr::list(path)?;

        for attr in iter {
            if let Ok(Some(val)) = xattr::get(path, &attr) {
                streams.insert(attr, Some(val));
            } else {
                streams.insert(attr, None);
            }
        }

        Ok(Self { streams })
    }
}

/// Stores filesystem metadata about objects.
///
/// See [this article](https://jp-andre.pagesperso-orange.fr/extend-attr.html) for details about
/// the values.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct MetadataV1 {
    /// The size of the entry in bytes.
    pub(crate) size: u64,
    /// The time the entry was created.
    pub(crate) created: Timestamp,
    /// The time the entry was last modified.
    pub(crate) modified: Timestamp,
    /// The time the entry was last accessed.
    pub(crate) accessed: Timestamp,
    /// The time the MFT entry was last modified.
    ///
    /// This means any changes to the file including changes to metadata.
    pub(crate) mft_modified: Timestamp,
    /// The NTFS attributes of the entry.
    pub(crate) attributes: NtfsAttributes,
    /// The NTFS reparse data.
    pub(crate) reparse_data: Option<Vec<u8>>,
    /// The NTFS access control list.
    pub(crate) acl: Option<Vec<u8>>,
    /// The NTFS dos name.
    pub(crate) dos_name: Option<Vec<u8>>,
    /// The NTFS object id.
    pub(crate) object_id: Option<Vec<u8>>,
    /// The NTFS encryption information.
    pub(crate) efs_info: Option<Vec<u8>>,
    /// The NTFS extended attributes.
    pub(crate) ea: Option<Vec<u8>>,
    /// The NTFS alternate data streams.
    pub(crate) streams: AlternateDataStreams,
    /// The inode of the entry.
    pub(crate) inode: u64,
}

impl From<MetadataV1> for Metadata {
    fn from(old: MetadataV1) -> Self {
        Self {
            size: old.size,
            created: Some(old.created),
            modified: Some(old.modified),
            accessed: Some(old.accessed),
            mft_modified: Some(old.mft_modified),
            ntfs_attributes: Some(old.attributes),
            unix_permissions: None,
            nlink: None,
            uid: None,
            gid: None,
            reparse_data: old.reparse_data,
            acl: old.acl,
            dos_name: old.dos_name,
            object_id: old.object_id,
            efs_info: old.efs_info,
            ea: old.ea,
            streams: Some(old.streams),
            inode: Some(old.inode),
        }
    }
}
