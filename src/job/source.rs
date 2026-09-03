use std::path::PathBuf;

use anyhow::Context;

use super::definition::JobDefinition;
use super::error::JobError;
use super::name::JobName;
use super::profile::{PiProfileSource, validate_pi_profile};
use super::state::content_revision;

const SOURCE_FILE_NAME: &str = "clockwork.yaml";
const PI_PROFILE_FILE_NAME: &str = "pi-profile.json";

/// Managed job sources live at `~/.agents/clockwork/jobs.d/<name>/clockwork.yaml`,
/// one directory per job, directory name == job name. `CLOCKWORK_JOBS_ROOT`
/// overrides the root for tests and isolated environments.
pub fn jobs_dir() -> Result<PathBuf, JobError> {
    if let Ok(dir) = std::env::var("CLOCKWORK_JOBS_ROOT") {
        return Ok(PathBuf::from(dir));
    }
    let home = dirs::home_dir().ok_or_else(|| JobError::SourceFailure {
        message: "could not determine home directory".to_string(),
    })?;
    Ok(home.join(".agents/clockwork/jobs.d"))
}

pub fn source_path(name: &JobName) -> Result<PathBuf, JobError> {
    Ok(jobs_dir()?.join(name.as_str()).join(SOURCE_FILE_NAME))
}

pub fn pi_profile_path(name: &JobName) -> Result<PathBuf, JobError> {
    Ok(jobs_dir()?.join(name.as_str()).join(PI_PROFILE_FILE_NAME))
}

/// Revision over the complete managed source: the canonical YAML bytes plus
/// the companion `pi-profile.json` bytes when present. A companion edit is
/// a source change and must move the optimistic revision.
pub fn combined_revision(yaml: &[u8], pi_profile: Option<&PiProfileSource>) -> String {
    let mut bytes = yaml.to_vec();
    if let Some(profile) = pi_profile {
        bytes.extend_from_slice(b"\0pi-profile.json\0");
        bytes.extend_from_slice(profile.raw.as_bytes());
    }
    content_revision(&bytes)
}

/// A parsed managed source plus its content revision. `pi_profile` carries
/// the raw launcher profile when the source directory provides one; the
/// planner validates it and the coordinator owns the derived runtime profile.
#[derive(Debug, Clone)]
pub struct VersionedJobSource {
    pub definition: JobDefinition,
    pub revision: String,
    pub pi_profile: Option<PiProfileSource>,
}

/// Private managed-source storage. Only the application service calls this.
pub(crate) trait SourceStore {
    fn load(&self, name: &JobName) -> Result<Option<VersionedJobSource>, JobError>;
    fn write_atomic(
        &self,
        definition: &JobDefinition,
        expected: Option<&str>,
    ) -> Result<String, JobError>;
    fn remove_atomic(&self, name: &JobName, expected: &str) -> Result<(), JobError>;
}

pub struct FsSourceStore;

