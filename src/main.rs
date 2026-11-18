use crate::{ast::NodeArena, parser::Parser};

mod ast;
mod parser;
mod scanner;

fn main() {
    let source = r#"
        let x = 1 + 2 / 3 - 4 * 5 * (1 - 2);
        x = 12;
    "#;

    let mut arena = NodeArena::new();
    let mut parser = Parser::new(source, &mut arena);
    let nodes = parser.parse().unwrap();
    for node in nodes {
        println!("{}", node.display(&arena));
    }
}
