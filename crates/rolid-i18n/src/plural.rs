//! The `plural=` expression of a gettext `Plural-Forms` header: a C
//! expression in `n` (`?:`, `||`, `&&`, `==`, `!=`, `<`, `<=`, `>`, `>=`,
//! `+`, `-`, `*`, `/`, `%`, `!`, parentheses, whole numbers).

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    N,
    Num(u64),
    Not(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

/// `nplurals` and the expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    pub nplurals: usize,
    pub expr: Expr,
}

impl Rules {
    /// Parse `nplurals=3; plural=(n==1) ? 0 : …;`.
    pub fn parse(header: &str) -> Result<Self, String> {
        let mut nplurals = None;
        let mut expr = None;
        for part in header.split(';') {
            let Some((k, v)) = part.split_once('=') else { continue };
            match k.trim() {
                "nplurals" => nplurals = Some(v.trim().parse::<usize>().map_err(|e| format!("nplurals: {e}"))?),
                "plural" => expr = Some(parse(v)?),
                _ => {}
            }
        }
        let nplurals = nplurals.ok_or("nplurals missing")?;
        if nplurals == 0 || nplurals > 6 {
            return Err(format!("nplurals={nplurals} out of range"));
        }
        Ok(Self { nplurals, expr: expr.ok_or("plural missing")? })
    }

    /// The form index for `n` (never outside 0..nplurals).
    pub fn index(&self, n: u64) -> usize {
        (self.expr.eval(n) as usize).min(self.nplurals - 1)
    }
}

impl Expr {
    pub fn eval(&self, n: u64) -> u64 {
        match self {
            Expr::N => n,
            Expr::Num(v) => *v,
            Expr::Not(e) => (e.eval(n) == 0) as u64,
            Expr::If(c, a, b) => {
                if c.eval(n) != 0 {
                    a.eval(n)
                } else {
                    b.eval(n)
                }
            }
            Expr::Bin(op, a, b) => {
                let (x, y) = (a.eval(n), b.eval(n));
                match op {
                    Op::Or => (x != 0 || y != 0) as u64,
                    Op::And => (x != 0 && y != 0) as u64,
                    Op::Eq => (x == y) as u64,
                    Op::Ne => (x != y) as u64,
                    Op::Lt => (x < y) as u64,
                    Op::Le => (x <= y) as u64,
                    Op::Gt => (x > y) as u64,
                    Op::Ge => (x >= y) as u64,
                    Op::Add => x.wrapping_add(y),
                    Op::Sub => x.wrapping_sub(y),
                    Op::Mul => x.wrapping_mul(y),
                    Op::Div => x.checked_div(y).unwrap_or(0),
                    Op::Rem => x.checked_rem(y).unwrap_or(0),
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    N,
    Num(u64),
    Op(&'static str),
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    const OPS: [&str; 18] = ["||", "&&", "==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "%", "!", "?", ":", "(", ")"];
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'n' {
            out.push(Tok::N);
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Tok::Num(s[start..i].parse().map_err(|e| format!("number: {e}"))?));
        } else if let Some(op) = OPS.iter().find(|op| s[i..].starts_with(**op)) {
            out.push(Tok::Op(op));
            i += op.len();
        } else {
            return Err(format!("unexpected '{}' in plural expression", s[i..].chars().next().unwrap()));
        }
    }
    Ok(out)
}

/// Parse a plural expression.
pub fn parse(s: &str) -> Result<Expr, String> {
    let toks = lex(s)?;
    let mut p = Parser { toks, pos: 0, depth: 0 };
    let e = p.ternary()?;
    if p.pos != p.toks.len() {
        return Err(format!("trailing input in plural expression: {:?}", &p.toks[p.pos..]));
    }
    Ok(e)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

const LEVELS: [&[(&str, Op)]; 6] = [
    &[("||", Op::Or)],
    &[("&&", Op::And)],
    &[("==", Op::Eq), ("!=", Op::Ne)],
    &[("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt)],
    &[("+", Op::Add), ("-", Op::Sub)],
    &[("*", Op::Mul), ("/", Op::Div), ("%", Op::Rem)],
];

impl Parser {
    fn peek_op(&self) -> Option<&'static str> {
        match self.toks.get(self.pos) {
            Some(Tok::Op(o)) => Some(o),
            _ => None,
        }
    }

    fn expect(&mut self, op: &str) -> Result<(), String> {
        if self.peek_op() == Some(op) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected '{op}' in plural expression"))
        }
    }

    fn ternary(&mut self) -> Result<Expr, String> {
        self.depth += 1;
        if self.depth > 64 {
            return Err("plural expression too deep".into());
        }
        let cond = self.binary(0)?;
        let e = if self.peek_op() == Some("?") {
            self.pos += 1;
            let a = self.ternary()?;
            self.expect(":")?;
            let b = self.ternary()?;
            Expr::If(Box::new(cond), Box::new(a), Box::new(b))
        } else {
            cond
        };
        self.depth -= 1;
        Ok(e)
    }

    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(op) = self.peek_op().and_then(|o| LEVELS[level].iter().find(|(s, _)| *s == o)).map(|(_, op)| *op) {
            self.pos += 1;
            let right = self.binary(level + 1)?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        match self.toks.get(self.pos).cloned() {
            Some(Tok::Op("!")) => {
                self.pos += 1;
                Ok(Expr::Not(Box::new(self.unary()?)))
            }
            Some(Tok::Op("(")) => {
                self.pos += 1;
                let e = self.ternary()?;
                self.expect(")")?;
                Ok(e)
            }
            Some(Tok::N) => {
                self.pos += 1;
                Ok(Expr::N)
            }
            Some(Tok::Num(v)) => {
                self.pos += 1;
                Ok(Expr::Num(v))
            }
            other => Err(format!("unexpected {other:?} in plural expression")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CS: &str = "nplurals=3; plural=(n==1) ? 0 : (n>=2 && n<=4) ? 1 : 2;";
    const PL: &str = "nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);";

    #[test]
    fn czech_polish_english_french() {
        let idx = |h: &str, ns: &[u64]| {
            let r = Rules::parse(h).unwrap();
            ns.iter().map(|n| r.index(*n)).collect::<Vec<_>>()
        };
        let ns = [0, 1, 2, 5, 21, 22, 104, 112, 1_000_002];
        assert_eq!(idx(CS, &ns), [2, 0, 1, 2, 2, 2, 2, 2, 2]);
        assert_eq!(idx(PL, &ns), [2, 0, 1, 2, 2, 1, 1, 2, 1]);
        assert_eq!(idx("nplurals=2; plural=(n != 1);", &ns), [1, 0, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(idx("nplurals=2; plural=(n > 1);", &ns), [0, 0, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(idx("nplurals=1; plural=0;", &ns), [0; 9]);
    }

    #[test]
    fn bad_expressions_are_errors_and_indices_stay_in_range() {
        for bad in ["nplurals=2; plural=n ==;", "nplurals=2; plural=(n;", "nplurals=x; plural=n;", "plural=n;", "nplurals=2; plural=n $ 1;"] {
            assert!(Rules::parse(bad).is_err(), "{bad}");
        }
        let r = Rules::parse("nplurals=2; plural=n;").unwrap();
        assert_eq!(r.index(7), 1);
        assert_eq!(parse("n / 0").unwrap().eval(5), 0);
        assert!(parse(&"(".repeat(200)).is_err());
    }
}
