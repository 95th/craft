use crate::scanner::{Scanner, Token::*};

mod scanner;

fn main() {
    let source = r#"
        let x = 10;
        print(x);
        let y = "hello world";
        print(y);
    "#;
    let mut scanner = Scanner::new(source);
    loop {
        let token = scanner.scan_token();
        if token == Eof {
            break;
        }

        match token {
            Eof => break,
            Error => println!(
                "[Line: {}] Error at '{}' : {}",
                scanner.line(),
                scanner.token_text(),
                scanner.error_message()
            ),
            _ => println!(
                "[Line: {}] {:?} ('{}')",
                scanner.line(),
                token,
                scanner.token_text()
            ),
        }
    }
}
