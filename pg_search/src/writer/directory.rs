use crate::env;
use derive_more::AsRef;
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;
use walkdir::WalkDir;

static PARADE_DATA_DIR_NAME: &str = "paradedb";
static SEARCH_DIR_NAME: &str = "pg_search";
static SEARCH_INDEX_CONFIG_FILE_NAME: &str = "search-index.json";
static TANTIVY_DIR_NAME: &str = "tantivy";
static WRITER_TRANSFER_DIR_NAME: &str = "writer_transfer";

/// The top-level folder name for ParadeDB extension inside the Postgres data directory.
#[derive(AsRef)]
#[as_ref(forward)]
pub struct SearchIndexDirPath(pub PathBuf);
/// The name of the index-specfic configuration file, enabling loading an index across connections.
#[derive(AsRef)]
#[as_ref(forward)]
pub struct SearchIndexConfigFilePath(pub PathBuf);
/// The name of the directory where the Tantivy index will be created.
#[derive(AsRef)]
#[as_ref(forward)]
pub struct TantivyDirPath(pub PathBuf);
/// The name of the directory where pipe files will be created for transfer to the writer process.
#[derive(AsRef)]
#[as_ref(forward)]
pub struct WriterTransferPipeFilePath(pub PathBuf);

pub trait SearchFs {
    /// Load a persisted index from disk, so it can be reused between connections.
    fn load_index<T: DeserializeOwned>(&self) -> Result<T, SearchDirectoryError>;
    /// Save a serialize index to disk, so it can be persisted between connections.
    fn save_index<T: Serialize>(&self, index: &T) -> Result<(), SearchDirectoryError>;
    // Remove the root directory from disk, blocking while file locks are released.
    fn remove(&self) -> Result<(), SearchDirectoryError>;
    // Return and ensure the existence of the Tantivy index path.
    fn tantivy_dir_path(&self, ensure_exists: bool)
        -> Result<TantivyDirPath, SearchDirectoryError>;
    // Return and ensure the existence of the writer pipe file path.
    fn writer_transfer_pipe_path(
        &self,
        ensure_exists: bool,
    ) -> Result<WriterTransferPipeFilePath, SearchDirectoryError>;
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct WriterDirectory {
    pub index_name: String,
    pub database_oid: u32,
    pub postgres_data_dir_path: PathBuf,
}

impl WriterDirectory {
    /// Useful in a connection process, where the database oid is available in the environment.
    pub fn from_index_name(index_name: &str) -> Self {
        let database_oid = env::postgres_database_oid();
        let postgres_data_dir_path = env::postgres_data_dir_path();
        Self {
            index_name: index_name.into(),
            database_oid,
            postgres_data_dir_path,
        }
    }

    /// Useful in a background process where the database oid must be specified.
    #[allow(dead_code)]
    pub fn from_db_id_and_index_name(database_oid: u32, index_name: &str) -> Self {
        let postgres_data_dir_path = env::postgres_data_dir_path();
        Self {
            index_name: index_name.into(),
            database_oid,
            postgres_data_dir_path,
        }
    }

    /// The root path for the directory tree.
    fn search_index_dir_path(
        &self,
        ensure_exists: bool,
    ) -> Result<SearchIndexDirPath, SearchDirectoryError> {
        let database_oid = &self.database_oid;
        let index_name = &self.index_name;
        let unique_index_dir_name = format!("{database_oid}_{index_name}");
        let search_index_dir_path = &self
            .postgres_data_dir_path
            .join(PARADE_DATA_DIR_NAME)
            .join(SEARCH_DIR_NAME)
            .join(unique_index_dir_name);

        if ensure_exists {
            Self::ensure_dir(search_index_dir_path)?;
        }
        Ok(SearchIndexDirPath(search_index_dir_path.to_path_buf()))
    }

    pub fn exists(&self) -> Result<bool, SearchDirectoryError> {
        // False to avoid creating if doesn't exist.
        let SearchIndexDirPath(path) = self.search_index_dir_path(false)?;
        Ok(path.exists())
    }

    fn search_index_config_file_path(
        &self,
        ensure_exists: bool,
    ) -> Result<SearchIndexConfigFilePath, SearchDirectoryError> {
        let SearchIndexDirPath(index_path) = self.search_index_dir_path(ensure_exists)?;
        let search_index_config_file_path = index_path.join(SEARCH_INDEX_CONFIG_FILE_NAME);

        Ok(SearchIndexConfigFilePath(search_index_config_file_path))
    }

