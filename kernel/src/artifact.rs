//! Canonical provenance rules for artifacts admitted to production boundaries.
//! Mutable references and executable model formats fail closed.

use anyhow::{bail, Result};
use std::path::Path;

pub fn validate_oci_digest_reference(reference: &str) -> Result<()> {
    let Some((repository, digest)) = reference.rsplit_once("@sha256:") else {
        bail!("OCI artifact must be pinned by @sha256 digest");
    };
    if repository.trim().is_empty()
        || repository.contains('@')
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("OCI artifact has a non-canonical sha256 reference");
    }
    Ok(())
}

pub fn validate_source_revision(revision: &str) -> Result<()> {
    if !matches!(revision.len(), 40 | 64)
        || !revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("source revision must be a full lowercase hexadecimal object id");
    }
    Ok(())
}

pub fn validate_model_artifact(path: &Path) -> Result<()> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "safetensors" | "gguf" => Ok(()),
        "bin" | "pt" | "pth" | "ckpt" | "pkl" | "pickle" => {
            bail!("executable or pickle-capable model format is prohibited")
        }
        _ => bail!("unknown model artifact format is prohibited"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_oci_references_are_canonical_and_mutable_tags_fail_closed() {
        let digest = "a".repeat(64);
        assert!(validate_oci_digest_reference(&format!("registry/plugin@sha256:{digest}")).is_ok());
        let invalid = vec![
            "registry/plugin:latest".to_string(),
            "registry/plugin@sha256:short".to_string(),
            format!("registry/plugin@sha256:{}", "A".repeat(64)),
            format!("registry/plugin@other@sha256:{digest}"),
        ];
        for reference in invalid {
            assert!(
                validate_oci_digest_reference(&reference).is_err(),
                "{reference}"
            );
        }
    }

    #[test]
    fn source_revisions_require_full_object_ids() {
        assert!(validate_source_revision(&"1".repeat(40)).is_ok());
        assert!(validate_source_revision(&"a".repeat(64)).is_ok());
        assert!(validate_source_revision("main").is_err());
        assert!(validate_source_revision(&"A".repeat(40)).is_err());
    }

    #[test]
    fn model_policy_allows_data_only_formats_and_rejects_pickle_or_unknown_files() {
        assert!(validate_model_artifact(Path::new("model.safetensors")).is_ok());
        assert!(validate_model_artifact(Path::new("model.GGUF")).is_ok());
        for unsafe_path in [
            "model.bin",
            "model.pt",
            "model.pth",
            "model.ckpt",
            "model.pkl",
            "model.onnx",
        ] {
            assert!(
                validate_model_artifact(Path::new(unsafe_path)).is_err(),
                "{unsafe_path}"
            );
        }
    }
}
