use std::{cell::RefCell, mem, ops::DerefMut};

use combine::{
    char,
    char::char,
    choice, combine_parse_partial, combine_parser_impl, eof, parse_mode, parser,
    range::recognize,
    skip_many1,
    stream::{state::State, RangeStream},
    ParseError, Parser, StreamOnce,
};

use crate::toml_edit::{
    decor::{InternalString, Repr},
    document::Document,
    formatted::decorated,
    parser::{
        errors::CustomError,
        inline_table::KEYVAL_SEP,
        key::key,
        table::table,
        trivia::{comment, line_ending, line_trailing, newline, ws},
        value::value,
        TomlError, TomlParser,
    },
    table::{Item, TableKeyValue},
};

toml_parser!(parse_comment, parser, {
    (comment(), line_ending())
        .map(|(c, e)| parser.borrow_mut().deref_mut().on_comment(c, e))
});

toml_parser!(parse_ws, parser, {
    ws().map(|w| parser.borrow_mut().deref_mut().on_ws(w))
});

toml_parser!(parse_newline, parser, {
    recognize(newline()).map(|w| parser.borrow_mut().deref_mut().on_ws(w))
});

toml_parser!(keyval, parser, {
    parse_keyval().and_then(|(k, kv)| parser.borrow_mut().deref_mut().on_keyval(k, kv))
});

// keyval = key keyval-sep val
parser! {
    fn parse_keyval['a, I]()(I) -> (InternalString, TableKeyValue)
    where
        [I: RangeStream<
         Range = &'a str,
         Item = char>,
         I::Error: ParseError<char, &'a str, <I as StreamOnce>::Position>,
         <I::Error as ParseError<char, &'a str, <I as StreamOnce>::Position>>::StreamError:
         From<std::num::ParseIntError> +
         From<std::num::ParseFloatError> +
         From<chrono::ParseError> +
         From<CustomError>
    ] {
        (
            (key(), ws()),
            char(KEYVAL_SEP),
            (ws(), value(), line_trailing())
        ).map(|(k, _, v)| {
            let (pre, v, suf) = v;
            let v = decorated(v, pre, suf);
            let ((raw, key), suf) = k;
            (
                key,
                TableKeyValue {
                    key: Repr::new("", raw, suf),
                    value: Item::Value(v),
                }
            )
        })
    }
}

impl TomlParser {
    // ;; TOML

    // toml = expression *( newline expression )

    // expression = ( ( ws comment ) /
    //                ( ws keyval ws [ comment ] ) /
    //                ( ws table ws [ comment ] ) /
    //                  ws )
    pub fn parse(s: &str) -> Result<Document, TomlError> {
        let parser = RefCell::new(Self::default());
        let input = State::new(s);

        let parsed = parse_ws(&parser)
            .with(choice((
                eof(),
                skip_many1(
                    choice((
                        parse_comment(&parser),
                        keyval(&parser),
                        table(&parser),
                        parse_newline(&parser),
                    ))
                    .skip(parse_ws(&parser)),
                ),
            )))
            .easy_parse(input);
        match parsed {
            Ok((_, ref rest)) if !rest.input.is_empty() => {
                Err(TomlError::from_unparsed(rest.positioner, s))
            }
            Ok(..) => Ok(*parser.into_inner().document),
            Err(e) => Err(TomlError::new(e, s)),
        }
    }

    fn on_ws(&mut self, w: &str) { self.document.trailing.push_str(w); }

    fn on_comment(&mut self, c: &str, e: &str) {
        self.document.trailing.push_str(c);
        self.document.trailing.push_str(e);
    }

    fn on_keyval(
        &mut self,
        key: InternalString,
        mut kv: TableKeyValue,
    ) -> Result<(), CustomError> {
        let prefix = mem::take(&mut self.document.trailing);
        kv.key.decor.prefix = prefix + &kv.key.decor.prefix;

        let root = self.document.as_table_mut();
        let table = Self::descend_path(root, self.current_table_path.as_slice(), 0)
            .expect("the table path is valid; qed");
        if table.contains_key(&key) {
            Err(CustomError::DuplicateKey {
                key,
                table: "<unknown>".into(), // TODO: get actual table name
            })
        } else {
            let tkv = TableKeyValue { key: kv.key, value: kv.value };
            table.items.insert(key, tkv);
            Ok(())
        }
    }
}
