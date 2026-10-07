//! Process-local admission shared by sessions, reviewed work and Git operations.
//! These leases protect host-owned operations; they do not claim OS confinement
//! of a provider's internal tools or coordination with external Git processes.
use crate::{Result, WorktreeError};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
use uuid::Uuid;

#[derive(Clone, Debug)]
enum Scope {
    Session { parent: Option<Uuid> },
    Build { paths: Vec<String> },
    Mutation { topology_only: bool },
}
#[derive(Clone, Debug)]
struct Reservation {
    repository: PathBuf,
    checkout: PathBuf,
    scope: Scope,
}
#[derive(Default)]
pub struct WorkspaceCoordinator {
    entries: Mutex<HashMap<Uuid, Reservation>>,
}
struct LeaseInner {
    coordinator: Arc<WorkspaceCoordinator>,
    id: Uuid,
    // Child sessions retain their parent's scope even if a BuildService owner
    // is dropped while native cleanup is unresolved.
    _parent: Option<WorkspaceLease>,
}
impl Drop for LeaseInner {
    fn drop(&mut self) {
        // Poisoning must retain protection, never silently authorize work.
        if let Ok(mut entries) = self.coordinator.entries.lock() {
            entries.remove(&self.id);
        }
    }
}
/// An unforgeable Rust capability. No UI/serde owner identifier is accepted.
#[derive(Clone)]
pub struct WorkspaceLease(Arc<LeaseInner>);

/// Host-derived identity with private fields; callers cannot forge a common-dir.
#[derive(Clone)]
pub struct VerifiedCheckoutIdentity {
    repository: PathBuf,
    checkout: PathBuf,
}
impl VerifiedCheckoutIdentity {
    pub async fn discover(checkout: &Path) -> Result<Self> {
        let (repository, checkout) = canonical_identity(checkout).await?;
        Ok(Self {
            repository,
            checkout,
        })
    }
}

