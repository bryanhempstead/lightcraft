//! `cameras.*`: the cameras in the library, what LightCraft uses for each, and cameras added by
//! hand. See [`crate::cameras`].

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::Result;

fn names(p: &Value, c: &str) -> Result<Option<Vec<String>>> {
    match p.get("names").or_else(|| p.get("name")) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(n)) => Ok(Some(vec![n.clone()])),
        Some(Value::Array(a)) => {
            let v: Vec<String> = a.iter().filter_map(Value::as_str).map(str::to_string).collect();
            if v.len() != a.len() {
                return Err(bad(c, "`names` must be camera names (strings)"));
            }
            Ok(Some(v))
        }
        Some(_) => Err(bad(c, "`names` must be a camera name or a list of them")),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "cameras.list",
            "Cameras",
            [],
            None,
            "{} → {cameras: [{name, make, model, photos, raws, decoder (native|libraw|embedded preview|rendered|unknown), profile: {status (adobe|embedded|own|fallback), standard, matching, …}, lenses: [{lens, photos, profile}], deltaE, added, resolved}], adobe, cache}",
            always,
            |s, _| Ok(crate::cameras::list(s))
        ),
        cmd!(
            "cameras.refresh",
            "Refresh Cameras",
            [],
            None,
            "{names?: [camera], all?: bool} resolve the library's cameras again (profiles and lens data from the Adobe data on this Mac, the decoder from one of their files); without names: the ones not resolved yet, or every one with all=true",
            always,
            |s, p| {
                const C: &str = "cameras.refresh";
                let only = names(p, C)?;
                let all = p.get("all").and_then(Value::as_bool).unwrap_or(false);
                let done = crate::cameras::refresh(s, only.as_deref(), all || only.is_some()).map_err(|e| bad(C, e))?;
                let mut v = crate::cameras::list(s);
                v["refreshed"] = json!(done);
                Ok(v)
            }
        ),
        cmd!(
            "cameras.add",
            "Add Camera",
            [],
            None,
            "{name} add a camera (as typed, or a name from cameras.known): its profiles and lens data are resolved and kept",
            always,
            |s, p| {
                const C: &str = "cameras.add";
                let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name` (e.g. \"Canon EOS R5\")"))?.to_string();
                crate::cameras::add(s, &name).map_err(|e| bad(C, e))
            }
        ),
        cmd!(
            "cameras.remove",
            "Remove Camera",
            [],
            None,
            "{name} forget a camera added by hand (its photos' cameras come back on refresh)",
            always,
            |s, p| {
                const C: &str = "cameras.remove";
                let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?.to_string();
                let removed = crate::cameras::remove(s, &name).map_err(|e| bad(C, e))?;
                Ok(json!({"removed": removed}))
            }
        ),
        cmd!(
            query "cameras.known",
            "Known Cameras",
            [],
            None,
            "{query?, limit?: 1..500 (20)} → [name]: cameras Camera Raw on this Mac has a profile for, matching the text",
            always,
            |_, p| {
                let q = str_param(p, "query").unwrap_or("");
                let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 500) as usize;
                Ok(json!(crate::cameras::known(q, limit)))
            }
        ),
    ]
}
