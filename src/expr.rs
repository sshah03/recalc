//! The `=` expression language: cell refs, A1 ranges (as function arguments
//! only), suffixed refs (`{B2.mark}`, see `META`), arithmetic, comparison,
//! `+` concatenation and the fixed function set in `Func`. Kept deliberately
//! small; Excel parity is not a goal.

use crate::model::{range_cells, ref_name};

/// Suffixes that read a cell's facts instead of its value: last run's exit
/// code, stderr, duration (ms), cost (USD), last writer, review mark, and
/// whether its citation was found.
pub const META: &[&str] = &["code", "err", "dur", "cost", "model", "by", "mark", "cited"];

/// `B2.mark` as (`B2`, Some(`mark`)); a plain name as (name, None).
pub fn split_meta(r: &str) -> (&str, Option<&str>) {
    match r.rsplit_once('.') {
        Some((base, m)) if META.contains(&m) => (base, Some(m)),
        _ => (r, None),
    }
}

#[derive(Debug, PartialEq)]
pub enum Ast {
    Num(f64),
    Str(String),
    Ref(String),
    Call(Func, Vec<Arg>),
    Neg(Box<Ast>),
    Bin(Op, Box<Ast>, Box<Ast>),
}

/// A function argument: an expression, or a range already expanded to its
/// member cell names. Ranges are only legal here, never as bare values.
#[derive(Debug, PartialEq)]
pub enum Arg {
    Expr(Ast),
    Range(Vec<String>),
}

/// The complete function set (see the module doc).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Func {
    // aggregates
    Sum,
    Avg,
    Count,
    Min,
    Max,
    Median,
    CountIf,
    CountIfs,
    SumIf,
    // numbers
    Round,
    Abs,
    // logic
    If,
    And,
    Or,
    Not,
    // text
    Concat,
    Len,
    Lower,
    Upper,
    Trim,
    // lookup
    Lookup,
    Match,
}

impl Func {
    /// Recognized case-insensitively, so `SUM(...)` and `sum(...)` both work.
    fn from_name(name: &str) -> Option<Func> {
        Some(match name.to_ascii_lowercase().as_str() {
            "sum" => Func::Sum,
            "avg" => Func::Avg,
            "count" => Func::Count,
            "min" => Func::Min,
            "max" => Func::Max,
            "median" => Func::Median,
            "round" => Func::Round,
            "abs" => Func::Abs,
            "countif" => Func::CountIf,
            "countifs" => Func::CountIfs,
            "sumif" => Func::SumIf,
            "if" => Func::If,
            "and" => Func::And,
            "or" => Func::Or,
            "not" => Func::Not,
            "concat" => Func::Concat,
            "len" => Func::Len,
            "lower" => Func::Lower,
            "upper" => Func::Upper,
            "trim" => Func::Trim,
            "lookup" => Func::Lookup,
            "match" => Func::Match,
            _ => return None,
        })
    }

