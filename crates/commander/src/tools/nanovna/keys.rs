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
    Max,
}

/// (group, key letter, single, double, triple) — used for tooltips and the docs.
pub const TABLE: [(Group, char, &str, &str, &str); 9] = [
    (Group::Run, 'R', "continuous", "single sweep", "hold"),
    (Group::Layout, 'L', "next layout 1/2/4", "swap pane order", "default four-pane set"),
    (Group::Trace, 'T', "next trace (again on the same: show/hide)", "trace menu", "store memory"),
    (Group::Store, 'S', "store memory", "memory on/off", "cycle math"),
    (Group::Auto, 'A', "auto-scale trace", "auto-scale all", "reset to auto"),
    (Group::Limits, 'K', "limits on/off", "edit limits", "SWR 1.5 preset"),
    (Group::Marker, 'M', "active marker (← → move it)", "next marker", "reference marker R"),
    (Group::Status, 'I', "device details", "reconnect", "copy status line"),
    (Group::Max, 'X', "maximize or restore the active trace's pane", "maximize the next pane", "restore the grid"),
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

/// Click mode for a pointer click on a multi-mode key: 0 = no click.
/// Double / triple clicks count as themselves; a single click with Shift is a
/// double (alt 1) and with Ctrl/Cmd a triple (alt 2), the rustcars convention.
pub fn click_mode(clicked: bool, double: bool, triple: bool, shift: bool, ctrl: bool) -> u8 {
    if triple {
        3
    } else if double {
        2
    } else if clicked {
        mode(shift, ctrl)
    } else {
        0
    }
}

/// Pane to maximize for a MAX key op (1 / 2 / 3), from the current layout.
/// Returns the new `maximized` value.
pub fn max_op(op: u8, panes: u8, maximized: Option<u8>, active_pane: u8) -> Option<u8> {
    let n = panes.max(1);
    if n == 1 {
        return None;
    }
    match op {
        1 => if maximized.is_some() { None } else { Some(active_pane.min(n - 1)) },
        2 => Some(maximized.map(|p| (p + 1) % n).unwrap_or(active_pane.min(n - 1))),
        _ => None,
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

    #[test]
    fn shift_and_ctrl_click_act_as_double_and_triple() {
        assert_eq!(click_mode(false, false, false, true, true), 0);
        assert_eq!(click_mode(true, false, false, false, false), 1);
        assert_eq!(click_mode(true, false, false, true, false), 2);
        assert_eq!(click_mode(true, false, false, false, true), 3);
        assert_eq!(click_mode(true, false, false, true, true), 3);
        assert_eq!(click_mode(true, true, false, false, false), 2);
        assert_eq!(click_mode(true, true, true, false, false), 3);
    }

    #[test]
    fn max_key_toggles_cycles_and_restores() {
        assert_eq!(max_op(1, 1, None, 0), None);
        assert_eq!(max_op(1, 4, None, 2), Some(2));
        assert_eq!(max_op(1, 4, Some(2), 2), None);
        assert_eq!(max_op(2, 4, Some(3), 0), Some(0));
        assert_eq!(max_op(2, 2, None, 1), Some(1));
        assert_eq!(max_op(3, 4, Some(1), 0), None);
        assert_eq!(group_of('x'), Some(Group::Max));
    }
}
