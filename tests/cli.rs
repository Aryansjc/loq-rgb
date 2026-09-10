//! End-to-end CLI tests. The CLI is pointed at a mock controller (`--mock`)
//! and an isolated XDG config dir so nothing touches the real hardware or
//! the user's config.

use std::path::Path;
use std::process::{Command, Output};

fn cli(xdg_home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_loq-rgb-cli"))
        .args(args)
        .env("XDG_CONFIG_HOME", xdg_home)
        .output()
        .expect("run loq-rgb-cli")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

#[test]
fn dump_packet_matches_golden_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let o = cli(
        dir.path(),
        &[
            "dump-packet",
            "--effect",
            "static",
            "--speed",
            "1",
            "--brightness",
            "2",
            "--colors",
            "ff0000,00ff00,0000ff,ffffff",
        ],
    );
    assert!(o.status.success());
    let hex = stdout(&o);
    let expected = "cc 16 01 01 02 ff 00 00 00 ff 00 00 00 ff ff ff ff 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";
    assert_eq!(hex.trim(), expected);
}

#[test]
fn colour_wave_frames_are_static_packets_and_off_golden() {
    let dir = tempfile::tempdir().unwrap();
    let wave = cli(
        dir.path(),
        &[
            "dump-packet",
            "--effect",
            "flow-right",
            "--speed",
            "4",
            "--brightness",
            "1",
        ],
    );
    assert!(wave.status.success());
    // The wave is host-rendered: its frames are static packets (effect 0x01),
    // never the firmware wave code, and no direction flag is used.
    let wave_out = stdout(&wave);
    let bytes: Vec<&str> = wave_out.trim().split(' ').collect();
    assert_eq!(bytes[2], "01", "wave frame is a static packet");
    assert_eq!(bytes[18], "00");
    assert_eq!(bytes[19], "00");

    // Legacy effect names still work (migrated to the colour wave).
    let legacy = cli(dir.path(), &["dump-packet", "--effect", "wave-right"]);
    assert!(legacy.status.success());
    let legacy_out = stdout(&legacy);
    let legacy_bytes: Vec<&str> = legacy_out.trim().split(' ').collect();
    assert_eq!(
        legacy_bytes[2], "01",
        "legacy wave maps onto the colour wave"
    );

    let off = cli(dir.path(), &["dump-packet", "--effect", "off"]);
    assert!(off.status.success());
    let off_out = stdout(&off);
    let bytes: Vec<&str> = off_out.trim().split(' ').collect();
    assert_eq!(bytes[2], "00"); // effect 0
    assert_eq!(bytes[4], "00"); // brightness 0
}

#[test]
fn invalid_effect_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let o = cli(dir.path(), &["dump-packet", "--effect", "rainbow"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("unknown effect"));
}

#[test]
fn invalid_colour_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let o = cli(dir.path(), &["apply", "--colors", "notacolour"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("invalid colour"));
}

#[test]
fn apply_with_mock_succeeds_and_reports() {
    let dir = tempfile::tempdir().unwrap();
    let o = cli(
        dir.path(),
        &[
            "--mock",
            "apply",
            "--effect",
            "breath",
            "--speed",
            "3",
            "--brightness",
            "1",
            "--colors",
            "112233,445566",
        ],
    );
    assert!(o.status.success(), "stderr: {}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("Breathing"), "out: {out}");
    assert!(out.contains("zone 1: #112233"));
    assert!(out.contains("zone 2: #445566"));
    assert!(
        out.contains("zone 3: #445566"),
        "fewer colours repeat the last: {out}"
    );
    assert!(out.contains("zone 4: #445566"));
    assert!(out.contains("applied to mock controller"));
}

#[test]
fn profile_lifecycle_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let apply = cli(
        dir.path(),
        &[
            "--mock", "apply", "--effect", "smooth", "--speed", "2", "--save", "Night",
        ],
    );
    assert!(apply.status.success());

    let list = cli(dir.path(), &["profiles"]);
    assert!(list.status.success());
    let out = stdout(&list);
    assert!(out.contains("Night"), "out: {out}");
    assert!(
        out.contains("* Night"),
        "saved profile should become active: {out}"
    );

    // Apply by profile name.
    let by_name = cli(dir.path(), &["--mock", "apply", "--profile", "Night"]);
    assert!(by_name.status.success(), "stderr: {}", stderr(&by_name));
    assert!(stdout(&by_name).contains("Smooth flow"));

    // Applying an unknown profile fails with a clear message.
    let missing = cli(dir.path(), &["--mock", "apply", "--profile", "Ghost"]);
    assert!(!missing.status.success());
    assert!(stderr(&missing).contains("no profile named"));
}

