use super::*;

#[test]
fn proportional_font_family_supports_bigram_arrow() {
    let fonts = font_definitions();
    assert!(fonts.font_data.contains_key(HACK_FONT_NAME));
    assert!(
        fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .unwrap()
            .iter()
            .any(|font| font == HACK_FONT_NAME)
    );
}
#[test]
fn storage_labels_cover_unsaved_and_saved_sessions() {
    assert_eq!(
        storage_status_label(StorageStatus::Unsaved, false),
        "Not saved"
    );
    assert_eq!(storage_status_label(StorageStatus::Saved, true), "Saved");
    assert_eq!(
        storage_status_label(StorageStatus::Dirty, true),
        "Unsaved changes"
    );
    assert_eq!(storage_status_label(StorageStatus::Saving, true), "Saving…");
    assert_eq!(
        storage_status_label(StorageStatus::Failed, true),
        "Storage failed"
    );
    assert_eq!(
        storage_status_label_for_operation(
            StorageStatus::Failed,
            true,
            Some(StorageOperation::Save),
        ),
        "Save failed"
    );
    assert_eq!(
        storage_status_label_for_operation(
            StorageStatus::Failed,
            true,
            Some(StorageOperation::Load),
        ),
        "Load failed"
    );
}
