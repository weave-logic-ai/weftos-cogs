use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cog-bme280"))
}

#[test]
fn help_names_the_cog() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("cog-bme280"));
    assert!(text.contains("--simulate"));
}

#[test]
fn simulate_prints_the_worked_example_as_synthetic() {
    let out = bin().args(["--once", "--simulate"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("\"status\":\"simulate\""));
    assert!(text.contains("\"provenance\":\"SYNTHETIC\""));
    assert!(text.contains("\"temp_c\":\"25.08\""));
    assert!(text.contains("\"catalog_id\":\"bme280\""));
    assert!(!text.contains("\"provenance\":\"MEASURED\""));
}

#[test]
fn a_missing_device_is_no_source_and_not_a_fake_reading() {
    let out = bin().arg("--once").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("\"status\":\"no_source\""));
    assert!(text.contains("\"temp_c\":null"));
    assert!(!text.contains("25.08"));
}
