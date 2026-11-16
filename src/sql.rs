//! SQL lexing for highlighting and bracket matching. Pure logic, no I/O.
//!
//! The lexer never fails. Unterminated strings and comments run to the end of
//! the input, which is what an editor wants while the user is still typing.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Whitespace,
    Keyword,
    Identifier,
    QuotedIdentifier,
    String,
    DollarString,
    Number,
    Parameter,
    Comment,
    /// `( ) [ ] , ; .`
    Punct,
    Operator,
}

/// A token as a byte range of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

/// Sorted, upper case. A test checks the order, which `binary_search` needs.
const KEYWORDS: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "ANALYZE",
    "AND",
    "ANY",
    "ARRAY",
    "AS",
    "ASC",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASCADE",
    "CASE",
    "CAST",
    "CHECK",
    "COLLATE",
    "COLUMN",
    "COMMENT",
    "COMMIT",
    "CONSTRAINT",
    "COPY",
    "CREATE",
    "CROSS",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "CURRENT_USER",
    "DATABASE",
    "DECLARE",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DO",
    "DROP",
    "ELSE",
    "END",
    "ESCAPE",
    "EXCEPT",
    "EXISTS",
    "EXPLAIN",
    "EXTENSION",
    "FALSE",
    "FETCH",
    "FIRST",
    "FOR",
    "FOREIGN",
    "FROM",
    "FULL",
    "FUNCTION",
    "GRANT",
    "GROUP",
    "HAVING",
    "IF",
    "ILIKE",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTERSECT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LANGUAGE",
    "LAST",
    "LATERAL",
    "LEFT",
    "LIKE",
    "LIMIT",
    "LOOP",
    "MATERIALIZED",
    "NATURAL",
    "NOT",
    "NULL",
    "NULLS",
    "OFFSET",
    "ON",
    "ONLY",
    "OR",
    "ORDER",
    "OUTER",
    "OVER",
    "PARTITION",
    "PERFORM",
    "PRIMARY",
    "PROCEDURE",
    "RAISE",
    "RANGE",
    "RECURSIVE",
    "REFERENCES",
    "REFRESH",
    "REINDEX",
    "RELEASE",
    "RENAME",
    "RESTRICT",
    "RETURN",
    "RETURNING",
    "RETURNS",
    "REVOKE",
    "RIGHT",
    "ROLLBACK",
    "ROWS",
    "SAVEPOINT",
    "SCHEMA",
    "SELECT",
    "SEQUENCE",
    "SESSION",
    "SET",
    "SIMILAR",
    "SOME",
    "TABLE",
    "TEMP",
    "TEMPORARY",
    "THEN",
    "TO",
    "TRANSACTION",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "TYPE",
    "UNION",
    "UNIQUE",
    "UNLOGGED",
    "UPDATE",
    "USER",
    "USING",
    "VACUUM",
    "VALUES",
    "VERBOSE",
    "VIEW",
    "WHEN",
    "WHERE",
    "WHILE",
    "WINDOW",
    "WITH",
];

/// All keywords, upper case and sorted.
pub fn keywords() -> &'static [&'static str] {
    KEYWORDS
}

