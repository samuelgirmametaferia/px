//! Panel alignment: every row of a box must render the same VISIBLE width,
//! no matter how the content is styled. This was broken twice (ANSI bytes
//! counted as width; widest line overflowed its pad target) — now it's
//! pinned by tests with the exact shapes that broke it.

use px::ui::table::{panel, visible_len};

fn styled(s: &str) -> String {
    format!("\x1b[1;35m{s}\x1b[0m")
}

#[test]
fn every_panel_row_has_equal_visible_width() {
    let lines = vec![
        styled("antigravity 2.12.2-1  (aur)"),
        styled("sl  (repo)"),
        "plain line without any styling".to_string(),
    ];
    let width = lines.iter().map(|l| visible_len(l)).max().unwrap();
    let out = panel("install plan", &lines, width);
    let widths: Vec<usize> = out.lines().map(visible_len).collect();
    assert!(
        widths.iter().all(|w| *w == widths[0]),
        "rows differ in width: {widths:?}\n{out}"
    );
}

#[test]
fn panel_widest_line_fits_exactly() {
    // the regression: the widest line overflowed w-1 padding, pushing the
    // right border out one cell
    let lines = vec![styled("antigravity 2.12.2-1  (aur)")];
    let width = visible_len(&lines[0]);
    let out = panel("install plan", &lines, width);
    let body = out.lines().nth(1).unwrap();
    assert_eq!(visible_len(body), visible_len(out.lines().next().unwrap()));
}

#[test]
fn overlong_lines_are_truncated_not_overflowing() {
    let long = "x".repeat(200);
    let lines = vec![styled(&long)];
    let out = panel("install plan", &lines, 80); // caller caps at 80
    let widths: Vec<usize> = out.lines().map(visible_len).collect();
    assert!(
        widths.iter().all(|w| *w == widths[0]),
        "overlong line broke the box: {widths:?}"
    );
    assert!(
        widths[0] <= 84,
        "box should stay ~80 wide, got {}",
        widths[0]
    );
}

#[test]
fn visible_len_ignores_ansi() {
    assert_eq!(visible_len("\x1b[1;32mhi\x1b[0m"), 2);
    assert_eq!(visible_len("plain"), 5);
    assert_eq!(visible_len(""), 0);
}
