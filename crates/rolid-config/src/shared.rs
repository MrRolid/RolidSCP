//! Files that two running copies of the program share (`profiles.toml`,
//! `settings.toml`, the vault).
//!
//! Each copy remembers the text it last read from or wrote to a file. When it
//! saves and the file has changed since (the other copy saved), the three
//! versions are merged: what the other copy changed is kept, what this copy
//! changed is added, and where both changed the same value, the change being
//! saved now wins. Read, merge and write happen under a lock, so two copies
//! saving at once do not lose each other's changes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use toml::Value;

/// The text this process last read from or wrote to each file.
static SYNCED: Mutex<BTreeMap<PathBuf, String>> = Mutex::new(BTreeMap::new());

pub(crate) fn remember(path: &Path, text: &str) {
    SYNCED.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_owned(), text.to_owned());
}

pub(crate) fn synced(path: &Path) -> Option<String> {
    SYNCED.lock().unwrap_or_else(|e| e.into_inner()).get(path).cloned()
}

/// Hold the lock of the folder of `path` (released when dropped): `flock`
/// on the folder itself, so no lock file is left in it.
pub(crate) fn lock(path: &Path) -> Result<std::fs::File, String> {
    let dir = path.parent().ok_or("bad path")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let f = std::fs::File::open(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    f.lock().map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(f)
}

/// Three-way merge of TOML values: `base` is what both copies started from,
/// `ours` what this copy has, `theirs` what is on the disk now.
///
/// Tables are merged key by key and arrays of tables with an `id` (the
/// servers) item by item; anything else changed on both sides takes `ours`.
pub fn merge3(base: Option<&Value>, ours: &Value, theirs: &Value) -> Value {
    if Some(ours) == base || ours == theirs {
        return theirs.clone();
    }
    if Some(theirs) == base {
        return ours.clone();
    }
    match (ours, theirs) {
        (Value::Table(o), Value::Table(t)) => {
            let b = match base {
                Some(Value::Table(b)) => Some(b),
                _ => None,
            };
            let mut out = toml::Table::new();
            // Keys of theirs first (their order), then the ones only we have.
            let keys = t.keys().chain(o.keys().filter(|k| !t.contains_key(*k))).cloned().collect::<Vec<_>>();
            for k in keys {
                let bv = b.and_then(|b| b.get(&k));
                if let Some(v) = merge_item(bv, o.get(&k), t.get(&k)) {
                    out.insert(k, v);
                }
            }
            Value::Table(out)
        }
        (Value::Array(o), Value::Array(t)) if keyed(o) && keyed(t) => {
            let b: Vec<Value> = match base {
                Some(Value::Array(b)) if keyed(b) => b.clone(),
                _ => Vec::new(),
            };
            let find = |list: &[Value], id: &str| list.iter().find(|v| id_of(v) == Some(id)).cloned();
            let mut ids: Vec<String> = t.iter().filter_map(id_of).map(str::to_owned).collect();
            ids.extend(o.iter().filter_map(id_of).filter(|id| !t.iter().any(|v| id_of(v) == Some(id))).map(str::to_owned));
            let out = ids
                .iter()
                .filter_map(|id| merge_item(find(&b, id).as_ref(), find(o, id).as_ref(), find(t, id).as_ref()))
                .collect();
            Value::Array(out)
        }
        _ => ours.clone(),
    }
}

/// One key or item: `None` = it is not there after the merge.
fn merge_item(base: Option<&Value>, ours: Option<&Value>, theirs: Option<&Value>) -> Option<Value> {
    match (ours, theirs) {
        (Some(o), Some(t)) => Some(merge3(base, o, t)),
        // They removed it: gone, unless we changed it meanwhile.
        (Some(o), None) => (base != Some(o)).then(|| o.clone()),
        // We removed it: gone, unless they changed it meanwhile.
        (None, Some(t)) => (base != Some(t)).then(|| t.clone()),
        (None, None) => None,
    }
}

fn id_of(v: &Value) -> Option<&str> {
    v.as_table()?.get("id")?.as_str()
}

/// An array of tables that all have a distinct `id`.
fn keyed(list: &[Value]) -> bool {
    let ids: Vec<&str> = list.iter().filter_map(id_of).collect();
    ids.len() == list.len() && {
        let mut s = ids.clone();
        s.sort_unstable();
        s.dedup();
        s.len() == ids.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Value {
        Value::Table(toml::from_str(s).unwrap())
    }

    #[test]
    fn changes_of_both_copies_are_kept() {
        let base = v("a = 1\nb = 2\nc = 3");
        let ours = v("a = 10\nb = 2\nc = 3");
        let theirs = v("a = 1\nb = 20\nc = 3\nd = 4");
        assert_eq!(merge3(Some(&base), &ours, &theirs), v("a = 10\nb = 20\nc = 3\nd = 4"));
        // Both changed the same value: ours (being saved now) wins.
        let theirs = v("a = 5\nb = 2\nc = 3");
        assert_eq!(merge3(Some(&base), &ours, &theirs), v("a = 10\nb = 2\nc = 3"));
        // Removed on one side and not changed on the other: gone.
        assert_eq!(merge3(Some(&base), &v("a = 1\nb = 2"), &v("a = 1\nb = 2\nc = 3\nd = 4")), v("a = 1\nb = 2\nd = 4"));
        assert_eq!(merge3(Some(&base), &v("a = 10\nb = 2\nc = 3"), &v("a = 1\nb = 2")), v("a = 10\nb = 2"));
        // Nested tables.
        let base = v("[x]\np = 1\nq = 1");
        assert_eq!(merge3(Some(&base), &v("[x]\np = 2\nq = 1"), &v("[x]\np = 1\nq = 2")), v("[x]\np = 2\nq = 2"));
    }

    #[test]
    fn servers_are_merged_by_id() {
        let base = v("[[profile]]\nid = \"a\"\nname = \"A\"\n[[profile]]\nid = \"b\"\nname = \"B\"");
        // We renamed a, they added c and removed b.
        let ours = v("[[profile]]\nid = \"a\"\nname = \"A2\"\n[[profile]]\nid = \"b\"\nname = \"B\"");
        let theirs = v("[[profile]]\nid = \"a\"\nname = \"A\"\n[[profile]]\nid = \"c\"\nname = \"C\"");
        let m = merge3(Some(&base), &ours, &theirs);
        assert_eq!(m, v("[[profile]]\nid = \"a\"\nname = \"A2\"\n[[profile]]\nid = \"c\"\nname = \"C\""));
        // We added d at the same time: kept too.
        let ours = v("[[profile]]\nid = \"a\"\nname = \"A\"\n[[profile]]\nid = \"b\"\nname = \"B\"\n[[profile]]\nid = \"d\"\nname = \"D\"");
        let m = merge3(Some(&base), &ours, &theirs);
        assert_eq!(m, v("[[profile]]\nid = \"a\"\nname = \"A\"\n[[profile]]\nid = \"c\"\nname = \"C\"\n[[profile]]\nid = \"d\"\nname = \"D\""));
    }

    #[test]
    fn no_base_keeps_both_sides() {
        // A file this copy never read (it did not exist at the start): their
        // keys stay, ours win where both have one.
        assert_eq!(merge3(None, &v("a = 1"), &v("a = 2\nb = 3")), v("a = 1\nb = 3"));
    }
}
