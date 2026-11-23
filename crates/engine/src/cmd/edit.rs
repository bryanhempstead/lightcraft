use serde_json::json;

use super::{CommandSpec, can_redo, can_undo, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{}", can_undo, |s, _| {
            s.end_interaction()?;
            let undone = s.undo_step()?;
            // a delete's files that couldn't come back from the Trash (emptied, path taken)
            if s.not_back.is_empty() {
                Ok(json!({"undone": undone}))
            } else {
                Ok(json!({"undone": undone, "notBack": std::mem::take(&mut s.not_back)}))
            }
        }),
        cmd!("edit.redo", "Redo", ["Edit"], Some("Cmd+Shift+Z"), "{}", can_redo, |s, _| Ok(json!({"redone": s.redo_step()?}))),
    ]
}