fn is_keyword(word: &str) -> bool {
    if word.len() > 20 {
        return false;
    }
    let upper = word.to_ascii_uppercase();
    KEYWORDS.binary_search(&upper.as_str()).is_ok()
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn is_ident_part(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

pub fn lex(src: &str) -> Vec<Token> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let b = bytes[i];
        let kind = match b {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0c => {
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | 0x0c) {
                    i += 1;
                }
                TokenKind::Whitespace
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                TokenKind::Comment
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = block_comment_end(bytes, i);
                TokenKind::Comment
            }
            b'\'' => {
                i = quoted_end(bytes, i, b'\'', false);
                TokenKind::String
            }
            b'"' => {
                i = quoted_end(bytes, i, b'"', false);
                TokenKind::QuotedIdentifier
            }
            b'$' => match dollar(src, i) {
                Some((end, kind)) => {
                    i = end;
                    kind
                }
                None => {
                    i += 1;
                    TokenKind::Operator
                }
            },
            b'0'..=b'9' => {
                i = number_end(bytes, i);
                TokenKind::Number
            }
            b'.' if bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                i = number_end(bytes, i + 1);
                TokenKind::Number
            }
            b'(' | b')' | b'[' | b']' | b',' | b';' | b'.' => {
                i += 1;
                TokenKind::Punct
            }
            _ if is_ident_start(b) => {
                // Prefixed strings: E'..' (backslash escapes), B'..', X'..', N'..'.
                if matches!(b, b'e' | b'E' | b'b' | b'B' | b'x' | b'X' | b'n' | b'N')
                    && bytes.get(i + 1) == Some(&b'\'')
                {
                    i = quoted_end(bytes, i + 1, b'\'', matches!(b, b'e' | b'E'));
                    TokenKind::String
                } else {
                    while i < bytes.len() && is_ident_part(bytes[i]) {
                        i += 1;
                    }
                    if is_keyword(&src[start..i]) {
                        TokenKind::Keyword
                    } else {
                        TokenKind::Identifier
                    }
                }
            }
            _ => {
                i += 1;
                TokenKind::Operator
            }
        };
        tokens.push(Token {
            kind,
            start,
            end: i,
        });
    }
    tokens
}

/// Index just past a possibly nested `/* ... */` that starts at `i`.
fn block_comment_end(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// Index just past a quoted run that opens at `i`. A doubled quote is an
/// escaped quote. With `backslash`, `\x` is an escape too.
fn quoted_end(bytes: &[u8], mut i: usize, quote: u8, backslash: bool) -> usize {
    i += 1;
    while i < bytes.len() {
        if backslash && bytes[i] == b'\\' {
            i += 2;
        } else if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
            } else {
                return i + 1;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

fn number_end(bytes: &[u8], mut i: usize) -> usize {
    let digits = |mut i: usize| {
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
        i
    };
    i = digits(i);
    if bytes.get(i) == Some(&b'.') && bytes.get(i + 1) != Some(&b'.') {
        i = digits(i + 1);
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            i = digits(j);
        }
    }
    i
}

/// A `$1` parameter or a `$tag$ ... $tag$` string that starts at `i`.
fn dollar(src: &str, i: usize) -> Option<(usize, TokenKind)> {
    let bytes = src.as_bytes();
    let next = *bytes.get(i + 1)?;
    if next.is_ascii_digit() {
        let mut j = i + 1;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        return Some((j, TokenKind::Parameter));
    }
    let mut j = i + 1;
    while j < bytes.len()
        && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] >= 0x80)
    {
        j += 1;
    }
    if bytes.get(j) != Some(&b'$') {
        return None;
    }
    let tag = &src[i..=j];
    let body = j + 1;
    let end = match src[body..].find(tag) {
        Some(at) => body + at + tag.len(),
        None => src.len(),
    };
    Some((end, TokenKind::DollarString))
}

/// One statement in a script, as byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Statement {
    /// First character, which may be a comment that belongs to the statement.
    pub start: usize,
    /// End of the last token, before any terminating `;`.
    pub end: usize,
    /// End including the terminating `;`, or `end` when there is none.
    pub terminated_end: usize,
}

impl Statement {
    pub fn text(self, src: &str) -> &str {
        &src[self.start..self.end]
    }
}

