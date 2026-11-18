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

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Error {
        Parse,
        Bind,
        TypeCheck,
    }

    fn check(source: &str) -> Result<(), Error> {
        let mut arena = NodeArena::new();
        let mut parser = Parser::new(source, &mut arena);
        let nodes = parser.parse().ok_or(Error::Parse)?;

        let binder = Binder::new(&arena);
        let bind_ctx = binder.bind(&nodes).ok_or(Error::Bind)?;

        let mut checker = TypeChecker::new(source, &arena, bind_ctx);
        checker.check(&nodes).ok_or(Error::TypeCheck)?;
        Ok(())
    }

    fn check_ok(source: &str) {
        check(source).unwrap();
    }

    fn check_err(source: &str, error: Error) {
        assert_eq!(check(source).unwrap_err(), error);
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
        check_ok(source)
    }

    #[test]
    fn test_fn_return() {
        let source = r#"
            fn foo(x: number) -> number { x + 1 }
            let x = 1;
            1 + foo(x);
        "#;
        check_ok(source)
    }

    #[test]
    fn test_tuples() {
        let source = r#"
            fn foo(a: number, b: number) -> number { 1 }
            let f: fn(number, number) -> number = foo;
            let a: (number, (number, number)) = (1, (2, 3));
            foo(a.0, (a.1).0);
        "#;
        check_ok(source)
    }

    #[test]
    fn test_tuples_fail_mismatch_inner() {
        let source = r#"
            fn foo(a: number, b: number) -> number { 1 }
            let f: fn(number, number) -> number = foo;
            let a: (number, (number)) = (1, (2, 3));
            foo(a.0, (a.1).0);
        "#;
        check_err(source, Error::TypeCheck);
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
        check_ok(source)
    }

    #[test]
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
        check_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_struct_missing_field() {
        let source = r#"
            struct Foo {
                a: number
            }
            let foo = struct Foo {
            };
        "#;
        check_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_struct_wrong_type() {
        let source = r#"
            struct Foo {
                a: number
            }
            let foo = struct Foo {
                a: true
            };
        "#;
        check_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_bad_addition() {
        let source = r#"
            true + 1;
        "#;
        check_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_missing_declaration() {
        let source = r#"
            let a = b + 1;
        "#;
        check_err(source, Error::Bind);
    }

    #[test]
    fn test_duplicate_field_in_struct() {
        let source = r#"
            struct Foo {
                a: number,
                a: number
            }
        "#;
        check_err(source, Error::Parse);
    }
}
