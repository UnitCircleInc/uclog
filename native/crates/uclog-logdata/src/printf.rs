//! C-printf formatting of logdata format strings (protocol.md §6.2).
//!
//! Argument sizes come from the logdata kinds, never from the conversion; length modifiers are
//! accepted and ignored.

/// A formatted argument.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// Integer with the width (in bytes) of its kind, so `%x` of a negative int32 prints the
    /// 32-bit two's complement, as C does.
    Int {
        v: i128,
        size: u8,
    },
    Float(f64),
    Str(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FormatError {
    #[error("format has more conversions than arguments")]
    TooFewArgs,
    #[error("format has fewer conversions than arguments")]
    TooManyArgs,
}

#[derive(Default, Clone, Copy)]
struct Spec {
    left: bool,
    plus: bool,
    space: bool,
    alt: bool,
    zero: bool,
    width: usize,
    prec: Option<usize>,
}

pub fn format(fmt: &str, args: &[Arg]) -> Result<String, FormatError> {
    let mut out = String::with_capacity(fmt.len() + 16);
    let mut it = args.iter();
    let b = fmt.as_bytes();
    let mut i = 0;
    let mut lit = 0;
    while i < b.len() {
        if b[i] != b'%' {
            i += 1;
            continue;
        }
        // Try to parse a conversion starting at i.
        let Some((spec, conv, end)) = parse(b, i + 1) else {
            i += 1;
            continue;
        };
        out.push_str(&fmt[lit..i]);
        if conv == b'%' {
            out.push('%');
        } else {
            let a = it.next().ok_or(FormatError::TooFewArgs)?;
            out.push_str(&convert(&spec, conv, a));
        }
        i = end;
        lit = end;
    }
    out.push_str(&fmt[lit..]);
    if it.next().is_some() {
        return Err(FormatError::TooManyArgs);
    }
    Ok(out)
}

fn parse(b: &[u8], mut i: usize) -> Option<(Spec, u8, usize)> {
    let mut s = Spec::default();
    while i < b.len() {
        match b[i] {
            b'-' => s.left = true,
            b'+' => s.plus = true,
            b' ' => s.space = true,
            b'#' => s.alt = true,
            b'0' => s.zero = true,
            _ => break,
        }
        i += 1;
    }
    while i < b.len() && b[i].is_ascii_digit() {
        s.width = s.width * 10 + (b[i] - b'0') as usize;
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let mut p = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            p = p * 10 + (b[i] - b'0') as usize;
            i += 1;
        }
        s.prec = Some(p);
    }
    for m in [&b"hh"[..], b"ll", b"h", b"l", b"z", b"j", b"t", b"L"] {
        if b[i.min(b.len())..].starts_with(m) {
            i += m.len();
            break;
        }
    }
    let conv = *b.get(i)?;
    if !b"diouxXcfFeEgGaAs%".contains(&conv) {
        return None;
    }
    Some((s, conv, i + 1))
}

fn pad(s: &Spec, sign: &str, prefix: &str, body: String, numeric: bool) -> String {
    let len = sign.len() + prefix.len() + body.chars().count();
    if len >= s.width {
        return format!("{sign}{prefix}{body}");
    }
    let n = s.width - len;
    if s.left {
        format!("{sign}{prefix}{body}{}", " ".repeat(n))
    } else if s.zero && numeric {
        format!("{sign}{prefix}{}{body}", "0".repeat(n))
    } else {
        format!("{}{sign}{prefix}{body}", " ".repeat(n))
    }
}

fn sign_of(s: &Spec, neg: bool) -> &'static str {
    if neg {
        "-"
    } else if s.plus {
        "+"
    } else if s.space {
        " "
    } else {
        ""
    }
}

fn as_int(a: &Arg) -> (i128, u8) {
    match a {
        Arg::Int { v, size } => (*v, *size),
        Arg::Float(f) => (*f as i128, 8),
        Arg::Str(s) => (s.parse().unwrap_or(0), 8),
    }
}

fn as_float(a: &Arg) -> f64 {
    match a {
        Arg::Int { v, .. } => *v as f64,
        Arg::Float(f) => *f,
        Arg::Str(s) => s.parse().unwrap_or(f64::NAN),
    }
}

