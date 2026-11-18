use crate::{ast::NodeArena, bind::Binder, parser::Parser, typechecker::TypeChecker};

mod arena;
mod ast;
mod bind;
mod interner;
mod parser;
mod scanner;
mod typechecker;
mod types;

fn main() {
    // let source = r#"
    //     let x = 1 + 2 / 3 - 4 * 5 * (1 - 2);
    //     x = 12;
    //     let y = if x == 12 || x == 2 && x > 1 { x + 1 } else if x > 12 { x + 2 } else { x + 3 };
    //     fn foo(a: number, b: number) -> number {
    //         if a > b {
    //             return 0;
    //         }
    //         a + b
    //     }
    //     foo(foo(1, 2), x);
    // "#;
    // let source = r#"
    //     fn foo(x: number) -> number { x + 1 }
    //     let x = 1;
    //     1 + foo(x);
    // "#;
    let source = r#"
        fn foo(x: number) {
            x
        }
    "#;

    let mut arena = NodeArena::new();
    let mut parser = Parser::new(source, &mut arena);
    let nodes = parser.parse().unwrap();

    // for node in nodes.iter() {
    //     println!("{}", node.display(&arena));
    // }
    // arena.dump(source);

    let binder = Binder::new(&arena);
    let bind_ctx = binder.bind(&nodes).unwrap();

    let mut checker = TypeChecker::new(source, &arena, bind_ctx);
    checker.check(&nodes);
}
