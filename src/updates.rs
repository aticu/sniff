//! Parses and represents update information.

use std::{collections::BTreeMap, fmt, fs::File, io, path::Path};

use serde::{Deserialize, Serialize};

use crate::timestamp::Timestamp;

/// Represents a single update transaction.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct Update {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) kb: String,
    pub(crate) timestamp: Option<Timestamp>,
    pub(crate) update_operation: String,
    pub(crate) operation_result: String,
    pub(crate) information_url: String,
    pub(crate) support_url: String,
    pub(crate) uninstall_notes: String,
    pub(crate) category: String,
    pub(crate) client_application_id: String,
    pub(crate) service_id: String,
    pub(crate) update_id: String,
    pub(crate) revision_number: String,
    pub(crate) unmapped_result_code: String,
    pub(crate) server_selection: String,
    pub(crate) hresult: String,
}

impl fmt::Display for Update {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.title)
    }
}

impl TryFrom<BTreeMap<String, String>> for Update {
    type Error = &'static str;

    fn try_from(mut map: BTreeMap<String, String>) -> Result<Self, Self::Error> {
        let title = map.remove("Title").ok_or("column `Title` not found")?;
        let description = map
            .remove("Description")
            .ok_or("column `Description` not found")?;
        let kb = map
            .remove("KB Number")
            .ok_or("column `KB Number` not found")?;
        let timestamp = map
            .remove("Install Date")
            .ok_or("column `Install Date` not found")?;
        let update_operation = map
            .remove("Update Operation")
            .ok_or("column `Update Operation` not found")?;
        let operation_result = map
            .remove("Operation Result")
            .ok_or("column `Operation Result` not found")?;
        let information_url = map
            .remove("Information URL")
            .ok_or("column `Information URL` not found")?;
        let support_url = map
            .remove("Support URL")
            .ok_or("column `Support URL` not found")?;
        let uninstall_notes = map
            .remove("Uninstall Notes")
            .ok_or("column `Uninstall Notes` not found")?;
        let category = map
            .remove("Category")
            .ok_or("column `Category` not found")?;
        let client_application_id = map
            .remove("Client Application ID")
            .ok_or("column `Client Application ID` not found")?;
        let service_id = map
            .remove("Service ID")
            .ok_or("column `Service ID` not found")?;
        let update_id = map
            .remove("Update ID")
            .ok_or("column `Update ID` not found")?;
        let revision_number = map
            .remove("Revision Number")
            .ok_or("column `Revision Number` not found")?;
        let unmapped_result_code = map
            .remove("Unmapped Result Code")
            .ok_or("column `Unmapped Result Code` not found")?;
        let server_selection = map
            .remove("Server Selection")
            .ok_or("column `Server Selection` not found")?;
        let hresult = map.remove("hResult").ok_or("column `hResult` not found")?;

        let timestamp = time::PrimitiveDateTime::parse(
            &timestamp,
            time::macros::format_description!("[day]/[month]/[year] [hour]:[minute]:[second]"),
        )
        .map(|ts| ts.assume_utc().into())
        .ok();

        Ok(Self {
            title,
            description,
            kb,
            timestamp,
            update_operation,
            operation_result,
            information_url,
            support_url,
            uninstall_notes,
            category,
            client_application_id,
            service_id,
            update_id,
            revision_number,
            unmapped_result_code,
            server_selection,
            hresult,
        })
    }
}

/// Represents a list of updates.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct Updates {
    /// The entries in the update file.
    pub(crate) updates: Vec<Update>,
    /// The time the update data was recorded.
    ///
    /// This corresponds to the last modification time of the file.
    pub(crate) recording_time: Timestamp,
}

impl Updates {
    /// Reads the update information from the specified path.
    pub(crate) fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();

        let mut updates: Vec<Update> = Vec::new();

        for raw_update in csv::Reader::from_reader(File::open(path)?).deserialize() {
            let raw_update: BTreeMap<String, String> = raw_update?;
            if let Ok(update) = raw_update.try_into() {
                updates.push(update);
            }
        }

        let metadata = path.metadata()?;

        Ok(Self {
            updates,
            recording_time: metadata.modified()?.into(),
        })
    }
}
