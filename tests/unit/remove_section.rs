//! Unit tests for the remove_overlay_section function.

use repoverlay::remove_overlay_section;

#[test]
fn empty_content() {
    let result = remove_overlay_section("", "test-overlay");
    assert_eq!(result, "");
}

#[test]
fn no_section_present() {
    let content = "*.log\n.DS_Store\n";
    let result = remove_overlay_section(content, "test-overlay");
    assert_eq!(result, "*.log\n.DS_Store\n");
}

#[test]
fn section_at_end() {
    let content = "*.log\n# repoverlay:test-overlay start\n.envrc\n.repoverlay\n# repoverlay:test-overlay end\n";
    let result = remove_overlay_section(content, "test-overlay");
    assert_eq!(result, "*.log\n");
}

#[test]
fn section_at_beginning() {
    let content = "# repoverlay:test-overlay start\n.envrc\n# repoverlay:test-overlay end\n*.log\n";
    let result = remove_overlay_section(content, "test-overlay");
    assert_eq!(result, "*.log\n");
}

#[test]
fn section_in_middle() {
    let content = "*.log\n# repoverlay:test-overlay start\n.envrc\n# repoverlay:test-overlay end\n.DS_Store\n";
    let result = remove_overlay_section(content, "test-overlay");
    assert_eq!(result, "*.log\n.DS_Store\n");
}

#[test]
fn only_section() {
    let content =
        "# repoverlay:test-overlay start\n.envrc\n.repoverlay\n# repoverlay:test-overlay end\n";
    let result = remove_overlay_section(content, "test-overlay");
    assert_eq!(result, "");
}

#[test]
fn removes_only_specified_overlay() {
    let content = "# repoverlay:overlay-a start\n.envrc\n# repoverlay:overlay-a end\n# repoverlay:overlay-b start\n.env\n# repoverlay:overlay-b end\n";
    let result = remove_overlay_section(content, "overlay-a");
    assert!(result.contains("overlay-b"));
    assert!(!result.contains("overlay-a"));
}