    fn ensure_dir(path: &Path) -> Result<(), SearchDirectoryError> {
        if !path.exists() {
            Self::create_dir_all(path)?
        }
        Ok(())
    }

    fn create_dir_all(path: &Path) -> Result<(), SearchDirectoryError> {
        fs::create_dir_all(path)
            .map_err(|err| SearchDirectoryError::CreateDirectory(path.to_path_buf(), err))
    }

    fn remove_dir_all_recursive(path: &Path) -> Result<(), SearchDirectoryError> {
        for child in fs::read_dir(path)
            .map_err(|err| SearchDirectoryError::ReadDirectoryEntry(path.to_path_buf(), err))?
        {
            let child_path = child
                .map_err(|err| SearchDirectoryError::ReadDirectoryEntry(path.to_path_buf(), err))?
                .path();

            if child_path.is_dir() {
                Self::remove_dir_all_recursive(&child_path)?;
            } else {
                let file = match File::open(&child_path) {
                    Err(err) => match err.kind() {
                        io::ErrorKind::NotFound => {
                            // If the file is not found, then we don't need to delete it.
                            continue;
                        }
                        _ => Err(SearchDirectoryError::OpenFileForRemoval(
                            child_path.to_path_buf(),
                            err,
                        )),
                    },
                    Ok(file) => Ok(file),
                }?;

                // Tantivy can sometimes hold an OS file lock on files in its index, so we
                // should wait for the lock to be released before we try to delete.
                file.lock_exclusive().map_err(|err| {
                    SearchDirectoryError::LockFileForRemoval(child_path.to_path_buf(), err)
                })?;

                match fs::remove_file(&child_path) {
                    Ok(()) => Ok(()),
                    // The file already doesn't exist, proceed.
                    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                    Err(err) => Err(SearchDirectoryError::RemoveFile(
                        child_path.to_path_buf(),
                        err,
                    )),
                }?;
            }
        }
        match fs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(err) => {
                // The directory already doesn't exist, proceed.
                if err.kind() == io::ErrorKind::NotFound {
                    return Ok(());
                }

                // We've done our best to delete everything.
                // If there's still files hanging around or if Tantivy
                // has created more, just ignore them.
                if err.to_string().contains("not empty") {
                    return Ok(());
                }

                let existing_files = Self::list_files(path);
                Err(SearchDirectoryError::RemoveDirectory(
                    path.to_path_buf(),
                    err,
                    existing_files,
                ))
            }
        }
    }

    fn list_files(directory: &Path) -> Vec<PathBuf> {
        WalkDir::new(directory)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .map(|e| e.into_path())
            .collect()
    }
}

impl SearchFs for WriterDirectory {
    fn load_index<T: DeserializeOwned>(&self) -> Result<T, SearchDirectoryError> {
        let SearchIndexConfigFilePath(config_path) = self.search_index_config_file_path(true)?;
        let serialized_data = fs::read_to_string(config_path)
            .map_err(|err| SearchDirectoryError::IndexFileRead(self.clone(), err))?;

        let new_self = serde_json::from_str(&serialized_data)
            .map_err(|err| SearchDirectoryError::IndexDeserialize(self.clone(), err))?;
        Ok(new_self)
    }

    fn save_index<T: Serialize>(&self, index: &T) -> Result<(), SearchDirectoryError> {
        let SearchIndexConfigFilePath(config_path) = self.search_index_config_file_path(true)?;

        let serialized_data = serde_json::to_string(index)
            .map_err(|err| SearchDirectoryError::IndexSerialize(self.clone(), err))?;

        let mut file = File::create(config_path)
            .map_err(|err| SearchDirectoryError::IndexFileCreate(self.clone(), err))?;

        file.write_all(serialized_data.as_bytes())
            .map_err(|err| SearchDirectoryError::IndexFileWrite(self.clone(), err))?;

        // Rust automatically flushes data to disk at the end of the scope,
        // so this call to "flush()" isn't strictly necessary.
        // We're doing it explicitly as a reminder in case we extend this method.
        file.flush()
            .map_err(|err| SearchDirectoryError::IndexFileFlush(self.clone(), err))?;

        Ok(())
    }

