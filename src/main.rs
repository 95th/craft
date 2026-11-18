use std::time::Instant;

use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

use crate::{
    ast::AstNodes, bind::Binder, chunk::Code, codegen::Codegen, diagnostics::Diagnostics,
    interner::StrInterner, ir::Lowering, parser::Parser, typechecker::TypeChecker, vm::VM,
};

mod arena;
mod ast;
mod bind;
mod chunk;
mod codegen;
mod diagnostics;
mod heap;
mod interner;
mod ir;
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

fn compile(source: &str, diagnostics: &mut Diagnostics) -> Result<Code, Error> {
    let ast = &mut AstNodes::new();
    let interner = &mut StrInterner::new();
    let mut parser = Parser::new(source, ast, interner, diagnostics);
    let nodes = parser.parse()?;

    let binder = Binder::new(ast, interner, diagnostics);
    let bind_ctx = binder.bind(&nodes)?;

    let checker = TypeChecker::new(ast, interner, diagnostics, bind_ctx);
    let ty_ctx = checker.check(&nodes)?;

    let codegen = Codegen::new(ast, interner, ty_ctx);
    let code = codegen.generate(&nodes);
    Ok(code)
}

fn lower(source: &str, diagnostics: &mut Diagnostics) -> Result<(), Error> {
    let ast = &mut AstNodes::new();
    let interner = &mut StrInterner::new();
    let mut parser = Parser::new(source, ast, interner, diagnostics);
    let nodes = parser.parse()?;

    let binder = Binder::new(ast, interner, diagnostics);
    let bind_ctx = binder.bind(&nodes)?;

    let checker = TypeChecker::new(ast, interner, diagnostics, bind_ctx);
    let ty_ctx = checker.check(&nodes)?;

    let lowering = Lowering::new(ast, ty_ctx);
    lowering.lower(&nodes);

    Ok(())
}

