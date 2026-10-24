//! Local detectors for secrets and personal numbers.
//!
//! Detection looks only at text Snapper already recognised on this machine.
//! Nothing here is sent anywhere.

use std::collections::HashMap;

use regex::Regex;
use snapper_core::Rect;

use crate::OcrWord;

/// A kind of sensitive text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finding {
    Email,
    AwsKey,
    GithubToken,
    Jwt,
    PrivateKey,
    Card,
    Tfn,
    Medicare,
    Ip,
    HighEntropy,
}

/// Find sensitive tokens in recognised words.
///
/// Each hit uses the word's rectangle. A word that matches a specific
/// detector is not also reported as high entropy.
pub fn detect(words: &[OcrWord]) -> Vec<(Finding, Rect)> {
    let mut found = Vec::new();
    for word in words {
        let kinds = classify(&word.text);
        if kinds.is_empty() {
            if is_high_entropy(&word.text) {
                found.push((Finding::HighEntropy, word.rect));
            }
        } else {
            for kind in kinds {
                found.push((kind, word.rect));
            }
        }
    }
    found
}

fn classify(text: &str) -> Vec<Finding> {
    let mut found = Vec::new();
    if private_key(text) {
        found.push(Finding::PrivateKey);
    }
    if email(text) {
        found.push(Finding::Email);
    }
    if aws_key(text) {
        found.push(Finding::AwsKey);
    }
    if github_token(text) {
        found.push(Finding::GithubToken);
    }
    if jwt(text) {
        found.push(Finding::Jwt);
    }
    // A credential token can contain a digit run that also passes a checksum.
    // It is still that token, not a card, tax file number, or Medicare number.
    let credential = found.iter().any(|kind| {
        matches!(
            kind,
            Finding::PrivateKey | Finding::AwsKey | Finding::GithubToken | Finding::Jwt
        )
    });
    if credential {
        return found;
    }
    if card(text) {
        found.push(Finding::Card);
    }
    if tfn(text) {
        found.push(Finding::Tfn);
    }
    if medicare(text) {
        found.push(Finding::Medicare);
    }
    if ipv4(text) {
        found.push(Finding::Ip);
    }
    found
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("detector regex")
}

fn email(text: &str) -> bool {
    static CELL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    CELL.get_or_init(|| regex(r"(?i)\b[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,}\b"))
        .is_match(text)
}

fn aws_key(text: &str) -> bool {
    static CELL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    CELL.get_or_init(|| regex(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"))
        .is_match(text)
}

fn github_token(text: &str) -> bool {
    static CELL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        regex(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}\b|\bgithub_pat_[A-Za-z0-9_]{20,}\b")
    })
    .is_match(text)
}

fn jwt(text: &str) -> bool {
    static CELL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    CELL.get_or_init(|| regex(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b"))
        .is_match(text)
}

fn private_key(text: &str) -> bool {
    static CELL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    CELL.get_or_init(|| regex(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"))
        .is_match(text)
}

fn ipv4(text: &str) -> bool {
    static CELL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        regex(
            r"\b(?:(?:25[0-5]|2[0-4][0-9]|1?[0-9]{1,2})\.){3}(?:25[0-5]|2[0-4][0-9]|1?[0-9]{1,2})\b",
        )
    })
    .is_match(text)
}

/// Card numbers start with 2 to 6 (Mastercard, Amex, Visa, Discover). This
/// keeps millisecond timestamps and other long ids from passing on Luhn alone.
fn card(text: &str) -> bool {
    digit_groups(text).into_iter().any(|digits| {
        (13..=19).contains(&digits.len())
            && digits.starts_with(['2', '3', '4', '5', '6'])
            && luhn(&digits)
    })
}

fn tfn(text: &str) -> bool {
    digit_groups(text)
        .into_iter()
        .any(|digits| digits.len() == 9 && tfn_check(digits.as_bytes()))
}

fn medicare(text: &str) -> bool {
    digit_groups(text)
        .into_iter()
        .any(|digits| digits.len() == 10 && medicare_check(&digits))
}

/// Groups of digits, ignoring spaces and dashes that sit between digits.
///
/// A longer run is kept whole so a card number is not also read as a TFN.
fn digit_groups(text: &str) -> Vec<String> {
    let mut groups = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    let mut start = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch.is_ascii_digit() {
            if current.is_empty() {
                start = index;
            }
            current.push(ch);
            index += 1;
            continue;
        }
        if matches!(ch, ' ' | '-')
            && !current.is_empty()
            && chars
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_digit())
        {
            index += 1;
            continue;
        }
        if !current.is_empty() {
            push_group(&mut groups, &mut current, &chars, start, index);
        }
        index += 1;
    }
    if !current.is_empty() {
        push_group(&mut groups, &mut current, &chars, start, chars.len());
    }
    groups
}

