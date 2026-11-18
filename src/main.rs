use crate::{
    ast::AstNodes, bind::Binder, chunk::Code, codegen::Codegen, diagnostics::Diagnostics,
    parser::Parser, typechecker::TypeChecker, vm::VM,
};

mod arena;
mod ast;
mod bind;
mod chunk;
mod codegen;
mod diagnostics;
mod interner;
mod parser;
mod scanner;
mod typechecker;
mod types;
mod vm;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Error {
    Parse,
    Bind,
    TypeCheck,
}

#[inline(always)]
fn stack_safe<T>(f: impl FnOnce() -> T) -> T {
    stacker::maybe_grow(32 * 1024, 1024 * 1024, f)
}

fn compile(source: &str, diagnostics: &mut Diagnostics) -> Result<Code, Error> {
    let mut ast = AstNodes::new();
    let mut parser = Parser::new(source, &mut ast, diagnostics);
    let nodes = parser.parse()?;

    let binder = Binder::new(&ast, diagnostics);
    let bind_ctx = binder.bind(&nodes)?;

    let checker = TypeChecker::new(&ast, diagnostics, bind_ctx);
    let ty_ctx = checker.check(&nodes)?;

    let codegen = Codegen::new(&ast, ty_ctx);
    let code = codegen.generate(&nodes);
    Ok(code)
}

fn main() {
    let source = r#"
        let a = 10;
        while a > 0 {
            print_number(a);
            a = a - 1;
        }
        print_number(100);
    "#;

    let mut diagnostics = Diagnostics::new();
    match compile(source, &mut diagnostics) {
        Ok(code) => {
            let mut vm = VM::new(code);
            vm.run();
        }
        Err(_) => {
            diagnostics.print(source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_ok(source: &str) {
        let mut diagnostics = Diagnostics::new();
        let result = compile(source, &mut diagnostics);
        diagnostics.print(source);
        result.expect("ok");
    }

    fn check_err(source: &str, error: Error) {
        let mut diagnostics = Diagnostics::new();
        let result = compile(source, &mut diagnostics);
        diagnostics.print(source);
        assert_eq!(result.unwrap_err(), error);
    }

    fn run_ok(source: &str) {
        let mut diagnostics = Diagnostics::new();
        let result = compile(source, &mut diagnostics);
        diagnostics.print(source);
        let code = result.expect("ok");
        let mut vm = VM::new(code);
        vm.run();
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
            foo(a.0, a.1.0);
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
            let foo = Foo {
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
            let foo = Foo {
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
                a: number,
                b: number
            }
            let foo = Foo {
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
            let foo = Foo {
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

    #[test]
    fn test_duplicate_struct() {
        let source = r#"
            struct Foo {}
            struct Foo {}
        "#;
        check_err(source, Error::Bind);
    }

    #[test]
    fn test_unit() {
        let source = r#"
            let x: () = ();
        "#;
        check_ok(source);
    }

    #[test]
    fn huge_nesting() {
        let mut s = String::new();
        for _ in 0..1_000_000 {
            s.push_str("fn foo(){");
        }
        s.push_str("1 + 1;");
        for _ in 0..1_000_000 {
            s.push('}');
        }
        check_ok(&s);
    }

    #[test]
    fn test_for() {
        let source = r#"
            for (let i = 0; i < 10; i = i + 1) {
                if i == 1 {
                    break;
                }
                if i == 2 {
                    continue;
                }
                i;
            }
        "#;
        check_ok(source);
    }

    #[test]
    fn test_for_init_var_outside() {
        let source = r#"
            for (let i = 0; i < 10; i = i + 1) {
            }
            i;
        "#;
        check_err(source, Error::Bind);
    }

    #[test]
    fn test_for_missing_first_semicolon() {
        let source = r#"
            for (i = 0 i < 10; i = i + 1) {
            }
        "#;
        check_err(source, Error::Parse);
    }

    #[test]
    fn test_struct_field_function() {
        let source = r#"
            struct Foo {
                f: fn(number) -> number
            }
            fn foo(n: number) -> number { n + 1 }
            let f = Foo { f: foo };
            f.f(10);
        "#;
        check_ok(source);
    }

    #[test]
    fn test_fn_duplicate_param() {
        let source = r#"
            fn foo(n: number, n: number) -> number { n + 1 }
        "#;
        check_err(source, Error::Bind);
    }

    #[test]
    fn test_identifier_in_condition() {
        let source = r#"
            let i = 1 > 2;
            if i {
            }
            while i {
            }
        "#;
        check_ok(source);
    }

    #[test]
    fn test_struct_in_condition() {
        let source = r#"
            struct Foo {}
            if Foo {}
            while Foo {}
        "#;
        check_err(source, Error::Bind);
    }

    #[test]
    fn test_struct_in_condition_parenthesized() {
        let source = r#"
            struct Foo {}
            if (Foo {}) {}
            while (Foo {}) {}
        "#;
        check_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_assign_block() {
        let source = r#"
            let a = 10;
            a = a + 1 - 4 / 2 * 1.2;
            print_number(a);
            a = {
                let b = 20;
                b + 1
            };
            print_number(a);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_if() {
        let source = r#"
            let a = 10;
            if a > 10 {
                print_number(a);
            } else {
                print_number(5);
            }
            print_number(4);
        "#;
        run_ok(source);
    }
}
