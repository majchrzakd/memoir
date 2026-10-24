//! Fractional indexing: order keys are strings that sort lexicographically
//! and always leave room for another key between any two neighbours. Moving
//! or reordering an item therefore changes only that item — one small sync
//! change, never a renumbering of its siblings.
//!
//! Algorithm from David Greenspan, "Implementing Fractional Indexing" (2020).
//! A key is an *integer part* — a head letter giving its length (`a`–`z`
//! for 2–27 characters, `A`–`Z` for 27–2 characters, negative) followed by
//! digits — plus an optional *fraction* that never ends in '0'. Appending or
//! prepending increments/decrements the integer, so keys stay short (the
//! 3 844th append is still only 3 characters); inserting between neighbours
//! extends the fraction.

/// '0'-'9', 'A'-'Z', 'a'-'z' — ascending in ASCII, so byte order = key order.
const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = DIGITS.len();
/// The smallest integer, "A" followed by 26 zeros; it has no predecessor.
const SMALLEST_INTEGER: &str = "A00000000000000000000000000";

/// A key strictly between `before` and `after` (`None` = open end).
///
/// # Panics
/// If a key is invalid or `before >= after`; keys always come from this module.
pub fn between(before: Option<&str>, after: Option<&str>) -> String {
    for key in [before, after].into_iter().flatten() {
        assert!(is_valid(key), "invalid sort key {key:?}");
    }

    match (before, after) {
        (None, None) => "a0".into(),
        (None, Some(b)) => {
            let (ib, fb) = split(b);
            if ib == SMALLEST_INTEGER {
                return format!("{ib}{}", midpoint(b"", Some(fb.as_bytes())));
            }
            if ib.len() < b.len() {
                // `b` has a fraction, so its bare integer part sorts before it.
                return ib.into();
            }
            decrement_integer(ib).expect("integer below the smallest one")
        }
        (Some(a), None) => {
            let (ia, fa) = split(a);
            match increment_integer(ia) {
                Some(i) => i,
                None => format!("{ia}{}", midpoint(fa.as_bytes(), None)),
            }
        }
        (Some(a), Some(b)) => {
            assert!(a < b, "sort keys out of order: {a:?} >= {b:?}");
            let (ia, fa) = split(a);
            let (ib, fb) = split(b);
            if ia == ib {
                return format!("{ia}{}", midpoint(fa.as_bytes(), Some(fb.as_bytes())));
            }
            let next = increment_integer(ia).expect("integer above the largest one");
            if next.as_str() < b {
                next
            } else {
                format!("{ia}{}", midpoint(fa.as_bytes(), None))
            }
        }
    }
}

pub fn is_valid(key: &str) -> bool {
    let Some(len) = key.bytes().next().and_then(integer_len) else {
        return false;
    };
    key.len() >= len
        && key.bytes().skip(1).all(|c| DIGITS.contains(&c))
        && !key[len..].ends_with('0')
        && &key[..len] != SMALLEST_INTEGER
}

/// Length of the integer part announced by its head letter.
fn integer_len(head: u8) -> Option<usize> {
    match head {
        b'a'..=b'z' => Some((head - b'a') as usize + 2),
        b'A'..=b'Z' => Some((b'Z' - head) as usize + 2),
        _ => None,
    }
}

fn split(key: &str) -> (&str, &str) {
    let len = integer_len(key.as_bytes()[0]).expect("validated key");
    key.split_at(len)
}

fn increment_integer(int: &str) -> Option<String> {
    let (head, digits) = (int.as_bytes()[0], &int.as_bytes()[1..]);
    let mut digits = digits.to_vec();
    for d in digits.iter_mut().rev() {
        let v = value(*d) + 1;
        if v < BASE {
            *d = DIGITS[v];
            return Some(ascii(head, &digits));
        }
        *d = b'0';
    }
    // Every digit carried: the integer gets one digit longer.
    match head {
        b'Z' => Some("a0".into()),
        b'z' => None,
        _ => {
            let head = head + 1;
            if head > b'a' {
                digits.push(b'0');
            } else {
                digits.pop();
            }
            Some(ascii(head, &digits))
        }
    }
}

