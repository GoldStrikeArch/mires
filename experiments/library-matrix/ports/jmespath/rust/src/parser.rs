//! Pratt parser. Projections consume only expressions above their binding power.
use crate::host::*;
use alloc::vec::Vec;
use mires_js::try_host;

#[derive(Clone, Copy)]
struct Token {
    kind: &'static str,
    value: V,
}

fn letter(c: u32) -> bool {
    (65..=90).contains(&c) || (97..=122).contains(&c) || c == 95
}
fn digit(c: u32) -> bool {
    (48..=57).contains(&c)
}
fn json(v: V) -> Answer {
    try_host(|| parse_json(v)).map_err(|_| "syntax")
}

fn lex(source: V) -> Result<Vec<Token>, &'static str> {
    let length = len(source);
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < length {
        let start = i;
        let c = unit(source, i);
        i += 1;
        let next = if i < length { unit(source, i) } else { 0 };
        let mut value = und();
        let kind = if letter(c) {
            while i < length && (letter(unit(source, i)) || digit(unit(source, i))) {
                i += 1;
            }
            value = slice(source, start, i);
            "Field"
        } else if c == 45 || digit(c) {
            while i < length && (unit(source, i) == 45 || digit(unit(source, i))) {
                i += 1;
            }
            value = num(parse_int(slice(source, start, i)));
            "Number"
        } else if matches!(c, 34 | 39 | 96) {
            while i < length {
                let ch = unit(source, i);
                if ch == c {
                    break;
                }
                if ch == 92
                    && i + 1 < length
                    && (unit(source, i + 1) == c || unit(source, i + 1) == 92)
                {
                    i += 1;
                }
                i += 1;
            }
            let inner = slice(source, start + 1, i);
            i += 1;
            if c == 34 {
                value = json(slice(source, start, i))?;
                "Quoted"
            } else if c == 39 {
                value = replace(inner, s("\\'"), s("'"));
                "Literal"
            } else {
                let raw = replace(trim_start(inner), s("\\`"), s("`"));
                let first = if len(raw) == 0 { 0 } else { unit(raw, 0) };
                let literal = matches!(first, 91 | 123 | 34)
                    || ["true", "false", "null"]
                        .iter()
                        .any(|k| raw.strictly_equals(s(k)));
                value = if literal {
                    json(raw)?
                } else if (digit(first) || first == 45) && json(raw).is_ok() {
                    json(raw)?
                } else {
                    json(s("\"") + raw + s("\""))?
                };
                "Literal"
            }
        } else {
            match (c, next) {
                (32 | 9 | 10, _) => continue,
                (91, 63) => {
                    i += 1;
                    "Filter"
                }
                (91, 93) => {
                    i += 1;
                    "Flatten"
                }
                (38, 38) => {
                    i += 1;
                    "And"
                }
                (124, 124) => {
                    i += 1;
                    "Or"
                }
                (33, 61) => {
                    i += 1;
                    "NE"
                }
                (61, 61) => {
                    i += 1;
                    "EQ"
                }
                (60, 61) => {
                    i += 1;
                    "LTE"
                }
                (62, 61) => {
                    i += 1;
                    "GTE"
                }
                (60, _) => "LT",
                (62, _) => "GT",
                (33, _) => "Not",
                (38, _) => "Expref",
                (124, _) => "Pipe",
                (91, _) => "Lbracket",
                (93, _) => "Rbracket",
                (123, _) => "Lbrace",
                (125, _) => "Rbrace",
                (40, _) => "Lparen",
                (41, _) => "Rparen",
                (42, _) => "Star",
                (64, _) => "Current",
                (44, _) => "Comma",
                (46, _) => "Dot",
                (58, _) => "Colon",
                _ => return Err("syntax"),
            }
        };
        tokens.push(Token { kind, value });
    }
    tokens.push(Token {
        kind: "EOF",
        value: und(),
    });
    Ok(tokens)
}

