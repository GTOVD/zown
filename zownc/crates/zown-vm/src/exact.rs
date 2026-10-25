//! Checked decimal and Gaussian integer (SPEC §11). Same rules as `zown/exact.py`.

use super::error::{OVERFLOW, TYPE_MISMATCH};
use super::value::{Dec, Value};
use super::{RunError, Vm};

pub const SCALE_MAX: u32 = 18;

fn pow10(k: u32) -> i128 {
    const P: [i128; 19] = [
        1,
        10,
        100,
        1_000,
        10_000,
        100_000,
        1_000_000,
        10_000_000,
        100_000_000,
        1_000_000_000,
        10_000_000_000,
        100_000_000_000,
        1_000_000_000_000,
        10_000_000_000_000,
        100_000_000_000_000,
        1_000_000_000_000_000,
        10_000_000_000_000_000,
        100_000_000_000_000_000,
        1_000_000_000_000_000_000,
    ];
    P[k as usize]
}

pub fn dec_text(coeff: i64, scale: u32) -> String {
    let sign = if coeff < 0 { "-" } else { "" };
    let digits = coeff.unsigned_abs().to_string();
    if scale == 0 {
        return format!("{sign}{digits}");
    }
    let digits = if digits.len() <= scale as usize {
        format!("{}{digits}", "0".repeat(scale as usize - digits.len() + 1))
    } else {
        digits
    };
    let cut = digits.len() - scale as usize;
    format!("{sign}{}.{}", &digits[..cut], &digits[cut..])
}

fn fit(vm: &Vm, op: &str, n: i128) -> Result<i64, RunError> {
    if n < i64::MIN as i128 || n > i64::MAX as i128 {
        return Err(vm.err(
            OVERFLOW,
            format!("`{op}` result {n} does not fit int"),
            Some(op.to_string()),
            "dc and cx stay inside the checked int; they are not big",
        ));
    }
    Ok(n as i64)
}

pub fn dec_eq(a: &Dec, b: &Dec) -> bool {
    let scale = a.scale.max(b.scale);
    a.coeff as i128 * pow10(scale - a.scale) == b.coeff as i128 * pow10(scale - b.scale)
}

pub fn dec_cmp(a: &Dec, b: &Dec) -> std::cmp::Ordering {
    let scale = a.scale.max(b.scale);
    let av = a.coeff as i128 * pow10(scale - a.scale);
    let bv = b.coeff as i128 * pow10(scale - b.scale);
    av.cmp(&bv)
}

fn align(vm: &Vm, op: &str, a: Dec, b: Dec) -> Result<(Dec, Dec), RunError> {
    if a.scale == b.scale {
        return Ok((a, b));
    }
    let (small, big, flip) = if a.scale < b.scale {
        (a, b, false)
    } else {
        (b, a, true)
    };
    let grown = fit(vm, op, small.coeff as i128 * pow10(big.scale - small.scale))?;
    let lifted = Dec {
        coeff: grown,
        scale: big.scale,
    };
    Ok(if flip { (big, lifted) } else { (lifted, big) })
}

pub fn dec_add(vm: &Vm, op: &str, a: Dec, b: Dec, sub: bool) -> Result<Dec, RunError> {
    let (a, b) = align(vm, op, a, b)?;
    let coeff = if sub {
        a.coeff as i128 - b.coeff as i128
    } else {
        a.coeff as i128 + b.coeff as i128
    };
    Ok(Dec {
        coeff: fit(vm, op, coeff)?,
        scale: a.scale,
    })
}

pub fn dec_mul(vm: &Vm, op: &str, a: Dec, b: Dec) -> Result<Dec, RunError> {
    let scale = a.scale + b.scale;
    if scale > SCALE_MAX {
        return Err(vm.err(
            OVERFLOW,
            format!("`{op}` decimal scale {scale} is above {SCALE_MAX}"),
            Some(op.to_string()),
            "",
        ));
    }
    Ok(Dec {
        coeff: fit(vm, op, a.coeff as i128 * b.coeff as i128)?,
        scale,
    })
}

pub fn dec_neg(vm: &Vm, op: &str, a: Dec) -> Result<Dec, RunError> {
    Ok(Dec {
        coeff: fit(vm, op, -(a.coeff as i128))?,
        scale: a.scale,
    })
}

