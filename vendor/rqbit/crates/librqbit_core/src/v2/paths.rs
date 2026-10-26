//! Deterministic storage names; raw metainfo bytes remain authoritative.
use super::{MAX_METAINFO_LENGTH, Result, V2Error, sha256};
use std::collections::{BTreeMap, BTreeSet};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

fn collision_key(name: &str) -> String {
    name.nfkd()
        .case_fold()
        .nfkd()
        .filter(|ch| !default_ignorable(*ch))
        .collect()
}

fn suffixed(mut prefix: String, raw: &[u8]) -> String {
    // %h cannot occur in an ordinary escaped component: literal '%' is
    // escaped, and byte escapes contain exactly two hexadecimal digits.
    while prefix.len() > 180 {
        prefix.pop();
    }
    prefix.push_str("%h");
    prefix.push_str(&hex::encode(sha256(raw)));
    prefix
}

// Unicode 17 Default_Ignorable_Code_Point, DerivedCoreProperties.txt.
// https://www.unicode.org/Public/17.0.0/ucd/DerivedCoreProperties.txt
// Copyright Unicode, Inc.; see vendor/UNICODE-LICENSE.txt.
// Used conservatively for collision detection, never to alter info bytes.
fn default_ignorable(ch: char) -> bool {
    matches!(ch as u32, 0x00ad | 0x034f | 0x061c | 0x115f..=0x1160
        | 0x17b4..=0x17b5 | 0x180b..=0x180f | 0x200b..=0x200f
        | 0x202a..=0x202e | 0x2060..=0x206f | 0x3164 | 0xfe00..=0xfe0f
        | 0xfeff | 0xffa0 | 0xfff0..=0xfff8 | 0x1bca0..=0x1bca3
        | 0x1d173..=0x1d17a | 0xe0000..=0xe0fff)
}

fn finish(mut out: String, raw: &[u8], shortened: bool) -> String {
    if shortened {
        out = suffixed(out, raw);
    }
    let key = collision_key(&out);
    if key.is_empty() || matches!(key.as_str(), "." | "..") {
        return suffixed(String::new(), raw);
    }
    let stem = key.split('.').next().unwrap_or_default();
    if matches!(stem, "con" | "prn" | "aux" | "nul" | "conin$" | "conout$")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        // Another reserved namespace, distinct from byte escapes and hashes.
        out.insert_str(0, "%r");
    }
    out
}

pub(super) fn component(raw: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let trailing = raw
        .iter()
        .rposition(|b| !matches!(b, b'.' | b' '))
        .map_or(0, |at| at + 1);
    let mut at = 0;
    // utf8_chunks scans once, without lossily turning invalid/overlong bytes
    // into dots or separators. Output allocation never grows past one component.
    for chunk in raw.utf8_chunks() {
        for ch in chunk.valid().chars() {
            let size = ch.len_utf8();
            if ch.is_control()
                || matches!(
                    ch,
                    '%' | '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*'
                )
                || (at >= trailing && matches!(ch, '.' | ' '))
            {
                for byte in &raw[at..at + size] {
                    write!(&mut out, "%{byte:02X}").unwrap();
                }
            } else {
                out.push(ch);
            }
            at += size;
            if out.len() > 240 {
                return finish(out, raw, true);
            }
        }
        for byte in chunk.invalid() {
            write!(&mut out, "%{byte:02X}").unwrap();
            at += 1;
            if out.len() > 240 {
                return finish(out, raw, true);
            }
        }
    }
    finish(out, raw, false)
}

pub(super) fn components<'a>(keys: impl Iterator<Item = &'a [u8]>) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut budget = MAX_METAINFO_LENGTH;
    for raw in keys {
        let name = component(raw);
        let key = collision_key(&name);
        budget = budget
            .checked_sub(
                name.len() + 2 * key.len() + std::mem::size_of::<(String, String, &[u8])>(),
            )
            .ok_or(V2Error::Invalid("file names exceed 32 MiB"))?;
        *counts.entry(key.clone()).or_default() += 1;
        names.push((raw, name, key));
    }
    let mut used = BTreeSet::new();
    names
        .into_iter()
        .map(|(raw, name, key)| {
            let name = if counts[&key] > 1 {
                suffixed(name, raw)
            } else {
                name
            };
            if !used.insert(collision_key(&name)) {
                return Err(V2Error::Invalid("sanitized file name collision"));
            }
            Ok(name)
        })
        .collect()
}
