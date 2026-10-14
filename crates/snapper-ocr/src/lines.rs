//! Group recognised words into lines of clipboard text.

use snapper_core::Rect;

use crate::OcrWord;

/// Join words into lines.
///
/// Words whose vertical ranges overlap stay on one line, ordered left to
/// right. Lines are separated with `\n` and ordered top to bottom.
pub fn join_lines(words: &[OcrWord]) -> String {
    if words.is_empty() {
        return String::new();
    }
    let mut order: Vec<usize> = (0..words.len()).collect();
    order.sort_by(|&left, &right| {
        words[left]
            .rect
            .y
            .cmp(&words[right].rect.y)
            .then(words[left].rect.x.cmp(&words[right].rect.x))
    });

    let mut lines: Vec<Vec<usize>> = Vec::new();
    for index in order {
        let rect = words[index].rect;
        if let Some(line) = lines.last_mut() {
            let anchor = words[line[0]].rect;
            if y_overlap(anchor, rect) {
                line.push(index);
                continue;
            }
        }
        lines.push(vec![index]);
    }

    lines
        .into_iter()
        .map(|mut line| {
            line.sort_by_key(|&index| words[index].rect.x);
            line.into_iter()
                .map(|index| words[index].text.trim())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn y_overlap(left: Rect, right: Rect) -> bool {
    let left_bottom = left.y.saturating_add(left.height as i32);
    let right_bottom = right.y.saturating_add(right.height as i32);
    left.y < right_bottom && right.y < left_bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, x: i32, y: i32) -> OcrWord {
        OcrWord {
            text: text.to_owned(),
            rect: Rect {
                x,
                y,
                width: 10,
                height: 12,
            },
            confidence: 1.0,
        }
    }

    #[test]
    fn words_on_one_line_join_left_to_right() {
        let words = vec![word("world", 40, 10), word("hello", 0, 12)];
        assert_eq!(join_lines(&words), "hello world");
    }

    #[test]
    fn separated_rows_become_lines() {
        let words = vec![word("second", 0, 40), word("first", 0, 0)];
        assert_eq!(join_lines(&words), "first\nsecond");
    }

    #[test]
    fn empty_input_is_empty() {
        assert_eq!(join_lines(&[]), "");
    }
}