pub fn cx_add(vm: &Vm, op: &str, a: (i64, i64), b: (i64, i64), sub: bool) -> Result<(i64, i64), RunError> {
    if sub {
        Ok((
            fit(vm, op, a.0 as i128 - b.0 as i128)?,
            fit(vm, op, a.1 as i128 - b.1 as i128)?,
        ))
    } else {
        Ok((
            fit(vm, op, a.0 as i128 + b.0 as i128)?,
            fit(vm, op, a.1 as i128 + b.1 as i128)?,
        ))
    }
}

pub fn cx_mul(vm: &Vm, op: &str, a: (i64, i64), b: (i64, i64)) -> Result<(i64, i64), RunError> {
    let re = fit(
        vm,
        op,
        a.0 as i128 * b.0 as i128 - a.1 as i128 * b.1 as i128,
    )?;
    let im = fit(
        vm,
        op,
        a.0 as i128 * b.1 as i128 + a.1 as i128 * b.0 as i128,
    )?;
    Ok((re, im))
}

pub fn cx_neg(vm: &Vm, op: &str, a: (i64, i64)) -> Result<(i64, i64), RunError> {
    Ok((fit(vm, op, -(a.0 as i128))?, fit(vm, op, -(a.1 as i128))?))
}

fn bad_dec(vm: &Vm, text: &str) -> RunError {
    vm.err(
        TYPE_MISMATCH,
        format!("`dc` expected a decimal literal, got {text:?}"),
        Some("dc".into()),
        "",
    )
}

pub fn parse_dec(vm: &Vm, text: &str) -> Result<Dec, RunError> {
    if text.is_empty() || text.contains('\n') || text.matches('.').count() > 1 || text.matches('-').count() > 1
    {
        return Err(bad_dec(vm, text));
    }
    let neg = text.starts_with('-');
    let body = if neg { &text[1..] } else { text };
    if body.is_empty() || body.starts_with('.') || body.ends_with('.') || body.contains('-') {
        return Err(bad_dec(vm, text));
    }
    let (scale, digits) = if let Some((whole, frac)) = body.split_once('.') {
        if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) || frac.is_empty()
        {
            return Err(bad_dec(vm, text));
        }
        (frac.len() as u32, format!("{whole}{frac}"))
    } else if body.chars().all(|c| c.is_ascii_digit()) {
        (0, body.to_string())
    } else {
        return Err(bad_dec(vm, text));
    };
    if scale > SCALE_MAX {
        return Err(vm.err(
            OVERFLOW,
            format!("`dc` decimal scale {scale} is above {SCALE_MAX}"),
            Some("dc".into()),
            "",
        ));
    }
    let mut coeff: i128 = digits.parse().unwrap_or(i128::MAX);
    if neg {
        coeff = -coeff;
    }
    Ok(Dec {
        coeff: fit(vm, "dc", coeff)?,
        scale,
    })
}

impl Vm {
    pub(super) fn b_iz(&mut self) -> Result<(), RunError> {
        use super::value::WidthTag;
        self.push(Value::Width(WidthTag {
            signed: true,
            bits: 64,
            pointer: true,
        }));
        Ok(())
    }

    pub(super) fn b_uz(&mut self) -> Result<(), RunError> {
        use super::value::WidthTag;
        self.push(Value::Width(WidthTag {
            signed: false,
            bits: 64,
            pointer: true,
        }));
        Ok(())
    }

    pub(super) fn b_dc(&mut self) -> Result<(), RunError> {
        let v = self.pop("dc")?;
        match v {
            Value::Str(s) => {
                let d = parse_dec(self, &s)?;
                self.push(Value::Dec(d));
            }
            Value::Int(n) => self.push(Value::Dec(Dec { coeff: n, scale: 0 })),
            Value::Big(b) => match b.to_i64() {
                Some(n) => self.push(Value::Dec(Dec { coeff: n, scale: 0 })),
                None => {
                    return Err(self.err(
                        OVERFLOW,
                        format!("`dc` coefficient {b} does not fit int"),
                        Some("dc".into()),
                        "",
                    ))
                }
            },
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`dc` expected an int, a big, or a decimal string, got {tn}"),
                    Some("dc".into()),
                    "a float is not a decimal",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn b_cx(&mut self) -> Result<(), RunError> {
        let im = self.pop_cx_int()?;
        let re = self.pop_cx_int()?;
        self.push(Value::Cx { re, im });
        Ok(())
    }

    fn pop_cx_int(&mut self) -> Result<i64, RunError> {
        match self.pop("cx")? {
            Value::Int(n) => Ok(n),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`cx` expected an int, got {tn}"),
                    Some("cx".into()),
                    "cx is a Gaussian integer, not a float",
                ))
            }
        }
    }
}
