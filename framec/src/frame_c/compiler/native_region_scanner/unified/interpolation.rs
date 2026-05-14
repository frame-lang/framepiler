//! Interpolation-aware string scanners (RFC-0010).
//!
//! Each function scans a string-literal form for a specific
//! target syntax (Python f-strings, JS template literals,
//! Kotlin/Swift/Dart `$`-interpolation, Ruby `"#{ }"`, GDScript
//! `% (val)`) and returns the end position plus a list of
//! `InterpRegion`s where Frame constructs (`$.x`, `@@:return`,
//! etc.) may appear inside the interpolation slots. The main
//! scanner loop then walks those regions for Frame sigils while
//! skipping the surrounding string content.

use super::InterpRegion;

// =========================================================================
// Interpolation-aware string scanners (RFC-0010)
//
// Each function scans a string literal and returns the end position plus
// a list of interpolation regions where Frame constructs may appear.
// The main scanner loop scans these regions for $./@@ while skipping
// the surrounding string content.
// =========================================================================

/// Scan a Python f-string: `f"...{expr}..."` or `f'...{expr}...'`.
/// Handles `{{` escape (not interpolation) and nested braces in expressions.
pub fn scan_fstring_regions(
    bytes: &[u8],
    i: usize,
    end: usize,
) -> Option<(usize, Vec<InterpRegion>)> {
    // Must start with f" or f' (case-insensitive F also valid)
    if i + 1 >= end {
        return None;
    }
    let prefix = bytes[i];
    if prefix != b'f' && prefix != b'F' {
        return None;
    }
    let quote = bytes[i + 1];
    if quote != b'"' && quote != b'\'' {
        return None;
    }

    let mut regions = Vec::new();
    let mut j = i + 2; // after f"

    while j < end {
        let b = bytes[j];
        if b == b'\\' {
            j += 2;
            continue;
        }
        if b == quote {
            return Some((j + 1, regions));
        }
        if b == b'{' {
            // {{ is an escape, not interpolation
            if j + 1 < end && bytes[j + 1] == b'{' {
                j += 2;
                continue;
            }
            // Interpolation expression — track brace depth
            let interp_start = j + 1;
            let mut depth = 1i32;
            j += 1;
            while j < end && depth > 0 {
                if bytes[j] == b'{' {
                    depth += 1;
                } else if bytes[j] == b'}' {
                    depth -= 1;
                } else if bytes[j] == b'\\' {
                    j += 1; // skip escaped char
                }
                if depth > 0 {
                    j += 1;
                }
            }
            // j is at the closing }
            regions.push(InterpRegion {
                start: interp_start,
                end: j,
                quote,
            });
            j += 1; // skip }
            continue;
        }
        j += 1;
    }
    Some((end, regions)) // unterminated
}

/// Scan a JS/TS template literal: `` `...${expr}...` ``.
/// Handles `\` escapes and nested braces in expressions.
pub fn scan_template_literal_regions(
    bytes: &[u8],
    i: usize,
    end: usize,
) -> Option<(usize, Vec<InterpRegion>)> {
    if i >= end || bytes[i] != b'`' {
        return None;
    }

    let mut regions = Vec::new();
    let mut j = i + 1; // after `

    while j < end {
        let b = bytes[j];
        if b == b'\\' {
            j += 2;
            continue;
        }
        if b == b'`' {
            return Some((j + 1, regions));
        }
        if b == b'$' && j + 1 < end && bytes[j + 1] == b'{' {
            let interp_start = j + 2; // after ${
            let mut depth = 1i32;
            j += 2;
            while j < end && depth > 0 {
                if bytes[j] == b'{' {
                    depth += 1;
                } else if bytes[j] == b'}' {
                    depth -= 1;
                } else if bytes[j] == b'\\' {
                    j += 1;
                }
                if depth > 0 {
                    j += 1;
                }
            }
            regions.push(InterpRegion {
                start: interp_start,
                end: j,
                quote: b'`',
            });
            j += 1; // skip }
            continue;
        }
        j += 1;
    }
    Some((end, regions)) // unterminated
}