#[test]
fn bare_apply_reapplies_active_profile() {
    let dir = tempfile::tempdir().unwrap();
    let save = cli(
        dir.path(),
        &[
            "--mock",
            "apply",
            "--effect",
            "flow-left",
            "--speed",
            "4",
            "--save",
            "Auto",
        ],
    );
    assert!(save.status.success(), "stderr: {}", stderr(&save));

    // Bare `apply` (autostart path) must re-apply the active profile.
    let apply = cli(dir.path(), &["--mock", "apply"]);
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    let out = stdout(&apply);
    assert!(out.contains("Colour wave ←"), "out: {out}");
    assert!(out.contains("speed: 4"), "out: {out}");
}

#[test]
fn delete_profile_requires_confirmation_flag() {
    let dir = tempfile::tempdir().unwrap();
    cli(dir.path(), &["--mock", "apply", "--save", "Alpha"]); // active = Alpha
    let o = cli(dir.path(), &["delete-profile", "Alpha"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("--yes"));
    // Profile still exists (nothing was deleted).
    let list = cli(dir.path(), &["profiles"]);
    assert!(stdout(&list).contains("Alpha"));
}

#[test]
fn delete_profile_keeps_others_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    cli(dir.path(), &["--mock", "apply", "--save", "Alpha"]); // active Alpha
    cli(dir.path(), &["--mock", "apply", "--save", "Beta"]); // active Beta

    let o = cli(dir.path(), &["delete-profile", "Alpha", "--yes"]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));

    // New process (restart): Alpha gone, Beta & Default untouched.
    let list = cli(dir.path(), &["profiles"]);
    let out = stdout(&list);
    assert!(!out.contains("Alpha"), "deleted profile reappeared: {out}");
    assert!(out.contains("Beta"));
    assert!(out.contains("Default"));
    assert!(out.contains("* Beta"), "Beta is still active: {out}");
}

#[test]
fn delete_active_profile_switches_active_never_touches_others() {
    let dir = tempfile::tempdir().unwrap();
    cli(dir.path(), &["--mock", "apply", "--save", "Alpha"]); // active Alpha
    cli(dir.path(), &["--mock", "apply", "--save", "Beta"]); // active Beta

    let o = cli(dir.path(), &["delete-profile", "Beta", "--yes"]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));
    assert!(stdout(&o).contains("switched to"));

    let list = cli(dir.path(), &["profiles"]);
    let out = stdout(&list);
    assert!(!out.contains("Beta"));
    assert!(out.contains("Alpha"), "sibling untouched: {out}");
    assert!(out.contains("Default"));
    assert!(out.contains("* Alpha"), "active moved to Alpha: {out}");
}

#[test]
fn delete_last_profile_leaves_only_reserved_off() {
    let dir = tempfile::tempdir().unwrap();
    // Fresh store has exactly one real profile: Default.
    let o = cli(dir.path(), &["delete-profile", "Default", "--yes"]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));

    let list = cli(dir.path(), &["profiles"]);
    let out = stdout(&list);
    assert!(
        !out.contains("Default"),
        "phantom Default resurrected: {out}"
    );
    assert!(out.contains("Off"), "reserved Off should remain: {out}");
    assert!(out.contains("* Off"), "active fell back to Off: {out}");
    // A second restart still does not resurrect Default.
    let again = cli(dir.path(), &["profiles"]);
    assert!(!stdout(&again).contains("Default"));
}

#[test]
fn reserved_off_profile_cannot_be_deleted() {
    let dir = tempfile::tempdir().unwrap();
    // Ensure Off exists first by deleting something (deletion adds it).
    cli(dir.path(), &["delete-profile", "Default", "--yes"]);
    let o = cli(dir.path(), &["delete-profile", "Off", "--yes"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("reserved"));
    let list = cli(dir.path(), &["profiles"]);
    assert!(stdout(&list).contains("Off"));
}

#[test]
fn detect_offers_actionable_output_even_without_hardware() {
    // On this dev machine the controller is present, so detect may succeed;
    // in a controller-less environment it must fail with the hint. Either
    // outcome is valid as long as output is coherent.
    let dir = tempfile::tempdir().unwrap();
    let o = cli(dir.path(), &["detect"]);
    if o.status.success() {
        let out = stdout(&o);
        assert!(out.contains("zones: 4"));
    } else {
        assert!(stderr(&o).contains("hint:"));
    }
}

#[test]
fn udev_print_contains_the_rule() {
    let dir = tempfile::tempdir().unwrap();
    let o = cli(dir.path(), &["udev", "--print"]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("SUBSYSTEM==\"hidraw\""));
    assert!(out.contains("ATTRS{idProduct}==\"c993\""));
}