/// Keep a digit run only when it is not glued to letters.
///
/// `ghp_1234567890abc` is one token. `4111-1111-1111-1111` is a card number.
fn push_group(
    groups: &mut Vec<String>,
    current: &mut String,
    chars: &[char],
    start: usize,
    end: usize,
) {
    let before_ok = start == 0 || !chars[start - 1].is_ascii_alphanumeric();
    let after_ok = end >= chars.len() || !chars[end].is_ascii_alphanumeric();
    if before_ok && after_ok {
        groups.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

fn luhn(number: &str) -> bool {
    let digits: Vec<u32> = number.chars().filter_map(|ch| ch.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let mut sum = 0u32;
    for (index, digit) in digits.iter().rev().enumerate() {
        let mut value = *digit;
        if index % 2 == 1 {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
    }
    // `is_multiple_of` arrived after the 1.81 rust-version in Cargo.toml.
    #[allow(clippy::manual_is_multiple_of)]
    {
        sum % 10 == 0
    }
}

/// Australian tax file number. Weights `[1, 4, 3, 7, 5, 8, 6, 9, 10]`, mod 11.
fn tfn_check(digits: &[u8]) -> bool {
    const WEIGHTS: [u32; 9] = [1, 4, 3, 7, 5, 8, 6, 9, 10];
    if digits.len() != 9 || digits.iter().all(|digit| *digit == b'0') {
        return false;
    }
    let mut sum = 0u32;
    for (digit, weight) in digits.iter().zip(WEIGHTS) {
        let Some(value) = (*digit as char).to_digit(10) else {
            return false;
        };
        sum += value * weight;
    }
    #[allow(clippy::manual_is_multiple_of)]
    {
        sum % 11 == 0
    }
}

/// Medicare card number. The first eight digits use weights
/// `[1, 3, 7, 9, 1, 3, 7, 9]`. The ninth digit is the check digit (mod 10).
/// The tenth digit is the individual reference number. The first digit is 2
/// to 6, which also rules out Australian mobile numbers (`04...`).
fn medicare_check(digits: &str) -> bool {
    const WEIGHTS: [u32; 8] = [1, 3, 7, 9, 1, 3, 7, 9];
    let values: Vec<u32> = digits.chars().filter_map(|ch| ch.to_digit(10)).collect();
    if values.len() != 10 || !(2..=6).contains(&values[0]) || values[9] == 0 {
        return false;
    }
    let sum: u32 = values
        .iter()
        .take(8)
        .zip(WEIGHTS)
        .map(|(digit, weight)| digit * weight)
        .sum();
    sum % 10 == values[8]
}

/// Shannon entropy in bits per character.
///
/// The threshold is 4.0 bits per character on a token of 20 characters or more,
/// after spaces are removed.
pub fn shannon_entropy(text: &str) -> f32 {
    let compact: Vec<char> = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    let len = compact.len();
    if len == 0 {
        return 0.0;
    }
    let mut counts: HashMap<char, usize> = HashMap::new();
    for ch in &compact {
        *counts.entry(*ch).or_insert(0) += 1;
    }
    let total = len as f32;
    let mut entropy = 0.0f32;
    for count in counts.values() {
        let probability = *count as f32 / total;
        entropy -= probability * probability.log2();
    }
    entropy
}

/// OCR engines return whole lines, so each space-separated token is checked on
/// its own. A token needs letters and digits, and a URL does not count.
fn is_high_entropy(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        token.chars().count() >= 20
            && token.chars().any(|ch| ch.is_ascii_digit())
            && token.chars().any(|ch| ch.is_ascii_alphabetic())
            && !token.contains("://")
            && shannon_entropy(token) >= 4.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OcrWord;

    fn word(text: &str) -> OcrWord {
        OcrWord {
            text: text.to_owned(),
            rect: Rect {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
            },
            confidence: 0.9,
        }
    }

    fn kinds(text: &str) -> Vec<Finding> {
        detect(&[word(text)])
            .into_iter()
            .map(|(kind, _)| kind)
            .collect()
    }

    #[test]
    fn email_positive_and_negative() {
        assert_eq!(kinds("Ada@example.com"), vec![Finding::Email]);
        assert_eq!(
            kinds("reach me at Ada@example.com today"),
            vec![Finding::Email]
        );
        assert!(kinds("ada@localhost").is_empty());
        assert!(kinds("not-an-email").is_empty());
        assert!(kinds("a@b").is_empty());
        assert_eq!(
            kinds("administrator@example.com"),
            vec![Finding::Email],
            "a long address is an email, not a high-entropy string"
        );
    }

    #[test]
    fn aws_key_positive_and_negative() {
        assert_eq!(kinds("AKIAIOSFODNN7EXAMPLE"), vec![Finding::AwsKey]);
        assert_eq!(kinds("ASIAIOSFODNN7EXAMPLE"), vec![Finding::AwsKey]);
        assert!(kinds("AKIA123").is_empty());
        assert!(kinds("akiaiosfodnn7example").is_empty());
        assert!(kinds("not a key").is_empty());
    }

    #[test]
    fn github_token_positive_and_negative() {
        assert_eq!(
            kinds("ghp_1234567890abcdefghij"),
            vec![Finding::GithubToken]
        );
        assert_eq!(
            kinds("github_pat_11AAAAAAA0123456789_extra"),
            vec![Finding::GithubToken]
        );
        assert!(kinds("ghp_short").is_empty());
        assert!(kinds("github").is_empty());
    }

    #[test]
    fn jwt_positive_and_negative() {
        let token = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.signaturevalue1";
        assert_eq!(kinds(token), vec![Finding::Jwt]);
        assert!(kinds("a.b.c").is_empty());
        assert!(kinds("hello.world.today").is_empty());
        assert!(kinds("not a jwt").is_empty());
    }

    #[test]
    fn private_key_positive_and_negative() {
        assert_eq!(
            kinds("-----BEGIN PRIVATE KEY-----"),
            vec![Finding::PrivateKey]
        );
        assert_eq!(
            kinds("-----BEGIN RSA PRIVATE KEY-----"),
            vec![Finding::PrivateKey]
        );
        assert_eq!(
            kinds("-----BEGIN OPENSSH PRIVATE KEY-----"),
            vec![Finding::PrivateKey]
        );
        assert!(kinds("BEGIN something").is_empty());
        assert!(kinds("-----BEGIN PUBLIC KEY-----").is_empty());
    }

    #[test]
    fn card_luhn_positive_and_negative() {
        assert_eq!(kinds("4111111111111111"), vec![Finding::Card]);
        assert_eq!(kinds("4111-1111-1111-1111"), vec![Finding::Card]);
        assert!(kinds("4111111111111112").is_empty());
        assert!(kinds("123456789012").is_empty(), "12 digits is too short");
        assert!(kinds("not a card").is_empty());
    }

    #[test]
    fn tfn_checksum_positive_and_negative() {
        assert_eq!(kinds("123456782"), vec![Finding::Tfn]);
        assert_eq!(kinds("123 456 782"), vec![Finding::Tfn]);
        assert!(kinds("123456789").is_empty());
        assert!(kinds("12345678").is_empty());
        assert!(
            kinds("4111111111111111")
                .iter()
                .all(|kind| *kind != Finding::Tfn),
            "a card number is not also a TFN"
        );
    }

    #[test]
    fn medicare_checksum_positive_and_negative() {
        assert_eq!(kinds("2123456701"), vec![Finding::Medicare]);
        assert!(kinds("2123456711").is_empty());
        assert!(
            kinds("2123456700").is_empty(),
            "reference number 0 is not used"
        );
        assert!(kinds("212345670").is_empty());
    }

    #[test]
    fn ipv4_positive_and_negative() {
        assert_eq!(kinds("8.8.8.8"), vec![Finding::Ip]);
        assert_eq!(kinds("192.168.0.1"), vec![Finding::Ip]);
        assert_eq!(kinds("see 10.0.0.1 now"), vec![Finding::Ip]);
        assert!(kinds("999.1.1.1").is_empty());
        assert!(kinds("1.2.3").is_empty());
        assert!(kinds("10.0.0.256").is_empty());
        assert!(kinds("version 1.2").is_empty());
    }

    #[test]
    fn high_entropy_positive_and_negative() {
        assert_eq!(kinds("Aa0Bb1Cc2Dd3Ee4Ff5Gg"), vec![Finding::HighEntropy]);
        assert!(shannon_entropy("Aa0Bb1Cc2Dd3Ee4Ff5Gg") >= 4.0);
        assert!(kinds("aaaaaaaaaaaaaaaaaaaa").is_empty());
        assert!(shannon_entropy("aaaaaaaaaaaaaaaaaaaa") < 4.0);
        assert!(kinds("hello world this is text").is_empty());
        assert!(kinds("short").is_empty());
        let hits = detect(&[word("Aa0Bb1Cc2Dd3Ee4Ff5Gg")]);
        assert_eq!(hits[0].1.width, 3);
    }

    #[test]
    fn common_numbers_and_lines_are_not_flagged() {
        // Passes Luhn, but a millisecond timestamp is not a card.
        assert!(kinds("1761300000003").is_empty());
        // Passes the Medicare checksum, but an 04 number is a mobile.
        assert!(kinds("0412345611").is_empty());
        assert!(kinds("000000000").is_empty());
        assert!(kinds("Quarterly revenue grew while expenses held steady overall").is_empty());
        assert!(kinds("https://github.com/jusso-dev/snapper/releases/v1").is_empty());
        assert_eq!(
            kinds("token q8Zr2LmX0vB7nT4kPw9sD1fH"),
            vec![Finding::HighEntropy]
        );
    }

    #[test]
    fn a_plain_sentence_has_no_findings() {
        assert!(kinds("Snapper saved the screenshot to Pictures").is_empty());
    }
}
