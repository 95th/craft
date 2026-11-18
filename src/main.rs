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
    let source = r#"
        struct Foo {
            a: number
        }

        fn bar(f: Foo) -> number {
            f.a + 10
        }

        let foo = struct Foo {
            a: 10
        };
        bar(foo);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn helper(source: &str) {
        let mut arena = NodeArena::new();
        let mut parser = Parser::new(source, &mut arena);
        let nodes = parser.parse().unwrap();

        let binder = Binder::new(&arena);
        let bind_ctx = binder.bind(&nodes).unwrap();

        let mut checker = TypeChecker::new(source, &arena, bind_ctx);
        checker.check(&nodes).unwrap();
    }

    #[test]
    fn test_simple() {
        let source = r#"
            let x = 1 + 2 / 3 - 4 * 5 * (1 - 2);
            x = 12;
            let y = if x == 12 || x == 2 && x > 1 { x + 1 } else if x > 12 { x + 2 } else { x + 3 };
            fn foo(a: number, b: number) -> number {
                if a > b {
                    return 0;
                }
                a + b
            }
            foo(foo(1, 2), x);
        "#;
        helper(source);
    }

    #[test]
    fn test_fn_return() {
        let source = r#"
            fn foo(x: number) -> number { x + 1 }
            let x = 1;
            1 + foo(x);
        "#;
        helper(source);
    }

    #[test]
    fn test_tuples() {
        let source = r#"
            fn foo(a: number, b: number) -> number { 1 }
            let f: fn(number, number) -> number = foo;
            let a: (number, (number, number)) = (1, (2, 3));
            foo(a.0, (a.1).0);
        "#;
        helper(source);
    }

    #[test]
    #[should_panic]
    fn test_tuples_fail_mismatch_inner() {
        let source = r#"
            fn foo(a: number, b: number) -> number { 1 }
            let f: fn(number, number) -> number = foo;
            let a: (number, (number)) = (1, (2, 3));
            foo(a.0, (a.1).0);
        "#;
        helper(source);
    }

    #[test]
    fn test_struct() {
        let source = r#"
            struct Foo {
                a: number
            }
            fn bar(f: Foo) -> number {
                f.a + 10
            }
            let foo = struct Foo {
                a: 10
            };
            bar(foo);
        "#;
        helper(source);
    }

    #[test]
    #[should_panic]
    fn test_struct_non_existent_field() {
        let source = r#"
            struct Foo {
                a: number
            }
            let foo = struct Foo {
                a: 10,
                b: 20
            };
        "#;
        helper(source);
    }

    #[test]
    #[should_panic]
    fn test_struct_missing_field() {
        let source = r#"
            struct Foo {
                a: number
            }
            let foo = struct Foo {
            };
        "#;
        helper(source);
    }

    #[test]
    #[should_panic]
    fn test_struct_wrong_type() {
        let source = r#"
            struct Foo {
                a: number
            }
            let foo = struct Foo {
                a: true
            };
        "#;
        helper(source);
    }
}
