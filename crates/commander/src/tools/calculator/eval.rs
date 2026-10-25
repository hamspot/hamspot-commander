//! Small recursive-descent expression evaluator for the calculator example.
//! Grammar: expr = term (('+'|'-') term)* ; term = unary (('*'|'/'|'%') unary)* ;
//! unary = ('-'|'+') unary | power ; power = postfix ('^' unary)? ; postfix = atom '!'* ;
//! atom = number | ident | ident '(' expr ')' | '(' expr ')'. Implicit multiplication like
//! `2x` or `2(3)` or `2pi` is supported.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Angle {
    Deg,
    Rad,
}

pub struct Env {
    pub angle: Angle,
    pub x: f64,
    pub ans: f64,
}

impl Default for Env {
    fn default() -> Self {
        Self {
            angle: Angle::Rad,
            x: 0.0,
            ans: 0.0,
        }
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
    env: &'a Env,
}

pub fn eval(src: &str, env: &Env) -> Result<f64, String> {
    let mut p = P {
        s: src.as_bytes(),
        i: 0,
        env,
    };
    let v = p.expr()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("unexpected '{}' at {}", p.s[p.i] as char, p.i + 1));
    }
    if v.is_nan() {
        Err("not a number".into())
    } else {
        Ok(v)
    }
}

/// True if the expression uses the variable `x` (so it can be graphed).
pub fn uses_x(src: &str) -> bool {
    let b = src.as_bytes();
    (0..b.len()).any(|i| {
        b[i] == b'x'
            && (i == 0 || !b[i - 1].is_ascii_alphabetic())
            && (i + 1 == b.len() || !b[i + 1].is_ascii_alphabetic())
    })
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') {
                v += self.term()?;
            } else if self.eat(b'-') {
                v -= self.term()?;
            } else {
                return Ok(v);
            }
        }
    }
    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            if self.eat(b'*') {
                v *= self.unary()?;
            } else if self.eat(b'/') {
                v /= self.unary()?;
            } else if self.eat(b'%') {
                v %= self.unary()?;
            } else if matches!(self.peek(), Some(c) if c == b'(' || c.is_ascii_alphanumeric() || c == b'.')
            {
                v *= self.power()?; // implicit multiplication
            } else {
                return Ok(v);
            }
        }
    }
    fn unary(&mut self) -> Result<f64, String> {
        if self.eat(b'-') {
            return Ok(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        self.power()
    }
    fn power(&mut self) -> Result<f64, String> {
        let base = self.postfix()?;
        if self.eat(b'^') {
            Ok(base.powf(self.unary()?))
        } else {
            Ok(base)
        }
    }
    fn postfix(&mut self) -> Result<f64, String> {
        let mut v = self.atom()?;
        while self.eat(b'!') {
            if v < 0.0 || v.fract() != 0.0 || v > 170.0 {
                return Err("factorial needs an integer 0..170".into());
            }
            v = (1..=v as u64).map(|k| k as f64).product();
        }
        Ok(v)
    }
    fn atom(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if !self.eat(b')') {
                    return Err("missing ')'".into());
                }
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let st = self.i;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.')
                {
                    self.i += 1;
                }
                if self.i < self.s.len() && (self.s[self.i] == b'e' || self.s[self.i] == b'E') {
                    let save = self.i;
                    self.i += 1;
                    if self.i < self.s.len() && (self.s[self.i] == b'-' || self.s[self.i] == b'+') {
                        self.i += 1;
                    }
                    if self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                        while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                            self.i += 1;
                        }
                    } else {
                        self.i = save;
                    }
                }
                let t = std::str::from_utf8(&self.s[st..self.i]).unwrap();
                t.parse::<f64>().map_err(|_| format!("bad number '{t}'"))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let st = self.i;
                while self.i < self.s.len() && self.s[self.i].is_ascii_alphanumeric() {
                    self.i += 1;
                }
                let name = std::str::from_utf8(&self.s[st..self.i])
                    .unwrap()
                    .to_ascii_lowercase();
                match name.as_str() {
                    "pi" => return Ok(std::f64::consts::PI),
                    "e" => return Ok(std::f64::consts::E),
                    "x" => return Ok(self.env.x),
                    "ans" => return Ok(self.env.ans),
                    _ => {}
                }
                if !self.eat(b'(') {
                    return Err(format!("unknown name '{name}'"));
                }
                let a = self.expr()?;
                if !self.eat(b')') {
                    return Err("missing ')'".into());
                }
                self.func(&name, a)
            }
            Some(c) => Err(format!("unexpected '{}'", c as char)),
            None => Err("unexpected end".into()),
        }
    }
    fn func(&self, name: &str, a: f64) -> Result<f64, String> {
        let deg = self.env.angle == Angle::Deg;
        let to_rad = |v: f64| if deg { v.to_radians() } else { v };
        let from_rad = |v: f64| if deg { v.to_degrees() } else { v };
        Ok(match name {
            "sin" => to_rad(a).sin(),
            "cos" => to_rad(a).cos(),
            "tan" => to_rad(a).tan(),
            "asin" => from_rad(a.asin()),
            "acos" => from_rad(a.acos()),
            "atan" => from_rad(a.atan()),
            "sinh" => a.sinh(),
            "cosh" => a.cosh(),
            "tanh" => a.tanh(),
            "asinh" => a.asinh(),
            "acosh" => a.acosh(),
            "atanh" => a.atanh(),
            "ln" => a.ln(),
            "log" => a.log10(),
            "log2" => a.log2(),
            "exp" => a.exp(),
            "sqrt" => a.sqrt(),
            "cbrt" => a.cbrt(),
            "abs" => a.abs(),
            _ => return Err(format!("unknown function '{name}'")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ev(s: &str) -> f64 {
        eval(s, &Env::default()).unwrap()
    }
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }
    #[test]
    fn arithmetic_and_precedence() {
        assert!(close(ev("1+2*3"), 7.0));
        assert!(close(ev("(1+2)*3"), 9.0));
        assert!(close(ev("2^3^2"), 512.0)); // right-assoc
        assert!(close(ev("-2^2"), -4.0));
        assert!(close(ev("10%4"), 2.0));
        assert!(close(ev("1.5e3"), 1500.0));
        assert!(close(ev("5!"), 120.0));
        assert!(close(ev("2pi"), 2.0 * std::f64::consts::PI));
    }
    #[test]
    fn functions_and_angles() {
        assert!(close(ev("sin(pi/2)"), 1.0));
        let deg = Env {
            angle: Angle::Deg,
            ..Env::default()
        };
        assert!(close(eval("sin(90)", &deg).unwrap(), 1.0));
        assert!(close(eval("asin(1)", &deg).unwrap(), 90.0));
        assert!(close(ev("sinh(0)"), 0.0));
        assert!(close(ev("log(1000)"), 3.0));
        assert!(close(ev("sqrt(16)+abs(-2)"), 6.0));
    }
    #[test]
    fn variables_and_errors() {
        let env = Env {
            x: 3.0,
            ans: 10.0,
            ..Env::default()
        };
        assert!(close(eval("2x+ans", &env).unwrap(), 16.0));
        assert!(uses_x("sin(x)*2") && !uses_x("exp(1)"));
        assert!(eval("1+", &Env::default()).is_err());
        assert!(eval("foo(1)", &Env::default()).is_err());
        assert!(eval("(1", &Env::default()).is_err());
        assert!(eval("sqrt(-1)", &Env::default()).is_err());
    }
}
