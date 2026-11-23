mod arena;
mod ast;
mod bind;
mod chunk;
mod codegen;
mod codegen2;
mod diagnostics;
mod heap;
mod interner;
mod ir;
mod ir_opt;
mod liveness;
mod parser;
mod register_chunk;
mod register_vm;
mod scanner;
mod typechecker;
mod types;
mod vm;

use std::time::Instant;

use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

use crate::{
    ast::AstNodes, bind::Binder, chunk::Code, codegen::Codegen, codegen2::Codegen2,
    diagnostics::Diagnostics, interner::StrInterner, ir::Lowering, parser::Parser,
    typechecker::TypeChecker, vm::VM,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Error {
    Parse,
    Bind,
    TypeCheck,
    Runtime,
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

fn lower_and_compile(
    source: &str,
    diagnostics: &mut Diagnostics,
) -> Result<register_chunk::Code, Error> {
    let ast = &mut AstNodes::new();
    let interner = &mut StrInterner::new();
    let mut parser = Parser::new(source, ast, interner, diagnostics);
    let nodes = parser.parse()?;

    let binder = Binder::new(ast, interner, diagnostics);
    let bind_ctx = binder.bind(&nodes)?;

    let checker = TypeChecker::new(ast, interner, diagnostics, bind_ctx);
    let mut ty_ctx = checker.check(&nodes)?;

    let lowering = Lowering::new(ast, &mut ty_ctx);
    let mut body = lowering.lower(&nodes);

    ir_opt::optimize(&mut body);

    let codegen = Codegen2::new(&body, &mut ty_ctx, interner);
    let code = codegen.generate();

    Ok(code)
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

    if cfg!(feature = "lower") {
        let start = Instant::now();
        let mut diagnostics = Diagnostics::new();
        match lower_and_compile(source, &mut diagnostics) {
            Ok(code) => {
                let mut vm = register_vm::VM::new(code);
                vm.run().expect("OK");
            }
            Err(_) => {
                diagnostics.print(source);
            }
        }
        println!("Took {:?}", start.elapsed());
    } else {
        let start = Instant::now();
        let mut diagnostics = Diagnostics::new();
        match compile(source, &mut diagnostics) {
            Ok(code) => {
                let mut vm = VM::new(code);
                vm.run()
            }
            Err(_) => {
                diagnostics.print(source);
            }
        }
        println!("Took {:?}", start.elapsed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Result<(), Error> {
        let mut diagnostics = Diagnostics::new();
        if cfg!(feature = "lower") {
            let code = lower_and_compile(source, &mut diagnostics);
            diagnostics.print(source);
            let mut vm = register_vm::VM::new(code?);
            vm.run()?;
        } else {
            let code = compile(source, &mut diagnostics);
            diagnostics.print(source);
            let mut vm = VM::new(code?);
            vm.run();
        }
        Ok(())
    }

    fn run_ok(source: &str) {
        run(source).expect("OK");
    }

    fn run_err(source: &str, error: Error) {
        assert_eq!(run(source).unwrap_err(), error)
    }

    #[test]
    fn test_simple() {
        let source = r#"
            let x = 1 + 2 / 3;
            print(x);
        "#;
        run_ok(source)
    }

    #[test]
    fn test_neg() {
        let source = r#"
            let x = -2;
            let y = -x;
            print(x);
            print(y);
        "#;
        run_ok(source)
    }

    #[test]
    fn test_simple_logic() {
        let source = r#"
            let x = 1 == 2 || 1 == 3;
            print(x);
        "#;
        run_ok(source)
    }

    #[test]
    fn test_simple_big() {
        let source = r#"
            let x: u64 = 1 + 2 / 3 - 4 * 5 * (1 - 2);
            x = 12;
            let y = if x == 12 || x == 2 && x > 1 { x + 1 } else if x > 12 { x + 2 } else { x + 3 };
            fn foo(a: u64, b: u64) -> u64 {
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
        run_err(source, Error::Bind);
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
        run_err(source, Error::TypeCheck);
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
                a: 10 + 3 + 4,
                b: "hello world",
                c: true == false != true
            };

            print(foo.b);
            print(foo.a);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_struct_multiple_fields_assign_in_different_order() {
        let source = r#"
            struct Foo {
                a: u64,
                b: string,
                c: bool
            }

            let foo = Foo {
                c: true,
                a: 10,
                b: "hello world"
            };

            print(foo.b);
            print(foo.c);
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
        run_err(source, Error::TypeCheck);
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
        run_err(source, Error::TypeCheck);
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
        run_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_bad_addition() {
        let source = r#"
            true + 1;
        "#;
        run_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_missing_declaration() {
        let source = r#"
            let a = b + 1;
        "#;
        run_err(source, Error::Bind);
    }

    #[test]
    fn test_duplicate_field_in_struct() {
        let source = r#"
            struct Foo {
                a: f64,
                a: f64
            }
        "#;
        run_err(source, Error::Parse);
    }

    #[test]
    fn test_duplicate_struct() {
        let source = r#"
            struct Foo {}
            struct Foo {}
        "#;
        run_err(source, Error::Bind);
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
    fn test_equivalent_while() {
        let source = r#"
            let i = 0;
            while i < 10 {
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
        run_err(source, Error::Bind);
    }

    #[test]
    fn test_for_missing_first_semicolon() {
        let source = r#"
            for (i = 0 i < 10; i = i + 1) {
            }
        "#;
        run_err(source, Error::Parse);
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
        run_err(source, Error::Bind);
    }

    #[test]
    fn test_struct_in_condition_parenthesized() {
        let source = r#"
            struct Foo {}
            if (Foo {}) {}
            while (Foo {}) {}
        "#;
        run_err(source, Error::TypeCheck);
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
        run_err(source, Error::TypeCheck);
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
    fn test_tuple_struct_repro_issue() {
        let source = r#"
            struct Bar(u64);
            struct Foo(u64, Bar);
            print(Bar(200).0);
            print(Foo(10, Bar(1000)).1.0);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_tuple_struct_constructor_as_value() {
        let source = r#"
            struct Foo(u64, string, i64);
            let c = Foo;
            let foo = c(10, "hello", -14);
            print(foo.0);
            print(foo.1);
            print(foo.2);
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
        run_err(source, Error::TypeCheck);
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
        run_err(source, Error::TypeCheck);
    }

    #[test]
    fn test_negative_u64_late_resolution() {
        let source = r#"
            let a = 10;
            print(-a);
            let b: u64 = a;
        "#;
        run_err(source, Error::TypeCheck);
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
    fn test_nested_arrays_simplest() {
        let source = r#"
            let a = [[1],[2],[3]];
            print(a[1][0] == 2);
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

    #[test]
    fn test_lower_unreachable_in_loop() {
        let source = r#"
            for(let i = 0; i < 10; i = i + 1) {
                print(i);
                continue;
                print(i + 1);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_lower_if_only() {
        let source = r#"
            let a = 1;
            if a == 1 {
                a = 2;
            }
            print(a);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_lower_if_else() {
        let source = r#"
            let a = 1;
            if a == 1 {
                a = 2;
            } else {
                a = 3;
            }
            print(a);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_lower_block_with_single_expr() {
        let source = r#"
            let a = { 10 };
            let b = {
                { 10 } + 12
            };
            print(a);
            print(b);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_lower_for_loop() {
        let source = r#"
            for (let i = 0; i < 10; i = i + 1) {
                print(i);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_lower_for_loop_with_continue() {
        let source = r#"
            for (let i = 0; i < 10; i = i + 1) {
                if i == 5 {
                    continue;
                }
                print(i);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_lower_for_loop_with_unconditional_continue() {
        let source = r#"
            for (let i = 0; i < 10; i = i + 1) {
                print(i);
                continue;
                print(20);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_lower_for_loop_with_break() {
        let source = r#"
            for (let i = 0; i < 10; i = i + 1) {
                if i == 5 {
                    break;
                }
                print(i);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_lower_for_loop_with_unconditional_break() {
        let source = r#"
            for (let i = 0; i < 10; i = i + 1) {
                print(i);
                break;
                print(20);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_while_loop() {
        let source = r#"
            let i = 0;
            while i < 10 {
                print(i);
                i = i + 1;
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_while_with_continue() {
        let source = r#"
            let i = 0;
            while i < 10 {
                i = i + 1;
                if i == 5 {
                    continue;
                }
                print(i);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_while_with_unconditional_continue() {
        let source = r#"
            let i = 0;
            while i < 10 {
                i = i + 1;
                continue;
                print(i);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_while_with_break() {
        let source = r#"
            let i = 0;
            while i < 10 {
                if i == 5 {
                    break;
                }
                print(i);
                i = i + 1;
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_while_with_unconditional_break() {
        let source = r#"
            let i = 0;
            while i < 10 {
                i = i + 1;
                break;
                print(i);
            }
        "#;
        run_ok(source);
    }

    #[test]
    fn test_dse_assign() {
        let source = r#"
            let i = 1;
            i = i + 1;
            i = 3;
            print(i);
        "#;
        run_ok(source);
    }

    #[test]
    fn test_dse_logic() {
        let source = r#"
            let x = 1 == 2 || 1 == 3;
        "#;
        run_ok(source)
    }

    #[test]
    fn test_dse_if() {
        let source = r#"
            if true {
                print(1);
            }
        "#;
        run_ok(source)
    }

    #[test]
    fn test_function_void() {
        let source = r#"
            fn foo(a: u64, b: u64, c: u64) {
                print(a + b + c);
            }

            let f = foo;
            let a = 10;
            let r = f(a + 2, a + 4, a + 6 + 7 + 8 + 9);
            print(r == ());
        "#;
        run_ok(source)
    }

    #[test]
    fn test_u64_div_by_zero() {
        let source = r#"
            let a: u64 = 1;
            print(a / 0);
        "#;
        run_err(source, Error::Runtime)
    }

    #[test]
    fn test_i64_div_by_zero() {
        let source = r#"
            let a: i64 = 1;
            print(a / 0);
        "#;
        run_err(source, Error::Runtime)
    }

    #[test]
    fn test_f64_div_by_zero() {
        let source = r#"
            let a: f64 = 1.0;
            print(a / 0.0);
        "#;
        run_ok(source)
    }

    #[test]
    fn test_array_out_of_bounds() {
        let source = r#"
            let a = [1];
            print(a[1]);
        "#;
        run_err(source, Error::Runtime)
    }

    #[test]
    fn test_explicit_return_at_the_end() {
        let source = r#"
            fn foo() -> u64 {
                return 10;
            }
            print(foo());
        "#;
        run_ok(source)
    }

    #[test]
    fn test_bad_implicit_return_after_return() {
        let source = r#"
            fn foo() -> u64 {
                return 10;
                "hello"
            }
            print(foo());
        "#;
        run_err(source, Error::TypeCheck)
    }

    #[test]
    fn test_explicit_return_expr_at_the_end() {
        let source = r#"
            fn foo() -> u64 {
                return 10
            }
            print(foo());
        "#;
        run_ok(source)
    }

    #[test]
    fn test_explicit_return_at_the_end_in_block() {
        let source = r#"
            fn foo() -> u64 {
                {
                    return 10;
                }
            }
            print(foo());
        "#;
        run_ok(source)
    }

    #[test]
    fn test_explicit_return_expr_at_the_end_in_block() {
        let source = r#"
            fn foo() -> u64 {
                {
                    return 10
                }
            }
            print(foo());
        "#;
        run_ok(source)
    }

    #[test]
    fn test_no_return() {
        let source = r#"
            fn foo() -> u64 {
            }
            print(foo());
        "#;
        run_err(source, Error::TypeCheck)
    }

    #[test]
    fn test_if_diverges() {
        let source = r#"
            fn foo() -> u64 {
                let x = if 1 == 2 {
                    return 0;
                } else {
                    50
                };
                x + 2
            }

            print(foo());
        "#;
        run_ok(source)
    }

    #[test]
    fn test_if_diverges_2() {
        let source = r#"
            fn foo() -> u64 {
                if 1 == 2 {
                    return 0;
                } else {
                    return 2;
                }
            }

            print(foo());
        "#;
        run_ok(source)
    }

    #[test]
    fn test_if_diverges_3() {
        let source = r#"
            fn foo() -> u64 {
                if 1 == 2 || return 1 {
                    return 0;
                } else {
                    return 2;
                }
            }

            print(foo());
        "#;
        run_ok(source)
    }

    #[test]
    fn test_if_diverges_4() {
        let source = r#"
            fn foo() -> u64 {
                if 1 == 2 || return 1 {
                    return 0;
                };
            }

            print(foo());
        "#;
        run_err(source, Error::TypeCheck)
    }
}