fn main() {
    let source = r#"
        fn fib(n: u64) -> u64 {
            if n < 2 {
                return n;
            }
            fib(n - 1) + fib(n - 2)
        }

        print(fib(35));
    "#;

    let start = Instant::now();
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
    println!("Took {:?}", start.elapsed());
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOWER: bool = true;

    fn run_ok(source: &str) {
        let mut diagnostics = Diagnostics::new();

        if LOWER {
            let result = lower(source, &mut diagnostics);
            diagnostics.print(source);
            result.expect("ok");
        } else {
            let result = compile(source, &mut diagnostics);
            diagnostics.print(source);
            let code = result.expect("ok");
            let mut vm = VM::new(code);
            vm.run();
        }
    }

    fn check_err(source: &str, error: Error) {
        let mut diagnostics = Diagnostics::new();
        let result = compile(source, &mut diagnostics);
        diagnostics.print(source);
        assert_eq!(result.unwrap_err(), error);
    }

    #[test]
    fn test_simple() {
        let source = r#"
            let x = 1 + 2 / 3 - 4 * 5 * (1 - 2);
            x = 12;
            let y = if x == 12 || x == 2 && x > 1 { x + 1 } else if x > 12 { x + 2 } else { x + 3 };
            fn foo(a: i64, b: i64) -> i64 {
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
            fn foo(x: f64) -> f64 { x + 1.0 }
            let x = 1.0;
            1.0 + foo(x);
        "#;
        run_ok(source)
    }

    #[test]
    fn test_fn_duplicate_param() {
        let source = r#"
            fn foo(n: f64, n: f64) -> f64 { n + 1 }
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
            fn foo(a: f64, b: f64) -> f64 { 1.0 }
            let f: fn(f64, f64) -> f64 = foo;
            let a: (f64, (f64, f64)) = (1.0, (2.0, 3.0));
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
            fn foo(a: f64, b: f64) -> f64 { 1.0 }
            let f: fn(f64, f64) -> f64 = foo;
            let a: (f64, (f64)) = (1.0, (2.0, 3.0));
            foo(a.0, (a.1).0);
        "#;
        check_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_struct() {
        let source = r#"
            struct Foo {
                a: f64
            }
            fn bar(f: Foo) -> f64 {
                f.a + 10.0
            }
            let foo = Foo {
                a: 10.0
            };
            bar(foo);
        "#;
        run_ok(source)
    }

    #[test]
    fn test_struct_multiple_fields() {
        let source = r#"
            struct Foo {
                a: u64,
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
                a: i64,
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
                a: f64
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
                a: f64,
                b: f64
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
                a: f64
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
                a: f64,
                a: f64
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
                f: fn(f64) -> f64
            }
            fn foo(n: f64) -> f64 { n + 1.0 }
            let f = Foo { f: foo };
            f.f(10.0);
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
            let a = 10.0;
            a = a + 1.0 - 4.0 / 2.0 * 1.2;
            print(a);
            a = {
                let b = 20.0;
                b + 1.0
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
            fn foo(n: f64) {
                print(n);
            }

            foo(10.0);
            foo(20.0);
            foo(30.0);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_recursion() {
        let source = r#"
            fn fib(n: u64) -> u64 {
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

            let f: fn(f64, bool) -> (f64, bool) = foo;
            print(f(10.0, true).0);
            print(f(20.0, false).0);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_nested_generic_functions() {
        let source = r#"
            fn foo<T>(a: T) -> T {
                fn foo<T>(a: T, b: T) -> (T, T) {
                    (a, b)
                }

                foo(a, a).1
            }
            print(foo(10));
        "#;
        run_ok(source);
    }

    #[test]
    fn test_generic_function_with_generic_args() {
        let source = r#"
            fn foo<T, U>(a: T, b: fn(T) -> string) -> string {
                b(a)
            }

            fn bar<T>(t: T) -> string {
                "Hello world"
            }

            print(foo(10, bar));
        "#;
        run_ok(source);
    }

    #[test]
    fn test_generic_unify_with_regular() {
        let source = r#"
            fn foo<T>(a: T) -> T {
                a
            }
            fn bar(n: f64) -> f64 {
                n + 1.0
            }
            let x = if true { foo } else { bar };
            print(x(10.0));
        "#;
        run_ok(source);
    }

    #[test]
    fn test_generic_unify_with_regular_fail() {
        let source = r#"
            fn foo<T>(a: T) -> T {
                a
            }
            fn bar(n: f64) -> f64 {
                n + 1.0
            }
            let x = if true { foo } else { bar };
            print(x(10));
        "#;
        check_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_recursive_types() {
        let source = r#"
            struct Foo {
                a: Bar
            }
            struct Bar {
                a: Foo
            }
            struct Baz {
                a: Baz
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_tuple_struct() {
        let source = r#"
            struct Bar(f64);
            struct Foo(f64, string, Bar);
            let bar = Bar(10.0);
            let foo = Foo(20.0, "hello", bar);
            print(foo.0);
            print(foo.1);
            print(foo.2);
            print(foo.2.0);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_empty_tuple_struct() {
        let source = r#"
            struct Foo();
            let foo = Foo();
            print(foo);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_array() {
        let source = r#"
            let a = [1.0, 2.0, 3.0];
            print(a);

            let b: [f64] = a;
            print(b[2]);

            fn foo(n: f64) -> [f64] {
                [n; 20]
            }
            print(foo(10.0)[1]);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_integer() {
        let source = r#"
            let a = 1;
            let b = [1, 2, 3];
            print(b[a]);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_negative_u64() {
        let source = r#"
            let a: u64 = 10;
            print(-a);
        "#;
        check_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_negative_integer_default() {
        let source = r#"
            let a = -1;
            print(a);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_assign_negative_integer_to_u64() {
        let source = r#"
            let a: u64 = -1;
        "#;
        check_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_negative_u64_late_resolution() {
        let source = r#"
            let a = 10;
            print(-a);
            let b: u64 = a;
        "#;
        check_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_repro_codegen_number_where_int_type_expected() {
        let source = r#"
            fn foo(n: u64) -> [u64] {
                [n; 20]
            }
            print(foo(10000000012028230)[1]);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_fn_equality() {
        let source = r#"
            fn foo() {}
            fn bar() {}

            print(foo == foo);
            print(foo == bar);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_obj_equality() {
        let source = r#"
            struct Foo {}
            let a = Foo {};
            let b = Foo {};

            print(a == a);
            print(a == b);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_tuple_equality() {
        let source = r#"
            let a = (1, 2, 3);
            let b = (1, 2, 3);

            print(a == a);
            print(a == b);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_nested_arrays_simple() {
        let source = r#"
            let a = [[1,2,3],[4, 5, 6]];
            print(a[1][2] == 6);

            let b = [[1; 2]; 3];
            print(b[1][1] == 1);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_nested_arrays_using_generic_method() {
        let source = r#"
            fn ten_of<T>(value: T) -> [T] {
                [value; 10]
            }

            let a = ten_of(ten_of(ten_of(5)));
            print(a[0][0][0]);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_generic_printer() {
        let source = r#"
            fn foo<T>(n: T) {
                print(n);
            }

            foo(1);
            foo("Hello");
            foo(30.0);
            foo(foo);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_generic_function_value() {
        let source = r#"
            fn foo<T>(n: T) {
                print(n);
            }

            let f = foo;
        "#;
        run_ok(source);
    }

    #[test]
    fn test_duplicate_string_string_table() {
        let source = r#"
            let a = "hello world";
            let b = "hello world";
            let c = "hello world";
            let d = "hello world";
            print(a);
            print(b);
            print(c);
            print(d);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_long_locals() {
        let mut source = String::new();
        for i in 0..1000 {
            source.push_str(&format!("let a{i} = {i};"))
        }
        for i in 0..1000 {
            source.push_str(&format!("a{i} = a{i} + 1;"))
        }
        for i in 0..1000 {
            source.push_str(&format!("print(a{i});"))
        }
        run_ok(&source);
    }

    #[test]
    fn test_locals_in_block() {
        let source = r#"
            let a = 1;
            {
                let b = 2;
                let c = 3;
            }
            let d = 4;
            print(d);

            let x = {
                let y = {
                    let z = 10;
                    z * 2
                };
                y + 5
            };
            print(x);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_function_implicit_return() {
        let source = r#"
            fn foo() -> u64 {
                let a = 10;
                let b = 20;
                a + b
            }

            print(foo());
        "#;
        run_ok(source);
    }

    #[test]
    fn test_lower_basic() {
        let source = r#"
            let a = 1 + 2 - 3;
            if a < 1 {
                a = 20;
            }
            print(a + 10);
        "#;
        run_ok(source);
    }
}
