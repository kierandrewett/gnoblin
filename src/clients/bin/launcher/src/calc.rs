//! Tiny arithmetic evaluator for the launcher's inline calculator result.
//! Recursive-descent: `+ - * / %`, `^` (right-assoc), parentheses, unary signs,
//! decimals. No external deps. Returns the formatted answer, or `None` when the
//! query isn't a (complete, well-formed) arithmetic expression — so plain app
//! searches like "firefox" never trigger it.

pub fn try_eval(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    // Cheap gate: must look like maths (a digit AND an operator) and contain
    // nothing but maths characters.
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    let has_op = s.chars().any(|c| "+-*/%^".contains(c));
    if !has_digit || !has_op {
        return None;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_digit() || "+-*/%^(). \t".contains(c))
    {
        return None;
    }
    let mut p = Parser {
        bytes: s.as_bytes(),
        pos: 0,
    };
    let v = p.expr()?;
    if p.peek().is_some() {
        return None; // trailing garbage → not a clean expression
    }
    if !v.is_finite() {
        return None;
    }
    Some(format_num(v))
}

fn format_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.6}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ') | Some(b'\t')) {
            self.pos += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.bytes.get(self.pos).copied()
    }
    /// `+` `-`
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            match self.peek() {
                Some(b'+') => {
                    self.pos += 1;
                    v += self.term()?;
                }
                Some(b'-') => {
                    self.pos += 1;
                    v -= self.term()?;
                }
                _ => break,
            }
        }
        Some(v)
    }
    /// `*` `/` `%`
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        loop {
            match self.peek() {
                Some(b'*') => {
                    self.pos += 1;
                    v *= self.power()?;
                }
                Some(b'/') => {
                    self.pos += 1;
                    let d = self.power()?;
                    if d == 0.0 {
                        return None;
                    }
                    v /= d;
                }
                Some(b'%') => {
                    self.pos += 1;
                    let d = self.power()?;
                    if d == 0.0 {
                        return None;
                    }
                    v %= d;
                }
                _ => break,
            }
        }
        Some(v)
    }
    /// `^` (right-associative)
    fn power(&mut self) -> Option<f64> {
        let b = self.unary()?;
        if self.peek() == Some(b'^') {
            self.pos += 1;
            let e = self.power()?;
            Some(b.powf(e))
        } else {
            Some(b)
        }
    }
    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(b'-') => {
                self.pos += 1;
                Some(-self.unary()?)
            }
            Some(b'+') => {
                self.pos += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }
    fn atom(&mut self) -> Option<f64> {
        match self.peek() {
            Some(b'(') => {
                self.pos += 1;
                let v = self.expr()?;
                if self.peek() == Some(b')') {
                    self.pos += 1;
                    Some(v)
                } else {
                    None
                }
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => self.number(),
            _ => None,
        }
    }
    fn number(&mut self) -> Option<f64> {
        self.skip_ws();
        let start = self.pos;
        while matches!(self.bytes.get(self.pos), Some(c) if c.is_ascii_digit() || *c == b'.') {
            self.pos += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .ok()?
            .parse()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evaluates() {
        assert_eq!(try_eval("2+2").as_deref(), Some("4"));
        assert_eq!(try_eval("2 * (3 + 4)").as_deref(), Some("14"));
        assert_eq!(try_eval("10 / 4").as_deref(), Some("2.5"));
        assert_eq!(try_eval("2^10").as_deref(), Some("1024"));
        assert_eq!(try_eval("-5 + 3").as_deref(), Some("-2"));
        assert_eq!(try_eval("17 % 5").as_deref(), Some("2"));
    }
    #[test]
    fn rejects_non_maths() {
        assert_eq!(try_eval("firefox"), None);
        assert_eq!(try_eval(""), None);
        assert_eq!(try_eval("2 +"), None); // incomplete
        assert_eq!(try_eval("2 + abc"), None);
        assert_eq!(try_eval("5"), None); // no operator → not a calc
        assert_eq!(try_eval("1/0"), None); // div by zero
    }
}
