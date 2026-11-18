use crate::{ast::NodeArena, bind::Binder, parser::Parser};

mod ast;
mod bind;
mod parser;
mod scanner;

fn main() {
    let source = r#"
        let x = 1 + 2 / 3 - 4 * 5 * (1 - 2);
        x = 12;
        let y = if x == 12 || x == 2 && x > 1 { x + 1 } else if x > 12 { x + 2 } else { x + 3 };
        fn foo(a: Foo, b: Foo) -> Foo {
            if a > b {
                return 0;
            }
            a + b
        }
        foo(foo(1, 2), x);
    "#;

    let mut arena = NodeArena::new();
    let mut parser = Parser::new(source, &mut arena);
    let nodes = parser.parse().unwrap();

    let mut binder = Binder::new(&arena);
    binder.bind(&nodes);

    for node in nodes {
        println!("{}", node.display(&arena));
    }
    arena.dump(source);
}
