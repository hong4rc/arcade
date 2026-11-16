//! The small templates of the configuration file: a text with `{field}` places that the daemon
//! fills in, for the text of a tab and for the title of the host terminal's window.
//!
//! One rule says what an empty field does: **a field that is empty is left out together with
//! the literal text just before it** (the text that joins it to what comes before), and the text
//! before the first field shown is left out too, unless it is the text at the very start of the
//! template. The text after the last field stays only while that last field is shown. So
//! `{session}:{number} {name} · {title}` with no title is `work:1 sh`, not `work:1 sh · `.
//! Write `{{` and `}}` for a brace in the text.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// The window's name: the one given by hand, else the automatic one.
    Name,
    /// The title the program in the window's focused pane set.
    Title,
    /// The name of the program in the foreground of the window's focused pane.
    Program,
    /// The last part of the directory of the window's focused pane (`~` for the home directory).
    Directory,
    /// The session's name.
    Session,
    /// The window's number.
    Number,
}

impl Field {
    fn parse(word: &str) -> Option<Field> {
        Some(match word {
            "name" => Field::Name,
            "title" => Field::Title,
            "program" => Field::Program,
            "directory" => Field::Directory,
            "session" => Field::Session,
            "number" => Field::Number,
            _ => return None,
        })
    }
}

/// A template: the text before each field, the fields, and the text after the last one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    parts: Vec<(String, Field)>,
    tail: String,
}

/// A piece of what a template made: the text, and the field it came from (`None` for text of
/// the template itself).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub field: Option<Field>,
}

impl Template {
    /// Read a template that may use the fields in `allowed`. The error says what is wrong.
    pub fn parse(source: &str, allowed: &[Field]) -> Result<Template, String> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = source.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '}' => return Err("a lone } (write }} for a brace)".to_string()),
                '{' => {
                    let mut word = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(c) => word.push(c),
                            None => return Err("a { that is not closed".to_string()),
                        }
                    }
                    let field = Field::parse(&word)
                        .filter(|f| allowed.contains(f))
                        .ok_or_else(|| format!("no field {{{word}}} here"))?;
                    parts.push((std::mem::take(&mut literal), field));
                }
                c => literal.push(c),
            }
        }
        Ok(Template {
            parts,
            tail: literal,
        })
    }

    /// Whether the template uses the field.
    pub fn uses(&self, field: Field) -> bool {
        self.parts.iter().any(|(_, f)| *f == field)
    }

    /// Fill the template in. `value` gives the text of a field; empty means there is none.
    pub fn render(&self, value: impl Fn(Field) -> String) -> Vec<Piece> {
        let mut out = Vec::new();
        let mut shown_any = false;
        let mut last_shown = false;
        for (at, (literal, field)) in self.parts.iter().enumerate() {
            let text = value(*field);
            last_shown = !text.is_empty();
            if text.is_empty() {
                continue;
            }
            if (at == 0 || shown_any) && !literal.is_empty() {
                out.push(Piece {
                    text: literal.clone(),
                    field: None,
                });
            }
            out.push(Piece {
                text,
                field: Some(*field),
            });
            shown_any = true;
        }
        if self.parts.is_empty() {
            // Only text.
            if !self.tail.is_empty() {
                out.push(Piece {
                    text: self.tail.clone(),
                    field: None,
                });
            }
        } else if last_shown && !self.tail.is_empty() {
            out.push(Piece {
                text: self.tail.clone(),
                field: None,
            });
        }
        out
    }

    /// The text a template makes, as one string.
    pub fn render_text(&self, value: impl Fn(Field) -> String) -> String {
        self.render(value).into_iter().map(|p| p.text).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[Field] = &[
        Field::Name,
        Field::Title,
        Field::Program,
        Field::Directory,
        Field::Session,
        Field::Number,
    ];

    fn make(source: &str, empty: &[Field]) -> String {
        Template::parse(source, ALL).unwrap().render_text(|f| {
            if empty.contains(&f) {
                return String::new();
            }
            match f {
                Field::Name => "sh",
                Field::Title => "T",
                Field::Program => "zsh",
                Field::Directory => "dir",
                Field::Session => "work",
                Field::Number => "1",
            }
            .to_string()
        })
    }

    #[test]
    fn fields_are_filled_in_any_order() {
        assert_eq!(
            make("{session}:{number} {name} · {title}", &[]),
            "work:1 sh · T"
        );
        assert_eq!(make("{title} < {name} [{directory}]", &[]), "T < sh [dir]");
        assert_eq!(make("{name}", &[]), "sh");
        assert_eq!(make("just text", &[]), "just text");
        assert_eq!(make("{{{name}}}", &[]), "{sh}");
    }

    #[test]
    fn an_empty_field_leaves_out_the_text_that_joins_it() {
        let f = "{session}:{number} {name} · {title}";
        assert_eq!(make(f, &[Field::Title]), "work:1 sh");
        assert_eq!(make(f, &[Field::Name]), "work:1 · T");
        assert_eq!(make(f, &[Field::Name, Field::Title]), "work:1");
        // The text before the first field shown goes with it unless it starts the template.
        assert_eq!(make("{title} · {name}", &[Field::Title]), "sh");
        assert_eq!(make("[{title}] {name}", &[Field::Title]), "sh");
        // The text before a field belongs to it: here the closing bracket goes with the title.
        assert_eq!(make("[{name}] {title}", &[Field::Title]), "[sh");
        assert_eq!(make("[{name}]", &[Field::Title]), "[sh]");
        // A closing text stays only with a shown last field.
        assert_eq!(make("({title})", &[Field::Title]), "");
        assert_eq!(make("({title})", &[]), "(T)");
        assert_eq!(make("{name} ({title})", &[Field::Title]), "sh");
        assert_eq!(make("{name} ({title})", &[]), "sh (T)");
    }

    #[test]
    fn what_cannot_be_read_is_an_error_that_says_what() {
        assert!(
            Template::parse("{nope}", ALL)
                .unwrap_err()
                .contains("{nope}")
        );
        assert!(Template::parse("{name", ALL).is_err());
        assert!(Template::parse("name}", ALL).is_err());
        // A field that is not allowed in this place is as unknown as any other.
        assert!(Template::parse("{session}", &[Field::Name]).is_err());
    }

    #[test]
    fn pieces_say_where_each_text_came_from() {
        let t = Template::parse("{number}-{name}", ALL).unwrap();
        let pieces = t.render(|f| {
            if f == Field::Number {
                "2".into()
            } else {
                "x".into()
            }
        });
        assert_eq!(
            pieces.iter().map(|p| p.field).collect::<Vec<_>>(),
            vec![Some(Field::Number), None, Some(Field::Name)]
        );
        assert!(t.uses(Field::Name) && !t.uses(Field::Title));
    }
}
