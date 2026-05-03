use super::super::CoreService;
use anyhow::Result;
use shared_protocol::ClearStateResult;
use std::collections::BTreeSet;
use std::io::ErrorKind;

impl CoreService {
    pub fn clear_all_state(&self) -> Result<ClearStateResult> {
        let cleared = {
            let mut db = self.open_db()?;
            db.clear_all_state()?
        };
        let mut result = cleared.result;

        for path in cleared
            .artifact_storage_paths
            .into_iter()
            .collect::<BTreeSet<_>>()
        {
            match std::fs::remove_file(&path) {
                Ok(()) => result.artifact_files_deleted += 1,
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    result.artifact_files_missing += 1;
                }
                Err(_) => {
                    result.artifact_files_failed += 1;
                }
            }
        }

        Ok(result)
    }
}