/// Splits a script into statements (R7.1, R7.2). A `;` ends a statement unless
/// it sits in a string, quoted name, comment or dollar-quoted body, or in the
/// body of a `CREATE FUNCTION ... BEGIN ATOMIC ... END`. Segments that hold
/// only comments are not statements.
pub fn statements(src: &str) -> Vec<Statement> {
    let mut found = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    let mut has_code = false;
    let mut is_create = false;
    let mut atomic_depth = 0usize;
    let mut previous_word = String::new();

    for token in lex(src) {
        match token.kind {
            TokenKind::Whitespace => continue,
            TokenKind::Punct if &src[token.start..token.end] == ";" && atomic_depth == 0 => {
                if let (Some(first), true) = (start, has_code) {
                    found.push(Statement {
                        start: first,
                        end,
                        terminated_end: token.end,
                    });
                }
                (start, has_code, is_create, previous_word) = (None, false, false, String::new());
                continue;
            }
            _ => {}
        }
        if start.is_none() {
            start = Some(token.start);
        }
        end = token.end;
        if token.kind == TokenKind::Comment {
            continue;
        }
        if !has_code {
            is_create = src[token.start..token.end].eq_ignore_ascii_case("create");
        }
        has_code = true;
        if matches!(token.kind, TokenKind::Keyword | TokenKind::Identifier) {
            let word = src[token.start..token.end].to_ascii_lowercase();
            if is_create && atomic_depth == 0 && previous_word == "begin" && word == "atomic" {
                atomic_depth = 1;
            } else if atomic_depth > 0 {
                match word.as_str() {
                    "case" => atomic_depth += 1,
                    "end" => atomic_depth -= 1,
                    _ => {}
                }
            }
            previous_word = word;
        } else {
            previous_word.clear();
        }
    }
    if let (Some(first), true) = (start, has_code) {
        found.push(Statement {
            start: first,
            end,
            terminated_end: end,
        });
    }
    found
}

