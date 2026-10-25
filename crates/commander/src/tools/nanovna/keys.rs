//! Keyboard equivalents for every single / double / triple click action.
//! Same convention as rustcars mode buttons: plain key = single click,
//! Shift = double click (alt 1, orange), Ctrl = triple click (alt 2, purple). Issue 73.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Run,
    Layout,
    Trace,
    Store,
    Auto,
    Limits,
    Marker,
    Status,
}

/// (group, key letter, single, double, triple) — used for tooltips and the docs.
pub const TABLE: [(Group, char, &str, &str, &str); 8] = [
    (Group::Run, 'R', "continuous", "single sweep", "hold"),
    (Group::Layout, 'L', "next layout 1/2/4", "swap pane order", "default four-pane set"),
    (Group::Trace, 'T', "next trace (again on the same: show/hide)", "trace menu", "store memory"),
    (Group::Store, 'S', "store memory", "memory on/off", "cycle math"),
    (Group::Auto, 'A', "auto-scale trace", "auto-scale all", "reset to auto"),
    (Group::Limits, 'K', "limits on/off", "edit limits", "SWR 1.5 preset"),
    (Group::Marker, 'M', "active marker (← → move it)", "next marker", "reference marker R"),
    (Group::Status, 'I', "device details", "reconnect", "copy status line"),
];

/// Click mode 1/2/3 for a key press: plain, Shift, Ctrl.
pub fn mode(shift: bool, ctrl: bool) -> u8 {
    if ctrl {
        3
    } else if shift {
        2
    } else {
        1
    }
}

pub fn group_of(letter: char) -> Option<Group> {
    TABLE.iter().find(|row| row.1 == letter.to_ascii_uppercase()).map(|row| row.0)
}

/// Tooltip text: what each click does and its keys.
pub fn hint(group: Group) -> String {
    let (_, k, a, b, c) = TABLE.iter().find(|row| row.0 == group).copied().expect("every group is in TABLE");
    format!("Click: {a} [{k}]\nDouble-click: {b} [Shift+{k}]\nTriple-click: {c} [Ctrl+{k}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_group_has_one_unique_key_and_three_actions() {
        for (i, row) in TABLE.iter().enumerate() {
            assert!(TABLE[..i].iter().all(|r| r.1 != row.1 && r.0 != row.0));
            assert_eq!(group_of(row.1.to_ascii_lowercase()), Some(row.0));
            assert!(!row.2.is_empty() && !row.3.is_empty() && !row.4.is_empty());
        }
        assert_eq!((mode(false, false), mode(true, false), mode(true, true)), (1, 2, 3));
        assert!(hint(Group::Run).contains("Shift+R") && hint(Group::Run).contains("Ctrl+R"));
    }
}
