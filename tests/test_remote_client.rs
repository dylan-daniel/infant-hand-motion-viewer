use infant_hand_motion_viewer::remote::is_valid_ssh_host;

#[test]
fn ssh_hosts_that_look_like_options_or_contain_whitespace_are_rejected() {
    assert!(is_valid_ssh_host("user@example.com"));
    assert!(is_valid_ssh_host("my-alias"));
    assert!(!is_valid_ssh_host("-oProxyCommand=evil"));
    assert!(!is_valid_ssh_host("host name"));
    assert!(!is_valid_ssh_host("host\n"));
    assert!(!is_valid_ssh_host(""));
}