/// Scan a `$"...{expr}..."` or `"...${expr}..."` string (Kotlin, Dart, C#).
/// Handles both `${expr}` (Kotlin/Dart) and `{expr}` (C# with `$"` prefix).
/// The `prefix_char` distinguishes: `$` for C#, `\0` for Kotlin/Dart (any `"`).
pub fn scan_dollar_string_regions(
    bytes: &[u8],
    i: usize,
    end: usize,
    prefix_char: u8,
) -> Option<(usize, Vec<InterpRegion>)> {
    if i >= end {
        return None;
    }

    let start_pos = if prefix_char != 0 {
        // C# style: $"..."
        if bytes[i] != prefix_char || i + 1 >= end || bytes[i + 1] != b'"' {
            return None;
        }
        i + 2
    } else {
        // Kotlin/Dart style: "..." with ${ inside
        if bytes[i] != b'"' {
            return None;
        }
        i + 1
    };

    let mut regions = Vec::new();
    let mut j = start_pos;

    while j < end {
        let b = bytes[j];
        if b == b'\\' {
            j += 2;
            continue;
        }
        if b == b'"' {
            return Some((j + 1, regions));
        }
        if b == b'$' && j + 1 < end && bytes[j + 1] == b'{' {
            let interp_start = j + 2;
            let mut depth = 1i32;
            j += 2;
            while j < end && depth > 0 {
                if bytes[j] == b'{' {
                    depth += 1;
                } else if bytes[j] == b'}' {
                    depth -= 1;
                } else if bytes[j] == b'\\' {
                    j += 1;
                }
                if depth > 0 {
                    j += 1;
                }
            }
            regions.push(InterpRegion {
                start: interp_start,
                end: j,
                quote: b'"',
            });
            j += 1;
            continue;
        }
        // C# also allows {expr} without $
        if prefix_char != 0 && b == b'{' {
            if j + 1 < end && bytes[j + 1] == b'{' {
                j += 2; // {{ escape
                continue;
            }
            let interp_start = j + 1;
            let mut depth = 1i32;
            j += 1;
            while j < end && depth > 0 {
                if bytes[j] == b'{' {
                    depth += 1;
                } else if bytes[j] == b'}' {
                    depth -= 1;
                }
                if depth > 0 {
                    j += 1;
                }
            }
            regions.push(InterpRegion {
                start: interp_start,
                end: j,
                quote: b'"',
            });
            j += 1;
            continue;
        }
        j += 1;
    }
    Some((end, regions))
}

/// Scan a Ruby interpolated string: `"...#{expr}..."`.
pub fn scan_hash_string_regions(
    bytes: &[u8],
    i: usize,
    end: usize,
) -> Option<(usize, Vec<InterpRegion>)> {
    if i >= end || bytes[i] != b'"' {
        return None;
    }

    let mut regions = Vec::new();
    let mut j = i + 1;

    while j < end {
        let b = bytes[j];
        if b == b'\\' {
            j += 2;
            continue;
        }
        if b == b'"' {
            return Some((j + 1, regions));
        }
        if b == b'#' && j + 1 < end && bytes[j + 1] == b'{' {
            let interp_start = j + 2;
            let mut depth = 1i32;
            j += 2;
            while j < end && depth > 0 {
                if bytes[j] == b'{' {
                    depth += 1;
                } else if bytes[j] == b'}' {
                    depth -= 1;
                } else if bytes[j] == b'\\' {
                    j += 1;
                }
                if depth > 0 {
                    j += 1;
                }
            }
            regions.push(InterpRegion {
                start: interp_start,
                end: j,
                quote: b'"',
            });
            j += 1;
            continue;
        }
        j += 1;
    }
    Some((end, regions))
}

/// Scan a Swift interpolated string: `"...\(expr)..."`.
pub fn scan_paren_string_regions(
    bytes: &[u8],
    i: usize,
    end: usize,
) -> Option<(usize, Vec<InterpRegion>)> {
    if i >= end || bytes[i] != b'"' {
        return None;
    }

    let mut regions = Vec::new();
    let mut j = i + 1;

    while j < end {
        let b = bytes[j];
        if b == b'\\' {
            if j + 1 < end && bytes[j + 1] == b'(' {
                // \( starts interpolation
                let interp_start = j + 2;
                let mut depth = 1i32;
                j += 2;
                while j < end && depth > 0 {
                    if bytes[j] == b'(' {
                        depth += 1;
                    } else if bytes[j] == b')' {
                        depth -= 1;
                    } else if bytes[j] == b'\\' {
                        j += 1;
                    }
                    if depth > 0 {
                        j += 1;
                    }
                }
                regions.push(InterpRegion {
                    start: interp_start,
                    end: j,
                    quote: b'"',
                });
                j += 1;
                continue;
            }
            // Regular escape
            j += 2;
            continue;
        }
        if b == b'"' {
            return Some((j + 1, regions));
        }
        j += 1;
    }
    Some((end, regions))
}