    /// The signature shown in arity errors and the UI cheatsheet.
    pub fn signature(self) -> &'static str {
        match self {
            Func::Sum => "sum(range or value, ...)",
            Func::Avg => "avg(range or value, ...)",
            Func::Count => "count(range or value, ...)",
            Func::Min => "min(range or value, ...)",
            Func::Max => "max(range or value, ...)",
            Func::Median => "median(range or value, ...)",
            Func::Round => "round(number[, digits])",
            Func::Abs => "abs(number)",
            Func::CountIf => "countif(range, criterion)",
            Func::CountIfs => "countifs(range, criterion, range, criterion, ...)",
            Func::SumIf => "sumif(range, criterion[, sum_range])",
            Func::If => "if(cond, then, else)",
            Func::And => "and(cond, ...)",
            Func::Or => "or(cond, ...)",
            Func::Not => "not(cond)",
            Func::Concat => "concat(value, ...)",
            Func::Len => "len(text)",
            Func::Lower => "lower(text)",
            Func::Upper => "upper(text)",
            Func::Trim => "trim(text)",
            Func::Lookup => "lookup(key, search_range, result_range)",
            Func::Match => "match(key, range)",
        }
    }

    /// Parse-time argument check. Each slot takes a range or an expression;
    /// trailing slots past `min` are optional, and variadics repeat the last.
    fn check_args(self, args: &[Arg]) -> Result<(), String> {
        const R: bool = true; // range
        const E: bool = false; // expression
        let (slots, min): (&[bool], usize) = match self {
            Func::Sum | Func::Avg | Func::Count | Func::Min | Func::Max | Func::Median => (&[R], 1),
            Func::Round => (&[E, E], 1),
            Func::Abs => (&[E], 1),
            Func::CountIf => (&[R, E], 2),
            // Pairs; checked below.
            Func::CountIfs => (&[R, E], 2),
            Func::SumIf => (&[R, E, R], 2),
            Func::If => (&[E, E, E], 3),
            Func::And | Func::Or => (&[E], 1),
            Func::Not | Func::Len | Func::Lower | Func::Upper | Func::Trim => (&[E], 1),
            Func::Concat => (&[E], 1),
            Func::Lookup => (&[E, R, R], 3),
            Func::Match => (&[E, R], 2),
        };
        let aggregate = matches!(
            self,
            Func::Sum | Func::Avg | Func::Count | Func::Min | Func::Max | Func::Median
        );
        // Aggregates take any mix of ranges, named ranges and values.
        if aggregate {
            return if args.is_empty() {
                Err(format!("expected {}", self.signature()))
            } else {
                Ok(())
            };
        }
        if self == Func::CountIfs {
            let pairs = args.len() >= 2 && args.len().is_multiple_of(2);
            let shaped = args.iter().enumerate().all(|(i, a)| {
                if i % 2 == 0 {
                    matches!(a, Arg::Range(_) | Arg::Expr(Ast::Ref(_)))
                } else {
                    matches!(a, Arg::Expr(_))
                }
            });
            return if pairs && shaped {
                Ok(())
            } else {
                Err(format!("expected {}", self.signature()))
            };
        }
        let variadic = matches!(self, Func::And | Func::Or | Func::Concat);
        let ok_len = if variadic {
            args.len() >= min
        } else {
            args.len() >= min && args.len() <= slots.len()
        };
        if !ok_len {
            return Err(format!("expected {}", self.signature()));
        }
        for (i, arg) in args.iter().enumerate() {
            let want_range = *slots.get(i).unwrap_or(&slots[slots.len() - 1]);
            // A range slot also takes a ref to a named range, resolved at eval.
            let ok = if want_range {
                matches!(arg, Arg::Range(_) | Arg::Expr(Ast::Ref(_)))
            } else {
                matches!(arg, Arg::Expr(_))
            };
            if !ok {
                return Err(format!("expected {}", self.signature()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    /// `&`: text joined, as in Excel and Sheets.
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

const MAX_DEPTH: usize = 200;

/// How deep the parser would recurse: open parens plus consecutive unary minuses.
fn nesting(toks: &[Tok]) -> usize {
    let (mut parens, mut unary, mut deepest) = (0usize, 0usize, 0usize);
    let mut expect_value = true;
    for t in toks {
        match t {
            Tok::LParen => {
                parens += 1;
                expect_value = true;
            }
            Tok::RParen => {
                parens = parens.saturating_sub(1);
                expect_value = false;
            }
            Tok::Op(Op::Sub) if expect_value => unary += 1,
            Tok::Op(_) | Tok::Comma => expect_value = true,
            _ => {
                unary = 0;
                expect_value = false;
            }
        }
        deepest = deepest.max(parens + unary);
    }
    deepest
}

/// Parse the text after the `=` prefix.
pub fn parse(src: &str) -> Result<Ast, String> {
    let toks = lex(src)?;
    // Parse and eval recurse per paren and unary minus; cap depth to avoid a
    // stack overflow.
    if nesting(&toks) > MAX_DEPTH {
        return Err(format!("nested more than {MAX_DEPTH} deep"));
    }
    let mut p = Parser { toks, pos: 0 };
    let ast = p.comparison()?;
    match p.peek() {
        None => Ok(ast),
        Some(t) => Err(format!("unexpected {t:?}")),
    }
}

/// Evaluate `{cell} <predicate>`, e.g. "!= ''". A parse error is an error;
/// an eval error means not met yet.
pub fn predicate(
    cell: &str,
    predicate: &str,
    lookup: &dyn Fn(&str) -> String,
) -> Result<bool, String> {
    let ast = parse(&format!("{{{cell}}} {predicate}"))
        .map_err(|e| format!("bad predicate {predicate:?}: {e}"))?;
    Ok(eval(&ast, lookup).is_ok_and(|v| v == "true"))
}

/// The cells whose facts (`{B2.mark}`) the expression reads, so a change to
/// those facts alone can trigger a recompute.
pub fn meta_reads(ast: &Ast) -> Vec<String> {
    let mut all = Vec::new();
    collect_refs(ast, &mut all);
    let mut out: Vec<String> = Vec::new();
    for r in all {
        if let (base, Some(_)) = split_meta(&r) {
            if !out.iter().any(|o| o == base) {
                out.push(base.to_string());
            }
        }
    }
    out
}

fn collect_refs(ast: &Ast, out: &mut Vec<String>) {
    match ast {
        Ast::Ref(name) => out.push(name.clone()),
        Ast::Call(_, args) => {
            for arg in args {
                match arg {
                    Arg::Expr(inner) => collect_refs(inner, out),
                    Arg::Range(cells) => out.extend(cells.iter().cloned()),
                }
            }
        }
        Ast::Neg(inner) => collect_refs(inner, out),
        Ast::Bin(_, l, r) => {
            collect_refs(l, out);
            collect_refs(r, out);
        }
        Ast::Num(_) | Ast::Str(_) => {}
    }
}

/// Every cell name the expression references, ranges expanded.
pub fn deps(ast: &Ast) -> Vec<String> {
    let mut out = Vec::new();
    collect_deps(ast, &mut out);
    out
}

fn collect_deps(ast: &Ast, out: &mut Vec<String>) {
    // `{B2.mark}` depends on B2.
    let push = |name: &String, out: &mut Vec<String>| {
        let name = split_meta(name).0;
        if !out.iter().any(|o| o == name) {
            out.push(name.to_string());
        }
    };
    match ast {
        Ast::Ref(name) => push(name, out),
        Ast::Call(_, args) => {
            for arg in args {
                match arg {
                    Arg::Expr(inner) => collect_deps(inner, out),
                    Arg::Range(cells) => {
                        for name in cells {
                            push(name, out);
                        }
                    }
                }
            }
        }
        Ast::Neg(inner) => collect_deps(inner, out),
        Ast::Bin(_, l, r) => {
            collect_deps(l, out);
            collect_deps(r, out);
        }
        Ast::Num(_) | Ast::Str(_) => {}
    }
}

/// Evaluate against current cell values. `lookup` returns "" for missing cells.
pub fn eval(ast: &Ast, lookup: &dyn Fn(&str) -> String) -> Result<String, String> {
    Ok(eval_val(ast, lookup)?.display())
}

/// What a lookup returns for a computed cell that hasn't answered yet (an
/// agent not run, a run in flight, a formula waiting on one), and for one
/// whose run failed without an answer. Both read as blank, except where a
/// number is needed: there, a pending answer makes the formula wait
/// (`WAITING`), and a failed one is its error.
pub const PENDING: &str = "\u{1}pending";
pub const FAILED: &str = "\u{1}failed";
/// The evaluation result for "an input it needs as a number isn't in yet".
pub const WAITING: &str = "waiting for an answer";

enum Val {
    Num(f64),
    Str(String),
    Pending,
    Failed,
}

impl Val {
    fn num(&self) -> Option<f64> {
        match self {
            Val::Num(n) => Some(*n),
            Val::Str(s) => parse_number(s),
            Val::Pending | Val::Failed => None,
        }
    }

    fn display(&self) -> String {
        match self {
            Val::Num(n) => fmt_num(*n),
            Val::Str(s) => s.clone(),
            Val::Pending | Val::Failed => String::new(),
        }
    }

    /// A cell's value as a lookup returned it.
    fn of(s: String) -> Val {
        match s.as_str() {
            PENDING => Val::Pending,
            FAILED => Val::Failed,
            _ => Val::Str(s),
        }
    }
}

/// A lookup's value with the not-yet marks read as blank, for functions over
/// ranges (`sum`, `count`, `countif`), which take what's there.
fn plain(s: String) -> String {
    if s == PENDING || s == FAILED {
        String::new()
    } else {
        s
    }
}

/// The reason a value can't be used as a number, if it's an answer not in.
fn not_in(v: &Val) -> Option<String> {
    match v {
        Val::Pending => Some(WAITING.into()),
        Val::Failed => Some("an input it needs has no answer: its run failed".into()),
        _ => None,
    }
}

/// A number as people and documents write one: `1200`, `1,200.50`,
/// `$1,200`, `-$5`, `5%` (0.05). Commas only in groups of three, so a list
/// like `1,2` isn't a number.
pub fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if let Ok(n) = t.parse::<f64>() {
        return Some(n);
    }
    let (t, percent) = match t.strip_suffix('%') {
        Some(r) => (r.trim_end(), true),
        None => (t, false),
    };
    let (mut neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let t = t.strip_prefix('$').unwrap_or(t);
    let t = match t.strip_prefix('-') {
        Some(r) if !neg => {
            neg = true;
            r
        }
        _ => t,
    };
    let (int, frac) = t.split_once('.').unwrap_or((t, ""));
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit());
    let mut groups = int.split(',');
    let first = groups.next().unwrap_or("");
    let grouped = int.contains(',');
    let int_ok = if grouped {
        digits(first) && first.len() <= 3 && groups.all(|g| g.len() == 3 && digits(g))
    } else {
        int.is_empty() || digits(int)
    };
    if !int_ok || (int.is_empty() && frac.is_empty()) || !(frac.is_empty() || digits(frac)) {
        return None;
    }
    let n: f64 = format!(
        "{}.{}",
        if int.is_empty() { "0" } else { int }.replace(',', ""),
        if frac.is_empty() { "0" } else { frac }
    )
    .parse()
    .ok()?;
    let n = if neg { -n } else { n };
    Some(if percent { n / 100.0 } else { n })
}

/// `n` to 15 significant digits, as a spreadsheet shows and compares it:
/// a sum of cents like 5312.399999999994 is 5312.4.
fn tidy(n: f64) -> f64 {
    if !n.is_finite() || n == 0.0 {
        return n;
    }
    format!("{n:.14e}").parse().unwrap_or(n)
}

fn fmt_num(n: f64) -> String {
    let n = tidy(n);
    if n.fract() == 0.0 && n.abs() < 9e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn eval_val(ast: &Ast, lookup: &dyn Fn(&str) -> String) -> Result<Val, String> {
    match ast {
        Ast::Num(n) => Ok(Val::Num(*n)),
        Ast::Str(s) => Ok(Val::Str(s.clone())),
        Ast::Ref(name) => Ok(Val::of(lookup(name))),
        Ast::Call(f, args) => eval_call(*f, args, lookup),
        Ast::Neg(inner) => {
            let v = eval_val(inner, lookup)?;
            if let Some(e) = not_in(&v) {
                return Err(e);
            }
            match arith(&v) {
                Some(n) => Ok(Val::Num(-n)),
                None => Err(format!("not a number: {:?}", v.display())),
            }
        }
        Ast::Bin(op, l, r) => {
            let l = eval_val(l, lookup)?;
            let r = eval_val(r, lookup)?;
            eval_bin(*op, l, r)
        }
    }
}

/// A value as arithmetic reads it: a blank (an empty or missing cell) is 0,
/// as in Excel; text that isn't a number has none.
fn arith(v: &Val) -> Option<f64> {
    if v.display().trim().is_empty() {
        Some(0.0)
    } else {
        v.num()
    }
}

fn eval_bin(op: Op, l: Val, r: Val) -> Result<Val, String> {
    let nums = l.num().zip(r.num());
    let bool_val = |b: bool| Val::Str(if b { "true" } else { "false" }.into());
    match op {
        Op::Concat => Ok(Val::Str(l.display() + &r.display())),
        Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::Lt | Op::Gt | Op::Le | Op::Ge
            if not_in(&l).or_else(|| not_in(&r)).is_some() =>
        {
            Err(not_in(&l).or_else(|| not_in(&r)).unwrap_or_default())
        }
        Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod => {
            // Arithmetic only, as in Excel: text joins with &.
            let (a, b) = arith(&l).zip(arith(&r)).ok_or_else(|| {
                let bad = if arith(&l).is_none() { l } else { r };
                let hint = if op == Op::Add {
                    "; join text with &"
                } else {
                    ""
                };
                format!("not a number: {:?}{hint}", bad.display())
            })?;
            match op {
                Op::Add => Ok(Val::Num(a + b)),
                Op::Sub => Ok(Val::Num(a - b)),
                Op::Mul => Ok(Val::Num(a * b)),
                Op::Div if b == 0.0 => Err("division by zero".into()),
                Op::Div => Ok(Val::Num(a / b)),
                Op::Mod if b == 0.0 => Err("division by zero".into()),
                Op::Mod => Ok(Val::Num(a % b)),
                _ => unreachable!(),
            }
        }
        Op::Eq | Op::Ne => {
            let equal = match nums {
                Some((a, b)) => tidy(a) == tidy(b),
                None => l.display() == r.display(),
            };
            Ok(bool_val(if op == Op::Eq { equal } else { !equal }))
        }
        Op::Lt | Op::Gt | Op::Le | Op::Ge => {
            let (a, b) = nums.ok_or_else(|| {
                let bad = if l.num().is_none() { l } else { r };
                format!("not a number: {:?}", bad.display())
            })?;
            let (a, b) = (tidy(a), tidy(b));
            Ok(bool_val(match op {
                Op::Lt => a < b,
                Op::Gt => a > b,
                Op::Le => a <= b,
                Op::Ge => a >= b,
                _ => unreachable!(),
            }))
        }
    }
}

fn eval_call(f: Func, args: &[Arg], lookup: &dyn Fn(&str) -> String) -> Result<Val, String> {
    // check_args ran at parse time, so argument shapes are guaranteed.
    let range = |i: usize| -> Result<Vec<String>, String> {
        let members = match &args[i] {
            Arg::Range(cells) => cells.clone(),
            // A named range: the ref's value names the rectangle.
            Arg::Expr(Ast::Ref(name)) => {
                let v = plain(lookup(name));
                v.split_once(':')
                    .and_then(|(a, b)| range_cells(a.trim(), b.trim()))
                    .ok_or_else(|| format!("{name} is not a range: {v:?} (want e.g. A2:A20)"))?
            }
            Arg::Expr(_) => unreachable!("check_args guarantees a range"),
        };
        Ok(members.iter().map(|n| plain(lookup(n))).collect())
    };
    let expr = |i: usize| -> Result<Val, String> {
        match &args[i] {
            Arg::Expr(ast) => eval_val(ast, lookup),
            Arg::Range(_) => unreachable!("check_args guarantees an expression"),
        }
    };

    match f {
        Func::Sum | Func::Avg | Func::Count | Func::Min | Func::Max | Func::Median => {
            let mut values = Vec::new();
            for (i, arg) in args.iter().enumerate() {
                match arg {
                    Arg::Range(_) => values.extend(range(i)?),
                    // A ref whose value is a range is a named range; otherwise one value.
                    Arg::Expr(Ast::Ref(name)) => {
                        let v = plain(lookup(name));
                        match v
                            .split_once(':')
                            .and_then(|(a, b)| range_cells(a.trim(), b.trim()))
                        {
                            Some(_) => values.extend(range(i)?),
                            None => values.push(v),
                        }
                    }
                    Arg::Expr(ast) => values.push(eval_val(ast, lookup)?.display()),
                }
            }
            eval_agg(f, &values)
        }
        Func::Round | Func::Abs => {
            let v = expr(0)?;
            if let Some(e) = not_in(&v) {
                return Err(e);
            }
            let n = v
                .num()
                .ok_or_else(|| format!("not a number: {:?}", v.display()))?;
            if f == Func::Abs {
                return Ok(Val::Num(n.abs()));
            }
            let digits = match args.len() {
                1 => 0.0,
                _ => {
                    let d = expr(1)?;
                    d.num()
                        .ok_or_else(|| format!("not a number of digits: {:?}", d.display()))?
                }
            };
            // Half away from zero; tidy first so 2.675 (really 2.67499…) rounds up.
            let scale = 10f64.powi(digits as i32);
            Ok(Val::Num((tidy(n * scale)).round() / scale))
        }
        Func::CountIf => {
            let crit = expr(1)?.display();
            let n = range(0)?
                .iter()
                .filter(|v| matches_criterion(v, &crit))
                .count();
            Ok(Val::Num(n as f64))
        }
        Func::CountIfs => {
            let mut hits: Option<Vec<bool>> = None;
            for pair in 0..args.len() / 2 {
                let values = range(2 * pair)?;
                let crit = expr(2 * pair + 1)?.display();
                let matched: Vec<bool> =
                    values.iter().map(|v| matches_criterion(v, &crit)).collect();
                hits = Some(match hits {
                    None => matched,
                    Some(prev) if prev.len() == matched.len() => {
                        prev.iter().zip(&matched).map(|(a, b)| *a && *b).collect()
                    }
                    Some(_) => return Err("countifs ranges differ in size".into()),
                });
            }
            Ok(Val::Num(
                hits.unwrap_or_default().iter().filter(|h| **h).count() as f64,
            ))
        }
        Func::SumIf => {
            let search = range(0)?;
            let crit = expr(1)?.display();
            let sums = if args.len() == 3 {
                range(2)?
            } else {
                search.clone()
            };
            if sums.len() != search.len() {
                return Err("sumif ranges differ in size".into());
            }
            let total: f64 = search
                .iter()
                .zip(&sums)
                .filter(|(v, _)| matches_criterion(v, &crit))
                .filter_map(|(_, s)| parse_number(s))
                .sum();
            Ok(Val::Num(total))
        }
        Func::If => {
            // Lazy: only the taken branch evaluates, so the other may error.
            if truthy(&expr(0)?)? {
                expr(1)
            } else {
                expr(2)
            }
        }
        Func::And | Func::Or => {
            let stop = f == Func::Or; // and stops on false, or stops on true
            for i in 0..args.len() {
                if truthy(&expr(i)?)? == stop {
                    return Ok(Val::Str(stop.to_string()));
                }
            }
            Ok(Val::Str((!stop).to_string()))
        }
        Func::Not => Ok(Val::Str((!truthy(&expr(0)?)?).to_string())),
        Func::Concat => {
            let mut out = String::new();
            for i in 0..args.len() {
                out.push_str(&expr(i)?.display());
            }
            Ok(Val::Str(out))
        }
        Func::Len => Ok(Val::Num(expr(0)?.display().chars().count() as f64)),
        Func::Lower => Ok(Val::Str(expr(0)?.display().to_lowercase())),
        Func::Upper => Ok(Val::Str(expr(0)?.display().to_uppercase())),
        Func::Trim => Ok(Val::Str(expr(0)?.display().trim().to_string())),
        Func::Lookup | Func::Match => {
            let key = expr(0)?;
            let search = range(1)?;
            let pos = search
                .iter()
                .position(|v| values_equal(v, &key))
                .ok_or_else(|| format!("{:?} not found in range", key.display()))?;
            if f == Func::Match {
                return Ok(Val::Num(pos as f64 + 1.0));
            }
            let results = range(2)?;
            if results.len() != search.len() {
                return Err("lookup ranges differ in size".into());
            }
            Ok(Val::Str(results[pos].clone()))
        }
    }
}

fn eval_agg(f: Func, values: &[String]) -> Result<Val, String> {
    if f == Func::Count {
        return Ok(Val::Num(
            values.iter().filter(|v| !v.is_empty()).count() as f64
        ));
    }
    // The numeric aggregates skip non-numeric values, spreadsheet-style.
    let nums: Vec<f64> = values.iter().filter_map(|v| parse_number(v)).collect();
    match f {
        Func::Sum => Ok(Val::Num(nums.iter().sum())),
        Func::Avg if nums.is_empty() => Err("avg of no numeric values".into()),
        Func::Avg => Ok(Val::Num(nums.iter().sum::<f64>() / nums.len() as f64)),
        Func::Median if nums.is_empty() => Err("median of no numeric values".into()),
        Func::Median => {
            let mut sorted = nums.clone();
            sorted.sort_by(f64::total_cmp);
            let mid = sorted.len() / 2;
            Ok(Val::Num(if sorted.len() % 2 == 1 {
                sorted[mid]
            } else {
                (sorted[mid - 1] + sorted[mid]) / 2.0
            }))
        }
        Func::Min | Func::Max => {
            let init = nums.first().copied().ok_or("no numeric values in range")?;
            let pick: fn(f64, f64) -> f64 = if f == Func::Min { f64::min } else { f64::max };
            Ok(Val::Num(nums.iter().copied().fold(init, pick)))
        }
        _ => unreachable!("only the aggregates reach eval_agg"),
    }
}

/// "true"/"false" (what comparisons produce), or a number (nonzero is true).
fn truthy(v: &Val) -> Result<bool, String> {
    match v {
        Val::Str(s) if s == "true" => Ok(true),
        Val::Str(s) if s == "false" => Ok(false),
        _ => match v.num() {
            Some(n) => Ok(n != 0.0),
            None => Err(format!("not a boolean: {:?}", v.display())),
        },
    }
}

/// Numeric-aware equality, matching `==`: "1.0" equals "1".
fn values_equal(value: &str, key: &Val) -> bool {
    matches!(
        eval_bin(Op::Eq, Val::Str(value.to_string()), Val::Str(key.display())),
        Ok(Val::Str(s)) if s == "true"
    )
}

/// A countif/sumif criterion: an operator prefix (`">5"`, `"<>x"`) or a bare
/// value for equality. A comparison that can't evaluate is a non-match.
fn matches_criterion(value: &str, criterion: &str) -> bool {
    let (op, rest) = match criterion {
        _ if criterion.starts_with(">=") => (Op::Ge, &criterion[2..]),
        _ if criterion.starts_with("<=") => (Op::Le, &criterion[2..]),
        _ if criterion.starts_with("!=") => (Op::Ne, &criterion[2..]),
        _ if criterion.starts_with("<>") => (Op::Ne, &criterion[2..]),
        _ if criterion.starts_with('>') => (Op::Gt, &criterion[1..]),
        _ if criterion.starts_with('<') => (Op::Lt, &criterion[1..]),
        _ => (Op::Eq, criterion),
    };
    matches!(
        eval_bin(op, Val::Str(value.to_string()), Val::Str(rest.trim().to_string())),
        Ok(Val::Str(s)) if s == "true"
    )
}

// ---- lexer & parser ----

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    /// `{name}`
    Ref(String),
    /// `{A1:B3}`, already expanded to member names.
    Range(Vec<String>),
    Op(Op),
    LParen,
    RParen,
    Comma,
}

/// Where a quoted-sheet reference starting at `i` (`'It''s'!$A$1`) ends,
/// or None when the quote opens a string.
fn quoted_sheet_ref(src: &str, i: usize) -> Option<usize> {
    let mut j = i + 1;
    loop {
        let k = src[j..].find('\'')? + j;
        if src[k + 1..].starts_with('\'') {
            j = k + 2;
            continue;
        }
        let tail = &src[k + 1..];
        if !tail.starts_with('!') || tail.starts_with("!=") {
            return None;
        }
        let len = tail[1..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '$')
            .unwrap_or(tail.len() - 1);
        return crate::model::A1Ref::parse(&tail[1..1 + len]).map(|_| k + 2 + len);
    }
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            ' ' | '\t' | '\n' => i += 1,
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                toks.push(Tok::Comma);
                i += 1;
            }
            '+' => {
                toks.push(Tok::Op(Op::Add));
                i += 1;
            }
            '&' => {
                toks.push(Tok::Op(Op::Concat));
                i += 1;
            }
            '-' => {
                toks.push(Tok::Op(Op::Sub));
                i += 1;
            }
            '*' => {
                toks.push(Tok::Op(Op::Mul));
                i += 1;
            }
            '/' => {
                toks.push(Tok::Op(Op::Div));
                i += 1;
            }
            '%' => {
                toks.push(Tok::Op(Op::Mod));
                i += 1;
            }
            '=' | '!' | '<' | '>' => {
                let two = bytes.get(i + 1) == Some(&b'=');
                let op = match (c, two) {
                    ('=', true) => Op::Eq,
                    ('!', true) => Op::Ne,
                    ('<', true) => Op::Le,
                    ('>', true) => Op::Ge,
                    ('<', false) => Op::Lt,
                    ('>', false) => Op::Gt,
                    _ => return Err(format!("unexpected {c:?}")),
                };
                toks.push(Tok::Op(op));
                i += if two { 2 } else { 1 };
            }
            '\'' | '"' => {
                // `'My Notes'!A2` (`''` is an apostrophe in a sheet's name):
                // a quoted sheet, then an A1 ref, not `!=`.
                if let Some(stop) = (c == '\'').then(|| quoted_sheet_ref(src, i)).flatten() {
                    let name = ref_name(&src[i..stop])
                        .ok_or_else(|| format!("bad reference {}", &src[i..stop]))?;
                    toks.push(Tok::Ref(name));
                    i = stop;
                    continue;
                }
                let end = src[i + 1..]
                    .find(c)
                    .ok_or_else(|| format!("unterminated string starting at {c}"))?;
                toks.push(Tok::Str(src[i + 1..i + 1 + end].to_string()));
                i += end + 2;
            }
            '{' => {
                let end = src[i..].find('}').ok_or("unterminated {")? + i;
                let body = &src[i + 1..end];
                // `{a, b, C3}`: a list of cells, accepted wherever a range is.
                if body.contains(',') {
                    let mut names = Vec::new();
                    for item in body.split(',') {
                        let item = item.trim();
                        let (core, meta) = split_meta(item);
                        let name = ref_name(core).ok_or_else(|| {
                            format!("bad reference {{{item}}} in the list {{{body}}}")
                        })?;
                        names.push(match meta {
                            Some(m) => format!("{name}.{m}"),
                            None => name,
                        });
                    }
                    toks.push(Tok::Range(names));
                    i = end + 1;
                    continue;
                }
                let (core, meta) = match body.trim().rsplit_once('.') {
                    Some((core, m)) if META.contains(&m) => (core, Some(m)),
                    Some((_, m)) if m.chars().all(|c| c.is_ascii_alphabetic()) => {
                        return Err(format!(
                            "bad reference {{{body}}}: a cell's .{m} isn't something a formula can read; it can read .{}",
                            META.join(", .")
                        ))
                    }
                    _ => (body, None),
                };
                let with_meta = |n: String| match meta {
                    Some(m) => format!("{n}.{m}"),
                    None => n,
                };
                match core.split_once(':') {
                    Some((from, to)) => {
                        let cells = range_cells(from.trim(), to.trim())
                            .ok_or_else(|| format!("bad range {{{body}}}"))?;
                        toks.push(Tok::Range(cells.into_iter().map(with_meta).collect()));
                    }
                    None => match ref_name(core) {
                        Some(name) => toks.push(Tok::Ref(with_meta(name))),
                        None => return Err(format!("bad reference {{{body}}}")),
                    },
                }
                i = end + 1;
            }
            '0'..='9' | '.' => {
                let end = src[i..]
                    .find(|ch: char| !ch.is_ascii_digit() && ch != '.')
                    .map_or(src.len(), |e| e + i);
                let n: f64 = src[i..end]
                    .parse()
                    .map_err(|_| format!("bad number {:?}", &src[i..end]))?;
                toks.push(Tok::Num(n));
                i = end;
            }
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let end = src[i..]
                    .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                    .map_or(src.len(), |e| e + i);
                // `Notes!A2` unbraced: a sheet, then an A1 ref (not `!=`).
                if bytes.get(end) == Some(&b'!') && bytes.get(end + 1) != Some(&b'=') {
                    let stop = src[end + 1..]
                        .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '$')
                        .map_or(src.len(), |e| e + end + 1);
                    let name = ref_name(&src[i..stop])
                        .ok_or_else(|| format!("bad reference {}", &src[i..stop]))?;
                    toks.push(Tok::Ref(name));
                    i = stop;
                    continue;
                }
                toks.push(Tok::Ident(src[i..end].to_string()));
                i = end;
            }
            _ => return Err(format!("unexpected {c:?}")),
        }
    }
    Ok(toks)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned()?;
        self.pos += 1;
        Some(t)
    }

    fn comparison(&mut self) -> Result<Ast, String> {
        let left = self.joined()?;
        if let Some(Tok::Op(op)) = self.peek() {
            if matches!(op, Op::Eq | Op::Ne | Op::Lt | Op::Gt | Op::Le | Op::Ge) {
                let op = *op;
                self.pos += 1;
                let right = self.joined()?;
                return Ok(Ast::Bin(op, Box::new(left), Box::new(right)));
            }
        }
        Ok(left)
    }

    /// `&` binds looser than arithmetic, tighter than comparison, as in Excel:
    /// `{a} & {b} + 1` joins a to b + 1.
    fn joined(&mut self) -> Result<Ast, String> {
        let mut left = self.additive()?;
        while let Some(Tok::Op(Op::Concat)) = self.peek() {
            self.pos += 1;
            let right = self.additive()?;
            left = Ast::Bin(Op::Concat, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Ast, String> {
        let mut left = self.term()?;
        while let Some(Tok::Op(op @ (Op::Add | Op::Sub))) = self.peek() {
            let op = *op;
            self.pos += 1;
            let right = self.term()?;
            left = Ast::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<Ast, String> {
        let mut left = self.unary()?;
        while let Some(Tok::Op(op @ (Op::Mul | Op::Div | Op::Mod))) = self.peek() {
            let op = *op;
            self.pos += 1;
            let right = self.unary()?;
            left = Ast::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Ast, String> {
        if matches!(self.peek(), Some(Tok::Op(Op::Sub))) {
            self.pos += 1;
            return Ok(Ast::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Ast, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Ast::Num(n)),
            Some(Tok::Str(s)) => Ok(Ast::Str(s)),
            Some(Tok::Ref(name)) => Ok(Ast::Ref(name)),
            Some(Tok::Ident(word)) => match word.to_ascii_lowercase().as_str() {
                // Keywords, not refs; case-insensitive like function names.
                lower @ ("true" | "false") => Ok(Ast::Str(lower.to_string())),
                _ if matches!(self.peek(), Some(Tok::LParen)) => match Func::from_name(&word) {
                    Some(f) => self.call(f),
                    None => Err(format!("unknown function {word}()")),
                },
                _ => Ok(Ast::Ref(word)),
            },
            Some(Tok::LParen) => {
                let inner = self.comparison()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(inner),
                    other => Err(format!("expected ), got {other:?}")),
                }
            }
            Some(Tok::Range(_)) => Err("range only allowed as a function argument".into()),
            other => Err(format!("expected a value, got {other:?}")),
        }
    }

    /// Parse arguments after a peeked `(` through `)` and check them against `f`.
    fn call(&mut self, f: Func) -> Result<Ast, String> {
        self.pos += 1; // (
        let mut args = Vec::new();
        if !matches!(self.peek(), Some(Tok::RParen)) {
            loop {
                if let Some(Tok::Range(_)) = self.peek() {
                    let Some(Tok::Range(cells)) = self.next() else {
                        unreachable!()
                    };
                    args.push(Arg::Range(cells));
                } else {
                    args.push(Arg::Expr(self.comparison()?));
                }
                match self.next() {
                    Some(Tok::Comma) => continue,
                    Some(Tok::RParen) => break,
                    other => return Err(format!("expected , or ), got {other:?}")),
                }
            }
        } else {
            self.pos += 1; // )
        }
        f.check_args(&args)?;
        Ok(Ast::Call(f, args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn ev(src: &str, cells: &[(&str, &str)]) -> Result<String, String> {
        let map: HashMap<&str, &str> = cells.iter().copied().collect();
        let ast = parse(src)?;
        eval(&ast, &|name| map.get(name).unwrap_or(&"").to_string())
    }

    #[test]
    fn suffixed_refs_read_run_and_review_facts() {
        let cells = [
            ("c.code", "1"),
            ("B2.mark", "verified"),
            ("B3.mark", ""),
            ("B4.mark", "verified"),
        ];
        assert_eq!(ev("{c.code} == 0", &cells).unwrap(), "false");
        assert_eq!(
            ev("countif({B2:B4.mark}, 'verified')", &cells).unwrap(),
            "2"
        );
        // The dependency is the cell itself; meta_reads names it.
        let ast = parse("countif({B2:B3.mark}, 'verified') + {x} + {c.dur}").unwrap();
        assert_eq!(deps(&ast), ["B2", "B3", "x", "c"]);
        assert_eq!(meta_reads(&ast), ["B2", "B3", "c"]);
        // An unknown fact says which ones exist.
        let err = parse("{B2.colour}").unwrap_err();
        assert!(err.contains(".code") && err.contains(".mark"), "{err}");
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(ev("2 + 3 * 4", &[]).unwrap(), "14");
        assert_eq!(ev("(2 + 3) * 4", &[]).unwrap(), "20");
        assert_eq!(ev("10 / 4", &[]).unwrap(), "2.5");
        assert_eq!(ev("7 % 3", &[]).unwrap(), "1");
        assert_eq!(ev("-{a} + 1", &[("a", "2")]).unwrap(), "-1");
    }

    #[test]
    fn round_abs_and_median() {
        let cells = [("a", "2.675"), ("b", "-3.5"), ("c", "10"), ("d", "x")];
        assert_eq!(ev("round({a}, 2)", &cells).unwrap(), "2.68");
        assert_eq!(ev("round({b})", &cells).unwrap(), "-4");
        assert_eq!(ev("round(1234.5678, -2)", &[]).unwrap(), "1200");
        assert_eq!(ev("abs({b})", &cells).unwrap(), "3.5");
        let col = [
            ("A1", "3"),
            ("A2", "1"),
            ("A3", "x"),
            ("A4", "2"),
            ("A5", "10"),
        ];
        assert_eq!(ev("median({A1:A4})", &col).unwrap(), "2");
        assert_eq!(ev("median({A1:A5})", &col).unwrap(), "2.5");
        assert!(ev("round({d})", &cells).is_err());
    }

    #[test]
    fn sums_of_cents_show_and_compare_as_written() {
        let cells = [("a", "0.1"), ("b", "0.2"), ("c", "5312.40")];
        assert_eq!(ev("{a} + {b}", &cells).unwrap(), "0.3");
        assert_eq!(ev("{a} + {b} == 0.3", &cells).unwrap(), "true");
        assert_eq!(ev("({a} + {b}) * 10 >= 3", &cells).unwrap(), "true");
        assert_eq!(ev("{a} + {b} + {c} - 0.3", &cells).unwrap(), "5312.4");
        // Real differences survive.
        assert_eq!(ev("{a} + {b} == 0.3000001", &cells).unwrap(), "false");
        assert_eq!(ev("1 / 3", &[]).unwrap(), "0.333333333333333");
    }

    #[test]
    fn refs_brace_and_bare() {
        assert_eq!(
            ev("{passed} + {failed}", &[("passed", "3"), ("failed", "1")]).unwrap(),
            "4"
        );
        assert_eq!(
            ev("passed + failed", &[("passed", "3"), ("failed", "1")]).unwrap(),
            "4"
        );
        // Anchors are fill instructions, not part of the name.
        assert_eq!(
            ev(
                "{$A$1} + sum({B$1:$B2})",
                &[("A1", "1"), ("B1", "2"), ("B2", "3")]
            )
            .unwrap(),
            "6"
        );
        assert_eq!(deps(&parse("{$A$1}").unwrap()), ["A1"]);
    }

    #[test]
    fn named_ranges_dereference_at_eval() {
        let cells = [("r", "A1:A3"), ("A1", "1"), ("A2", "2"), ("A3", "4")];
        assert_eq!(ev("sum({r})", &cells).unwrap(), "7");
        assert_eq!(ev("countif({r}, '>1')", &cells).unwrap(), "2");
        assert_eq!(
            ev("lookup(2, {r}, {A1:A3})", &cells).unwrap(),
            "2",
            "named and literal ranges mix"
        );
        // A ref whose value isn't a range is one value; text in a sum is skipped.
        assert_eq!(ev("sum({r})", &[("r", "banana")]).unwrap(), "0");
        assert_eq!(ev("sum({a} + 1)", &[("a", "2")]).unwrap(), "3");
        // Ranges, named ranges and values mix.
        let plan = [
            ("p1", "done"),
            ("p2", "todo"),
            ("p3", "done"),
            ("r", "A1:A3"),
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "4"),
        ];
        assert_eq!(ev("countif({p1, p2, p3}, 'done')", &plan).unwrap(), "2");
        assert_eq!(
            deps(&parse("countif({p1, p2.mark}, 'done')").unwrap()),
            ["p1", "p2"]
        );
        assert!(parse("sum({p1, 2})").unwrap_err().contains("bad reference"));
        assert_eq!(ev("count({p1}, {p2}, {p3})", &plan).unwrap(), "3");
        assert_eq!(ev("sum({r}, {A1:A2}, 10)", &plan).unwrap(), "20");
        assert_eq!(ev("max({A1}, {A3}, 3)", &plan).unwrap(), "4");
        // A range slot elsewhere still takes a ref or a literal range only.
        assert!(parse("countif({a} + 1, 'x')").is_err());
        // Expression slots still reject literal ranges.
        assert!(parse("len({A1:A3})").is_err());
    }

    #[test]
    fn comparisons() {
        assert_eq!(ev("{failed} == 0", &[("failed", "0")]).unwrap(), "true");
        assert_eq!(ev("{failed} == 0", &[("failed", "2")]).unwrap(), "false");
        assert_eq!(ev("{score} > 0.9", &[("score", "0.95")]).unwrap(), "true");
        assert_eq!(ev("{a} != ''", &[("a", "")]).unwrap(), "false");
        assert_eq!(ev("{a} != ''", &[("a", "hi")]).unwrap(), "true");
        assert_eq!(ev("{done} == true", &[("done", "true")]).unwrap(), "true");
        // Keywords match the function names' case-insensitivity.
        assert_eq!(ev("{done} == TRUE", &[("done", "true")]).unwrap(), "true");
        // Numeric equality when both sides parse: "1.0" == "1".
        assert_eq!(ev("{a} == 1", &[("a", "1.0")]).unwrap(), "true");
    }

    #[test]
    fn text_joins_with_ampersand_and_plus_only_adds() {
        assert_eq!(
            ev("{a} & {b}", &[("a", "foo"), ("b", "bar")]).unwrap(),
            "foobar"
        );
        assert_eq!(ev("{a} & 1", &[("a", "v")]).unwrap(), "v1");
        assert_eq!(
            ev("{a} + {b} & ' of 9'", &[("a", "2"), ("b", "3")]).unwrap(),
            "5 of 9"
        );
        // + on text is an error that says how to join it, as - and * are.
        let e = ev("{a} + 1", &[("a", "v")]).unwrap_err();
        assert!(
            e.contains("not a number") && e.contains("join text with &"),
            "{e}"
        );
        // A blank is 0 in arithmetic, as in Excel.
        assert_eq!(ev("{a} + 1", &[("a", "")]).unwrap(), "1");
        assert_eq!(ev("{a} - 1", &[("a", "")]).unwrap(), "-1");
        assert_eq!(ev("-{a} * 3", &[("a", "")]).unwrap(), "0");
    }

    #[test]
    fn numbers_as_people_write_them() {
        for (text, want) in [
            ("1,200", Some(1200.0)),
            ("$1,200.50", Some(1200.5)),
            ("-$5", Some(-5.0)),
            ("$-5", Some(-5.0)),
            ("5%", Some(0.05)),
            ("12.5 %", Some(0.125)),
            ("1,234,567", Some(1234567.0)),
            (".5", Some(0.5)),
            ("1,2", None),
            ("12,34", None),
            ("1,2345", None),
            ("$", None),
            ("abc", None),
            ("5%%", None),
        ] {
            assert_eq!(parse_number(text), want, "{text}");
        }
        let cells = [("a", "$1,200"), ("b", "5%"), ("c", "1,000"), ("d", "abc")];
        assert_eq!(ev("sum({a}, {b}, {c}, {d})", &cells).unwrap(), "2200.05");
        assert_eq!(ev("{a} * 2", &cells).unwrap(), "2400");
        assert_eq!(ev("{c} * {b}", &cells).unwrap(), "50");
    }

    #[test]
    fn ampersand_joins_text_after_arithmetic() {
        let cells = [("a", "Acme"), ("n", "2")];
        assert_eq!(ev("{a} & ' Inc'", &cells).unwrap(), "Acme Inc");
        assert_eq!(ev("{a} & {n} + 1", &cells).unwrap(), "Acme3", "+ before &");
        assert_eq!(ev("1 & 2", &cells).unwrap(), "12", "numbers join as text");
        assert_eq!(
            ev("{a} & 'x' == 'Acmex'", &cells).unwrap(),
            "true",
            "& before =="
        );
    }

    #[test]
    fn aggregates() {
        let cells = [
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "3"),
            ("A4", ""),
            ("A5", "n/a"),
        ];
        assert_eq!(ev("sum({A1:A5})", &cells).unwrap(), "6");
        assert_eq!(ev("avg({A1:A3})", &cells).unwrap(), "2");
        assert_eq!(ev("count({A1:A5})", &cells).unwrap(), "4");
        assert_eq!(ev("min({A1:A3})", &cells).unwrap(), "1");
        assert_eq!(ev("max({A1:A3})", &cells).unwrap(), "3");
        // Function names are case-insensitive.
        assert_eq!(ev("SUM({A1:A3})", &cells).unwrap(), "6");
    }

    #[test]
    fn logic_functions() {
        assert_eq!(
            ev("if({a} > 1, 'big', 'small')", &[("a", "2")]).unwrap(),
            "big"
        );
        assert_eq!(
            ev("if({a} > 1, 'big', 'small')", &[("a", "0")]).unwrap(),
            "small"
        );
        // Lazy: the untaken branch may error.
        assert_eq!(ev("if(1 == 1, 'ok', 1/0)", &[]).unwrap(), "ok");
        assert_eq!(ev("and({a} > 0, {a} < 10)", &[("a", "5")]).unwrap(), "true");
        assert_eq!(ev("and(true, false)", &[]).unwrap(), "false");
        assert_eq!(ev("or(false, {a} == 1)", &[("a", "1")]).unwrap(), "true");
        assert_eq!(ev("not({a} == 1)", &[("a", "1")]).unwrap(), "false");
        // Numbers are truthy by nonzero; text is an error.
        assert_eq!(ev("if({n}, 'y', 'n')", &[("n", "3")]).unwrap(), "y");
        assert!(ev("if({s}, 1, 2)", &[("s", "word")])
            .unwrap_err()
            .contains("not a boolean"));
    }

    #[test]
    fn conditional_aggregates() {
        let cells = [
            ("A1", "1"),
            ("A2", "5"),
            ("A3", "10"),
            ("A4", "n/a"),
            ("B1", "2"),
            ("B2", "3"),
            ("B3", "4"),
            ("B4", "5"),
        ];
        assert_eq!(ev("countif({A1:A4}, '>2')", &cells).unwrap(), "2");
        assert_eq!(ev("countif({A1:A4}, 'n/a')", &cells).unwrap(), "1");
        assert_eq!(ev("countif({A1:A4}, '<>n/a')", &cells).unwrap(), "3");
        assert_eq!(ev("sumif({A1:A4}, '>=5')", &cells).unwrap(), "15");
        assert_eq!(ev("sumif({A1:A4}, '>2', {B1:B4})", &cells).unwrap(), "7");
        // countifs: every pair must match on the same row.
        assert_eq!(
            ev("countifs({A1:A4}, '>2', {B1:B4}, '<5')", &cells).unwrap(),
            "2"
        );
        assert_eq!(
            ev("countifs({A1:A4}, '>2', {B1:B4}, 4)", &cells).unwrap(),
            "1"
        );
        assert!(ev("countifs({A1:A4}, '>2', {B1:B3}, 4)", &cells)
            .unwrap_err()
            .contains("differ in size"));
        assert!(parse("countifs({A1:A4}, '>2', {B1:B4})").is_err());
        assert!(parse("countifs({A1:A4})").is_err());
        // The criterion can itself be computed.
        assert_eq!(
            ev(
                "countif({A1:A3}, '>' & {t})",
                &[("A1", "1"), ("A2", "5"), ("A3", "10"), ("t", "4")]
            )
            .unwrap(),
            "2"
        );
        assert!(ev("sumif({A1:A4}, '>0', {B1:B2})", &cells)
            .unwrap_err()
            .contains("differ in size"));
    }

    #[test]
    fn text_functions() {
        assert_eq!(
            ev("concat({a}, '-', {b})", &[("a", "x"), ("b", "y")]).unwrap(),
            "x-y"
        );
        assert_eq!(ev("len({a})", &[("a", "hello")]).unwrap(), "5");
        assert_eq!(ev("lower('HeLLo')", &[]).unwrap(), "hello");
        assert_eq!(ev("upper('hi')", &[]).unwrap(), "HI");
        assert_eq!(ev("trim({a})", &[("a", "  x  ")]).unwrap(), "x");
    }

    #[test]
    fn lookup_and_match() {
        let cells = [
            ("A1", "serde"),
            ("A2", "tokio"),
            ("A3", "clap"),
            ("B1", "ok"),
            ("B2", "fail"),
            ("B3", "ok"),
        ];
        assert_eq!(
            ev("lookup('tokio', {A1:A3}, {B1:B3})", &cells).unwrap(),
            "fail"
        );
        assert_eq!(ev("match('clap', {A1:A3})", &cells).unwrap(), "3");
        // Numeric-aware key equality, like ==.
        assert_eq!(
            ev("match(2, {C1:C2})", &[("C1", "1.0"), ("C2", "2.0")]).unwrap(),
            "2"
        );
        assert!(ev("match('nope', {A1:A3})", &cells)
            .unwrap_err()
            .contains("not found"));
    }

    #[test]
    fn call_shape_errors() {
        // Wrong arity or argument shape names the signature.
        assert!(parse("if(1, 2)")
            .unwrap_err()
            .contains("if(cond, then, else)"));
        assert!(parse("sum()")
            .unwrap_err()
            .contains("sum(range or value, ...)"));
        assert!(parse("countif({A1:A2})")
            .unwrap_err()
            .contains("countif(range, criterion)"));
        assert!(parse("lookup('k', {A1:A2})")
            .unwrap_err()
            .contains("lookup(key"));
        assert!(parse("frobnicate(1)")
            .unwrap_err()
            .contains("unknown function"));
        assert!(parse("not()").unwrap_err().contains("not(cond)"));
    }

    #[test]
    fn errors() {
        assert!(ev("{a} - 1", &[("a", "word")])
            .unwrap_err()
            .contains("not a number"));
        assert_eq!(ev("1 / 0", &[]).unwrap_err(), "division by zero");
        assert!(ev("avg({A1:A2})", &[]).is_err());
        assert!(parse("{A1:A3}").is_err()); // bare range
        assert!(parse("{bad name}").is_err());
        assert!(parse("1 +").is_err());
        assert!(parse("'unterminated").is_err());
    }

    #[test]
    fn deps_collection() {
        let ast = parse("{a} + sum({B1:B2}) + a").unwrap();
        assert_eq!(deps(&ast), ["a", "B1", "B2"]);
    }

    #[test]
    fn deep_nesting_fails_to_parse_instead_of_overflowing() {
        let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
        assert!(parse(&deep).unwrap_err().contains("nested"));
        assert!(parse(&format!("{}1", "-".repeat(100_000))).is_err());
        // Long but flat is fine.
        let flat = vec!["1"; 1000].join(" - ");
        assert!(parse(&flat).is_ok());
        assert!(parse(&format!("{}1{}", "(".repeat(50), ")".repeat(50))).is_ok());
    }
}
