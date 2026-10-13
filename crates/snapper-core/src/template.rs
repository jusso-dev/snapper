//! Filename templates.
//!
//! [`render`] walks the template once from left to right. Unknown
//! placeholders are copied through unchanged. Only `{app}` and `{title}` are
//! passed through [`sanitise`]; other placeholders are already filename-safe.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

/// Values substituted into a filename template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateContext {
    /// Clock used for `{date}`, `{time}`, `{datetime}`, `{year}`, `{month}` and `{day}`.
    pub now: DateTime<Local>,
    /// Foreground application name. Missing or empty becomes `unknown`.
    pub app: Option<String>,
    /// Window title. Missing or empty becomes `unknown`.
    pub title: Option<String>,
    /// Image width in physical pixels.
    pub width: u32,
    /// Image height in physical pixels.
    pub height: u32,
    /// Caller-supplied sequence number for `{counter}`.
    pub counter: u32,
}

/// Expand `template` using `ctx`.
///
/// Placeholders:
/// - `{date}` `YYYY-MM-DD`
/// - `{time}` `HH-mm-ss`
/// - `{datetime}` `YYYY-MM-DD_HH-mm-ss`
/// - `{year}` `YYYY`, `{month}` `MM`, `{day}` `DD`
/// - `{app}`, `{title}` sanitised, or `unknown` when empty
/// - `{width}`, `{height}`, `{counter}` decimal, no padding
/// - `{random}` eight lowercase hex digits
///
/// A `{...}` whose name is not in that list is left as written, including the
/// braces. An unclosed `{` is copied to the end of the string.
pub fn render(template: &str, ctx: &TemplateContext) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + '{'.len_utf8()..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                push_placeholder(&mut out, key, ctx);
                rest = &after[end + '}'.len_utf8()..];
            }
            None => {
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

fn push_placeholder(out: &mut String, key: &str, ctx: &TemplateContext) {
    match key {
        "date" => push_time(out, ctx, "%Y-%m-%d"),
        "time" => push_time(out, ctx, "%H-%M-%S"),
        "datetime" => push_time(out, ctx, "%Y-%m-%d_%H-%M-%S"),
        "year" => push_time(out, ctx, "%Y"),
        "month" => push_time(out, ctx, "%m"),
        "day" => push_time(out, ctx, "%d"),
        "app" => out.push_str(&label(ctx.app.as_deref())),
        "title" => out.push_str(&label(ctx.title.as_deref())),
        "width" => out.push_str(&ctx.width.to_string()),
        "height" => out.push_str(&ctx.height.to_string()),
        "counter" => out.push_str(&ctx.counter.to_string()),
        "random" => out.push_str(&random_token()),
        _ => {
            out.push('{');
            out.push_str(key);
            out.push('}');
        }
    }
}

fn push_time(out: &mut String, ctx: &TemplateContext, pattern: &str) {
    out.push_str(&ctx.now.format(pattern).to_string());
}

fn label(value: Option<&str>) -> String {
    let cleaned = sanitise(value.unwrap_or(""));
    if cleaned.is_empty() {
        "unknown".to_owned()
    } else {
        cleaned
    }
}

fn random_token() -> String {
    let value: u32 = rand::random();
    format!("{value:08x}")
}

/// Make `name` safe as a single path component on Windows, macOS and Linux.
///
/// `< > : " / \\ | ? *` and control characters become `_`. Trailing dots and
/// spaces are removed. The result is capped at 200 characters. Reserved
/// Windows device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`,
/// `LPT1`–`LPT9`), compared case-insensitively and ignoring a trailing
/// extension, are prefixed with `_`.
pub fn sanitise(name: &str) -> String {
    let mut cleaned = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            cleaned.push('_');
        } else {
            cleaned.push(ch);
        }
    }
    let trimmed = cleaned.trim_end_matches(['.', ' ']);
    let mut capped: String = trimmed.chars().take(200).collect();
    // Capping can reintroduce a trailing dot or space if it lands on one.
    let retrimmed = capped.trim_end_matches(['.', ' ']);
    if retrimmed.len() != capped.len() {
        capped = retrimmed.to_owned();
    }
    if is_reserved_windows_name(&capped) {
        capped.insert(0, '_');
    }
    capped
}

fn is_reserved_windows_name(name: &str) -> bool {
    let stem = match name.split_once('.') {
        Some((stem, _)) => stem,
        None => name,
    };
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes = upper.as_bytes();
    bytes.len() == 4
        && (bytes.starts_with(b"COM") || bytes.starts_with(b"LPT"))
        && (b'1'..=b'9').contains(&bytes[3])
}

/// Return `dir/stem.ext`, or `dir/stem (n).ext` when that path already exists.
///
/// `ext` may include a leading dot. An empty extension produces a name with
/// no dot. This checks existence only; a file created between the check and
/// the caller's write can still collide.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let ext = ext.trim_start_matches('.');
    let plain = file_name(stem, ext, None);
    let plain_path = dir.join(&plain);
    if !plain_path.exists() {
        return plain_path;
    }
    for n in 1..=u32::MAX {
        let path = dir.join(file_name(stem, ext, Some(n)));
        if !path.exists() {
            return path;
        }
    }
    dir.join(file_name(stem, ext, Some(u32::MAX)))
}

