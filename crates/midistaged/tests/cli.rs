#[test]
fn cli_exposes_explicit_serve_and_read_only_inventory_commands() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_midistaged"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("serve"));
    assert!(text.contains("list-ports"));
}
