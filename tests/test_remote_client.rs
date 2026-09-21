use infant_hand_motion_viewer::remote::{frame_number_from_name, is_valid_ssh_host};

#[test]
fn ssh_hosts_that_look_like_options_or_contain_whitespace_are_rejected() {
    assert!(is_valid_ssh_host("user@example.com"));
    assert!(is_valid_ssh_host("my-alias"));
    assert!(!is_valid_ssh_host("-oProxyCommand=evil"));
    assert!(!is_valid_ssh_host("host name"));
    assert!(!is_valid_ssh_host("host\n"));
    assert!(!is_valid_ssh_host(""));
}

#[test]
fn frame_numbers_are_parsed_from_file_names() {
    assert_eq!(frame_number_from_name("frame_00042.jpg"), Some(42));
    assert_eq!(frame_number_from_name("dir/sub/0007.png"), Some(7));
    assert_eq!(frame_number_from_name("noframe.jpg"), None);
}
