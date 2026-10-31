//! `PerKey<T>`: one value per physical key, serialised as a JSON object keyed by key code in layout order.

use crate::layout;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

/// Values keyed by key index. Only physical keys (layout table) may be present.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PerKey<T>(pub BTreeMap<u8, T>);

impl<T> PerKey<T> {
    pub fn new() -> Self {
        PerKey(BTreeMap::new())
    }

    /// Every physical key mapped through `f`.
    pub fn from_fn(mut f: impl FnMut(&layout::Key) -> T) -> Self {
        PerKey(layout::KEYS.iter().map(|k| (k.index, f(k))).collect())
    }

    pub fn get(&self, idx: usize) -> Option<&T> {
        u8::try_from(idx).ok().and_then(|i| self.0.get(&i))
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &T)> {
        self.0.iter().map(|(i, v)| (*i as usize, v))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Key indices whose value differs between `self` and `other` (or exists in only one), ascending.
    pub fn changed_keys(&self, other: &Self) -> Vec<usize>
    where
        T: PartialEq,
    {
        let mut keys: Vec<u8> = self.0.keys().chain(other.0.keys()).copied().collect();
        keys.sort_unstable();
        keys.dedup();
        keys.into_iter().filter(|k| self.0.get(k) != other.0.get(k)).map(usize::from).collect()
    }
}

/// Code of a key index, for messages (`idx:N` for non-keys).
pub fn key_name(idx: usize) -> String {
    layout::by_index(idx).map_or_else(|| format!("idx:{idx}"), |k| k.code.to_string())
}

impl<T: Serialize> Serialize for PerKey<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for (idx, v) in &self.0 {
            let key = layout::by_index(*idx as usize)
                .ok_or_else(|| serde::ser::Error::custom(format!("index {idx} is not a key")))?;
            m.serialize_entry(key.code, v)?;
        }
        m.end()
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for PerKey<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = PerKey<T>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object keyed by key code (KeyA, Escape, ...)")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = BTreeMap::new();
                while let Some((code, v)) = map.next_entry::<String, T>()? {
                    let key = layout::by_code(&code)
                        .ok_or_else(|| serde::de::Error::custom(format!("unknown key code {code:?}")))?;
                    if out.insert(key.index, v).is_some() {
                        return Err(serde::de::Error::custom(format!("key {code:?} given twice")));
                    }
                }
                Ok(PerKey(out))
            }
        }
        d.deserialize_map(V(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_by_code_in_layout_order() {
        let mut p = PerKey::new();
        p.0.insert(110, 1u8);
        p.0.insert(22, 2u8);
        assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"Escape":2,"ControlLeft":1}"#);
        let back: PerKey<u8> = serde_json::from_str(r#"{"controlleft":1,"Escape":2}"#).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn empty_and_errors() {
        let empty: PerKey<u8> = serde_json::from_str("{}").unwrap();
        assert!(empty.is_empty());
        assert!(serde_json::from_str::<PerKey<u8>>(r#"{"Nope":1}"#).is_err());
        assert!(serde_json::from_str::<PerKey<u8>>(r#"{"KeyA":1,"keya":2}"#).is_err());
        assert!(serde_json::from_str::<PerKey<u8>>("[]").is_err());
        let mut bad = PerKey::new();
        bad.0.insert(0, 1u8);
        assert!(serde_json::to_string(&bad).is_err(), "index 0 is not a key");
    }

    #[test]
    fn from_fn_and_changes() {
        let a = PerKey::from_fn(|k| k.index);
        assert_eq!(a.len(), 61);
        let mut b = a.clone();
        assert!(a.changed_keys(&b).is_empty());
        b.0.insert(46, 0);
        b.0.remove(&22);
        assert_eq!(a.changed_keys(&b), vec![22, 46]);
        assert_eq!(a.get(46), Some(&46));
        assert_eq!(a.get(1000), None);
        assert_eq!(key_name(46), "KeyW");
        assert_eq!(key_name(0), "idx:0");
    }
}
