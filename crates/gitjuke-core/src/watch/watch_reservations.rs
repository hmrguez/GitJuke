//! Stream a pending job's Heads entries, retaining no global reserved-ID set.
//! SQLite evaluates one entry at a time to preserve json_each(value)/id semantics.
use crate::{Error, Result, publication_completion::observe};
use rusqlite::Connection;
use serde::{
    Deserializer,
    de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use serde_json::value::RawValue;
use std::fmt;

fn invalid(_: serde_json::Error) -> Error {
    Error::StorageUnavailable(
        "Invalid stored head reservation; original database preserved.".into(),
    )
}
struct Entries<'a, F>(&'a mut F);
impl<'de, F: FnMut(&RawValue) -> Result<()>> Visitor<'de> for Entries<'_, F> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("head reservation entries")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> std::result::Result<(), A::Error> {
        while let Some(value) = a.next_element::<&RawValue>()? {
            (self.0)(value).map_err(de::Error::custom)?;
        }
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> std::result::Result<(), A::Error> {
        while let Some((_, value)) = a.next_entry::<IgnoredAny, &RawValue>()? {
            (self.0)(value).map_err(de::Error::custom)?;
        }
        Ok(())
    }
}
pub(crate) fn each(
    c: &Connection,
    job: &str,
    mut found: impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    // SQLite has already projected/canonicalized the exact Heads path. Borrow
    // that JSON rather than collecting its entries or reinterpreting root keys.
    // Whole-job/path parsing and variable bytes remain separate open gates.
    let heads: &RawValue = serde_json::from_str(job).map_err(invalid)?;
    let mut entry = |raw: &RawValue| -> Result<()> {
        // json_each over this one-element wrapper has exactly one candidate.
        // In particular, string entries use their decoded SQL value just as in
        // the old query, including nested JSON text and malformed-text failures.
        let mut q=c.prepare("SELECT CASE WHEN json_type(value,'$.id')='text' THEN json_extract(value,'$.id') END FROM json_each(?)")?;
        let id: Option<String> = q.query_row([format!("[{}]", raw.get())], |r| r.get(0))?;
        observe(&q, 1);
        if let Some(id) = id {
            found(&id)?;
        }
        Ok(())
    };
    let mut d = serde_json::Deserializer::from_str(heads.get());
    match heads.get().as_bytes().first() {
        Some(b'[') => d.deserialize_seq(Entries(&mut entry)).map_err(invalid)?,
        Some(b'{') => d.deserialize_map(Entries(&mut entry)).map_err(invalid)?,
        _ => entry(heads)?,
    }
    Ok(())
}