fn convert(s: &Spec, conv: u8, a: &Arg) -> String {
    match conv {
        b'd' | b'i' => {
            let (v, _) = as_int(a);
            int_body(s, v < 0, v.unsigned_abs().to_string(), "")
        }
        b'u' | b'o' | b'x' | b'X' => {
            let (v, size) = as_int(a);
            let u = if v < 0 { (v as u128) & ((1u128 << (size as u32 * 8)) - 1) } else { v as u128 };
            let (digits, prefix) = match conv {
                b'u' => (u.to_string(), ""),
                b'o' => (format!("{u:o}"), if s.alt && u != 0 { "0" } else { "" }),
                b'x' => (format!("{u:x}"), if s.alt && u != 0 { "0x" } else { "" }),
                _ => (format!("{u:X}"), if s.alt && u != 0 { "0X" } else { "" }),
            };
            let s2 = Spec { plus: false, space: false, ..*s };
            int_body(&s2, false, digits, prefix)
        }
        b'c' => {
            let ch = match a {
                Arg::Str(t) => t.chars().next().unwrap_or('\0'),
                _ => char::from_u32(as_int(a).0 as u32).unwrap_or('\u{FFFD}'),
            };
            pad(s, "", "", ch.to_string(), false)
        }
        b's' => {
            let t = match a {
                Arg::Str(t) => t.clone(),
                Arg::Int { v, .. } => v.to_string(),
                Arg::Float(f) => py_float_str(*f),
            };
            let t = match s.prec {
                Some(p) => t.chars().take(p).collect(),
                None => t,
            };
            pad(s, "", "", t, false)
        }
        _ => float_conv(s, conv, as_float(a)),
    }
}

fn int_body(s: &Spec, neg: bool, mut digits: String, prefix: &str) -> String {
    if let Some(p) = s.prec {
        if digits.len() < p {
            digits = "0".repeat(p - digits.len()) + &digits;
        }
    }
    // C: the 0 flag is ignored when a precision is given for integer conversions.
    let s2 = Spec { zero: s.zero && s.prec.is_none(), ..*s };
    pad(&s2, sign_of(s, neg), prefix, digits, true)
}

fn float_conv(s: &Spec, conv: u8, v: f64) -> String {
    let upper = conv.is_ascii_uppercase();
    let neg = v.is_sign_negative() && !v.is_nan();
    let a = v.abs();
    if !a.is_finite() {
        let t = if a.is_nan() { "nan" } else { "inf" };
        let t = if upper { t.to_uppercase() } else { t.to_string() };
        return pad(s, sign_of(s, neg), "", t, false);
    }
    let prec = s.prec.unwrap_or(6);
    let body = match conv.to_ascii_lowercase() {
        b'f' => {
            let mut t = format!("{a:.prec$}");
            if s.alt && prec == 0 {
                t.push('.');
            }
            t
        }
        b'e' => exp_fmt(a, prec, s.alt),
        b'g' => {
            let p = if prec == 0 { 1 } else { prec };
            let x = if a == 0.0 { 0 } else { exp_of(a, p - 1) };
            let mut t = if x < -4 || x >= p as i32 {
                exp_fmt(a, p - 1, s.alt)
            } else {
                let fp = (p as i32 - 1 - x).max(0) as usize;
                let mut t = format!("{a:.fp$}");
                if s.alt && !t.contains('.') {
                    t.push('.');
                }
                t
            };
            if !s.alt {
                t = strip_zeros(&t);
            }
            t
        }
        _ => hex_float(a, s.prec),
    };
    let body = if upper { body.to_uppercase() } else { body };
    let prefix = if conv.eq_ignore_ascii_case(&b'a') {
        if upper {
            "0X"
        } else {
            "0x"
        }
    } else {
        ""
    };
    let body =
        if prefix.is_empty() { body } else { body.trim_start_matches("0X").trim_start_matches("0x").to_string() };
    pad(s, sign_of(s, neg), prefix, body, true)
}

/// Decimal exponent of `a` after rounding to `p` fractional digits in %e form.
fn exp_of(a: f64, p: usize) -> i32 {
    let t = format!("{a:.p$e}");
    t[t.find('e').unwrap() + 1..].parse().unwrap()
}

fn exp_fmt(a: f64, prec: usize, alt: bool) -> String {
    let t = format!("{a:.prec$e}");
    let (m, e) = t.split_at(t.find('e').unwrap());
    let e: i32 = e[1..].parse().unwrap();
    let mut m = m.to_string();
    if alt && prec == 0 {
        m.push('.');
    }
    format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
}

