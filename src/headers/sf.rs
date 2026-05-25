//! Small shared helpers for parsing and emitting RFC 9651 Structured Field items.
//!
//! `sfparse` hands back byte ranges into the input plus an `escape` flag rather than decoded
//! values (that's what makes it zero-allocation); these helpers turn those ranges into `Cow`s,
//! write the quoting/escaping back out, and skip over list items we don't model.

use super::ParseError;
use sfparse::{Parser, Value};
use std::{
    borrow::Cow,
    fmt::{self, Write},
    ops::Range,
};

/// Materialize an SF String from its source `range` and `escape` flag: borrow when there are no
/// escape sequences, otherwise allocate and strip the backslashes.
pub(crate) fn string_value(input: &str, range: Range<usize>, escape: bool) -> Cow<'_, str> {
    let raw = &input[range];
    if escape {
        Cow::Owned(unescape(raw))
    } else {
        Cow::Borrowed(raw)
    }
}

fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(escaped) = chars.next() {
                out.push(escaped);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Write `value` as an SF String — wrapped in double quotes with `\` and `"` backslash-escaped.
pub(crate) fn write_sf_string(f: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    f.write_char('"')?;
    for c in value.chars() {
        if matches!(c, '\\' | '"') {
            f.write_char('\\')?;
        }
        f.write_char(c)?;
    }
    f.write_char('"')
}

/// Drain and discard the remaining parameters of the current item.
pub(crate) fn drain_params(parser: &mut Parser<'_>) -> Result<(), ParseError> {
    while parser.parse_param().map_err(|_| ParseError)?.is_some() {}
    Ok(())
}

/// Skip a list item we don't model: drain any inner-list members, then its parameters.
pub(crate) fn skip_item(parser: &mut Parser<'_>, value: Value) -> Result<(), ParseError> {
    if matches!(value, Value::InnerList) {
        while parser.parse_inner_list().map_err(|_| ParseError)?.is_some() {}
    }
    drain_params(parser)
}