fn decrement_integer(int: &str) -> Option<String> {
    let (head, digits) = (int.as_bytes()[0], &int.as_bytes()[1..]);
    let mut digits = digits.to_vec();
    for d in digits.iter_mut().rev() {
        let v = value(*d);
        if v > 0 {
            *d = DIGITS[v - 1];
            return Some(ascii(head, &digits));
        }
        *d = DIGITS[BASE - 1];
    }
    match head {
        b'a' => Some(format!("Z{}", DIGITS[BASE - 1] as char)),
        b'A' => None,
        _ => {
            let head = head - 1;
            if head < b'Z' {
                digits.push(DIGITS[BASE - 1]);
            } else {
                digits.pop();
            }
            Some(ascii(head, &digits))
        }
    }
}

/// A fraction strictly between fractions `a` and `b` (`None` = 1.0).
fn midpoint(a: &[u8], b: Option<&[u8]>) -> String {
    if let Some(b) = b {
        // Strip the common prefix, reading missing digits of `a` as '0'.
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(b'0') == b[n] {
            n += 1;
        }
        if n > 0 {
            let rest = midpoint(a.get(n..).unwrap_or(&[]), Some(&b[n..]));
            return format!("{}{rest}", std::str::from_utf8(&b[..n]).expect("ASCII"));
        }
    }

    let da = a.first().map_or(0, |&c| value(c));
    let db = b.map_or(BASE, |b| value(b[0]));
    if db - da > 1 {
        return (DIGITS[(da + db).div_ceil(2)] as char).to_string();
    }
    match b {
        Some(b) if b.len() > 1 => (b[0] as char).to_string(),
        _ => format!(
            "{}{}",
            DIGITS[da] as char,
            midpoint(a.get(1..).unwrap_or(&[]), None)
        ),
    }
}

fn value(c: u8) -> usize {
    DIGITS
        .iter()
        .position(|&d| d == c)
        .expect("validated key digit")
}

fn ascii(head: u8, digits: &[u8]) -> String {
    let mut s = String::with_capacity(1 + digits.len());
    s.push(head as char);
    s.extend(digits.iter().map(|&d| d as char));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn known_keys() {
        assert_eq!(between(None, None), "a0");
        assert_eq!(between(Some("a0"), None), "a1");
        assert_eq!(between(Some("az"), None), "b00");
        assert_eq!(between(None, Some("a0")), "Zz");
        assert_eq!(between(None, Some("b00")), "az");
        assert_eq!(between(Some("a0"), Some("a1")), "a0V");
        assert_eq!(between(Some("a0V"), Some("a1")), "a0l");
        assert_eq!(between(Some("Zz"), Some("a0")), "ZzV");
        assert_eq!(between(Some("a0"), Some("a2")), "a1");
        assert_eq!(between(None, Some("a0V")), "a0");
    }

    #[test]
    fn appends_and_prepends_stay_short() {
        let mut last = between(None, None);
        for _ in 0..5000 {
            last = between(Some(&last), None);
        }
        assert!(last.len() <= 4, "append key grew to {last}");

        let mut first = between(None, None);
        for _ in 0..5000 {
            first = between(None, Some(&first));
        }
        assert!(first.len() <= 4, "prepend key grew to {first}");
    }

    #[test]
    fn validity() {
        for ok in ["a0", "a1V", "Zz", "b00", "zzzzzzzzzzzzzzzzzzzzzzzzzzz"] {
            assert!(is_valid(ok), "{ok}");
        }
        for bad in ["", "a", "a10", "a0-", "_0", "b0", SMALLEST_INTEGER] {
            assert!(!is_valid(bad), "{bad}");
        }
    }

    proptest! {
        /// Inserting at random positions keeps the list strictly ordered.
        #[test]
        fn random_insertions_stay_ordered(
            positions in prop::collection::vec(any::<prop::sample::Index>(), 1..400)
        ) {
            let mut keys: Vec<String> = Vec::new();
            for pos in positions {
                let i = pos.index(keys.len() + 1);
                let before = i.checked_sub(1).map(|j| keys[j].as_str());
                let after = keys.get(i).map(String::as_str);
                let key = between(before, after);
                prop_assert!(is_valid(&key), "invalid {}", key);
                keys.insert(i, key);
            }
            prop_assert!(keys.windows(2).all(|w| w[0] < w[1]));
        }
    }
}
