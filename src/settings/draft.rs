//! One sparse draft and one transaction for every settings page.
use super::store::{SettingsStore, Snapshot};
use anyhow::{Context, Result};
use toml::Value;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Key(String),
    Index(usize),
}
pub type Path = Vec<Part>;
pub fn key(path: &[Part], name: &str) -> Path {
    let mut p = path.to_vec();
    p.push(Part::Key(name.into()));
    p
}
pub fn index(path: &[Part], i: usize) -> Path {
    let mut p = path.to_vec();
    p.push(Part::Index(i));
    p
}
pub fn get<'a>(tree: &'a Value, path: &[Part]) -> Option<&'a Value> {
    path.iter().try_fold(tree, |v, p| match p {
        Part::Key(k) => v.get(k),
        Part::Index(i) => v.get(*i),
    })
}
pub fn put(tree: &mut Value, path: &[Part], value: Option<Value>) -> Result<()> {
    if path.is_empty() {
        *tree = value.context("Cannot delete configuration root")?;
        return Ok(());
    }
    match &path[0] {
        Part::Key(k) => {
            let table = tree.as_table_mut().context("Expected a table")?;
            if path.len() == 1 {
                match value {
                    Some(v) => {
                        table.insert(k.clone(), v);
                    }
                    None => {
                        table.remove(k);
                    }
                }
            } else {
                if !table.contains_key(k) {
                    if value.is_none() {
                        return Ok(());
                    }
                    table.insert(
                        k.clone(),
                        match &path[1] {
                            Part::Key(_) => Value::Table(Default::default()),
                            Part::Index(_) => Value::Array(vec![]),
                        },
                    );
                }
                put(table.get_mut(k).unwrap(), &path[1..], value)?;
            }
        }
        Part::Index(i) => {
            let values = tree.as_array_mut().context("Expected a list")?;
            if path.len() == 1 {
                match value {
                    Some(v) if *i == values.len() => values.push(v),
                    Some(v) => *values.get_mut(*i).context("Entry no longer exists")? = v,
                    None => {
                        anyhow::ensure!(*i < values.len(), "Entry no longer exists");
                        values.remove(*i);
                    }
                }
            } else {
                put(
                    values.get_mut(*i).context("Entry no longer exists")?,
                    &path[1..],
                    value,
                )?;
            }
        }
    }
    Ok(())
}
pub struct Draft {
    pub base: Snapshot,
    pub tree: Value,
    pub committed_theme: crate::appearance::Theme,
}
impl Draft {
    pub fn new(base: Snapshot) -> Self {
        Self {
            tree: base.tree.clone(),
            committed_theme: base.effective.appearance.theme.unwrap_or_default(),
            base,
        }
    }
    pub fn dirty(&self) -> bool {
        self.tree != self.base.tree
    }
    pub fn reload(&mut self, latest: Snapshot) -> Result<()> {
        self.committed_theme = latest.effective.appearance.theme.unwrap_or_default();
        let merged = super::store::rebase(&self.base.tree, &self.tree, &latest.tree)?;
        self.tree = merged;
        self.base = latest;
        Ok(())
    }
    pub fn saved(&mut self, snapshot: Snapshot) {
        self.committed_theme = snapshot.effective.appearance.theme.unwrap_or_default();
        self.tree = snapshot.tree.clone();
        self.base = snapshot;
    }
    pub fn save(&mut self, store: &SettingsStore) -> Result<()> {
        let saved = store.save(&self.base, &self.tree)?;
        self.saved(saved);
        Ok(())
    }
    pub fn remove_overrides(
        &mut self,
        path: &[Part],
        fields: &[super::fields::Field],
    ) -> Result<()> {
        // Frozen preset and execution definitions are authored values, not inherited preferences.
        if matches!(path.first(),Some(Part::Key(k)) if k=="preset")
            || path.iter().any(|p| matches!(p,Part::Key(k) if k=="checks"))
        {
            return Ok(());
        }
        // Entity lists and user-authored check definitions are never defaults.
        for field in fields {
            if field.readonly
                || matches!(
                    field.key,
                    "path" | "favorite" | "checks" | "discovery" | "project" | "preset"
                )
            {
                continue;
            }
            put(&mut self.tree, &key(path, field.key), None)?;
        }
        Ok(())
    }
    pub fn inherited(&self, path: &[Part], field: &super::fields::Field) -> Value {
        // Project leaf overrides inherit the current *draft* global preference.
        if matches!(path.first(),Some(Part::Key(k)) if k=="project") && path.len() >= 3 {
            let global = path[2..].to_vec();
            return get(&self.tree, &global)
                .cloned()
                .unwrap_or_else(|| field.default.clone());
        }
        field.default.clone()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_false_empty_and_array_order_remain_distinct() {
        let mut tree: Value = toml::from_str("schema=1").unwrap();
        let p = key(&key(&[], "launch"), "agent_args");
        put(
            &mut tree,
            &p,
            Some(Value::Array(vec!["".into(), "中文 名称".into()])),
        )
        .unwrap();
        assert_eq!(get(&tree, &index(&p, 0)).unwrap().as_str(), Some(""));
        put(&mut tree, &index(&p, 0), None).unwrap();
        put(&mut tree, &index(&p, 0), None).unwrap();
        assert_eq!(get(&tree, &p).unwrap().as_array().unwrap().len(), 0);
        put(&mut tree, &p, None).unwrap();
        assert!(get(&tree, &p).is_none());
        let p = key(&key(&[], "launch"), "new_window");
        put(&mut tree, &p, Some(false.into())).unwrap();
        assert_eq!(get(&tree, &p).unwrap().as_bool(), Some(false));
    }
}
