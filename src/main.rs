use crate::{ast::NodeArena, parser::Parser};

mod ast;
mod parser;
mod scanner;

fn main() {
    let source = r#"
        1 + 2 / 3 - 4 * 5 * (1 - 2);
    "#;

    let mut nodes = NodeArena::new();
    let mut parser = Parser::new(source, &mut nodes);
    let expr = parser.parse();
    println!("{}", expr.display(&nodes));
}