    fn remove(&self) -> Result<(), SearchDirectoryError> {
        let SearchIndexDirPath(index_path) = self.search_index_dir_path(false)?;
        if index_path.exists() {
            Self::remove_dir_all_recursive(&index_path)?
        }
        Ok(())
    }

    fn tantivy_dir_path(
        &self,
        ensure_exists: bool,
    ) -> Result<TantivyDirPath, SearchDirectoryError> {
        let SearchIndexDirPath(index_path) = self.search_index_dir_path(ensure_exists)?;
        let tantivy_dir_path = index_path.join(TANTIVY_DIR_NAME);

        Self::ensure_dir(&tantivy_dir_path)?;
        Ok(TantivyDirPath(tantivy_dir_path))
    }

    fn writer_transfer_pipe_path(
        &self,
        ensure_exists: bool,
    ) -> Result<WriterTransferPipeFilePath, SearchDirectoryError> {
        let pid = std::process::id();
        let transfer_pipe_dir = &self
            .postgres_data_dir_path
            .join(PARADE_DATA_DIR_NAME)
            .join(SEARCH_DIR_NAME)
            .join(WRITER_TRANSFER_DIR_NAME);
        let transfer_pipe_file = transfer_pipe_dir.join(pid.to_string());

        if ensure_exists {
            Self::ensure_dir(transfer_pipe_dir)?;
        }

        Ok(WriterTransferPipeFilePath(transfer_pipe_file.to_path_buf()))
    }
}

#[derive(Debug, Error)]
pub enum SearchDirectoryError {
    #[error("could not read directory entry {0:?}: {1}")]
    ReadDirectoryEntry(PathBuf, #[source] std::io::Error),

    #[error("could not deserialize index at '{0:?}, {1}")]
    IndexDeserialize(WriterDirectory, #[source] serde_json::Error),

    #[error("could not read from file to load index {0:?} at {1}")]
    IndexFileRead(WriterDirectory, #[source] std::io::Error),

    #[error("could not serialize index '{0:?}': {1}")]
    IndexSerialize(WriterDirectory, #[source] serde_json::Error),

    #[error("could not create file to save index {0:?} at {1}")]
    IndexFileCreate(WriterDirectory, #[source] std::io::Error),

    #[error("could not write to file to save index {0:?} at {1}")]
    IndexFileWrite(WriterDirectory, #[source] std::io::Error),

    #[error("could not flush file to disk to save index {0:?} at {1}")]
    IndexFileFlush(WriterDirectory, #[source] std::io::Error),

    #[error("could not create directory at {0:?}: {1}")]
    CreateDirectory(PathBuf, #[source] std::io::Error),

    #[error("could not remove directory at {0}, existing files: {2:#?}, {1}")]
    RemoveDirectory(PathBuf, #[source] std::io::Error, Vec<PathBuf>),

    #[error("could not remove file at {0:?}: {1}")]
    RemoveFile(PathBuf, #[source] std::io::Error),

    #[error("could not open file for locking and removal: {1}")]
    OpenFileForRemoval(PathBuf, #[source] std::io::Error),

    #[error("could not lock file for removal: {1}")]
    LockFileForRemoval(PathBuf, #[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use anyhow::Result;
    use rstest::*;

    fn is_directory_empty<P: AsRef<Path>>(path: P) -> Result<bool> {
        let mut entries = fs::read_dir(&path)?;
        if entries.next().is_none() {
            Ok(true)
        } else {
            print_directory_contents(&path)?;
            Ok(false)
        }
    }

    fn print_directory_contents<P: AsRef<Path>>(path: P) -> io::Result<()> {
        let entries = fs::read_dir(path)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                println!("File: {}", path.display());
            } else if path.is_dir() {
                println!("Directory: {}", path.display());
            }
        }

        Ok(())
    }

    #[rstest]
    fn test_remove_directory(mock_dir: MockWriterDirectory) -> Result<()> {
        let SearchIndexDirPath(root) = mock_dir.writer_dir.search_index_dir_path(true)?;

        let tantivy_path = root.join(TANTIVY_DIR_NAME);

        std::fs::create_dir_all(&tantivy_path)?;
        File::create(tantivy_path.join("meta.json"))?;
        File::create(root.join(SEARCH_INDEX_CONFIG_FILE_NAME))?;

        mock_dir.writer_dir.remove()?;

        // There should be nothing in the parent folder.
        assert!(is_directory_empty(root.parent().unwrap())?);

        Ok(())
    }
}