impl FsSourceStore {
    fn validate_directory(name: &JobName) -> Result<(), JobError> {
        let dir = jobs_dir()?.join(name.as_str());
        let metadata = match std::fs::symlink_metadata(&dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(JobError::SourceFailure {
                    message: format!("failed to inspect {}: {error}", dir.display()),
                });
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(JobError::SourceFailure {
                message: format!(
                    "managed source path is not a real directory: {}",
                    dir.display()
                ),
            });
        }
        for entry in std::fs::read_dir(&dir).map_err(|error| JobError::SourceFailure {
            message: format!("failed to read {}: {error}", dir.display()),
        })? {
            let entry = entry.map_err(|error| JobError::SourceFailure {
                message: format!("failed to read {}: {error}", dir.display()),
            })?;
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                return Err(JobError::SourceFailure {
                    message: format!("source entry name is not UTF-8: {}", entry.path().display()),
                });
            };
            if file_name.starts_with('.')
                || matches!(file_name, SOURCE_FILE_NAME | PI_PROFILE_FILE_NAME)
            {
                continue;
            }
            return Err(JobError::SourceFailure {
                message: format!(
                    "unsupported entry in managed source '{}': {file_name}",
                    dir.display()
                ),
            });
        }
        Ok(())
    }

    /// Enumerate candidate managed source names without parsing their files.
    /// Validation needs this lower-level view to report malformed YAML rather
    /// than stopping at the first source-load error.
    pub fn names() -> Result<Vec<JobName>, JobError> {
        let dir = jobs_dir()?;
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&dir).map_err(|error| JobError::SourceFailure {
            message: format!("failed to read {}: {error}", dir.display()),
        })?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| JobError::SourceFailure {
                message: format!("failed to read {}: {error}", dir.display()),
            })?;
            if !entry
                .file_type()
                .map_err(|error| JobError::SourceFailure {
                    message: format!("failed to inspect {}: {error}", entry.path().display()),
                })?
                .is_dir()
            {
                continue;
            }
            let raw = entry
                .file_name()
                .into_string()
                .map_err(|_| JobError::SourceFailure {
                    message: format!(
                        "job directory name is not UTF-8: {}",
                        entry.path().display()
                    ),
                })?;
            names.push(
                JobName::parse(&raw).map_err(|error| JobError::SourceFailure { message: error })?,
            );
        }
        names.sort();
        Ok(names)
    }

    /// Canonical source bytes used for both storage and revision
    /// calculation, so the planner can predict the written revision and a
    /// reload hashes the exact bytes that were written.
    /// Read the companion launcher profile without requiring the source to
    /// exist. Create planning needs this to predict the written revision and
    /// to fail closed on a malformed companion.
    pub fn companion_profile(name: &JobName) -> Result<Option<PiProfileSource>, JobError> {
        Self::validate_directory(name)?;
        let path = pi_profile_path(name)?;
        match std::fs::read_to_string(&path) {
            Ok(raw) => {
                validate_pi_profile(&raw, true).map_err(JobError::invalid_input)?;
                Ok(Some(PiProfileSource { raw }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(JobError::SourceFailure {
                message: format!("failed to read {}: {e}", path.display()),
            }),
        }
    }

    pub fn serialize(definition: &JobDefinition) -> Result<Vec<u8>, JobError> {
        let raw = serde_norway::to_string(definition).map_err(|e| JobError::SourceFailure {
            message: format!("failed to serialize job definition: {e}"),
        })?;
        Ok(format!("{raw}\n").into_bytes())
    }
}

impl SourceStore for FsSourceStore {
    fn load(&self, name: &JobName) -> Result<Option<VersionedJobSource>, JobError> {
        Self::validate_directory(name)?;
        let path = source_path(name)?;
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(JobError::SourceFailure {
                    message: format!("failed to read {}: {e}", path.display()),
                });
            }
        };
        let definition: JobDefinition =
            serde_norway::from_str(&raw).map_err(|e| JobError::SourceFailure {
                message: format!("failed to parse {}: {e}", path.display()),
            })?;
        // Directory and name identity rule.
        if definition.name != *name {
            return Err(JobError::integrity(
                Some(name.clone()),
                format!(
                    "source name '{}' does not match directory name '{}'",
                    definition.name,
                    name.as_str()
                ),
            ));
        }
        let pi_profile = Self::companion_profile(name)?;
        Ok(Some(VersionedJobSource {
            revision: combined_revision(raw.as_bytes(), pi_profile.as_ref()),
            definition,
            pi_profile,
        }))
    }

    fn write_atomic(
        &self,
        definition: &JobDefinition,
        expected: Option<&str>,
    ) -> Result<String, JobError> {
        let path = source_path(&definition.name)?;
        let current = self.load(&definition.name)?;

        match (expected, &current) {
            (None, Some(_)) => {
                return Err(JobError::JobAlreadyExists(definition.name.clone()));
            }
            (Some(exp), Some(existing)) if existing.revision != exp => {
                return Err(JobError::RevisionConflict {
                    job: Some(definition.name.clone()),
                    expected: exp.to_string(),
                    actual: existing.revision.clone(),
                });
            }
            (Some(exp), None) => {
                return Err(JobError::RevisionConflict {
                    job: Some(definition.name.clone()),
                    expected: exp.to_string(),
                    actual: "<absent>".to_string(),
                });
            }
            _ => {}
        }

        let raw = Self::serialize(definition)?;
        let companion = match &current {
            Some(source) => source.pi_profile.clone(),
            None => Self::companion_profile(&definition.name)?,
        };
        let revision = combined_revision(&raw, companion.as_ref());

        let dir = path.parent().ok_or_else(|| JobError::SourceFailure {
            message: format!("invalid source path {}", path.display()),
        })?;
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))
            .map_err(|e| JobError::SourceFailure {
                message: format!("{e:#}"),
            })?;
        crate::store::paths::set_dir_permissions(dir).map_err(|e| JobError::SourceFailure {
            message: format!("{e:#}"),
        })?;

        let tmp = dir.join(format!(".{SOURCE_FILE_NAME}.tmp-{}", std::process::id()));
        std::fs::write(&tmp, &raw).map_err(|e| JobError::SourceFailure {
            message: format!("failed to write {}: {e}", tmp.display()),
        })?;
        crate::store::paths::set_file_permissions(&tmp).map_err(|e| JobError::SourceFailure {
            message: format!("{e:#}"),
        })?;
        std::fs::rename(&tmp, &path).map_err(|e| JobError::SourceFailure {
            message: format!(
                "failed to rename {} to {}: {e}",
                tmp.display(),
                path.display()
            ),
        })?;

        Ok(revision)
    }

    fn remove_atomic(&self, name: &JobName, expected: &str) -> Result<(), JobError> {
        let current = self.load(name)?;
        let existing = current.ok_or_else(|| JobError::RevisionConflict {
            job: Some(name.clone()),
            expected: expected.to_string(),
            actual: "<absent>".to_string(),
        })?;
        if existing.revision != expected {
            return Err(JobError::RevisionConflict {
                job: Some(name.clone()),
                expected: expected.to_string(),
                actual: existing.revision.clone(),
            });
        }
        // Remove the complete managed source directory, including the
        // companion pi-profile.json. Leaving the companion behind would
        // recreate a broken managed source on the next scan.
        let dir = jobs_dir()?.join(name.as_str());
        std::fs::remove_dir_all(&dir).map_err(|e| JobError::SourceFailure {
            message: format!("failed to remove {}: {e}", dir.display()),
        })?;
        Ok(())
    }
}