fn file_name(stem: &str, ext: &str, suffix: Option<u32>) -> String {
    let stem = match suffix {
        Some(n) => format!("{stem} ({n})"),
        None => stem.to_owned(),
    };
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn fixed_now() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2024, 3, 5, 14, 6, 7)
            .single()
            .expect("fixed local time")
    }

    fn ctx() -> TemplateContext {
        TemplateContext {
            now: fixed_now(),
            app: Some("Firefox".to_owned()),
            title: Some("Hello".to_owned()),
            width: 1920,
            height: 1080,
            counter: 7,
        }
    }

    fn expect(template: &str, expected: &str) {
        assert_eq!(render(template, &ctx()), expected, "template {template}");
    }

    #[test]
    fn placeholders_render_with_a_fixed_clock() {
        let cases = [
            ("{date}", "2024-03-05"),
            ("{time}", "14-06-07"),
            ("{datetime}", "2024-03-05_14-06-07"),
            ("{year}", "2024"),
            ("{month}", "03"),
            ("{day}", "05"),
            ("{app}", "Firefox"),
            ("{title}", "Hello"),
            ("{width}", "1920"),
            ("{height}", "1080"),
            ("{counter}", "7"),
            ("Snapper_{date}_{time}", "Snapper_2024-03-05_14-06-07"),
            ("{year}{month}{day}", "20240305"),
            ("{width}x{height}", "1920x1080"),
            ("{nope}", "{nope}"),
            ("pre {date} post", "pre 2024-03-05 post"),
            ("{date", "{date"),
            ("", ""),
            ("no placeholders", "no placeholders"),
            ("{}", "{}"),
        ];
        assert!(cases.len() >= 15);
        for (template, expected) in cases {
            expect(template, expected);
        }
    }

    #[test]
    fn app_and_title_are_sanitised_and_empty_is_unknown() {
        let mut context = ctx();
        context.app = None;
        context.title = Some(String::new());
        assert_eq!(render("{app}-{title}", &context), "unknown-unknown");

        context.app = Some("a<b>:c".to_owned());
        context.title = Some("..".to_owned());
        assert_eq!(render("{app}_{title}", &context), "a_b__c_unknown");

        context.app = Some("CON".to_owned());
        context.title = Some("report: Q1".to_owned());
        assert_eq!(render("{app}_{title}", &context), "_CON_report_ Q1");

        context.title = Some("   ".to_owned());
        assert_eq!(render("{title}", &context), "unknown");
    }

    #[test]
    fn random_is_eight_hex_digits() {
        let rendered = render("shot_{random}_end", &ctx());
        let token = rendered
            .strip_prefix("shot_")
            .and_then(|rest| rest.strip_suffix("_end"))
            .expect("random sits between the literals");
        assert_eq!(token.len(), 8);
        assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert!(token.chars().all(|ch| !ch.is_ascii_uppercase()));
    }

    #[test]
    fn sanitise_replaces_forbidden_characters() {
        assert_eq!(sanitise("a<b>:c"), "a_b__c");
        assert_eq!(sanitise("a/b\\c"), "a_b_c");
        assert_eq!(sanitise("a\"b|c?d*e"), "a_b_c_d_e");
        assert_eq!(sanitise("a\nb\tc"), "a_b_c");
        assert_eq!(sanitise("café"), "café");
    }

    #[test]
    fn sanitise_trims_trailing_dots_and_spaces_and_caps_length() {
        assert_eq!(sanitise("shot. "), "shot");
        assert_eq!(sanitise("shot..."), "shot");
        assert_eq!(sanitise("  keep leading"), "  keep leading");
        let long = "é".repeat(201);
        let capped = sanitise(&long);
        assert_eq!(capped.chars().count(), 200);
        assert!(long.starts_with(&capped));
    }

    #[test]
    fn sanitise_prefixes_reserved_windows_names() {
        assert_eq!(sanitise("CON"), "_CON");
        assert_eq!(sanitise("con"), "_con");
        assert_eq!(sanitise("PRN"), "_PRN");
        assert_eq!(sanitise("AUX"), "_AUX");
        assert_eq!(sanitise("NUL"), "_NUL");
        assert_eq!(sanitise("COM1"), "_COM1");
        assert_eq!(sanitise("COM9"), "_COM9");
        assert_eq!(sanitise("LPT1"), "_LPT1");
        assert_eq!(sanitise("LPT9"), "_LPT9");
        assert_eq!(sanitise("COM1.txt"), "_COM1.txt");
        assert_eq!(sanitise("CON "), "_CON");
        assert_eq!(sanitise("COM0"), "COM0");
        assert_eq!(sanitise("COM10"), "COM10");
        assert_eq!(sanitise("CONSOLE"), "CONSOLE");
        assert_eq!(sanitise("icon"), "icon");
    }

    #[test]
    fn unique_path_skips_existing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = unique_path(dir.path(), "shot", "png");
        assert_eq!(first, dir.path().join("shot.png"));
        assert!(!first.exists());
        std::fs::write(&first, b"a").expect("create");

        let second = unique_path(dir.path(), "shot", ".png");
        assert_eq!(second, dir.path().join("shot (1).png"));
        assert!(!second.exists());
        std::fs::write(&second, b"b").expect("create");

        let third = unique_path(dir.path(), "shot", "png");
        assert_eq!(third, dir.path().join("shot (2).png"));
        assert!(!third.exists());

        let bare = unique_path(dir.path(), "notes", "");
        assert_eq!(bare.file_name().and_then(|n| n.to_str()), Some("notes"));
        assert!(!bare.exists());
    }
}
