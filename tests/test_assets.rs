use infant_hand_motion_viewer::assets;

#[test]
fn test_embedded_assets_presence() {
    assert!(!assets::MANO_FACES.is_empty(), "MANO faces asset must not be empty");
    assert!(!assets::MANO_MODEL.is_empty(), "MANO model asset must not be empty");
    assert!(!assets::UI_FONT.is_empty(), "UI font asset must not be empty");
    assert_eq!(
        &assets::MANO_MODEL[0..4],
        b"MANO",
        "MANO model must start with MANO magic"
    );

    assert!(!assets::ICON_FOLDER_CLOSED.is_empty());
    assert!(!assets::ICON_FOLDER_OPEN.is_empty());
    assert!(!assets::ICON_FILE_HEXPORT.is_empty());
    assert!(!assets::ICON_CHANGE_ROOT.is_empty());
    assert!(!assets::ICON_REFRESH.is_empty());
    assert!(!assets::ICON_PLAY.is_empty());
    assert!(!assets::ICON_PAUSE.is_empty());
    assert!(!assets::ICON_SPEED.is_empty());
}