fn power(k: &str) -> i32 {
    match k {
        "Pipe" => 1,
        "Or" => 2,
        "And" => 3,
        "EQ" | "NE" | "GT" | "LT" | "GTE" | "LTE" => 5,
        "Flatten" => 9,
        "Star" => 20,
        "Filter" => 21,
        "Dot" => 40,
        "Not" => 45,
        "Lbrace" => 50,
        "Lbracket" => 55,
        "Lparen" => 60,
        _ => 0,
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}
impl Parser {
    fn look(&self, offset: usize) -> &'static str {
        self.tokens
            .get(self.pos + offset)
            .map(|token| token.kind)
            .unwrap_or("EOF")
    }
    fn next(&mut self) -> Result<Token, &'static str> {
        let token = *self.tokens.get(self.pos).ok_or("syntax")?;
        self.pos += 1;
        Ok(token)
    }
    fn take(&mut self, kind: &str) -> Result<(), &'static str> {
        if self.next()?.kind == kind {
            Ok(())
        } else {
            Err("syntax")
        }
    }
    fn expr(&mut self, rbp: i32) -> Answer {
        let token = self.next()?;
        let mut left = self.nud(token)?;
        while rbp < power(self.look(0)) {
            let op = self.next()?;
            left = self.led(op.kind, left)?;
        }
        Ok(left)
    }
    fn projection(&mut self, rbp: i32) -> Answer {
        let k = self.look(0);
        if power(k) < 10 {
            Ok(node("Identity"))
        } else {
            match k {
                "Lbracket" | "Filter" => self.expr(rbp),
                "Dot" => {
                    self.next()?;
                    self.dot(rbp)
                }
                _ => Err("syntax"),
            }
        }
    }
    fn dot(&mut self, rbp: i32) -> Answer {
        match self.look(0) {
            "Field" | "Quoted" | "Star" => self.expr(rbp),
            "Lbracket" => {
                self.next()?;
                self.multilist()
            }
            "Lbrace" => {
                self.next()?;
                self.hash()
            }
            _ => Ok(und()),
        }
    }
    fn multilist(&mut self) -> Answer {
        let mut children_ = Vec::new();
        while self.look(0) != "Rbracket" {
            children_.push(self.expr(0)?);
            if self.look(0) == "Comma" {
                self.next()?;
                if self.look(0) == "Rbracket" {
                    return Err("syntax");
                }
            }
        }
        self.take("Rbracket")?;
        Ok(children("MultiSelectList", &children_))
    }
    fn hash(&mut self) -> Answer {
        let mut pairs = Vec::new();
        loop {
            let t = self.next()?;
            if !matches!(t.kind, "Field" | "Quoted") {
                return Err("syntax");
            }
            self.take("Colon")?;
            let pair = named("KeyValuePair", t.value);
            set(pair, "value", self.expr(0)?);
            pairs.push(pair);
            if self.look(0) == "Comma" {
                self.next()?;
            } else if self.look(0) == "Rbrace" {
                self.next()?;
                break;
            }
        }
        Ok(children("MultiSelectHash", &pairs))
    }
    fn bracket(&mut self, left: V) -> Answer {
        if self.look(0) == "Colon" || self.look(1) == "Colon" {
            let mut parts = [nil(); 3];
            let mut i = 0;
            while self.look(0) != "Rbracket" && i < 3 {
                let t = self.next()?;
                match t.kind {
                    "Colon" => i += 1,
                    "Number" => parts[i] = t.value,
                    _ => return Err("syntax"),
                }
            }
            self.take("Rbracket")?;
            let ix = children("IndexExpression", &[left, children("Slice", &parts)]);
            Ok(children("Projection", &[ix, self.projection(20)?]))
        } else {
            let t = self.next()?;
            self.take("Rbracket")?;
            Ok(children("IndexExpression", &[left, leaf("Index", t.value)]))
        }
    }
    fn args(&mut self, commas: bool) -> Result<Vec<V>, &'static str> {
        let mut args = Vec::new();
        while self.look(0) != "Rparen" {
            let v = if self.look(0) == "Current" {
                self.next()?;
                node("Current")
            } else {
                self.expr(0)?
            };
            args.push(v);
            if commas && self.look(0) == "Comma" {
                self.next()?;
            }
        }
        self.take("Rparen")?;
        Ok(args)
    }
    fn nud(&mut self, t: Token) -> Answer {
        Ok(match t.kind {
            "Literal" => leaf("Literal", t.value),
            "Quoted" if self.look(0) == "Lparen" => return Err("syntax"),
            "Field" | "Quoted" => named("Field", t.value),
            "Not" => children("NotExpression", &[self.expr(45)?]),
            "Current" => node("Current"),
            "Expref" => children("ExpressionReference", &[self.expr(0)?]),
            "Star" => children("ValueProjection", &[node("Identity"), self.projection(20)?]),
            "Filter" | "Flatten" => return self.led(t.kind, node("Identity")),
            "Lbrace" => return self.hash(),
            "Lbracket" => {
                if matches!(self.look(0), "Colon" | "Number") {
                    return self.bracket(node("Identity"));
                }
                if self.look(0) == "Star" && self.look(1) == "Rbracket" {
                    self.pos += 2;
                    children("Projection", &[node("Identity"), self.projection(20)?])
                } else {
                    return self.multilist();
                }
            }
            "Lparen" => self.args(false)?.first().copied().unwrap_or(und()),
            _ => return Err("syntax"),
        })
    }
    fn led(&mut self, k: &'static str, left: V) -> Answer {
        Ok(match k {
            "Dot" if self.look(0) == "Star" => {
                self.next()?;
                children("ValueProjection", &[left, self.projection(40)?])
            }
            "Dot" => children("Subexpression", &[left, self.dot(40)?]),
            "Or" | "And" | "Pipe" => children(
                match k {
                    "Or" => "OrExpression",
                    "And" => "AndExpression",
                    _ => "Pipe",
                },
                &[left, self.expr(power(k))?],
            ),
            "Lparen" => {
                if left.is_undefined() {
                    return Err("syntax");
                }
                let n = named("Function", field(left, "name"));
                set(n, "children", list(&self.args(true)?));
                n
            }
            "Filter" => {
                let condition = self.expr(0)?;
                self.take("Rbracket")?;
                let right = if self.look(0) == "Flatten" {
                    node("Identity")
                } else {
                    self.projection(21)?
                };
                children("FilterProjection", &[left, right, condition])
            }
            "Flatten" => children(
                "Projection",
                &[children("Flatten", &[left]), self.projection(9)?],
            ),
            "EQ" | "NE" | "LT" | "GT" | "LTE" | "GTE" => {
                let n = named("Comparator", s(k));
                let right = self.expr(5)?;
                set(n, "children", list(&[left, right]));
                n
            }
            "Lbracket" => {
                if matches!(self.look(0), "Colon" | "Number") {
                    return self.bracket(left);
                }
                self.take("Star")?;
                self.take("Rbracket")?;
                children("Projection", &[left, self.projection(20)?])
            }
            _ => return Err("syntax"),
        })
    }
}

pub fn compile(source: V) -> Answer {
    let mut parser = Parser {
        tokens: lex(source)?,
        pos: 0,
    };
    let n = parser.expr(0)?;
    if parser.look(0) == "EOF" {
        Ok(n)
    } else {
        Err("syntax")
    }
}