/// The statement the cursor is in (R7.3). Between statements it is the one
/// before, or the one after when there is none before (R7.4).
pub fn statement_at(src: &str, offset: usize) -> Option<Statement> {
    let all = statements(src);
    // Last one wins, so a cursor right after `;` and before the next word
    // belongs to the next statement.
    if let Some(inside) = all
        .iter()
        .rev()
        .find(|s| s.start <= offset && offset <= s.terminated_end)
    {
        return Some(*inside);
    }
    all.iter()
        .rev()
        .find(|s| s.terminated_end < offset)
        .or_else(|| all.iter().find(|s| s.start > offset))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind::*;

    /// Non-whitespace tokens as (kind, text).
    fn lexed(src: &str) -> Vec<(TokenKind, &str)> {
        lex(src)
            .into_iter()
            .filter(|t| t.kind != Whitespace)
            .map(|t| (t.kind, &src[t.start..t.end]))
            .collect()
    }

    #[test]
    fn keywords_are_sorted_and_upper_case() {
        let mut sorted = KEYWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, KEYWORDS);
        assert!(KEYWORDS.iter().all(|k| *k == k.to_ascii_uppercase()));
    }

    #[test]
    fn tokens_cover_the_whole_input_without_gaps() {
        let src = "select 'a''b', \"q\"\"x\", $1, 1.5e3 -- c\n/* a /* b */ c */ $f$ x $f$ ü";
        let mut at = 0;
        for t in lex(src) {
            assert_eq!(t.start, at);
            assert!(t.end > t.start);
            at = t.end;
        }
        assert_eq!(at, src.len());
    }

    #[test]
    fn basic_select() {
        assert_eq!(
            lexed("SELECT a, b2 FROM t WHERE x >= 10;"),
            vec![
                (Keyword, "SELECT"),
                (Identifier, "a"),
                (Punct, ","),
                (Identifier, "b2"),
                (Keyword, "FROM"),
                (Identifier, "t"),
                (Keyword, "WHERE"),
                (Identifier, "x"),
                (Operator, ">"),
                (Operator, "="),
                (Number, "10"),
                (Punct, ";"),
            ]
        );
    }

    #[test]
    fn keywords_ignore_case() {
        assert_eq!(lexed("select")[0].0, Keyword);
        assert_eq!(lexed("SeLeCt")[0].0, Keyword);
        assert_eq!(lexed("selected")[0].0, Identifier);
    }

    #[test]
    fn strings_with_doubled_quotes() {
        assert_eq!(lexed("'it''s'"), vec![(String, "'it''s'")]);
        assert_eq!(lexed("'open"), vec![(String, "'open")]);
    }

    #[test]
    fn escape_strings_honor_backslash() {
        assert_eq!(
            lexed(r"E'a\'b' x"),
            vec![(String, r"E'a\'b'"), (Identifier, "x")]
        );
        // In a standard string a backslash is an ordinary character.
        assert_eq!(lexed(r"'a\' x"), vec![(String, r"'a\'"), (Identifier, "x")]);
    }

    #[test]
    fn quoted_identifiers() {
        assert_eq!(
            lexed("\"my \"\"col\"\"\""),
            vec![(QuotedIdentifier, "\"my \"\"col\"\"\"")]
        );
    }

    #[test]
    fn comments() {
        assert_eq!(
            lexed("a -- x\nb"),
            vec![(Identifier, "a"), (Comment, "-- x"), (Identifier, "b")]
        );
        assert_eq!(
            lexed("/* a /* b */ c */ d"),
            vec![(Comment, "/* a /* b */ c */"), (Identifier, "d")]
        );
        assert_eq!(lexed("/* open"), vec![(Comment, "/* open")]);
    }

    #[test]
    fn semicolons_inside_strings_and_comments_stay_inside() {
        for src in ["'a;b'", "-- a;b", "/* a;b */", "$$a;b$$", "\"a;b\""] {
            let all = lexed(src);
            assert_eq!(all.len(), 1, "{src}: {all:?}");
        }
    }

    #[test]
    fn dollar_quoted_bodies() {
        assert_eq!(
            lexed("$$ a ; b $$ x"),
            vec![(DollarString, "$$ a ; b $$"), (Identifier, "x")]
        );
        assert_eq!(
            lexed("$body$ $$ inner $$ $body$"),
            vec![(DollarString, "$body$ $$ inner $$ $body$")]
        );
        assert_eq!(lexed("$f$ open"), vec![(DollarString, "$f$ open")]);
    }

    #[test]
    fn parameters_are_not_dollar_quotes() {
        assert_eq!(
            lexed("a = $1 and b = $22"),
            vec![
                (Identifier, "a"),
                (Operator, "="),
                (Parameter, "$1"),
                (Keyword, "and"),
                (Identifier, "b"),
                (Operator, "="),
                (Parameter, "$22"),
            ]
        );
    }

    #[test]
    fn dollar_inside_identifier_stays_in_identifier() {
        assert_eq!(lexed("a$b"), vec![(Identifier, "a$b")]);
        assert_eq!(lexed("$"), vec![(Operator, "$")]);
    }

    #[test]
    fn numbers() {
        for n in ["1", "1.5", ".5", "1e10", "1.5E-3", "1_000"] {
            assert_eq!(lexed(n), vec![(Number, n)], "{n}");
        }
        assert_eq!(
            lexed("t.c"),
            vec![(Identifier, "t"), (Punct, "."), (Identifier, "c")]
        );
    }

    #[test]
    fn non_ascii_identifiers_do_not_split_characters() {
        assert_eq!(
            lexed("select naïve_ñ from t"),
            vec![
                (Keyword, "select"),
                (Identifier, "naïve_ñ"),
                (Keyword, "from"),
                (Identifier, "t"),
            ]
        );
        // Must not panic on any prefix of multi-byte text.
        let src = "'日本語' -- 💡\n\"ü\"";
        for n in 0..=src.len() {
            if src.is_char_boundary(n) {
                lex(&src[..n]);
            }
        }
    }

    #[test]
    fn prefixed_strings() {
        assert_eq!(
            lexed("x'1F' b'01'"),
            vec![(String, "x'1F'"), (String, "b'01'")]
        );
        assert_eq!(
            lexed("e = 1"),
            vec![(Identifier, "e"), (Operator, "="), (Number, "1")]
        );
    }

    // statements

    fn texts(src: &str) -> Vec<&str> {
        statements(src).into_iter().map(|s| s.text(src)).collect()
    }

    #[test]
    fn splits_on_semicolons_and_trims() {
        assert_eq!(
            texts("select 1; select 2;\n\nselect 3"),
            ["select 1", "select 2", "select 3"]
        );
        assert_eq!(texts("  select 1  ;  "), ["select 1"]);
        assert!(texts("").is_empty() && texts(" \n ; ;").is_empty());
    }

    #[test]
    fn semicolons_inside_strings_comments_and_dollar_bodies_do_not_split() {
        let src = "select 'a;b'; select \"x;y\"; -- c;d\nselect 1; /* ; */ select $$ ; $$; select $f$ a; $f$";
        assert_eq!(
            texts(src),
            [
                "select 'a;b'",
                "select \"x;y\"",
                "-- c;d\nselect 1",
                "/* ; */ select $$ ; $$",
                "select $f$ a; $f$"
            ]
        );
    }

    #[test]
    fn a_function_with_a_dollar_quoted_body_is_one_statement() {
        let src = "create function f() returns int language plpgsql as $$ begin return 1; end; $$; select f();";
        let all = texts(src);
        assert_eq!(all.len(), 2);
        assert!(all[0].starts_with("create function") && all[0].ends_with("$$"));
        assert_eq!(all[1], "select f()");
    }

    #[test]
    fn a_begin_atomic_body_is_one_statement() {
        let src = "create function f(a int) returns int language sql begin atomic select case when a > 0 then 1 else 2 end; end; select 2;";
        let all = texts(src);
        assert_eq!(all.len(), 2, "{all:?}");
        assert!(all[0].ends_with("end"), "{}", all[0]);
        assert_eq!(all[1], "select 2");
    }

    #[test]
    fn plain_begin_and_end_are_ordinary_statements() {
        assert_eq!(
            texts("begin; select 1; commit;"),
            ["begin", "select 1", "commit"]
        );
        assert_eq!(
            texts("begin atomic; select 1;"),
            ["begin atomic", "select 1"],
            "only inside CREATE"
        );
    }

    #[test]
    fn comment_only_segments_are_not_statements() {
        assert!(texts("-- nothing\n/* here */").is_empty());
        assert_eq!(texts("-- note\nselect 1;"), ["-- note\nselect 1"]);
        assert_eq!(texts("-- only\n;select 1"), ["select 1"]);
    }

    #[test]
    fn terminated_end_includes_the_semicolon() {
        let all = statements("select 1; select 2");
        assert_eq!((all[0].end, all[0].terminated_end), (8, 9));
        assert_eq!((all[1].end, all[1].terminated_end), (18, 18));
    }

    fn at(src: &str, offset: usize) -> Option<&str> {
        statement_at(src, offset).map(|s| s.text(src))
    }

    #[test]
    fn the_statement_under_the_cursor() {
        let src = "select 1;\nselect 2;\nselect 3";
        assert_eq!(at(src, 0), Some("select 1"));
        assert_eq!(at(src, 4), Some("select 1"));
        assert_eq!(at(src, 9), Some("select 1"), "right after the semicolon");
        assert_eq!(at(src, 12), Some("select 2"));
        assert_eq!(at(src, src.len()), Some("select 3"));
    }

    #[test]
    fn between_statements_the_previous_one_wins() {
        let src = "select 1;\n\n\nselect 2;";
        assert_eq!(at(src, 10), Some("select 1"));
        assert_eq!(at(src, 11), Some("select 1"));
        assert_eq!(at(src, 12), Some("select 2"));
    }

    #[test]
    fn before_the_first_statement_the_next_one_wins() {
        let src = "\n\n  select 1;";
        assert_eq!(at(src, 0), Some("select 1"));
        assert_eq!(at(src, 1), Some("select 1"));
    }

    #[test]
    fn touching_statements_prefer_the_next() {
        assert_eq!(at("select 1;select 2", 9), Some("select 2"));
    }

    #[test]
    fn no_statements_means_none() {
        assert_eq!(at("", 0), None);
        assert_eq!(at("-- hi", 2), None);
    }

    #[test]
    fn a_multi_line_statement_is_found_from_any_line() {
        let src = "select a,\n  b\nfrom t\nwhere x = 1;\nselect 2";
        for offset in [0, 10, 16, 25] {
            assert_eq!(
                at(src, offset),
                Some("select a,\n  b\nfrom t\nwhere x = 1"),
                "{offset}"
            );
        }
    }
}
