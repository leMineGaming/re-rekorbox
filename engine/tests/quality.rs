//! The automatic listening checks (rbx_engine::quality) all pass.

#[test]
fn all_quality_checks_pass() {
    let checks = rbx_engine::quality::run_all();
    assert!(checks.len() >= 25);
    let failed: Vec<String> = checks.iter().filter(|c| !c.passed).map(|c| format!("{}: {} = {} ({})", c.group, c.name, c.value, c.detail)).collect();
    assert!(failed.is_empty(), "{failed:#?}");
}