fn strip_zeros(t: &str) -> String {
    let (m, e) = match t.find('e') {
        Some(i) => t.split_at(i),
        None => (t, ""),
    };
    let m = if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.') } else { m };
    format!("{m}{e}")
}

fn hex_float(a: f64, prec: Option<usize>) -> String {
    if a == 0.0 {
        return match prec {
            Some(p) if p > 0 => format!("0x0.{}p+0", "0".repeat(p)),
            _ => "0x0p+0".into(),
        };
    }
    let bits = a.to_bits();
    let mut exp = ((bits >> 52) & 0x7FF) as i32;
    let mut mant = bits & ((1u64 << 52) - 1);
    let lead = if exp == 0 {
        exp = -1022;
        0
    } else {
        exp -= 1023;
        1
    };
    let mut digits = format!("{mant:013x}");
    if let Some(p) = prec {
        if p < 13 {
            let shift = (13 - p) * 4;
            let round = 1u64 << (shift - 1);
            mant = (mant + round) >> shift;
            digits = if p == 0 { String::new() } else { format!("{mant:0p$x}") };
        } else {
            digits += &"0".repeat(p - 13);
        }
    } else {
        digits = digits.trim_end_matches('0').to_string();
    }
    let dot = if digits.is_empty() { "" } else { "." };
    format!("0x{lead}{dot}{digits}p{}{}", if exp < 0 { '-' } else { '+' }, exp.abs())
}

/// Python `str(float)`, used for `%s` of a double.
fn py_float_str(f: f64) -> String {
    if f.is_finite() && f == f.trunc() && f.abs() < 1e16 {
        format!("{f:.1}")
    } else {
        f.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i(v: i128) -> Arg {
        Arg::Int { v, size: 4 }
    }

    #[test]
    fn ints() {
        assert_eq!(format("|%5d|%-5d|%05d|%+d|", &[i(42), i(42), i(42), i(42)]).unwrap(), "|   42|42   |00042|+42|");
        assert_eq!(format("%x %X %#x %08x", &[i(-1), i(255), i(255), i(0xbeef)]).unwrap(), "ffffffff FF 0xff 0000beef");
        assert_eq!(format("%.3d %lu %lld %zu", &[i(5), i(7), i(-9), i(3)]).unwrap(), "005 7 -9 3");
        assert_eq!(format("%o %#o", &[i(8), i(8)]).unwrap(), "10 010");
        assert_eq!(format("100%% %c", &[i(65)]).unwrap(), "100% A");
    }

    #[test]
    fn floats() {
        let f = Arg::Float;
        assert_eq!(
            format("%f %.3f %e %E", &[f(1.5), f(4.56789), f(12345.678), f(0.000123)]).unwrap(),
            "1.500000 4.568 1.234568e+04 1.230000E-04"
        );
        assert_eq!(
            format("%g %g %g %g", &[f(100000.0), f(1e6), f(0.0001), f(1e-5)]).unwrap(),
            "100000 1e+06 0.0001 1e-05"
        );
        assert_eq!(format("%.2g %g %G", &[f(123.0), f(0.5), f(f64::INFINITY)]).unwrap(), "1.2e+02 0.5 INF");
        assert_eq!(format("%8.2f|%-8.2f|%08.2f", &[f(-1.5), f(1.5), f(-1.5)]).unwrap(), "   -1.50|1.50    |-0001.50");
        assert_eq!(format("%a", &[f(1.0)]).unwrap(), "0x1p+0");
    }

    #[test]
    fn strings_and_errors() {
        assert_eq!(
            format(
                "%s|%5s|%-5s|%.2s",
                &[Arg::Str("ab".into()), Arg::Str("ab".into()), Arg::Str("ab".into()), Arg::Str("abc".into())]
            )
            .unwrap(),
            "ab|   ab|ab   |ab"
        );
        assert_eq!(format("%d %d", &[i(1)]), Err(FormatError::TooFewArgs));
        assert_eq!(format("%d", &[i(1), i(2)]), Err(FormatError::TooManyArgs));
        assert_eq!(format("at 50%", &[]).unwrap(), "at 50%");
        assert_eq!(format("% o", &[i(8)]).unwrap(), "10");
    }
}
