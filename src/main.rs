use std::time::Instant;

use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

use crate::{
    ast::AstNodes, bind::Binder, chunk::Code, codegen::Codegen, diagnostics::Diagnostics,
    heap::Heap, parser::Parser, typechecker::TypeChecker, vm::VM,
};

mod arena;
mod ast;
mod bind;
mod chunk;
mod codegen;
mod diagnostics;
mod heap;
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

type FxIndexMap<K, V> = IndexMap<K, V, FxBuildHasher>;

#[inline(always)]
fn stack_safe<T>(f: impl FnOnce() -> T) -> T {
    stacker::maybe_grow(32 * 1024, 1024 * 1024, f)
}

fn compile(source: &str, heap: &mut Heap, diagnostics: &mut Diagnostics) -> Result<Code, Error> {
    let mut ast = AstNodes::new();
    let mut parser = Parser::new(source, &mut ast, diagnostics);
    let nodes = parser.parse()?;

    let binder = Binder::new(&ast, diagnostics);
    let bind_ctx = binder.bind(&nodes)?;

    let checker = TypeChecker::new(&ast, diagnostics, bind_ctx);
    let ty_ctx = checker.check(&nodes)?;

    let codegen = Codegen::new(&ast, heap, ty_ctx);
    let code = codegen.generate(&nodes);
    Ok(code)
}

fn main() {
    let source = r#"
        fn fib(n: number) -> number {
            if n < 2 {
                return n;
            }
            fib(n - 1) + fib(n - 2)
        }

        print(fib(35));
    "#;

    let start = Instant::now();
    let mut heap = Heap::new();
    let mut diagnostics = Diagnostics::new();
    match compile(source, &mut heap, &mut diagnostics) {
        Ok(code) => {
            let mut vm = VM::new(code, heap);
            vm.run();
        }
        Err(_) => {
            diagnostics.print(source);
        }
    }
    println!("Took {:?}", start.elapsed());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_ok(source: &str) {
        let mut heap = Heap::new();
        let mut diagnostics = Diagnostics::new();
        let result = compile(source, &mut heap, &mut diagnostics);
        diagnostics.print(source);
        let code = result.expect("ok");
        let mut vm = VM::new(code, heap);
        vm.run();
    }

    fn check_err(source: &str, error: Error) {
        let mut heap = Heap::new();
        let mut diagnostics = Diagnostics::new();
        let result = compile(source, &mut heap, &mut diagnostics);
        diagnostics.print(source);
        assert_eq!(result.unwrap_err(), error);
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
        run_ok(source)
    }

    #[test]
    fn test_and_or() {
        let source = r#"
            let x = 1;
            if x == 12 && x > 1 { print(x + 1); }
            if x == 1 || x > 1 {  print(x + 2); }
            if x == 1 && x > 1 {  print(x + 3); }
            if x == 12 || x > 1 { print(x + 4); }
        "#;
        run_ok(source)
    }

    #[test]
    fn test_fn_return() {
        let source = r#"
            fn foo(x: number) -> number { x + 1 }
            let x = 1;
            1 + foo(x);
        "#;
        run_ok(source)
    }

    #[test]
    fn test_fn_duplicate_param() {
        let source = r#"
            fn foo(n: number, n: number) -> number { n + 1 }
        "#;
        check_err(source, Error::Bind);
    }

    #[test]
    fn test_tuple() {
        let source = r#"
            let s = (1, 2, ("hello world", 20));
            print(s.2.0);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_tuple_2() {
        let source = r#"
            fn foo(a: number, b: number) -> number { 1 }
            let f: fn(number, number) -> number = foo;
            let a: (number, (number, number)) = (1, (2, 3));
            foo(a.0, a.1.0);
        "#;
        run_ok(source)
    }

    #[test]
    fn test_tuple_assign_fields() {
        let source = r#"
            let foo = (10, "hello world", true);
            print(foo.0);
            print(foo.1);

            foo.0 = 20;
            foo.1 = "bye world";
            print(foo.0);
            print(foo.1);
        "#;
        run_ok(source);
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
        run_ok(source)
    }

    #[test]
    fn test_struct_multiple_fields() {
        let source = r#"
            struct Foo {
                a: number,
                b: string,
                c: bool
            }

            let foo = Foo {
                a: 10,
                b: "hello world",
                c: true
            };

            print(foo.b);
            print(foo.a);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_struct_assign_fields() {
        let source = r#"
            struct Foo {
                a: number,
                b: string,
                c: bool
            }

            let foo = Foo {
                a: 10,
                b: "hello world",
                c: true
            };
            print(foo.a);
            print(foo.b);

            foo.a = 20;
            foo.b = "bye world";
            print(foo.a);
            print(foo.b);
        "#;
        run_ok(source);
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
        run_ok(source);
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
        run_ok(&s);
    }

    #[test]
    fn test_for() {
        let source = r#"
            for (let i = 0; i < 10; i = i + 1) {
                print(i);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_for_without_init() {
        let source = r#"
            let i = 0;
            for (; i < 10; i = i + 1) {
                print(i);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_for_without_init_and_incr() {
        let source = r#"
            let i = 0;
            for (; i < 10;) {
                print(i);
                i = i + 1;
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_for_no_clauses() {
        let source = r#"
            let i = 0;
            for (;;) {
                print(i);
                i = i + 1;
                if i >= 10 {
                    break;
                }
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_for_break_and_continue() {
        let source = r#"
            for (let i = 0; i < 10; i = i + 1) {
                if i == 1 {
                    break;
                }
                if i == 2 {
                    continue;
                }
                print(i);
            }
        "#;
        run_ok(source);
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
        run_ok(source);
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
        run_ok(source);
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
            print(a);
            a = {
                let b = 20;
                b + 1
            };
            print(a);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_if() {
        let source = r#"
            let a = 10;
            if a > 10 {
                print(a);
            } else {
                print(5);
            }
            print(4);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_while() {
        let source = r#"
            let a = 10;
            while a > 0 {
                print(a);
                a = a - 1;
            }
            print(100);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_while_break() {
        let source = r#"
            let i = 0;
            while true {
                print(i);
                i = i + 1;
                if i == 10 {
                    break;
                }
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_while_continue() {
        let source = r#"
            let i = 0;
            while true {
                print(i);
                i = i + 1;
                if i < 10 {
                    continue;
                }
                break;
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_functions() {
        let source = r#"
            fn foo(n: number) {
                print(n);
            }

            foo(10);
            foo(20);
            foo(30);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_recursion() {
        let source = r#"
            fn fib(n: number) -> number {
                if n < 2 {
                    return n;
                }
                fib(n - 1) + fib(n - 2)
            }

            print(fib(35));
        "#;
        run_ok(source);
    }

    #[test]
    fn test_string() {
        let source = r#"
            let s = "hello world";
            print(s);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_generic_function() {
        let source = r#"
            fn foo<T, U>(a: T, b: U) -> (T, U) {
                (a, b)
            }

            let f: fn<T>(number, T) -> (number, T) = foo;
            print(f(10, true).0);
            print(f(20, false).0);
        "#;
        run_ok(source);
    }
}
