//! Real-filesystem folder grants (Cowork-style file access).
//!
//! By default every file tool is confined to the sandbox workspace. A user can
//! explicitly grant a real folder (e.g. their Documents directory) with
//! `kerna folders add <name> <path>`, after which file tools can address it via
//! `root: "<name>"`. Grants are read-only unless created with `--read-write`,
//! and every write still passes through the normal fail-closed permission
//! check for the tool (`fs.write`) on top of this boundary check — this module
//! only decides *where* a path may resolve, not whether the call is allowed.

use crate::config::Config;
use crate::sandbox::path_within_root;
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// The always-available root: the sandboxed workspace (read-write).
pub const WORKSPACE_ROOT: &str = "workspace";

/// Resolve a root name to its base directory and whether it's read-only.
/// `"workspace"` (or omitted/empty) always resolves to the sandbox dir.
pub fn resolve_root<'a>(
    config: &'a Config,
    sandbox_dir: &'a Path,
    root_name: &str,
) -> Result<(PathBuf, bool)> {
    if root_name.is_empty() || root_name == WORKSPACE_ROOT {
        return Ok((sandbox_dir.to_path_buf(), false));
    }
    let grant = config
        .folders
        .iter()
        .find(|g| g.name == root_name)
        .ok_or_else(|| {
            anyhow!(
                "Unknown folder root '{}'. Granted folders: {}. Grant one with `kerna folders add`.",
                root_name,
                if config.folders.is_empty() {
                    "(none)".to_string()
                } else {
                    config
                        .folders
                        .iter()
                        .map(|g| g.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            )
        })?;
    Ok((PathBuf::from(&grant.path), !grant.read_write))
}

/// Join `rel` onto `root` and verify the result stays inside `root`. Rejects
/// absolute paths and `..` traversal outright (before touching the
/// filesystem), then re-confirms with a canonicalizing boundary check against
/// the nearest *existing* ancestor — this catches a symlinked directory
/// higher up the path being used to escape, even when the leaf file itself
/// doesn't exist yet (e.g. a fresh write target).
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    if crate::sandbox::is_out_of_workspace_path(rel) {
        return Err(anyhow!(
            "path '{}' escapes the folder boundary (absolute paths, '..', and '~' are not allowed)",
            rel
        ));
    }
    let candidate = root.join(rel);
    let existing_ancestor = std::iter::successors(Some(candidate.as_path()), |p| p.parent())
        .find(|p| p.exists())
        .unwrap_or(root);
    if !path_within_root(root, existing_ancestor) {
        return Err(anyhow!(
            "path '{}' resolves outside the granted folder boundary",
            rel
        ));
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FolderGrant;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kerna_folders_test_{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn workspace_root_resolves_to_sandbox_and_is_read_write() {
        let cfg = Config::default();
        let sandbox = temp_dir("ws");
        let (root, read_only) = resolve_root(&cfg, &sandbox, "workspace").unwrap();
        assert_eq!(root, sandbox);
        assert!(!read_only);
    }

    #[test]
    fn empty_root_name_defaults_to_workspace() {
        let cfg = Config::default();
        let sandbox = temp_dir("ws2");
        let (root, _) = resolve_root(&cfg, &sandbox, "").unwrap();
        assert_eq!(root, sandbox);
    }

    #[test]
    fn unknown_root_name_errors_with_available_list() {
        let mut cfg = Config::default();
        cfg.folders.push(FolderGrant {
            name: "documents".to_string(),
            path: "/tmp/docs".to_string(),
            read_write: false,
        });
        let sandbox = temp_dir("ws3");
        let err = resolve_root(&cfg, &sandbox, "desktop").unwrap_err();
        assert!(err.to_string().contains("documents"));
    }

    #[test]
    fn granted_folder_resolves_with_declared_readonly_flag() {
        let mut cfg = Config::default();
        cfg.folders.push(FolderGrant {
            name: "documents".to_string(),
            path: "/home/user/Documents".to_string(),
            read_write: false,
        });
        cfg.folders.push(FolderGrant {
            name: "scratch".to_string(),
            path: "/home/user/scratch".to_string(),
            read_write: true,
        });
        let sandbox = temp_dir("ws4");
        let (_, ro_docs) = resolve_root(&cfg, &sandbox, "documents").unwrap();
        let (_, ro_scratch) = resolve_root(&cfg, &sandbox, "scratch").unwrap();
        assert!(ro_docs, "documents grant should be read-only by default");
        assert!(!ro_scratch, "scratch grant was created read-write");
    }

    #[test]
    fn safe_join_rejects_traversal_and_absolute() {
        let root = temp_dir("boundary");
        assert!(safe_join(&root, "../escape.txt").is_err());
        assert!(safe_join(&root, "/etc/passwd").is_err());
        assert!(safe_join(&root, "~/secrets").is_err());
    }

    #[test]
    fn safe_join_accepts_in_root_paths_existing_and_new() {
        let root = temp_dir("boundary2");
        fs::write(root.join("existing.txt"), "hi").unwrap();
        assert!(safe_join(&root, "existing.txt").is_ok());
        assert!(safe_join(&root, "subdir/new_file.txt").is_ok());
    }

    #[test]
    fn safe_join_rejects_symlink_escape() {
        let root = temp_dir("boundary3");
        let outside = temp_dir("outside3");
        fs::write(outside.join("secret.txt"), "nope").unwrap();
        let link = root.join("escape_link");
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(&outside, &link);
            if link.exists() {
                assert!(safe_join(&root, "escape_link/secret.txt").is_err());
            }
        }
        #[cfg(windows)]
        {
            // Symlink creation on Windows requires elevated privileges in CI;
            // skip rather than flake, the canonicalize-based check is the same
            // code path exercised by the Unix branch and by path_within_root's
            // own tests.
            let _ = link;
        }
    }
}