fn conflict(reason: &str) -> WorktreeError {
    WorktreeError::Git(format!("workspace admission denied: {reason}; stop and remove the owning session or complete its build cleanup first"))
}
fn overlaps(a: &str, b: &str) -> bool {
    a == "."
        || b == "."
        || a == b
        || a.starts_with(&format!("{b}/"))
        || b.starts_with(&format!("{a}/"))
}
fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}
impl WorkspaceCoordinator {
    pub fn shared() -> Arc<Self> {
        static INSTANCE: OnceLock<Arc<WorkspaceCoordinator>> = OnceLock::new();
        INSTANCE.get_or_init(|| Arc::new(Self::default())).clone()
    }
    fn insert(
        self: &Arc<Self>,
        entry: Reservation,
        parent: Option<&WorkspaceLease>,
    ) -> Result<WorkspaceLease> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| conflict("admission state poisoned"))?;
        let parent_id = parent.map(|p| p.0.id);
        if let Some(parent) = parent {
            if !Arc::ptr_eq(self, &parent.0.coordinator) {
                return Err(conflict("foreign build capability"));
            }
            let existing = entries
                .get(&parent.0.id)
                .ok_or_else(|| conflict("build capability expired"))?;
            if !matches!(existing.scope, Scope::Build { .. })
                || !same_path(&existing.repository, &entry.repository)
            {
                return Err(conflict("invalid build capability"));
            }
            if matches!(entry.scope, Scope::Session { .. })
                && !same_path(&existing.checkout, &entry.checkout)
            {
                return Err(conflict("role checkout differs from its build reservation"));
            }
        }
        for (id, other) in entries.iter() {
            if !same_path(&other.repository, &entry.repository) || Some(*id) == parent_id {
                continue;
            }
            let blocked = match (&entry.scope, &other.scope) {
                (
                    Scope::Mutation {
                        topology_only: true,
                    },
                    Scope::Mutation { .. },
                )
                | (
                    Scope::Mutation { .. },
                    Scope::Mutation {
                        topology_only: true,
                    },
                ) => true,
                (
                    Scope::Mutation {
                        topology_only: true,
                    },
                    _,
                ) => false,
                (
                    _,
                    Scope::Mutation {
                        topology_only: true,
                    },
                ) => true,
                (Scope::Mutation { .. }, _) | (_, Scope::Mutation { .. }) => true,
                (Scope::Build { paths: a }, Scope::Build { paths: b }) => {
                    a.iter().any(|x| b.iter().any(|y| overlaps(x, y)))
                }
                (Scope::Session { .. }, Scope::Session { .. }) => {
                    same_path(&entry.checkout, &other.checkout)
                }
                // Ordinary sessions have no bounded logical write set. Distinct
                // ordinary checkouts can coexist, but cannot bypass Build
                // scopes through a freshly isolated checkout. Host role children
                // inherit their already-admitted parent's bounded scope.
                (Scope::Session { parent }, Scope::Build { .. }) => parent.is_none(),
                (Scope::Build { .. }, Scope::Session { parent }) => parent.is_none(),
            };
            if blocked {
                return Err(conflict(
                    "another live, starting or cleanup owner protects this repository/check-out",
                ));
            }
        }
        let id = Uuid::new_v4();
        entries.insert(id, entry);
        Ok(WorkspaceLease(Arc::new(LeaseInner {
            coordinator: self.clone(),
            id,
            _parent: parent.cloned(),
        })))
    }
    pub async fn session(
        self: &Arc<Self>,
        checkout: &Path,
        parent: Option<&WorkspaceLease>,
    ) -> Result<WorkspaceLease> {
        let (repository, checkout) = canonical_identity(checkout).await?;
        self.insert(
            Reservation {
                repository,
                checkout,
                scope: Scope::Session {
                    parent: parent.map(|p| p.0.id),
                },
            },
            parent,
        )
    }
    pub fn build(
        self: &Arc<Self>,
        repository: PathBuf,
        checkout: PathBuf,
        paths: Vec<String>,
    ) -> Result<WorkspaceLease> {
        if paths.is_empty() {
            return Err(conflict("build has no declared write scope"));
        }
        let paths = paths
            .iter()
            .map(|path| {
                grok_workflows::normalize_write_path(path)
                    .map(|path| path.to_lowercase())
                    .map_err(|_| conflict("invalid declared write scope"))
            })
            .collect::<Result<Vec<_>>>()?;
        let repository = std::fs::canonicalize(repository)?;
        let checkout = std::fs::canonicalize(checkout)?;
        self.insert(
            Reservation {
                repository,
                checkout,
                scope: Scope::Build { paths },
            },
            None,
        )
    }
    /// Current read-only queue evidence; this does not reserve a workspace.
    pub fn build_blocked_reason(&self, repository: &Path) -> Result<Option<String>> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| conflict("admission state poisoned"))?;
        let ordinary_owner = entries.values().any(|entry| {
            same_path(&entry.repository, repository)
                && matches!(entry.scope, Scope::Session { parent: None })
        });
        Ok(ordinary_owner.then(|| "A native session retains ownership of this repository. Stop and remove that session after reviewing its work; this build will then retry admission.".into()))
    }
    pub async fn mutation(
        self: &Arc<Self>,
        checkout: &Path,
        topology_only: bool,
        parent: Option<&WorkspaceLease>,
    ) -> Result<WorkspaceLease> {
        let (repository, checkout) = canonical_identity(checkout).await?;
        self.insert(
            Reservation {
                repository,
                checkout,
                scope: Scope::Mutation { topology_only },
            },
            parent,
        )
    }
}
impl WorkspaceLease {
    pub fn bind_checkout(&self, checkout: &Path) -> Result<()> {
        let checkout = std::fs::canonicalize(checkout)?;
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .output()?;
        let repository = if output.status.success() {
            let path =
                String::from_utf8(output.stdout).map_err(|_| conflict("non-UTF8 Git identity"))?;
            std::fs::canonicalize(path.trim())?
        } else if String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
            checkout.clone()
        } else {
            return Err(conflict("attached repository identity unavailable"));
        };
        self.bind_verified_checkout(&VerifiedCheckoutIdentity {
            repository,
            checkout,
        })
    }

    pub fn bind_verified_checkout(&self, identity: &VerifiedCheckoutIdentity) -> Result<()> {
        let repository = &identity.repository;
        let checkout = &identity.checkout;
        let mut entries = self
            .0
            .coordinator
            .entries
            .lock()
            .map_err(|_| conflict("admission state poisoned"))?;
        let own = entries
            .get(&self.0.id)
            .ok_or_else(|| conflict("build capability expired"))?;
        if !matches!(own.scope, Scope::Build { .. }) {
            return Err(conflict("only build owners can attach checkouts"));
        }
        if !same_path(repository, &own.repository) {
            return Err(conflict("attached checkout belongs to another repository"));
        }
        if entries.iter().any(|(id, r)| {
            *id != self.0.id
                && same_path(&r.repository, &own.repository)
                && same_path(&r.checkout, checkout)
        }) {
            return Err(conflict("checkout already has an owner"));
        }
        entries
            .get_mut(&self.0.id)
            .expect("validated owner")
            .checkout = checkout.clone();
        Ok(())
    }
}
pub async fn canonical_identity(checkout: &Path) -> Result<(PathBuf, PathBuf)> {
    let checkout = tokio::fs::canonicalize(checkout).await?;
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(&checkout)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .map_err(|_| conflict("Git identity lookup timed out"))??;
    let repository = if output.status.success() {
        let text =
            String::from_utf8(output.stdout).map_err(|_| conflict("non-UTF8 Git identity"))?;
        tokio::fs::canonicalize(text.trim()).await?
    } else if String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
        // Non-Git projects still have a canonical checkout admission owner.
        checkout.clone()
    } else {
        return Err(conflict("Git repository identity could not be verified"));
    };
    Ok((repository, checkout))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn aliases_share_session_ownership_and_drop_releases() {
        let root = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(WorkspaceCoordinator::default());
        let lease = coordinator.session(root.path(), None).await.unwrap();
        assert!(coordinator
            .session(&root.path().join("."), None)
            .await
            .is_err());
        assert!(coordinator
            .mutation(root.path(), false, None)
            .await
            .is_err());
        drop(lease);
        assert!(coordinator.session(root.path(), None).await.is_ok());
    }
    #[tokio::test]
    async fn build_capability_and_disjoint_paths_are_required() {
        let root = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(WorkspaceCoordinator::default());
        let a = coordinator
            .build(root.path().into(), root.path().into(), vec!["src/a".into()])
            .unwrap();
        assert!(coordinator
            .build(root.path().into(), root.path().into(), vec!["src".into()])
            .is_err());
        assert!(coordinator
            .build(
                root.path().into(),
                root.path().into(),
                vec!["./SRC//a/".into()]
            )
            .is_err());
        assert!(coordinator
            .build(
                root.path().into(),
                root.path().into(),
                vec!["../escape".into()]
            )
            .is_err());
        let _b = coordinator
            .build(root.path().into(), root.path().into(), vec!["docs".into()])
            .unwrap();
        assert!(coordinator.session(root.path(), None).await.is_err());
        // A second build with the same project address cannot donate authority.
        // Distinct production builds bind distinct retained checkout addresses.
        drop(_b);
        let child = coordinator.session(root.path(), Some(&a)).await.unwrap();
        drop(a);
        assert!(coordinator
            .build(root.path().into(), root.path().into(), vec!["src".into()])
            .is_err());
        assert!(coordinator
            .mutation(root.path(), false, None)
            .await
            .is_err());
        drop(child);
        assert!(coordinator.mutation(root.path(), true, None).await.is_ok());
    }
}
