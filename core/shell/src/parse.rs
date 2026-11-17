//! Divide a linha em palavras: espaço separa; aspas simples e duplas agrupam.

/// Máximo de palavras de uma linha, comando incluso.
pub(crate) const MAX_ARGS: usize = 16;

/// Palavras de uma linha, já sem as aspas.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Args<'a> {
    items: [&'a str; MAX_ARGS],
    len: usize,
}

impl<'a> Args<'a> {
    pub(crate) fn as_slice(&self) -> &[&'a str] {
        &self.items[..self.len]
    }
}

/// Divide `buf` em palavras, reescrevendo-o no lugar para tirar as aspas.
/// `None` quando passa de `MAX_ARGS` palavras. Aspa aberta vale até o fim da linha.
pub(crate) fn tokenize(buf: &mut [u8]) -> Option<Args<'_>> {
    let mut ranges = [(0usize, 0usize); MAX_ARGS];
    let mut count = 0;
    let mut write = 0;
    let mut start = 0;
    let mut in_word = false;
    let mut quote = 0u8;
    let mut read = 0;
    while read < buf.len() {
        let byte = buf[read];
        read += 1;
        if quote != 0 && byte != quote {
            buf[write] = byte;
            write += 1;
            continue;
        }
        let separator = quote == 0 && (byte == b' ' || byte == b'\t');
        if separator {
            if in_word {
                *ranges.get_mut(count)? = (start, write);
                count += 1;
                in_word = false;
            }
            continue;
        }
        if !in_word {
            in_word = true;
            start = write;
        }
        if quote == byte {
            quote = 0;
        } else if quote == 0 && (byte == b'\'' || byte == b'"') {
            quote = byte;
        } else {
            buf[write] = byte;
            write += 1;
        }
    }
    if in_word {
        *ranges.get_mut(count)? = (start, write);
        count += 1;
    }
    let buf: &[u8] = buf;
    let mut items = [""; MAX_ARGS];
    for (item, &(from, to)) in items.iter_mut().zip(&ranges[..count]) {
        *item = core::str::from_utf8(&buf[from..to]).unwrap_or("");
    }
    Some(Args { items, len: count })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        let mut buf = line.as_bytes().to_vec();
        let args = tokenize(&mut buf).expect("within the argument limit");
        args.as_slice()
            .iter()
            .map(|word| (*word).to_string())
            .collect()
    }

    #[test]
    fn splits_on_runs_of_spaces_and_tabs() {
        assert_eq!(words("  echo   a\tb  "), ["echo", "a", "b"]);
    }

    #[test]
    fn empty_and_blank_lines_have_no_words() {
        assert_eq!(words("").len(), 0);
        assert_eq!(words("   \t ").len(), 0);
    }

    #[test]
    fn double_quotes_group_words() {
        assert_eq!(words("echo \"a  b\" c"), ["echo", "a  b", "c"]);
    }

    #[test]
    fn single_quotes_group_words_and_hide_double_quotes() {
        assert_eq!(words("echo 'a \"b\" c'"), ["echo", "a \"b\" c"]);
    }

    #[test]
    fn adjacent_quoted_and_bare_text_form_one_word() {
        assert_eq!(words("echo a\"b c\"d"), ["echo", "ab cd"]);
    }

    #[test]
    fn empty_quotes_are_an_empty_word() {
        assert_eq!(words("echo \"\" x"), ["echo", "", "x"]);
    }

    #[test]
    fn unterminated_quote_runs_to_the_end_of_the_line() {
        assert_eq!(words("echo \"a b"), ["echo", "a b"]);
    }

    #[test]
    fn quotes_keep_multibyte_text_intact() {
        assert_eq!(words("echo 'açaí é bom'"), ["echo", "açaí é bom"]);
    }

    #[test]
    fn accepts_exactly_the_maximum_and_rejects_one_more() {
        let line = "a ".repeat(MAX_ARGS);
        assert_eq!(words(&line).len(), MAX_ARGS);
        let mut over = "a ".repeat(MAX_ARGS + 1).into_bytes();
        assert!(tokenize(&mut over).is_none());
    }

    #[test]
    fn as_slice_is_limited_to_the_parsed_words() {
        let mut buf = b"a b".to_vec();
        let args = tokenize(&mut buf).unwrap();
        assert_eq!(args.as_slice().len(), 2);
    }
}
