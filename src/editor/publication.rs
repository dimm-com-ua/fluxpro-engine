//! Preparing a new immutable, immediately active definition version.
use crate::editor::EditorDocument;
use crate::models::{process_def::ProcessStatus, version_id::VersionId};

impl EditorDocument {
    /// Suggests the next patch version without wrapping overflowing components.
    pub fn next_publication_version(&self) -> Result<VersionId, String> {
        let v = &self.definition.version;
        let patch = v
            .patch()
            .checked_add(1)
            .ok_or("Patch version exhausted; choose a new minor version")?;
        VersionId::new(format!("{}.{}.{}", v.major(), v.minor(), patch))
    }

    /// Keeps layout and content, but publishes a fresh identity in Active state now.
    /// Existing definitions must keep their key and advance their numeric version.
    pub fn prepare_publication(
        &self,
        base: Option<&EditorDocument>,
        version: &str,
    ) -> Result<Self, String> {
        let version = VersionId::new(version.trim())?;
        if let Some(base) = base {
            if self.definition.key != base.definition.key {
                return Err("An existing process must keep its key. Use New process to create another process.".into());
            }
            if version <= base.definition.version {
                return Err("Choose a version newer than the published version.".into());
            }
        }
        let mut published = Self::new(self.definition.clone())?;
        for (id, position) in &self.positions {
            if let Some(target) = published.positions.get_mut(id) {
                *target = crate::editor::Position::new(position.x, position.y);
            }
        }
        published.definition.uuid = None;
        published.definition.version = version;
        published.definition.status = ProcessStatus::Active;
        published.definition.effective_from = None; // Engine registration supplies current UTC.
        published.definition.deprecated_at = None;
        published.definition.compile().map_err(|e| e.to_string())?;
        Ok(published)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_preserves_layout_and_content_and_requires_new_version() {
        let original = EditorDocument::default();
        let mut edited = original.clone();
        edited
            .positions
            .insert("start".into(), crate::editor::Position::new(400., 220.));
        edited.definition.name = "Edited process".into();
        let version = original.next_publication_version().unwrap();
        let published = edited
            .prepare_publication(Some(&original), version.as_str())
            .unwrap();
        assert_eq!(published.positions, edited.positions);
        assert_eq!(published.definition.name, "Edited process");
        assert!(matches!(published.definition.status, ProcessStatus::Active));
        assert!(published.definition.uuid.is_none());
        assert!(published.definition.deprecated_at.is_none());
        assert!(
            edited
                .prepare_publication(Some(&original), original.definition.version.as_str())
                .is_err()
        );
        edited.definition.key = crate::models::id_field::IdField::new("other").unwrap();
        assert!(
            edited
                .prepare_publication(Some(&original), version.as_str())
                .is_err()
        );
    }
    #[test]
    fn new_process_must_compile_before_publication() {
        let mut doc = EditorDocument::default();
        assert!(doc.prepare_publication(None, "1.0.0").is_ok());
        doc.definition.nodes.clear();
        assert!(doc.prepare_publication(None, "1.0.0").is_err());
    }
}
